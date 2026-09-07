use std::path::Path;

pub fn desktop_entry(executable: &Path) -> Result<String, String> {
    let raw = executable.to_str().ok_or("Executable path is not Unicode")?;
    if raw.contains(['\n', '\r']) {
        return Err("Executable path contains a newline".into());
    }
    // Desktop Entry escaping and Exec quoting are two separate parsing layers.
    let quoted = raw
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('`', "\\\\`")
        .replace('$', "\\\\$")
        .replace('%', "%%");
    Ok(format!("[Desktop Entry]\nType=Application\nName=Tilekeep\nComment=Tiling window manager\nExec=\"{quoted}\" --autostart\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"))
}
#[cfg(target_os = "linux")]
fn entry_path() -> Result<std::path::PathBuf, String> {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().join("autostart/com.userfirst.tilekeep.desktop"))
        .ok_or("No user configuration directory".into())
}
#[cfg(target_os = "linux")]
pub fn enabled(exe: &Path) -> bool {
    entry_path().ok().and_then(|p| std::fs::read_to_string(p).ok()) == desktop_entry(exe).ok()
}
#[cfg(target_os = "linux")]
pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
    let path = entry_path()?;
    if enabled {
        crate::settings::atomic_write(&path, desktop_entry(exe)?.as_bytes())
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
#[cfg(windows)]
fn reg(args: &[&str]) -> Result<std::process::Output, String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg.exe").args(args).creation_flags(0x08000000).output().map_err(|e| e.to_string())
}
#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
pub fn enabled(exe: &Path) -> bool {
    let wanted = format!("\"{}\" --autostart", exe.display());
    reg(&["query", KEY, "/v", "Tilekeep"])
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains(&wanted))
}
#[cfg(windows)]
pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
    if !enabled && !self::enabled(exe) {
        return Ok(());
    }
    let value = format!("\"{}\" --autostart", exe.display());
    let o = if enabled {
        reg(&["add", KEY, "/v", "Tilekeep", "/t", "REG_SZ", "/d", &value, "/f"])?
    } else {
        reg(&["delete", KEY, "/v", "Tilekeep", "/f"])?
    };
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_exec_quotes_special_characters() {
        let entry = desktop_entry(Path::new("/tmp/a space/$cash%/tilekeep")).unwrap();
        assert!(entry.contains("Exec=\"/tmp/a space/\\\\$cash%%/tilekeep\" --autostart"));
        assert!(desktop_entry(Path::new("/tmp/x\ny")).is_err());
    }
}
