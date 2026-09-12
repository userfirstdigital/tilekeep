use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    pub gap: i32,
    pub automatic_updates: bool,
    pub float_secondary_windows: bool,
    pub startup_snapshot: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self { schema: 1, gap: 1, automatic_updates: true, float_secondary_windows: true, startup_snapshot: None }
    }
}
pub fn config_dir() -> Result<PathBuf, String> {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().join("tilekeep"))
        .ok_or("No user configuration directory".into())
}
pub fn install_path() -> Result<PathBuf, String> {
    let d = directories::BaseDirs::new().ok_or("No user data directory")?;
    #[cfg(windows)]
    return Ok(d.data_local_dir().join("Programs/Tilekeep/tilekeep.exe"));
    #[cfg(not(windows))]
    Ok(d.data_local_dir().join("tilekeep/bin/tilekeep"))
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_file(path, bytes, false)
}
pub fn atomic_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_file(path, bytes, true)
}
fn write_file(path: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("File has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).and_then(|_| file.as_file().sync_all()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        file.as_file().set_permissions(fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn load(path: &Path) -> Result<Settings, String> {
    match fs::read(path) {
        Ok(bytes) => {
            let s: Settings =
                serde_json::from_slice(&bytes).map_err(|e| format!("Settings were not overwritten: {e}"))?;
            if s.schema != 1 || !(0..=64).contains(&s.gap) {
                return Err("Unsupported settings schema or gap (expected 0–64)".into());
            }
            Ok(s)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn save(path: &Path, s: &Settings) -> Result<(), String> {
    atomic_write(path, &serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?)
}
pub fn install() -> Result<PathBuf, String> {
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = install_path()?;
    if source == target {
        return Ok(target);
    }
    atomic_executable(&target, &fs::read(&source).map_err(|e| e.to_string())?)?;
    Ok(target)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_and_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        assert_eq!(load(&p).unwrap(), Settings::default());
        let s = Settings { gap: 4, automatic_updates: false, ..Settings::default() };
        save(&p, &s).unwrap();
        assert_eq!(load(&p).unwrap(), s);
        fs::write(&p, r#"{"future":true,"gap":2}"#).unwrap();
        let loaded = load(&p).unwrap();
        assert_eq!(loaded.gap, 2);
        assert!(loaded.float_secondary_windows, "older settings inherit the enabled default");
    }
    #[test]
    fn invalid_settings_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        for bad in ["broken", r#"{"schema":5}"#, r#"{"gap":-1}"#] {
            fs::write(&p, bad).unwrap();
            assert!(load(&p).is_err());
            assert_eq!(fs::read_to_string(&p).unwrap(), bad);
        }
    }
}
