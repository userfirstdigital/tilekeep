//! Portable, bounded snapshot data. Launch only recorded executables, never shell commands.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppWindow {
    pub token: String,
    pub app: String,
    pub title: String,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub executable: Option<PathBuf>,
    pub rect: crate::geometry::Rect,
    #[serde(default)]
    pub floating: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Node {
    Leaf {
        windows: Vec<String>,
        #[serde(default)]
        active: usize,
    },
    Split {
        axis: String,
        ratio: f64,
        #[serde(default, rename = "preserveSpace")]
        preserve_space: bool,
        first: Box<Node>,
        second: Box<Node>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Monitor {
    pub name: String,
    pub area: crate::geometry::Rect,
    pub root: Node,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: u32,
    #[serde(default)]
    pub name: String,
    pub gap: i32,
    pub monitors: Vec<Monitor>,
    pub windows: Vec<AppWindow>,
}
fn directory() -> Result<PathBuf, String> {
    Ok(crate::settings::config_dir()?.join("snapshots"))
}
fn path_in(dir: &std::path::Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.len() > 80 || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err("Invalid snapshot ID".into());
    }
    Ok(dir.join(format!("{id}.json")))
}
fn path(id: &str) -> Result<PathBuf, String> {
    path_in(&directory()?, id)
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
}
impl Entry {
    pub fn label(&self) -> String {
        let date = self
            .id
            .split('-')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown date".into());
        format!("{} — {date}", if self.name.is_empty() { "Snapshot" } else { &self.name })
    }
}
pub fn list() -> Vec<Entry> {
    directory().map(|d| list_in(&d)).unwrap_or_default()
}
fn list_in(dir: &std::path::Path) -> Vec<Entry> {
    let mut entries = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && e.path().extension().is_some_and(|s| s == "json"))
        .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter_map(|id| load_path(&path_in(dir, &id).ok()?).ok().map(|s| Entry { id, name: s.name }))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.id.cmp(&a.id));
    entries
}
pub(crate) fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("Use a name of 1–80 characters, without line breaks or control characters".into());
    }
    Ok(name.into())
}
pub fn rename(id: &str, name: &str) -> Result<(), String> {
    rename_in(&directory()?, id, name)
}
fn rename_in(dir: &std::path::Path, id: &str, name: &str) -> Result<(), String> {
    let name = valid_name(name)?;
    let p = path_in(dir, id)?;
    load_path(&p)?;
    // Preserve unknown fields and the exact saved geometry when editing metadata.
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    value["name"] = name.into();
    crate::settings::atomic_write(&p, &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?)
}
pub fn delete(id: &str, settings_path: &std::path::Path) -> Result<(), String> {
    delete_in(&directory()?, id, settings_path)
}
fn delete_in(dir: &std::path::Path, id: &str, settings_path: &std::path::Path) -> Result<(), String> {
    let p = path_in(dir, id)?;
    load_path(&p)?;
    let mut settings = crate::settings::load(settings_path)?;
    // Archive first. If updating startup settings fails, put the snapshot back.
    let deleted = dir.join("deleted");
    std::fs::create_dir_all(&deleted).map_err(|e| e.to_string())?;
    let dest = path_in(&deleted, id)?;
    if dest.exists() {
        return Err("A deleted snapshot with this ID already exists".into());
    }
    std::fs::rename(&p, &dest).map_err(|e| e.to_string())?;
    if settings.startup_snapshot.as_deref() == Some(id) {
        settings.startup_snapshot = None;
        if let Err(e) = crate::settings::save(settings_path, &settings) {
            std::fs::rename(&dest, &p)
                .map_err(|rollback| format!("{e}; snapshot remains in deleted folder: {rollback}"))?;
            return Err(e);
        }
    }
    Ok(())
}
pub fn validate(s: &Snapshot) -> Result<(), String> {
    fn node(n: &Node, depth: usize, tokens: &HashSet<&str>, used: &mut HashSet<String>) -> bool {
        if depth > 64 {
            return false;
        }
        match n {
            Node::Leaf { windows, active } => {
                windows.len() <= 256
                    && (windows.is_empty() || *active < windows.len())
                    && windows.iter().all(|w| tokens.contains(w.as_str()) && used.insert(w.clone()))
            }
            Node::Split { axis, ratio, first, second, .. } => {
                matches!(axis.as_str(), "x" | "y")
                    && ratio.is_finite()
                    && (0.0..=1.0).contains(ratio)
                    && node(first, depth + 1, tokens, used)
                    && node(second, depth + 1, tokens, used)
            }
        }
    }
    let tokens = s.windows.iter().map(|w| w.token.as_str()).collect::<HashSet<_>>();
    let mut used = HashSet::new();
    if s.schema != 1
        || (!s.name.is_empty() && valid_name(&s.name).is_err())
        || !(0..=64).contains(&s.gap)
        || s.windows.len() > 256
        || tokens.len() != s.windows.len()
        || s.monitors.is_empty()
        || s.monitors.len() > 32
        || !s.monitors.iter().all(|m| node(&m.root, 0, &tokens, &mut used))
    {
        return Err("Invalid or unsupported snapshot".into());
    }
    for r in s.windows.iter().map(|w| w.rect).chain(s.monitors.iter().map(|m| m.area)) {
        if r.x.unsigned_abs() > 1_000_000
            || r.y.unsigned_abs() > 1_000_000
            || !(1..=100_000).contains(&r.w)
            || !(1..=100_000).contains(&r.h)
        {
            return Err("Invalid snapshot geometry".into());
        }
    }
    Ok(())
}
pub fn save(snapshot: Snapshot) -> Result<String, String> {
    validate(&snapshot)?;
    #[cfg(target_os = "linux")]
    let snapshot = {
        let mut snapshot = snapshot;
        for w in &mut snapshot.windows {
            w.executable = std::fs::read_link(format!("/proc/{}/exe", w.pid)).ok();
        }
        snapshot
    };
    let id = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis().to_string();
    crate::settings::atomic_write(&path(&id)?, &serde_json::to_vec_pretty(&snapshot).map_err(|e| e.to_string())?)?;
    Ok(id)
}
pub fn load(id: &str) -> Result<Snapshot, String> {
    let path = path(id)?;
    load_path(&path)
}
fn load_path(path: &std::path::Path) -> Result<Snapshot, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 {
        return Err("Snapshot exceeds size limit".into());
    }
    let snapshot =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    validate(&snapshot)?;
    Ok(snapshot)
}
pub fn matching(saved: &[AppWindow], live: &[AppWindow]) -> HashMap<String, String> {
    let mut used = HashSet::new();
    let mut result = HashMap::new();
    for w in saved {
        let candidate = live
            .iter()
            .filter(|l| !used.contains(&l.token) && l.app == w.app)
            .min_by_key(|l| (l.token != w.token, l.title != w.title));
        if let Some(l) = candidate {
            used.insert(l.token.clone());
            result.insert(w.token.clone(), l.token.clone());
        }
    }
    result
}
pub fn launch_missing(snapshot: &Snapshot, live: &[AppWindow]) -> Vec<String> {
    let matches = matching(&snapshot.windows, live);
    let mut launched = HashSet::new();
    let mut errors = Vec::new();
    for w in &snapshot.windows {
        if matches.contains_key(&w.token) || !launched.insert(w.app.clone()) {
            continue;
        }
        let result = (|| {
            let exe = w.executable.as_ref().ok_or("No executable recorded")?;
            if !exe.is_absolute() || !exe.is_file() {
                return Err("Recorded executable is missing".to_string());
            }
            if std::env::current_exe().ok().as_ref() == Some(exe) {
                return Err("Will not launch a second Tilekeep".into());
            }
            std::process::Command::new(exe).spawn().map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(e) = result {
            errors.push(format!("{}: {e}", w.app));
        }
    }
    errors
}
pub struct Restore {
    pub snapshot: Snapshot,
    start: std::time::Instant,
    last: Option<usize>,
    pub done: bool,
}
impl Restore {
    pub fn new(id: &str, live: &[AppWindow]) -> Result<Self, String> {
        let snapshot = load(id)?;
        for e in launch_missing(&snapshot, live) {
            log::warn!("Snapshot: {e}");
        }
        Ok(Self { snapshot, start: std::time::Instant::now(), last: None, done: false })
    }
    pub fn poll(&mut self, desktop: &mut crate::engine::Desktop, live: &[AppWindow]) -> Result<bool, String> {
        let count = matching(&self.snapshot.windows, live).len();
        let changed = self.last != Some(count);
        if changed {
            desktop.restore_snapshot(&self.snapshot, live)?;
            self.last = Some(count);
        }
        self.done = count == self.snapshot.windows.len() || self.start.elapsed().as_secs() > 30;
        if let Some(c) = crate::control::current().filter(|_| changed || self.done) {
            c.snapshot_restored(self.snapshot.gap, self.snapshot.windows.len() - count);
        }
        Ok(changed)
    }
}

#[cfg(target_os = "linux")]
pub struct Bridge;
#[cfg(target_os = "linux")]
#[zbus::interface(name = "com.userfirst.Tilekeep")]
impl Bridge {
    fn snapshot_restored(&self, json: &str) -> zbus::fdo::Result<()> {
        #[derive(Deserialize)]
        struct Restored {
            gap: i32,
            missing: usize,
        }
        let result: Restored = serde_json::from_str(json).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        if !(0..=64).contains(&result.gap) || result.missing > 256 {
            return Err(zbus::fdo::Error::Failed("Invalid restore result".into()));
        }
        if let Some(c) = crate::control::current() {
            c.snapshot_restored(result.gap, result.missing);
        }
        Ok(())
    }
    fn save_snapshot(&self, json: &str) -> zbus::fdo::Result<String> {
        let result = (|| {
            if json.len() > 2 * 1024 * 1024 {
                return Err("Snapshot exceeds size limit".into());
            }
            let snapshot = serde_json::from_str(json).map_err(|e| e.to_string())?;
            let id = save(snapshot)?;
            if let Some(c) = crate::control::current() {
                c.snapshots_changed(&format!("Snapshot {id} saved"));
            }
            Ok(id)
        })();
        result.map_err(zbus::fdo::Error::Failed)
    }
    fn read_snapshot(&self, live_json: &str) -> zbus::fdo::Result<String> {
        let result = (|| {
            let c = crate::control::current().ok_or("Controller is not ready")?;
            let id = c.requested_snapshot().ok_or("No snapshot selected")?;
            let snapshot = load(&id)?;
            let live: Vec<AppWindow> = serde_json::from_str(live_json).map_err(|e| e.to_string())?;
            let errors = launch_missing(&snapshot, &live);
            c.status(if errors.is_empty() {
                "restoring snapshot"
            } else {
                "snapshot: some applications could not launch"
            });
            for e in errors {
                log::warn!("Snapshot launch: {e}");
            }
            serde_json::to_string(&snapshot).map_err(|e| e.to_string())
        })();
        result.map_err(zbus::fdo::Error::Failed)
    }
}
#[cfg(target_os = "linux")]
pub fn bridge() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name("com.userfirst.Tilekeep"))
        .and_then(|b| b.serve_at("/Tilekeep", Bridge))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(dir: &std::path::Path, id: &str) -> PathBuf {
        let p = path_in(dir, id).unwrap();
        std::fs::write(&p, r#"{"schema":1,"gap":1,"windows":[],"monitors":[{"name":"test","area":{"x":0,"y":0,"w":100,"h":100},"root":{"kind":"leaf","windows":[]}}],"future":{"keep":true}}"#).unwrap();
        p
    }
    #[test]
    fn legacy_snapshot_gets_date_and_rename_preserves_identity_layout_and_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let id = "1788898432453";
        let p = fixture(dir.path(), id);
        let entries = list_in(dir.path());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].name.is_empty());
        assert!(entries[0].label().starts_with("Snapshot — 2026-09-"));
        let mut before: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        rename_in(dir.path(), id, "  Work & café '$`  ").unwrap();
        let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        before["name"] = "Work & café '$`".into();
        assert_eq!(before, after);
        let entries = list_in(dir.path());
        assert_eq!(entries[0].id, id);
        assert!(entries[0].label().starts_with("Work & café '$` — 2026-09-"));
    }
    #[test]
    fn invalid_names_do_not_change_saved_data() {
        let dir = tempfile::tempdir().unwrap();
        let p = fixture(dir.path(), "123");
        let before = std::fs::read(&p).unwrap();
        for bad in ["".into(), "  ".into(), "x\ny".into(), "x\0y".into(), "a".repeat(81)] {
            assert!(rename_in(dir.path(), "123", &bad).is_err());
            assert_eq!(std::fs::read(&p).unwrap(), before);
        }
        assert!(rename_in(dir.path(), "../123", "test").is_err());
        assert!(rename_in(dir.path(), "456", "test").is_err());
        assert_eq!(valid_name(&"é".repeat(80)).unwrap().chars().count(), 80);
    }
    #[test]
    fn delete_archives_exact_file_clears_startup_and_leaves_other_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let p = fixture(dir.path(), "123");
        fixture(dir.path(), "456");
        let before = std::fs::read(&p).unwrap();
        let settings_path = dir.path().join("settings.json");
        let settings = crate::settings::Settings { startup_snapshot: Some("123".into()), gap: 4, ..Default::default() };
        crate::settings::save(&settings_path, &settings).unwrap();
        delete_in(dir.path(), "123", &settings_path).unwrap();
        assert!(!p.exists());
        assert_eq!(std::fs::read(dir.path().join("deleted/123.json")).unwrap(), before);
        let current = crate::settings::load(&settings_path).unwrap();
        assert!(current.startup_snapshot.is_none());
        assert_eq!(current.gap, 4);
        assert_eq!(list_in(dir.path()).iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["456"]);
        assert!(delete_in(dir.path(), "123", &settings_path).is_err());
    }
    #[test]
    fn delete_preserves_different_startup_and_refuses_archive_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let p = fixture(dir.path(), "123");
        let settings_path = dir.path().join("settings.json");
        let settings = crate::settings::Settings { startup_snapshot: Some("456".into()), ..Default::default() };
        crate::settings::save(&settings_path, &settings).unwrap();
        delete_in(dir.path(), "123", &settings_path).unwrap();
        assert_eq!(crate::settings::load(&settings_path).unwrap(), settings);
        fixture(dir.path(), "123");
        assert!(delete_in(dir.path(), "123", &settings_path).is_err());
        assert!(p.exists());
        assert!(delete_in(dir.path(), "../123", &settings_path).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn settings_write_failure_restores_archived_snapshot() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = fixture(dir.path(), "123");
        let settings_dir = dir.path().join("config");
        let settings_path = settings_dir.join("settings.json");
        let settings = crate::settings::Settings { startup_snapshot: Some("123".into()), ..Default::default() };
        crate::settings::save(&settings_path, &settings).unwrap();
        std::fs::set_permissions(&settings_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = delete_in(dir.path(), "123", &settings_path);
        std::fs::set_permissions(&settings_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if unsafe { libc::geteuid() } == 0 {
            return;
        } // root bypasses directory write permissions
        assert!(result.is_err());
        assert!(p.exists());
        assert!(!dir.path().join("deleted/123.json").exists());
        assert_eq!(crate::settings::load(&settings_path).unwrap(), settings);
    }
    #[test]
    fn listing_excludes_invalid_files_directories_and_archives_without_hiding_old_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        for i in 100..145 {
            fixture(dir.path(), &i.to_string());
        }
        std::fs::create_dir(dir.path().join("500.json")).unwrap();
        std::fs::write(dir.path().join("600.json"), "broken").unwrap();
        std::fs::write(dir.path().join("700.backup"), "ignored").unwrap();
        let entries = list_in(dir.path());
        assert_eq!(entries.len(), 45);
        assert_eq!(entries[0].id, "144");
        assert_eq!(entries[44].id, "100");
    }
    #[test]
    fn native_snapshot_restores_tree_with_new_handles_and_keeps_extra_windows() {
        use crate::{
            engine::{AppIdentity, Desktop, MonitorId},
            geometry::Rect,
            tree::WindowId,
        };
        let mut desktop = Desktop::new(1);
        desktop.sync_monitors(&[(MonitorId(1), Rect::new(0, 0, 1000, 800))]);
        desktop.window_appeared(WindowId(11), MonitorId(1), Some(AppIdentity("one".into())));
        desktop.window_appeared(WindowId(22), MonitorId(1), Some(AppIdentity("two".into())));
        let windows = desktop
            .placements()
            .iter()
            .map(|p| AppWindow {
                token: p.window.0.to_string(),
                app: if p.window.0 == 11 { "one" } else { "two" }.into(),
                title: String::new(),
                pid: 0,
                executable: None,
                rect: p.rect,
                floating: false,
            })
            .collect::<Vec<_>>();
        let saved = desktop.snapshot(windows.clone());
        let encoded = serde_json::to_vec(&saved).unwrap();
        let saved: Snapshot = serde_json::from_slice(&encoded).unwrap();
        validate(&saved).unwrap();
        let mut live = windows;
        live[0].token = "111".into();
        live[1].token = "222".into();
        let mut extra = live[0].clone();
        extra.app = "extra".into();
        extra.token = "333".into();
        live.push(extra);
        assert_eq!(desktop.restore_snapshot(&saved, &live).unwrap(), 2);
        assert!(desktop.is_floating(WindowId(333)));
        for p in desktop.placements() {
            let original = saved
                .windows
                .iter()
                .find(|w| w.app == live.iter().find(|l| l.token == p.window.0.to_string()).unwrap().app)
                .unwrap();
            assert_eq!(p.rect, original.rect);
        }
    }
    #[test]
    fn malformed_snapshot_nodes_are_rejected() {
        let mut s = Snapshot {
            schema: 1,
            name: String::new(),
            gap: 1,
            windows: vec![],
            monitors: vec![Monitor {
                name: "screen".into(),
                area: crate::geometry::Rect::new(0, 0, 100, 100),
                root: Node::Leaf { windows: vec!["missing".into()], active: 0 },
            }],
        };
        assert!(validate(&s).is_err());
        s.monitors[0].root = Node::Split {
            axis: "z".into(),
            ratio: 0.5,
            preserve_space: false,
            first: Box::new(Node::Leaf { windows: vec![], active: 0 }),
            second: Box::new(Node::Leaf { windows: vec![], active: 0 }),
        };
        assert!(validate(&s).is_err());
    }
    #[test]
    fn snapshot_ids_cannot_escape_the_directory() {
        for id in ["../x", "/tmp/x", "", "x.json"] {
            assert!(path(id).is_err());
        }
    }
    #[test]
    fn exact_vacancy_flag_round_trips_and_old_snapshots_default_false() {
        let old = r#"{"kind":"split","axis":"x","ratio":0.98,"first":{"kind":"leaf","windows":[]},"second":{"kind":"leaf","windows":[]}}"#;
        let mut node: Node = serde_json::from_str(old).unwrap();
        let Node::Split { preserve_space, .. } = &mut node else { panic!("expected split") };
        assert!(!*preserve_space);
        *preserve_space = true;
        let encoded = serde_json::to_string(&node).unwrap();
        let restored: Node = serde_json::from_str(&encoded).unwrap();
        assert!(matches!(restored, Node::Split { preserve_space: true, .. }));
    }
    #[test]
    fn matching_does_not_reuse_a_window() {
        let a = AppWindow {
            token: "old".into(),
            app: "browser".into(),
            title: "A".into(),
            pid: 0,
            executable: None,
            rect: crate::geometry::Rect::new(0, 0, 100, 100),
            floating: false,
        };
        let mut b = a.clone();
        b.token = "other".into();
        let mut live = a.clone();
        live.token = "new".into();
        let map = matching(&[a, b], &[live]);
        assert_eq!(map.len(), 1);
        assert_eq!(map["old"], "new");
    }
}
