use crate::settings::Settings;
use std::{
    collections::VecDeque,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug)]
pub enum Action {
    Pause,
    Unstack,
    Retile,
    Compact,
    Gap(i32),
    Autostart,
    AutomaticUpdates,
    CheckUpdates,
    OpenSettings,
    SaveSnapshot,
    LoadSnapshot(String),
    StartupSnapshot(Option<String>),
    Quit,
}
#[derive(Clone, Debug)]
pub enum Command {
    Pause(bool),
    Unstack,
    Retile,
    Compact,
    Gap(i32),
    SaveSnapshot,
    LoadSnapshot(String),
    Quit,
}
#[derive(Clone, Debug)]
pub struct State {
    pub settings: Settings,
    pub paused: bool,
    pub autostart: bool,
    pub status: String,
    pub update: String,
    pub snapshots: Vec<String>,
}
pub struct Controller {
    state: Mutex<State>,
    queue: Mutex<VecDeque<Command>>,
    requested: Mutex<Option<String>>,
    path: PathBuf,
    exe: PathBuf,
    _lock: File,
}
static INSTANCE: OnceLock<Arc<Controller>> = OnceLock::new();
impl Controller {
    pub fn start(gap: Option<i32>) -> Result<Arc<Self>, String> {
        let dir = crate::settings::config_dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("instance.lock"))
            .map_err(|e| e.to_string())?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "Tilekeep is already running".to_string())?;
        let path = dir.join("settings.json");
        let mut settings = crate::settings::load(&path)?;
        if let Some(g) = gap {
            settings.gap = g;
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let startup = settings.startup_snapshot.clone();
        let state = State {
            settings,
            paused: false,
            autostart: crate::autostart::enabled(&exe),
            status: "Starting".into(),
            update: crate::updater::availability(),
            snapshots: crate::snapshots::list(),
        };
        let queue = startup.iter().map(|id| Command::LoadSnapshot(id.clone())).collect();
        let c = Arc::new(Self {
            state: Mutex::new(state),
            queue: Mutex::new(queue),
            requested: Mutex::new(startup),
            path,
            exe,
            _lock: lock,
        });
        INSTANCE.set(c.clone()).map_err(|_| "Controller already initialized")?;
        Ok(c)
    }
    pub fn state(&self) -> State {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn status(&self, status: &str) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).status = status.into();
    }
    pub fn update_status(&self, status: &str) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).update = status.into();
    }
    pub fn snapshots_changed(&self, status: &str) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.snapshots = crate::snapshots::list();
        s.status = status.into();
    }
    pub fn requested_snapshot(&self) -> Option<String> {
        self.requested.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn snapshot_restored(&self, gap: i32, missing: usize) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.settings.gap = gap;
        if let Err(e) = crate::settings::save(&self.path, &s.settings) {
            log::warn!("Snapshot settings: {e}");
        }
        s.status = if missing == 0 { "running".into() } else { format!("restored; waiting for {missing} windows") };
    }
    pub fn action(self: &Arc<Self>, action: Action) {
        if let Err(e) = self.handle(action) {
            log::warn!("Tray action failed: {e}");
            self.status(&format!("Error: {e}"));
        }
    }
    fn handle(self: &Arc<Self>, action: Action) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let command = match action {
            Action::Pause => {
                state.paused = !state.paused;
                Some(Command::Pause(state.paused))
            }
            Action::Retile => Some(Command::Retile),
            Action::Unstack => Some(Command::Unstack),
            Action::Compact => Some(Command::Compact),
            Action::Quit => Some(Command::Quit),
            Action::SaveSnapshot => Some(Command::SaveSnapshot),
            Action::LoadSnapshot(id) => {
                crate::snapshots::load(&id)?;
                *self.requested.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
                Some(Command::LoadSnapshot(id))
            }
            Action::StartupSnapshot(id) => {
                if let Some(id) = &id {
                    crate::snapshots::load(id)?;
                }
                let mut next = state.settings.clone();
                next.startup_snapshot = id;
                crate::settings::save(&self.path, &next)?;
                state.settings = next;
                None
            }
            Action::Gap(g) if (0..=64).contains(&g) => {
                let mut next = state.settings.clone();
                next.gap = g;
                crate::settings::save(&self.path, &next)?;
                state.settings = next;
                Some(Command::Gap(g))
            }
            Action::Gap(_) => return Err("Gap must be between 0 and 64".into()),
            Action::Autostart => {
                let next = !state.autostart;
                crate::autostart::set(next, &self.exe)?;
                state.autostart = next;
                None
            }
            Action::AutomaticUpdates => {
                let mut next = state.settings.clone();
                next.automatic_updates = !next.automatic_updates;
                crate::settings::save(&self.path, &next)?;
                state.settings = next;
                None
            }
            Action::CheckUpdates => {
                drop(state);
                crate::updater::check_background(self.clone());
                return Ok(());
            }
            Action::OpenSettings => {
                crate::settings::save(&self.path, &state.settings)?;
                #[cfg(target_os = "linux")]
                std::process::Command::new("xdg-open")
                    .arg(self.path.parent().ok_or("No settings directory")?)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                #[cfg(windows)]
                std::process::Command::new("explorer.exe")
                    .arg(self.path.parent().ok_or("No settings directory")?)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                None
            }
        };
        if let Some(command) = command {
            self.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(command);
        }
        Ok(())
    }
}
pub fn current() -> Option<&'static Arc<Controller>> {
    INSTANCE.get()
}
pub fn paused() -> bool {
    current().is_some_and(|c| c.state().paused)
}
pub fn drain() -> Vec<Command> {
    current().map(|c| c.queue.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()).unwrap_or_default()
}
