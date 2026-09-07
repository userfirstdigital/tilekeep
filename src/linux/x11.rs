use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::randr;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt, EventMask, GrabMode, MapState, ModMask,
    Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

use crate::engine::{Desktop, DropEffect, MonitorId, RESIZE_TOLERANCE_PX};
use crate::geometry::{is_pure_move, Point, Rect};
use crate::tree::{AppIdentity, WindowId};

const POLL_INTERVAL: Duration = Duration::from_millis(16);
const REFRESH_INTERVAL: Duration = Duration::from_millis(250);

use super::Options;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Hotkey {
    Compact,
    Retile,
    ToggleFloat,
    StackNext,
    StackPrev,
    Quit,
}

impl Hotkey {
    const ALL: [(Hotkey, u32); 6] = [
        (Hotkey::Compact, b'K' as u32),
        (Hotkey::Retile, b'L' as u32),
        (Hotkey::ToggleFloat, b'F' as u32),
        (Hotkey::StackNext, b'N' as u32),
        (Hotkey::StackPrev, b'B' as u32),
        (Hotkey::Quit, b'Q' as u32),
    ];

    fn label(self) -> &'static str {
        match self {
            Hotkey::Compact => "Super+Shift+K  compact the monitor under the cursor",
            Hotkey::Retile => "Super+Shift+L  re-read monitors and re-apply layout",
            Hotkey::ToggleFloat => "Super+Shift+F  toggle floating for the active window",
            Hotkey::StackNext => "Super+Shift+N  next window in the focused stack",
            Hotkey::StackPrev => "Super+Shift+B  previous window in the focused stack",
            Hotkey::Quit => "Super+Shift+Q  quit",
        }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    window: WindowId,
    start: Rect,
}

struct Atoms {
    active_window: Atom,
    allowed_actions: Atom,
    action_resize: Atom,
    client_list_stacking: Atom,
    current_desktop: Atom,
    frame_extents: Atom,
    moveresize_window: Atom,
    net_wm_desktop: Atom,
    net_wm_name: Atom,
    net_wm_state: Atom,
    state_hidden: Atom,
    state_max_horz: Atom,
    state_max_vert: Atom,
    supporting_wm_check: Atom,
    utf8_string: Atom,
    window_type: Atom,
    type_desktop: Atom,
    type_dialog: Atom,
    type_dock: Atom,
    type_menu: Atom,
    type_normal: Atom,
    type_notification: Atom,
    type_splash: Atom,
    type_toolbar: Atom,
    workarea: Atom,
    wm_class: Atom,
    wm_name: Atom,
    wm_transient_for: Atom,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Result<Self, String> {
        macro_rules! atom {
            ($name:literal) => {
                conn.intern_atom(false, $name.as_bytes()).map_err(err)?.reply().map_err(err)?.atom
            };
        }
        Ok(Self {
            active_window: atom!("_NET_ACTIVE_WINDOW"),
            allowed_actions: atom!("_NET_WM_ALLOWED_ACTIONS"),
            action_resize: atom!("_NET_WM_ACTION_RESIZE"),
            client_list_stacking: atom!("_NET_CLIENT_LIST_STACKING"),
            current_desktop: atom!("_NET_CURRENT_DESKTOP"),
            frame_extents: atom!("_NET_FRAME_EXTENTS"),
            moveresize_window: atom!("_NET_MOVERESIZE_WINDOW"),
            net_wm_desktop: atom!("_NET_WM_DESKTOP"),
            net_wm_name: atom!("_NET_WM_NAME"),
            net_wm_state: atom!("_NET_WM_STATE"),
            state_hidden: atom!("_NET_WM_STATE_HIDDEN"),
            state_max_horz: atom!("_NET_WM_STATE_MAXIMIZED_HORZ"),
            state_max_vert: atom!("_NET_WM_STATE_MAXIMIZED_VERT"),
            supporting_wm_check: atom!("_NET_SUPPORTING_WM_CHECK"),
            utf8_string: atom!("UTF8_STRING"),
            window_type: atom!("_NET_WM_WINDOW_TYPE"),
            type_desktop: atom!("_NET_WM_WINDOW_TYPE_DESKTOP"),
            type_dialog: atom!("_NET_WM_WINDOW_TYPE_DIALOG"),
            type_dock: atom!("_NET_WM_WINDOW_TYPE_DOCK"),
            type_menu: atom!("_NET_WM_WINDOW_TYPE_MENU"),
            type_normal: atom!("_NET_WM_WINDOW_TYPE_NORMAL"),
            type_notification: atom!("_NET_WM_WINDOW_TYPE_NOTIFICATION"),
            type_splash: atom!("_NET_WM_WINDOW_TYPE_SPLASH"),
            type_toolbar: atom!("_NET_WM_WINDOW_TYPE_TOOLBAR"),
            workarea: atom!("_NET_WORKAREA"),
            wm_class: AtomEnum::WM_CLASS.into(),
            wm_name: AtomEnum::WM_NAME.into(),
            wm_transient_for: AtomEnum::WM_TRANSIENT_FOR.into(),
        })
    }
}

struct X11 {
    conn: RustConnection,
    root: Window,
    screen_num: usize,
    atoms: Atoms,
}

impl X11 {
    fn connect() -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(err)?;
        let root = conn.setup().roots[screen_num].root;
        let atoms = Atoms::new(&conn)?;
        let wm = property_u32(&conn, root, atoms.supporting_wm_check, AtomEnum::WINDOW.into()).unwrap_or_default();
        if wm.is_empty() {
            return Err("the X display has no EWMH-compatible window manager".into());
        }
        Ok(Self { conn, root, screen_num, atoms })
    }

    fn window_id(window: Window) -> WindowId {
        WindowId(window as isize)
    }

    fn window(id: WindowId) -> Window {
        id.0 as Window
    }

    fn windows(&self) -> Vec<Window> {
        property_u32(&self.conn, self.root, self.atoms.client_list_stacking, AtomEnum::WINDOW.into())
            .unwrap_or_default()
    }

    fn active_window(&self) -> Option<Window> {
        property_u32(&self.conn, self.root, self.atoms.active_window, AtomEnum::WINDOW.into())
            .and_then(|v| v.first().copied())
            .filter(|w| *w != NONE)
    }

    fn property_atoms(&self, window: Window, property: Atom) -> Vec<Atom> {
        property_u32(&self.conn, window, property, AtomEnum::ATOM.into()).unwrap_or_default()
    }

    fn property_text(&self, window: Window, property: Atom, kind: Atom) -> Option<String> {
        let reply = self.conn.get_property(false, window, property, kind, 0, u32::MAX).ok()?.reply().ok()?;
        let text = String::from_utf8_lossy(&reply.value).trim_matches(char::from(0)).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn title(&self, window: Window) -> Option<String> {
        self.property_text(window, self.atoms.net_wm_name, self.atoms.utf8_string)
            .or_else(|| self.property_text(window, self.atoms.wm_name, AtomEnum::STRING.into()))
    }

    fn identity(&self, window: Window) -> Option<AppIdentity> {
        let reply = self
            .conn
            .get_property(false, window, self.atoms.wm_class, AtomEnum::STRING, 0, u32::MAX)
            .ok()?
            .reply()
            .ok()?;
        let parts: Vec<&[u8]> = reply.value.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
        let value = parts.last().or_else(|| parts.first())?;
        Some(AppIdentity(String::from_utf8_lossy(value).trim().to_lowercase()))
    }

    fn current_desktop(&self) -> Option<u32> {
        property_u32(&self.conn, self.root, self.atoms.current_desktop, AtomEnum::CARDINAL.into())
            .and_then(|v| v.first().copied())
    }

    fn on_current_desktop(&self, window: Window) -> bool {
        let Some(current) = self.current_desktop() else { return true };
        property_u32(&self.conn, window, self.atoms.net_wm_desktop, AtomEnum::CARDINAL.into())
            .and_then(|v| v.first().copied())
            .is_none_or(|desktop| desktop == current || desktop == u32::MAX)
    }

    fn is_tileable(&self, window: Window) -> bool {
        let Ok(cookie) = self.conn.get_window_attributes(window) else { return false };
        let Ok(attrs) = cookie.reply() else { return false };
        if attrs.override_redirect || attrs.map_state == MapState::UNMAPPED || self.title(window).is_none() {
            return false;
        }
        if property_u32(&self.conn, window, self.atoms.wm_transient_for, AtomEnum::WINDOW.into())
            .is_some_and(|v| !v.is_empty())
        {
            return false;
        }
        let types = self.property_atoms(window, self.atoms.window_type);
        let rejected = [
            self.atoms.type_desktop,
            self.atoms.type_dialog,
            self.atoms.type_dock,
            self.atoms.type_menu,
            self.atoms.type_notification,
            self.atoms.type_splash,
            self.atoms.type_toolbar,
        ];
        if types.iter().any(|t| rejected.contains(t)) || (!types.is_empty() && !types.contains(&self.atoms.type_normal))
        {
            return false;
        }
        let actions = self.property_atoms(window, self.atoms.allowed_actions);
        actions.is_empty() || actions.contains(&self.atoms.action_resize)
    }

    fn states(&self, window: Window) -> Vec<Atom> {
        self.property_atoms(window, self.atoms.net_wm_state)
    }

    fn should_place(&self, window: Window) -> bool {
        if !self.on_current_desktop(window) {
            return false;
        }
        let states = self.states(window);
        !states.contains(&self.atoms.state_hidden)
            && !states.contains(&self.atoms.state_max_horz)
            && !states.contains(&self.atoms.state_max_vert)
    }

    fn rect(&self, window: Window) -> Option<Rect> {
        let geometry = self.conn.get_geometry(window).ok()?.reply().ok()?;
        let translated = self.conn.translate_coordinates(window, self.root, 0, 0).ok()?.reply().ok()?;
        let ext = property_u32(&self.conn, window, self.atoms.frame_extents, AtomEnum::CARDINAL.into())
            .filter(|v| v.len() >= 4)
            .unwrap_or_else(|| vec![0; 4]);
        let left = ext[0] as i32;
        let right = ext[1] as i32;
        let top = ext[2] as i32;
        let bottom = ext[3] as i32;
        Some(Rect::new(
            translated.dst_x as i32 - left,
            translated.dst_y as i32 - top,
            geometry.width as i32 + left + right,
            geometry.height as i32 + top + bottom,
        ))
    }

    fn describe(&self, window: Window) -> String {
        let title = self.title(window).unwrap_or_default();
        let class = self.identity(window).map(|i| i.0).unwrap_or_default();
        let rect = self.rect(window).unwrap_or_default();
        format!("[{class}] \"{title}\" @ {},{} {}x{}", rect.x, rect.y, rect.w, rect.h)
    }

    fn place(&self, window: Window, rect: Rect, raise: bool) -> Result<(), String> {
        // EWMH _NET_MOVERESIZE_WINDOW uses the decorated, root-relative rectangle. Window
        // managers are free to reparent clients, so ConfigureWindow alone is not sufficient.
        let flags = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12);
        let data = ClientMessageData::from([flags, rect.x as u32, rect.y as u32, rect.w as u32, rect.h as u32]);
        let event = ClientMessageEvent::new(32, window, self.atoms.moveresize_window, data);
        self.conn
            .send_event(false, self.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)
            .map_err(err)?;
        if raise {
            self.activate(window)?;
        }
        self.conn.flush().map_err(err)
    }

    fn activate(&self, window: Window) -> Result<(), String> {
        let event = ClientMessageEvent::new(
            32,
            window,
            self.atoms.active_window,
            ClientMessageData::from([2, CURRENT_TIME, self.active_window().unwrap_or(NONE), 0, 0]),
        );
        self.conn
            .send_event(false, self.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)
            .map_err(err)?;
        self.conn.flush().map_err(err)
    }

    fn pointer(&self) -> Option<(Point, bool, bool)> {
        let p = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let mask: u16 = p.mask.into();
        Some((
            Point { x: p.root_x as i32, y: p.root_y as i32 },
            mask & u16::from(x11rb::protocol::xproto::KeyButMask::BUTTON1) != 0,
            mask & u16::from(x11rb::protocol::xproto::KeyButMask::CONTROL) != 0,
        ))
    }

    fn monitors(&self) -> Vec<(MonitorId, Rect)> {
        let mut monitors = randr::get_monitors(&self.conn, self.root, true)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|reply| {
                reply
                    .monitors
                    .into_iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let id = if m.name == NONE { i as isize + 1 } else { m.name as isize };
                        (MonitorId(id), Rect::new(m.x as i32, m.y as i32, m.width as i32, m.height as i32))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if monitors.is_empty() {
            let screen = &self.conn.setup().roots[self.screen_num];
            monitors.push((
                MonitorId(self.root as isize),
                Rect::new(0, 0, screen.width_in_pixels.into(), screen.height_in_pixels.into()),
            ));
        }
        if let Some(work) = self.work_area() {
            for (_, monitor) in &mut monitors {
                if let Some(clipped) = intersection(*monitor, work) {
                    *monitor = clipped;
                }
            }
        }
        monitors
    }

    fn work_area(&self) -> Option<Rect> {
        let desktop = self.current_desktop().unwrap_or(0) as usize;
        let values = property_u32(&self.conn, self.root, self.atoms.workarea, AtomEnum::CARDINAL.into())?;
        let offset = desktop.checked_mul(4)?;
        let v = values.get(offset..offset + 4)?;
        Some(Rect::new(v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32))
    }

    fn monitor_of(&self, window: Window, monitors: &[(MonitorId, Rect)]) -> Option<MonitorId> {
        let rect = self.rect(window)?;
        monitor_nearest(Point { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 }, monitors)
    }

    fn register_hotkeys(&self) -> Result<HashMap<u8, Hotkey>, String> {
        let setup = self.conn.setup();
        let count = setup.max_keycode - setup.min_keycode + 1;
        let mapping = self.conn.get_keyboard_mapping(setup.min_keycode, count).map_err(err)?.reply().map_err(err)?;
        let per = mapping.keysyms_per_keycode as usize;
        let mut keys = HashMap::new();
        for (hotkey, keysym) in Hotkey::ALL {
            let code = mapping
                .keysyms
                .chunks(per)
                .position(|symbols| symbols.contains(&keysym) || symbols.contains(&(keysym + 32)))
                .map(|i| setup.min_keycode + i as u8);
            let Some(code) = code else {
                log::warn!("{} — key not found in the current keyboard map", hotkey.label());
                continue;
            };
            let base = ModMask::M4 | ModMask::SHIFT;
            let mut registered = true;
            for modifiers in [base, base | ModMask::LOCK, base | ModMask::M2, base | ModMask::LOCK | ModMask::M2] {
                match self.conn.grab_key(false, self.root, modifiers, code, GrabMode::ASYNC, GrabMode::ASYNC) {
                    Ok(cookie) => {
                        if cookie.check().is_err() {
                            registered = false;
                        }
                    }
                    Err(_) => registered = false,
                }
            }
            if registered {
                keys.insert(code, hotkey);
                log::info!("{}", hotkey.label());
            } else {
                let _ = self.conn.ungrab_key(code, self.root, ModMask::ANY);
                log::warn!("{} — NOT registered; another program owns this chord", hotkey.label());
            }
        }
        self.conn.flush().map_err(err)?;
        Ok(keys)
    }
}

struct App {
    x: X11,
    desktop: Desktop,
    opts: Options,
    known: HashSet<WindowId>,
    hotkeys: HashMap<u8, Hotkey>,
    drag: Option<Drag>,
    button_down: bool,
    last_refresh: Instant,
    restore: Option<crate::snapshots::Restore>,
}

impl App {
    fn new(x: X11, opts: Options) -> Result<Self, String> {
        let hotkeys = x.register_hotkeys()?;
        Ok(Self {
            restore: None,
            x,
            desktop: Desktop::new(opts.gap),
            opts,
            known: HashSet::new(),
            hotkeys,
            drag: None,
            button_down: false,
            last_refresh: Instant::now(),
        })
    }

    fn bootstrap(&mut self) {
        self.desktop.sync_monitors(&self.x.monitors());
        self.refresh_windows();
        self.apply();
    }

    fn refresh_windows(&mut self) {
        let windows = self.x.windows();
        let present: HashSet<WindowId> = windows.iter().copied().map(X11::window_id).collect();
        for gone in self.known.difference(&present).copied().collect::<Vec<_>>() {
            if self.desktop.window_vanished(gone) {
                log::info!("vanished {gone:?}; its slot stays empty");
            }
        }
        self.known = present;
        let monitors = self.x.monitors();
        self.desktop.sync_monitors(&monitors);
        let active = self.x.active_window().map(X11::window_id);
        let mut changed = false;
        for window in windows {
            let id = X11::window_id(window);
            if !self.desktop.contains(id) && !self.desktop.is_floating(id) && self.x.is_tileable(window) {
                if let Some(monitor) = self.x.monitor_of(window, &monitors) {
                    if self.desktop.window_appeared(id, monitor, self.x.identity(window)) {
                        log::info!("tracking {}", self.x.describe(window));
                        changed = true;
                    }
                }
            }
        }
        if let Some(active) = active {
            self.desktop.focus_changed(active);
        }
        if changed && self.drag.is_none() {
            self.apply();
        }
    }

    fn apply(&mut self) {
        if crate::control::paused() {
            return;
        }
        for placement in self.desktop.placements() {
            let window = X11::window(placement.window);
            if !self.known.contains(&placement.window) || !self.x.should_place(window) {
                continue;
            }
            if self.opts.dry_run {
                log::info!("dry-run: would place {} at {:?}", self.x.describe(window), placement.rect);
            } else if let Err(e) = self.x.place(window, placement.rect, placement.active) {
                log::warn!("place {} failed: {e}", self.x.describe(window));
            }
        }
    }

    fn update_drag(&mut self) {
        if crate::control::paused() {
            self.drag = None;
            self.button_down = false;
            return;
        }
        let Some((point, down, ctrl)) = self.x.pointer() else { return };
        if down && !self.button_down {
            if let Some(window) = self.x.active_window() {
                let id = X11::window_id(window);
                if !self.desktop.is_floating(id) && (self.desktop.contains(id) || self.x.is_tileable(window)) {
                    if let Some(start) = self.x.rect(window) {
                        self.drag = Some(Drag { window: id, start });
                    }
                }
            }
        } else if !down && self.button_down {
            if let Some(drag) = self.drag.take() {
                self.finish_drag(drag, point, ctrl);
            }
        }
        self.button_down = down;
    }

    fn finish_drag(&mut self, drag: Drag, point: Point, ctrl: bool) {
        let window = X11::window(drag.window);
        let Some(after) = self.x.rect(window) else { return };
        if is_pure_move(drag.start, after, RESIZE_TOLERANCE_PX) {
            let effect = self.desktop.drop_window(drag.window, point, ctrl);
            log::info!("drop {} at {:?} -> {:?}", self.x.describe(window), point, effect);
            if effect == DropEffect::Ignored && !self.desktop.contains(drag.window) {
                return;
            }
        } else {
            let changed = self.desktop.window_resized(drag.window, drag.start, after);
            log::info!("resize {} {:?} -> {:?} (ratio changed: {changed})", self.x.describe(window), drag.start, after);
        }
        self.apply();
    }

    fn hotkey(&mut self, hotkey: Hotkey) -> bool {
        if crate::control::paused() && hotkey != Hotkey::Quit {
            return true;
        }
        log::info!("hotkey: {}", hotkey.label());
        let active = self.x.active_window().map(X11::window_id);
        match hotkey {
            Hotkey::Quit => return false,
            Hotkey::Retile => {
                self.desktop.sync_monitors(&self.x.monitors());
                self.apply();
            }
            Hotkey::Compact => {
                let monitors = self.x.monitors();
                let target = self
                    .x
                    .pointer()
                    .and_then(|(p, _, _)| monitor_nearest(p, &monitors))
                    .or_else(|| active.and_then(|w| self.desktop.monitor_of(w)));
                if let Some(monitor) = target {
                    self.desktop.compact(monitor);
                    self.apply();
                }
            }
            Hotkey::ToggleFloat => {
                if let Some(id) = active {
                    let monitors = self.x.monitors();
                    if let Some(monitor) = self.x.monitor_of(X11::window(id), &monitors) {
                        let floating = self.desktop.toggle_float(id, monitor);
                        log::info!(
                            "{} is now {}",
                            self.x.describe(X11::window(id)),
                            if floating { "floating" } else { "tiled" }
                        );
                        self.apply();
                    }
                }
            }
            Hotkey::StackNext | Hotkey::StackPrev => {
                let delta = if hotkey == Hotkey::StackNext { 1 } else { -1 };
                if let Some(id) = self.desktop.cycle_stack(delta) {
                    self.apply();
                    if self.opts.dry_run {
                        log::info!("dry-run: would activate {}", self.x.describe(X11::window(id)));
                    } else if let Err(e) = self.x.activate(X11::window(id)) {
                        log::warn!("activate {} failed: {e}", self.x.describe(X11::window(id)));
                    }
                }
            }
        }
        true
    }

    fn run(mut self) -> Result<(), String> {
        self.bootstrap();
        if let Some(c) = crate::control::current() {
            c.status("running");
        }
        loop {
            crate::tray::poll();
            for command in crate::control::drain() {
                use crate::control::Command as C;
                match command {
                    C::Quit => return Ok(()),
                    C::Pause(value) => {
                        self.drag = None;
                        if !value {
                            self.apply();
                        }
                    }
                    C::Gap(g) => {
                        self.desktop.set_gap(g);
                        self.opts.gap = g;
                        self.apply();
                    }
                    C::Retile => {
                        self.hotkey(Hotkey::Retile);
                    }
                    C::Compact => {
                        self.hotkey(Hotkey::Compact);
                    }
                    C::SaveSnapshot => match crate::snapshots::save(self.desktop.snapshot(self.snapshot_windows())) {
                        Ok(id) => {
                            if let Some(c) = crate::control::current() {
                                c.snapshots_changed(&format!("Snapshot {id} saved"));
                            }
                        }
                        Err(e) => log::warn!("Snapshot: {e}"),
                    },
                    C::LoadSnapshot(id) => match crate::snapshots::Restore::new(&id, &self.snapshot_windows()) {
                        Ok(r) => self.restore = Some(r),
                        Err(e) => log::warn!("Snapshot: {e}"),
                    },
                }
            }
            while let Some(event) = self.x.conn.poll_for_event().map_err(err)? {
                if let Event::KeyPress(key) = event {
                    if let Some(hotkey) = self.hotkeys.get(&key.detail).copied() {
                        if !self.hotkey(hotkey) {
                            return Ok(());
                        }
                    }
                }
            }
            self.update_drag();
            if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
                self.refresh_windows();
                if self.drag.is_none() {
                    self.restore_snapshot();
                }
                self.last_refresh = Instant::now();
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
    fn snapshot_windows(&self) -> Vec<crate::snapshots::AppWindow> {
        let pid_atom = self.x.conn.intern_atom(false, b"_NET_WM_PID").ok().and_then(|c| c.reply().ok()).map(|r| r.atom);
        self.x
            .windows()
            .into_iter()
            .filter(|w| self.x.is_tileable(*w))
            .filter_map(|w| {
                let id = X11::window_id(w);
                let pid = pid_atom
                    .and_then(|a| property_u32(&self.x.conn, w, a, AtomEnum::CARDINAL.into()))
                    .and_then(|p| p.first().copied())
                    .unwrap_or(0);
                Some(crate::snapshots::AppWindow {
                    token: id.0.to_string(),
                    app: self.x.identity(w).map(|a| a.0).unwrap_or_default(),
                    title: self.x.title(w).unwrap_or_default(),
                    pid,
                    executable: std::fs::read_link(format!("/proc/{pid}/exe")).ok(),
                    rect: self.x.rect(w)?,
                    floating: self.desktop.is_floating(id),
                })
            })
            .collect()
    }
    fn restore_snapshot(&mut self) {
        if let Some(mut restore) = self.restore.take() {
            let live = self.snapshot_windows();
            match restore.poll(&mut self.desktop, &live) {
                Ok(true) => {
                    self.apply();
                    let matched = crate::snapshots::matching(&restore.snapshot.windows, &live);
                    for w in restore.snapshot.windows.iter().filter(|w| w.floating) {
                        if let Some(id) = matched.get(&w.token).and_then(|s| s.parse::<isize>().ok()) {
                            let _ = self.x.place(X11::window(WindowId(id)), w.rect, false);
                        }
                    }
                }
                Ok(false) => (),
                Err(e) => log::warn!("Snapshot: {e}"),
            }
            if !restore.done {
                self.restore = Some(restore);
            }
        }
    }
}

pub fn run(opts: Options) -> Result<(), String> {
    App::new(X11::connect()?, opts)?.run()
}

pub fn list() -> Result<(), String> {
    let x = X11::connect()?;
    let monitors = x.monitors();
    println!("Monitors (work areas):");
    for (id, rect) in &monitors {
        println!("  {:?} @ {},{} {}x{}", id, rect.x, rect.y, rect.w, rect.h);
    }
    println!("Tileable X11 windows:");
    for window in x.windows().into_iter().filter(|w| x.is_tileable(*w)) {
        println!("  {:?} {}", x.monitor_of(window, &monitors), x.describe(window));
    }
    Ok(())
}

fn property_u32(conn: &RustConnection, window: Window, property: Atom, kind: Atom) -> Option<Vec<u32>> {
    conn.get_property(false, window, property, kind, 0, u32::MAX).ok()?.reply().ok()?.value32().map(Iterator::collect)
}

fn monitor_nearest(point: Point, monitors: &[(MonitorId, Rect)]) -> Option<MonitorId> {
    monitors
        .iter()
        .min_by_key(|(_, rect)| {
            let dx = if point.x < rect.x {
                rect.x - point.x
            } else if point.x >= rect.right() {
                point.x - rect.right() + 1
            } else {
                0
            };
            let dy = if point.y < rect.y {
                rect.y - point.y
            } else if point.y >= rect.bottom() {
                point.y - rect.bottom() + 1
            } else {
                0
            };
            dx as i64 * dx as i64 + dy as i64 * dy as i64
        })
        .map(|(id, _)| *id)
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_clips_a_monitor_to_the_desktop_work_area() {
        assert_eq!(
            intersection(Rect::new(0, 0, 1920, 1080), Rect::new(0, 0, 3840, 1040)),
            Some(Rect::new(0, 0, 1920, 1040))
        );
        assert_eq!(intersection(Rect::new(0, 0, 10, 10), Rect::new(20, 20, 5, 5)), None);
    }

    #[test]
    fn monitor_selection_uses_containment_then_nearest() {
        let monitors = [(MonitorId(1), Rect::new(0, 0, 100, 100)), (MonitorId(2), Rect::new(100, 0, 100, 100))];
        assert_eq!(monitor_nearest(Point { x: 150, y: 50 }, &monitors), Some(MonitorId(2)));
        assert_eq!(monitor_nearest(Point { x: -20, y: 50 }, &monitors), Some(MonitorId(1)));
    }
}
