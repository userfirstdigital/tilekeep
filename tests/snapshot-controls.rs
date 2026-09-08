//! Controller/dialog integration with a private config and scripted dialog replies.
//! No compositor, tray host, real dialog, or user snapshot is touched.
#![cfg(target_os = "linux")]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use windowmanager::control::{Action, Controller};

#[test]
fn rename_cancel_delete_and_startup_selection_via_controller() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("tilekeep");
    let snapshots = config.join("snapshots");
    fs::create_dir_all(&snapshots).unwrap();
    let id = "1788898432453";
    let file = snapshots.join(format!("{id}.json"));
    fs::write(&file, r#"{"schema":1,"gap":1,"windows":[],"monitors":[{"name":"test","area":{"x":0,"y":0,"w":100,"h":100},"root":{"kind":"leaf","windows":[]}}]}"#).unwrap();
    let helper = dir.path().join("kdialog");
    fs::write(
        &helper,
        "#!/bin/sh\n/bin/sleep 0.05\n/bin/cat \"$TILEKEEP_TEST_REPLY\"\nexit \"$(/bin/cat \"$TILEKEEP_TEST_EXIT\")\"\n",
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let reply = dir.path().join("reply");
    let exit = dir.path().join("exit");
    std::env::set_var("XDG_CONFIG_HOME", dir.path());
    std::env::set_var("PATH", dir.path());
    std::env::set_var("TILEKEEP_TEST_REPLY", &reply);
    std::env::set_var("TILEKEEP_TEST_EXIT", &exit);
    fs::write(&reply, "Work & café '$`\n").unwrap();
    fs::write(&exit, "0").unwrap();
    let c = Controller::start(None).unwrap();
    c.action(Action::StartupSnapshot(Some(id.into())));
    c.action(Action::RenameSnapshot(id.into()));
    let wait = |predicate: &dyn Fn() -> bool| {
        let start = Instant::now();
        while !predicate() {
            assert!(start.elapsed() < Duration::from_secs(5), "controller action timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    wait(&|| c.state().snapshots[0].name == "Work & café '$`");
    assert_eq!(c.state().settings.startup_snapshot.as_deref(), Some(id));
    assert_eq!(windowmanager::snapshots::load(id).unwrap().name, "Work & café '$`");
    let bytes = fs::read(&file).unwrap();
    // Cancel rename and delete: data and startup selection must be unchanged.
    fs::write(&exit, "1").unwrap();
    for action in [Action::RenameSnapshot(id.into()), Action::DeleteSnapshot(id.into())] {
        c.action(action);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(fs::read(&file).unwrap(), bytes);
        assert_eq!(c.state().settings.startup_snapshot.as_deref(), Some(id));
    }
    c.action(Action::LoadSnapshot(id.into()));
    assert_eq!(c.requested_snapshot().as_deref(), Some(id));
    fs::write(&exit, "0").unwrap();
    c.action(Action::DeleteSnapshot(id.into()));
    wait(&|| c.state().snapshots.is_empty());
    assert!(!file.exists());
    assert!(c.state().settings.startup_snapshot.is_none());
    assert!(windowmanager::settings::load(&config.join("settings.json")).unwrap().startup_snapshot.is_none());
    assert!(c.requested_snapshot().is_none());
    assert!(windowmanager::control::drain().is_empty());
    assert_eq!(fs::read(snapshots.join("deleted").join(format!("{id}.json"))).unwrap(), bytes);
    c.action(Action::LoadSnapshot(id.into()));
    assert!(c.state().status.starts_with("Error:"));
}
