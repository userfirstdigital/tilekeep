//! Native Plasma/Wayland support. KWin is the only process allowed to enumerate and move
//! Wayland windows, so the window-facing half runs as a temporary KWin script.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Options;

const PLUGIN: &str = "tilekeep-runtime";
const MODIFIER_OBSERVER: &str = "tilekeep-control-observer";
const MODIFIER_MARKER: &str = "tilekeep-control-marker";
static STOP: AtomicBool = AtomicBool::new(false);

pub fn is_plasma_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
        && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|v| v.to_ascii_lowercase().contains("kde"))
}

pub fn run(options: Options) -> Result<(), String> {
    require("qdbus6")?;
    // Own the integration before touching any KWin-global scripts or effects.
    // A second Tilekeep process must not disturb the running instance while it
    // discovers that the integration is already owned.
    let lock_dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let lock_path = lock_dir.join(format!("tilekeep-kwin-{}.lock", unsafe { libc::geteuid() }));
    let _owner = acquire_lock(&lock_path)?;
    let _snapshot_bridge = if crate::control::current().is_some() { Some(crate::snapshots::bridge()?) } else { None };
    let modifier_effect = !options.dry_run
        && match load_modifier_effect() {
            Ok(()) => true,
            Err(e) => {
                log::warn!("Ctrl move/resize gestures are unavailable: {e}");
                false
            }
        };
    STOP.store(false, Ordering::Relaxed);
    let source = include_str!("kwin.qml")
        .replace("__TILEKEEP_GAP__", &options.gap.to_string())
        .replace("__TILEKEEP_DRY_RUN__", if options.dry_run { "true" } else { "false" });
    // KWin caches QML by URL even after unloading it. A nonce also handles PID
    // reuse, and create_new prevents following an attacker-supplied /tmp link.
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let path = std::env::temp_dir().join(format!("tilekeep-{}-{nonce}.qml", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("could not create {}: {e}", path.display()))?;
    if let Err(e) = file.write_all(source.as_bytes()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("could not write {}: {e}", path.display()));
    }
    drop(file);

    let mut reservations = Vec::new();
    let started = (|| {
        let _ = scripting(&["unloadScript", PLUGIN]);
        // KWin 6.7 derives script IDs from the number of loaded scripts, not
        // their highest ID. A hole can make a new script collide with another
        // script's D-Bus object, silently running that script instead. Reserve
        // the colliding indices with our own dormant scripts; never stop or
        // start unrelated scripts to clear the collision.
        let paths = output_text(qdbus(&[]))?;
        for index in 0..64 {
            let name = format!("tilekeep-id-reservation-{}-{nonce}-{index}", std::process::id());
            let id: i32 = output_text(scripting(&["loadDeclarativeScript", &path.to_string_lossy(), &name]))?
                .parse()
                .map_err(|e| format!("KWin returned an invalid reservation id: {e}"))?;
            if id < 0 {
                return Err("KWin refused the script ID reservation".into());
            }
            reservations.push(name);
            if !paths.lines().any(|line| line.trim() == format!("/Scripting/Script{}", id + 1)) {
                break;
            }
            if index == 63 {
                return Err("Could not reserve a free KWin script ID".into());
            }
        }
        let loaded = scripting(&["loadDeclarativeScript", &path.to_string_lossy(), PLUGIN]);
        let id: i32 = output_text(loaded)?.parse().map_err(|e| format!("KWin returned an invalid script id: {e}"))?;
        if id < 0 {
            return Err("KWin refused to load its Tilekeep integration script".into());
        }
        output_text(qdbus(&[&format!("/Scripting/Script{id}"), "org.kde.kwin.Script.run"]))
    })();
    for name in reservations {
        let _ = scripting(&["unloadScript", &name]);
    }
    let _ = std::fs::remove_file(&path);
    if let Err(e) = started {
        let _ = scripting(&["unloadScript", PLUGIN]);
        if modifier_effect {
            unload_modifier_effect();
        }
        return Err(e);
    }
    log::info!("native Plasma/Wayland integration loaded; Super+Shift+Q stops it");
    if let Some(c) = crate::control::current() {
        c.status("running");
    }

    // Keep the command attached to the script. This also lets Ctrl-C and SIGTERM unload it
    // instead of leaving an unexpected KWin script behind for the rest of the session.
    unsafe {
        libc::signal(libc::SIGINT, stop_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop_signal as *const () as libc::sighandler_t);
    }
    while !STOP.load(Ordering::Relaxed) && script_loaded() {
        crate::tray::poll();
        for command in crate::control::drain() {
            use crate::control::Command as C;
            let action = match command {
                C::Pause(true) => "TilekeepPause".into(),
                C::Pause(false) => "TilekeepResume".into(),
                C::Retile => "TilekeepRetile".into(),
                C::Unstack => "TilekeepUnstack".into(),
                C::Compact => "TilekeepCompact".into(),
                C::Gap(g) => format!("TilekeepSetGap{g}"),
                C::SaveSnapshot => "TilekeepSaveSnapshot".into(),
                C::LoadSnapshot(_) => "TilekeepLoadSnapshot".into(),
                C::Quit => {
                    STOP.store(true, Ordering::Relaxed);
                    continue;
                }
            };
            let result = Command::new("qdbus6")
                .args([
                    "org.kde.kglobalaccel",
                    "/component/kwin",
                    "org.kde.kglobalaccel.Component.invokeShortcut",
                    &action,
                ])
                .output();
            if let Err(e) = output_text(result.map_err(|e| e.to_string())) {
                log::warn!("Tray command {action}: {e}");
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    if script_loaded() {
        // Quiesce while the QML context is alive. The scoped QObject connections
        // also make direct teardown safe if the shortcut service is unavailable.
        let _ = Command::new("qdbus6")
            .args([
                "org.kde.kglobalaccel",
                "/component/kwin",
                "org.kde.kglobalaccel.Component.invokeShortcut",
                "TilekeepQuit",
            ])
            .output();
        let _ = scripting(&["unloadScript", PLUGIN]);
    }
    if modifier_effect {
        unload_modifier_effect();
    }
    Ok(())
}

fn effect_dir(name: &str) -> Result<PathBuf, String> {
    let base = directories::BaseDirs::new().ok_or("could not locate the per-user data directory")?;
    Ok(base.data_dir().join("kwin/effects").join(name))
}

fn install_effect_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    let parent = path.parent().ok_or("invalid KWin effect path")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("could not stage {}: {e}", path.display()))?;
    temporary.write_all(contents).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    temporary.persist(path).map_err(|e| format!("could not install {}: {}", path.display(), e.error))?;
    Ok(())
}

fn install_modifier_effects() -> Result<(), String> {
    for (name, metadata, script) in [
        (
            MODIFIER_OBSERVER,
            include_bytes!("effects/tilekeep-control-observer/metadata.json").as_slice(),
            include_bytes!("effects/tilekeep-control-observer/contents/code/main.js").as_slice(),
        ),
        (
            MODIFIER_MARKER,
            include_bytes!("effects/tilekeep-control-marker/metadata.json").as_slice(),
            include_bytes!("effects/tilekeep-control-marker/contents/code/main.js").as_slice(),
        ),
    ] {
        let directory = effect_dir(name)?;
        install_effect_file(&directory.join("metadata.json"), metadata)?;
        install_effect_file(&directory.join("contents/code/main.js"), script)?;
    }
    Ok(())
}

fn effect(method: &str, name: &str) -> Result<String, String> {
    output_text(qdbus(&["/Effects", &format!("org.kde.kwin.Effects.{method}"), name]))
}

fn load_modifier_effect() -> Result<(), String> {
    install_modifier_effects()?;
    // Clean up state left by a killed process before enabling this instance.
    let _ = effect("unloadEffect", MODIFIER_MARKER);
    let _ = effect("unloadEffect", MODIFIER_OBSERVER);
    let result = match effect("loadEffect", MODIFIER_OBSERVER) {
        Ok(value) if value == "true" => Ok(()),
        Ok(_) => Err("KWin could not load its Tilekeep modifier observer".into()),
        Err(error) => Err(error),
    };
    if result.is_err() {
        // A failed D-Bus reply does not prove KWin did not finish loading it.
        // Always restore an inert state before continuing without Ctrl gestures.
        unload_modifier_effect();
    }
    result
}

fn unload_modifier_effect() {
    let _ = effect("unloadEffect", MODIFIER_MARKER);
    let _ = effect("unloadEffect", MODIFIER_OBSERVER);
}

fn acquire_lock(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("could not open the Plasma integration lock: {e}"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
            "Tilekeep is already running; use Super+Shift+L to re-tile or Super+Shift+Q to quit that instance".into()
        } else {
            format!("could not lock the Plasma integration: {error}")
        });
    }
    // Do not unlink lock files: an overlapping opener could otherwise hold a
    // different inode and believe it owns the same integration.
    Ok(file)
}

pub fn list() -> Result<(), String> {
    Err("--list is not exposed by KWin on Wayland; run wm --dry-run to log the windows and placements KWin sees".into())
}

extern "C" fn stop_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn script_loaded() -> bool {
    scripting(&["isScriptLoaded", PLUGIN]).ok().and_then(|o| output_text(Ok(o)).ok()).is_some_and(|s| s == "true")
}

fn require(command: &str) -> Result<(), String> {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|_| ())
        .map_err(|e| format!("{command} is required for native Plasma support: {e}"))
}

fn scripting(args: &[&str]) -> Result<Output, String> {
    let Some((method, values)) = args.split_first() else { return Err("missing KWin scripting method".into()) };
    let member = format!("org.kde.kwin.Scripting.{method}");
    let mut full = vec!["/Scripting", member.as_str()];
    full.extend_from_slice(values);
    qdbus(&full)
}

fn qdbus(args: &[&str]) -> Result<Output, String> {
    Command::new("qdbus6")
        .arg("org.kde.KWin")
        .args(args)
        .output()
        .map_err(|e| format!("could not talk to KWin over D-Bus: {e}"))
}

fn output_text(result: Result<Output, String>) -> Result<String, String> {
    let output = result?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if message.is_empty() { "KWin D-Bus call failed".into() } else { message })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_lock_is_exclusive_and_released_on_drop() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("tilekeep-lock-test-{}-{nonce}", std::process::id()));
        let first = acquire_lock(&path).unwrap();
        assert!(acquire_lock(&path).unwrap_err().contains("already running"));
        drop(first);
        drop(acquire_lock(&path).unwrap());
        std::fs::remove_file(path).unwrap();
    }
}
