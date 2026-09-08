//! Dialogs run off the tray/event thread. User text is data, never shell source.
use std::process::{Command, Output};

fn answer(output: Output) -> Result<Option<String>, String> {
    match output.status.code() {
        Some(0) => String::from_utf8(output.stdout)
            .map(|s| Some(s.trim_end_matches(['\r', '\n']).into()))
            .map_err(|e| e.to_string()),
        Some(1) => Ok(None), // Cancel / No / window dismissed
        _ => Err("Snapshot dialog failed to open or exited unexpectedly".into()),
    }
}

#[cfg(target_os = "linux")]
fn show(name: Option<&str>, message: &str) -> Result<Option<String>, String> {
    let mut kde = Command::new("kdialog");
    kde.args(["--title", "Tilekeep snapshots"]);
    match name {
        Some(name) => {
            kde.args(["--inputbox", message, "--", name]);
        }
        None => {
            kde.args(["--warningyesno", message, "--yes-label", "Delete", "--no-label", "Cancel"]);
        }
    }
    match kde.output() {
        Ok(output) => answer(output),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut gtk = Command::new("zenity");
            gtk.args(["--title=Tilekeep snapshots", "--no-markup"]);
            match name {
                Some(name) => {
                    gtk.args(["--entry", &format!("--text={message}"), &format!("--entry-text={name}")]);
                }
                None => {
                    gtk.args([
                        "--question",
                        &format!("--text={message}"),
                        "--ok-label=Delete",
                        "--cancel-label=Cancel",
                    ]);
                }
            }
            answer(gtk.output().map_err(|e| format!("Install kdialog or zenity for snapshot dialogs: {e}"))?)
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(windows)]
fn show(name: Option<&str>, message: &str) -> Result<Option<String>, String> {
    use std::os::windows::process::CommandExt;
    // Fixed script; even quotes, dollars and backticks in a name stay literal.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName Microsoft.VisualBasic
Add-Type -AssemblyName System.Windows.Forms
if ($env:TILEKEEP_DIALOG_MODE -eq 'rename') {
  $name = [Microsoft.VisualBasic.Interaction]::InputBox($env:TILEKEEP_DIALOG_MESSAGE, 'Tilekeep snapshots', $env:TILEKEEP_DIALOG_NAME)
  if ($name -eq '') { exit 1 }
  [Console]::Write($name)
} else {
  $result = [System.Windows.Forms.MessageBox]::Show($env:TILEKEEP_DIALOG_MESSAGE, 'Tilekeep snapshots', [System.Windows.Forms.MessageBoxButtons]::OKCancel, [System.Windows.Forms.MessageBoxIcon]::Warning, [System.Windows.Forms.MessageBoxDefaultButton]::Button2)
  if ($result -ne [System.Windows.Forms.DialogResult]::OK) { exit 1 }
}
exit 0
"#;
    answer(
        Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-STA", "-Command", SCRIPT])
            .env("TILEKEEP_DIALOG_MODE", if name.is_some() { "rename" } else { "delete" })
            .env("TILEKEEP_DIALOG_NAME", name.unwrap_or_default())
            .env("TILEKEEP_DIALOG_MESSAGE", message)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW: no leftover console
            .output()
            .map_err(|e| e.to_string())?,
    )
}

pub fn rename(entry: &crate::snapshots::Entry) -> Result<Option<String>, String> {
    let mut name = entry.name.clone();
    let mut prompt = "Snapshot name (1–80 characters):".to_string();
    loop {
        let Some(answer) = show(Some(&name), &prompt)? else {
            return Ok(None);
        };
        match crate::snapshots::valid_name(&answer) {
            Ok(name) => return Ok(Some(name)),
            Err(e) => {
                name = answer;
                prompt = format!("{e}.\n\nSnapshot name:");
            }
        }
    }
}
pub fn delete(entry: &crate::snapshots::Entry, startup: bool) -> Result<bool, String> {
    let message = format!("Delete this snapshot?\n\n{}\n\n{}No applications will be closed. The saved file will be moved to snapshots/deleted for recovery.", entry.label(),
        if startup { "This also turns off loading this snapshot at startup.\n\n" } else { "" });
    show(None, &message).map(|s| s.is_some())
}
