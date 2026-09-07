use crate::control::{Action, Controller, State};
use std::{cell::RefCell, sync::Arc};
const GAPS: [i32; 9] = [0, 1, 2, 4, 6, 8, 10, 16, 32];
fn title(s: &State) -> String {
    format!("Tilekeep — tiling window manager ({})", if s.paused { "paused" } else { &s.status })
}
// Code-native four-tile icon: no external image or theme dependency.
fn pixels(paused: bool) -> Vec<u8> {
    let mut p = vec![0; 32 * 32 * 4];
    for y in 3..29 {
        for x in 3..29 {
            if (15..17).contains(&x) || (15..17).contains(&y) {
                continue;
            }
            let color = if paused {
                [150, 150, 150, 255]
            } else if x < 16 && y < 16 {
                [70, 200, 150, 255]
            } else {
                [80, 150, 245, 255]
            };
            p[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4].copy_from_slice(&color);
        }
    }
    p
}
thread_local! {static TRAY:RefCell<Option<PlatformTray>>=const {RefCell::new(None)};}
pub fn start(c: Arc<Controller>) -> Result<(), String> {
    let tray = PlatformTray::new(c)?;
    TRAY.with(|t| *t.borrow_mut() = Some(tray));
    Ok(())
}
pub fn poll() {
    TRAY.with(|t| {
        if let Some(t) = t.borrow_mut().as_mut() {
            t.poll();
        }
    });
}
pub fn stop() {
    TRAY.with(|t| *t.borrow_mut() = None);
}

#[cfg(target_os = "linux")]
struct LinuxMenu {
    control: Arc<Controller>,
    state: State,
}
#[cfg(target_os = "linux")]
impl LinuxMenu {
    fn item(label: &str, action: Action) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label: label.into(),
            activate: Box::new(move |s: &mut Self| s.control.action(action.clone())),
            ..Default::default()
        }
        .into()
    }
    fn check(label: &str, checked: bool, action: Action) -> ksni::MenuItem<Self> {
        ksni::menu::CheckmarkItem {
            label: label.into(),
            checked,
            activate: Box::new(move |s: &mut Self| s.control.action(action.clone())),
            ..Default::default()
        }
        .into()
    }
}
#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxMenu {
    fn id(&self) -> String {
        "com.userfirst.tilekeep".into()
    }
    fn title(&self) -> String {
        title(&self.state)
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let mut data = pixels(self.state.paused);
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon { width: 32, height: 32, data }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            description: format!(
                "v{} · {} px gap\n{}",
                env!("CARGO_PKG_VERSION"),
                self.state.settings.gap,
                self.state.update
            ),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.control.action(Action::Retile);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::{menu::*, MenuItem};
        let s = &self.state;
        vec![
            StandardItem {
                label: format!(
                    "Tilekeep {} — {}",
                    env!("CARGO_PKG_VERSION"),
                    if s.paused { "Paused" } else { &s.status }
                ),
                enabled: false,
                ..Default::default()
            }
            .into(),
            Self::check("Pause tiling", s.paused, Action::Pause),
            Self::item("Retile", Action::Retile),
            Self::item("Compact this monitor", Action::Compact),
            SubMenu {
                label: format!("Window gap: {} px", s.settings.gap),
                submenu: GAPS
                    .iter()
                    .map(|&g| Self::check(&format!("{g} px"), s.settings.gap == g, Action::Gap(g)))
                    .collect(),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            Self::item("Save snapshot now", Action::SaveSnapshot),
            SubMenu {
                label: "Load snapshot".into(),
                enabled: !s.snapshots.is_empty(),
                submenu: s
                    .snapshots
                    .iter()
                    .map(|id| Self::item(&format!("Snapshot {id}"), Action::LoadSnapshot(id.clone())))
                    .collect(),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Snapshot at startup".into(),
                submenu: std::iter::once(Self::check(
                    "None",
                    s.settings.startup_snapshot.is_none(),
                    Action::StartupSnapshot(None),
                ))
                .chain(s.snapshots.iter().map(|id| {
                    Self::check(
                        &format!("Snapshot {id}"),
                        s.settings.startup_snapshot.as_ref() == Some(id),
                        Action::StartupSnapshot(Some(id.clone())),
                    )
                }))
                .collect(),
                ..Default::default()
            }
            .into(),
            Self::check("Start with Linux", s.autostart, Action::Autostart),
            Self::check(
                "Silent updates (install on next launch)",
                s.settings.automatic_updates,
                Action::AutomaticUpdates,
            ),
            Self::item("Check for updates", Action::CheckUpdates),
            StandardItem { label: s.update.clone(), enabled: false, ..Default::default() }.into(),
            Self::item("Open settings folder", Action::OpenSettings),
            MenuItem::Separator,
            Self::item("Quit Tilekeep", Action::Quit),
        ]
    }
}
#[cfg(target_os = "linux")]
struct PlatformTray {
    handle: ksni::blocking::Handle<LinuxMenu>,
    control: Arc<Controller>,
    last: String,
}
#[cfg(target_os = "linux")]
impl PlatformTray {
    fn new(control: Arc<Controller>) -> Result<Self, String> {
        use ksni::blocking::TrayMethods;
        let menu = LinuxMenu { state: control.state(), control: control.clone() };
        let handle = menu.spawn().map_err(|e| e.to_string())?;
        Ok(Self { handle, control, last: String::new() })
    }
    fn poll(&mut self) {
        let state = self.control.state();
        let fingerprint = format!("{state:?}");
        if self.last != fingerprint {
            self.last = fingerprint;
            self.handle.update(move |t| t.state = state);
        }
    }
}
#[cfg(target_os = "linux")]
impl Drop for PlatformTray {
    fn drop(&mut self) {
        self.handle.shutdown();
    }
}

#[cfg(windows)]
struct PlatformTray {
    icon: tray_icon::TrayIcon,
    control: Arc<Controller>,
    actions: Vec<(tray_icon::menu::MenuId, Action)>,
    last: String,
}
#[cfg(windows)]
impl PlatformTray {
    fn new(control: Arc<Controller>) -> Result<Self, String> {
        let icon = tray_icon::TrayIconBuilder::new()
            .with_tooltip(title(&control.state()))
            .with_icon(tray_icon::Icon::from_rgba(pixels(false), 32, 32).map_err(|e| e.to_string())?)
            .build()
            .map_err(|e| e.to_string())?;
        let mut tray = Self { icon, control, actions: Vec::new(), last: String::new() };
        tray.poll();
        Ok(tray)
    }
    fn poll(&mut self) {
        use tray_icon::menu::*;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some((_, a)) = self.actions.iter().find(|(id, _)| *id == event.id) {
                self.control.action(a.clone());
            }
        }
        let s = self.control.state();
        let fingerprint = format!("{s:?}");
        if self.last == fingerprint {
            return;
        }
        self.last = fingerprint;
        let menu = Menu::new();
        self.actions.clear();
        let items = [
            (format!("Tilekeep {}", env!("CARGO_PKG_VERSION")), None, None),
            ("Pause tiling".into(), Some(Action::Pause), Some(s.paused)),
            ("Retile".into(), Some(Action::Retile), None),
            ("Compact this monitor".into(), Some(Action::Compact), None),
            ("Start with Windows".into(), Some(Action::Autostart), Some(s.autostart)),
            ("Silent updates (next launch)".into(), Some(Action::AutomaticUpdates), Some(s.settings.automatic_updates)),
            ("Check for updates".into(), Some(Action::CheckUpdates), None),
            (s.update.clone(), None, None),
            ("Open settings folder".into(), Some(Action::OpenSettings), None),
        ];
        for (text, action, checked) in items {
            let id = if let Some(checked) = checked {
                let item = CheckMenuItem::new(text, true, checked, None);
                let _ = menu.append(&item);
                item.id().clone()
            } else {
                let item = MenuItem::new(text, action.is_some(), None);
                let _ = menu.append(&item);
                item.id().clone()
            };
            if let Some(action) = action {
                self.actions.push((id, action));
            }
        }
        let gaps = Submenu::new(format!("Window gap: {} px", s.settings.gap), true);
        for g in GAPS {
            let item = CheckMenuItem::new(format!("{g} px"), true, s.settings.gap == g, None);
            let _ = gaps.append(&item);
            self.actions.push((item.id().clone(), Action::Gap(g)));
        }
        let _ = menu.append(&gaps);
        let save = MenuItem::new("Save snapshot now", true, None);
        let _ = menu.append(&save);
        self.actions.push((save.id().clone(), Action::SaveSnapshot));
        let load = Submenu::new("Load snapshot", !s.snapshots.is_empty());
        let startup = Submenu::new("Snapshot at startup", true);
        let none = CheckMenuItem::new("None", true, s.settings.startup_snapshot.is_none(), None);
        let _ = startup.append(&none);
        self.actions.push((none.id().clone(), Action::StartupSnapshot(None)));
        for id in &s.snapshots {
            let item = MenuItem::new(format!("Snapshot {id}"), true, None);
            let _ = load.append(&item);
            self.actions.push((item.id().clone(), Action::LoadSnapshot(id.clone())));
            let item = CheckMenuItem::new(
                format!("Snapshot {id}"),
                true,
                s.settings.startup_snapshot.as_ref() == Some(id),
                None,
            );
            let _ = startup.append(&item);
            self.actions.push((item.id().clone(), Action::StartupSnapshot(Some(id.clone()))));
        }
        let _ = menu.append(&load);
        let _ = menu.append(&startup);
        let quit = MenuItem::new("Quit Tilekeep", true, None);
        let _ = menu.append(&quit);
        self.actions.push((quit.id().clone(), Action::Quit));
        self.icon.set_menu(Some(Box::new(menu)));
        let _ = self.icon.set_tooltip(Some(title(&s)));
        if let Ok(icon) = tray_icon::Icon::from_rgba(pixels(s.paused), 32, 32) {
            let _ = self.icon.set_icon(Some(icon));
        }
    }
}
