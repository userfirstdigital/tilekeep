//! Signed background download; install at the next launch, never during a drag.
use crate::control::Controller;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
const ENDPOINT: &str = "https://github.com/userfirstdigital/tilekeep/releases/latest/download/latest.json";
static CHECKING: AtomicBool = AtomicBool::new(false);
#[derive(Deserialize, Serialize, Debug)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub platforms: BTreeMap<String, Artifact>,
}
#[derive(Deserialize, Serialize, Debug)]
pub struct Artifact {
    pub url: String,
    pub signature: String,
}
fn key() -> Result<minisign_verify::PublicKey, String> {
    let key = option_env!("TILEKEEP_UPDATE_PUBKEY")
        .filter(|s| !s.trim().is_empty())
        .ok_or("Updates unavailable: release signing key not configured")?;
    minisign_verify::PublicKey::from_base64(key.trim()).map_err(|e| e.to_string())
}
pub fn availability() -> String {
    match key() {
        Ok(_) => "Updates: not checked yet".into(),
        Err(e) => e,
    }
}
fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
pub fn verify_manifest(
    bytes: &[u8],
    signature: &str,
    key: &minisign_verify::PublicKey,
    current: &str,
) -> Result<Manifest, String> {
    key.verify(bytes, &minisign_verify::Signature::decode(signature).map_err(|e| e.to_string())?, false)
        .map_err(|e| e.to_string())?;
    let m: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if m.schema != 1 {
        return Err("Unsupported update manifest".into());
    }
    let version = semver::Version::parse(&m.version).map_err(|e| e.to_string())?;
    if !version.pre.is_empty() || version <= semver::Version::parse(current).map_err(|e| e.to_string())? {
        return Err("No newer stable release".into());
    }
    Ok(m)
}
fn download(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let u = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    if u.scheme() != "https" {
        return Err("Update URLs must use HTTPS".into());
    }
    let response = client.get(u).send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Update exceeds size limit".into());
    }
    Ok(bytes)
}
fn check() -> Result<String, String> {
    let key = key()?;
    if std::env::current_exe().map_err(|e| e.to_string())? != crate::settings::install_path()? {
        return Err("Updates require the per-user installation (--install)".into());
    }
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(120))
        .user_agent("Tilekeep updater")
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = download(&client, ENDPOINT, 1024 * 1024)?;
    let signature =
        String::from_utf8(download(&client, &format!("{ENDPOINT}.minisig"), 8192)?).map_err(|e| e.to_string())?;
    let m = match verify_manifest(&bytes, &signature, &key, env!("CARGO_PKG_VERSION")) {
        Ok(m) => m,
        Err(e) if e == "No newer stable release" => return Ok("Up to date".into()),
        Err(e) => return Err(e),
    };
    let artifact = m.platforms.get(&target()).ok_or("No update for this operating system / architecture")?;
    let binary = download(&client, &artifact.url, 128 * 1024 * 1024)?;
    key.verify(&binary, &minisign_verify::Signature::decode(&artifact.signature).map_err(|e| e.to_string())?, false)
        .map_err(|e| e.to_string())?;
    let dir = crate::settings::config_dir()?.join("update");
    crate::settings::atomic_write(&dir.join("binary"), &binary)?;
    crate::settings::atomic_write(&dir.join("manifest.minisig"), signature.as_bytes())?;
    // Commit marker last. Startup verifies all three files again.
    crate::settings::atomic_write(&dir.join("manifest.json"), &bytes)?;
    Ok(format!("v{} ready — installs silently on next launch", m.version))
}
pub fn check_background(c: Arc<Controller>) {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    c.update_status("Checking for updates…");
    std::thread::spawn(move || {
        let result = check();
        match result {
            Ok(s) => c.update_status(&s),
            Err(e) => {
                log::warn!("Update check: {e}");
                c.update_status(&e);
            }
        }
        CHECKING.store(false, Ordering::SeqCst);
    });
}
pub fn schedule(c: Arc<Controller>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(60));
        loop {
            if c.state().settings.automatic_updates {
                check_background(c.clone());
            }
            std::thread::sleep(Duration::from_secs(6 * 60 * 60));
        }
    });
}
fn pending() -> Result<Option<Vec<u8>>, String> {
    let dir = crate::settings::config_dir()?.join("update");
    if !dir.join("manifest.json").exists() {
        return Ok(None);
    }
    let key = key()?;
    let m = verify_manifest(
        &std::fs::read(dir.join("manifest.json")).map_err(|e| e.to_string())?,
        &std::fs::read_to_string(dir.join("manifest.minisig")).map_err(|e| e.to_string())?,
        &key,
        env!("CARGO_PKG_VERSION"),
    )?;
    let binary = std::fs::read(dir.join("binary")).map_err(|e| e.to_string())?;
    let artifact = m.platforms.get(&target()).ok_or("Pending update platform mismatch")?;
    key.verify(&binary, &minisign_verify::Signature::decode(&artifact.signature).map_err(|e| e.to_string())?, false)
        .map_err(|e| e.to_string())?;
    Ok(Some(binary))
}
fn install_pending(binary: &[u8]) -> Result<(), String> {
    let target = crate::settings::install_path()?;
    let previous = std::fs::read(&target).map_err(|e| e.to_string())?;
    crate::settings::atomic_write(&target.with_extension("previous"), &previous)?;
    crate::settings::atomic_executable(&target, binary)?;
    let marker = crate::settings::config_dir()?.join("update/manifest.json");
    std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn apply_on_launch() -> Result<bool, String> {
    if std::env::current_exe().map_err(|e| e.to_string())? != crate::settings::install_path()? {
        return Ok(false);
    }
    let Some(binary) = pending()? else { return Ok(false) };
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        install_pending(&binary)?;
        Err(std::process::Command::new(crate::settings::install_path()?)
            .args(std::env::args_os().skip(1))
            .exec()
            .to_string())
    }
    #[cfg(windows)]
    {
        let _ = binary;
        use std::os::windows::process::CommandExt;
        let helper = crate::settings::install_path()?.with_file_name("tilekeep-update-helper.exe");
        crate::settings::atomic_write(
            &helper,
            &std::fs::read(std::env::current_exe().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
        )?;
        std::process::Command::new(helper)
            .args(["--apply-update", &std::process::id().to_string()])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(true)
    }
}
#[cfg(windows)]
pub fn windows_helper(pid: u32) -> Result<(), String> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    // SAFETY: wait-only process handle, no access to another process's memory.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let result = WaitForSingleObject(handle, 30_000);
            let _ = CloseHandle(handle);
            if result.0 != 0 {
                return Err("Previous process did not exit".into());
            }
        }
    }
    let binary = pending()?.ok_or("No pending update")?;
    install_pending(&binary)?;
    std::process::Command::new(crate::settings::install_path()?).spawn().map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_key_is_not_reported_as_up_to_date() {
        if option_env!("TILEKEEP_UPDATE_PUBKEY").is_none() {
            assert!(availability().contains("not configured"));
        }
    }
    #[test]
    fn http_download_is_refused_without_network_access() {
        let c = reqwest::blocking::Client::new();
        assert!(download(&c, "http://example.com/update", 10).unwrap_err().contains("HTTPS"));
    }
    #[test]
    fn signed_manifest_accepts_only_authentic_new_stable_versions() {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = minisign_verify::PublicKey::from_base64(&pair.pk.to_base64()).unwrap();
        for (version, accepted) in [("0.3.0", true), ("0.2.0", false), ("0.1.0", false), ("0.3.0-beta.1", false)] {
            let bytes =
                serde_json::to_vec(&Manifest { schema: 1, version: version.into(), platforms: BTreeMap::new() })
                    .unwrap();
            let signature = minisign::sign(Some(&pair.pk), &pair.sk, bytes.as_slice(), None, None).unwrap().to_string();
            assert_eq!(verify_manifest(&bytes, &signature, &key, "0.2.0").is_ok(), accepted);
            let mut changed = bytes.clone();
            changed[0] = b'[';
            assert!(verify_manifest(&changed, &signature, &key, "0.2.0").is_err());
            let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            let wrong = minisign_verify::PublicKey::from_base64(&other.pk.to_base64()).unwrap();
            assert!(verify_manifest(&bytes, &signature, &wrong, "0.2.0").is_err());
        }
    }
}
