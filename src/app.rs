//! The message loop: turns Win32 events into `Desktop` calls and applies placements.

use windows::Win32::Foundation::GetLastError;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetForegroundWindow, GetMessageW, KillTimer, PostQuitMessage, SetForegroundWindow, SetTimer,
    TranslateMessage, MSG, WM_HOTKEY, WM_TIMER,
};

use crate::engine::{Desktop, DropEffect, RESIZE_TOLERANCE_PX};
use crate::geometry::{is_pure_move, Rect};
use crate::tree::{AppIdentity, WindowId};
use crate::win32::events::{self, EventHooks, EventKind, RawEvent};
use crate::win32::hotkeys::{self, Hotkey};
use crate::win32::overlay::Overlay;
use crate::win32::{ctrl_held, cursor_pos, dpi, hwnd, monitors, window, window_id};

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub dry_run: bool,
    pub gap: i32,
    pub float_secondary_windows: bool,
}

const PREVIEW_INTERVAL_MS: u32 = 16;

struct Drag {
    window: WindowId,
    start: Rect,
    timer: usize,
}

pub struct App {
    desktop: Desktop,
    opts: Options,
    drag: Option<Drag>,
    overlay: Option<Overlay>,
    restore: Option<crate::snapshots::Restore>,
}

/// A `WINEVENT_OUTOFCONTEXT` callback runs *inside* `GetMessageW` without it returning, so a pump
/// that only drains after a retrieved message would sit blocked with events queued behind it.
/// `hook_proc` now posts an immediate wake (`WM_APP`) after every push, so this thread timer is
/// only the fallback for whatever that wake cannot reach (e.g. it was dropped for a full queue).
const PUMP_INTERVAL_MS: u32 = 500;

pub fn run(opts: Options) -> Result<(), String> {
    dpi::enable_per_monitor_v2();
    let _hooks = EventHooks::install();
    hotkeys::register_all();
    let overlay = match Overlay::create() {
        Ok(o) => Some(o),
        Err(e) => {
            log::warn!("no drop preview: {e}");
            None
        }
    };
    let mut app = App { desktop: Desktop::new(opts.gap), opts, drag: None, overlay, restore: None };
    app.bootstrap();
    if let Some(c) = crate::control::current() {
        c.status("running");
    }

    // SAFETY: a thread timer (no window, no callback) just posts WM_TIMER to this thread's queue.
    // Passing 0 for the id lets Windows assign one; we store whatever it returns.
    let pump_timer_id = unsafe { SetTimer(None, 0, PUMP_INTERVAL_MS, None) };
    if pump_timer_id == 0 {
        log::warn!("SetTimer failed; queued window events will only be drained when another message arrives");
    }

    let mut msg = MSG::default();
    loop {
        // SAFETY: msg is a valid out-pointer; None means any window on this thread.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 == -1 {
            // A broken pump and a clean quit must not look alike: this path names itself and
            // exits non-zero, rather than returning Ok like WM_QUIT does.
            // SAFETY: plain query of this thread's last-error value.
            let err = unsafe { GetLastError() };
            log::error!("GetMessageW failed: {err:?}; shutting down");
            kill_pump_timer(pump_timer_id);
            hotkeys::unregister_all();
            return Err(format!("GetMessageW failed: {err:?}"));
        }
        if got.0 == 0 {
            break; // WM_QUIT
        }
        match msg.message {
            WM_HOTKEY => app.on_hotkey(msg.wParam.0 as i32),
            // A thread timer's WM_TIMER carries a null hwnd and the id SetTimer returned in
            // wParam; matching only that (not just WM_TIMER) tells the drag preview tick apart
            // from the pump's fallback tick, which does nothing here (the drain below covers it).
            WM_TIMER if msg.hwnd.0.is_null() && app.drag.as_ref().is_some_and(|d| d.timer == msg.wParam.0) => {
                app.on_tick()
            }
            WM_TIMER if msg.hwnd.0.is_null() && msg.wParam.0 == pump_timer_id => {}
            _ => unsafe {
                // SAFETY: msg was filled by GetMessageW.
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
        for ev in events::drain() {
            app.on_event(ev);
        }
        crate::tray::poll();
        for command in crate::control::drain() {
            use crate::control::Command as C;
            match command {
                C::Quit => unsafe { PostQuitMessage(0) }, // SAFETY: this thread's message queue.
                C::Pause(value) => {
                    app.cancel_drag("pause state changed");
                    if !value {
                        app.apply();
                    }
                }
                C::Gap(g) => {
                    app.desktop.set_gap(g);
                    app.opts.gap = g;
                    app.apply();
                }
                C::FloatSecondaryWindows(value) => app.opts.float_secondary_windows = value,
                C::Retile => app.on_hotkey(Hotkey::Retile.id()),
                C::Unstack => {
                    if !crate::control::paused() {
                        if let Some(w) = app.desktop.focused() {
                            app.desktop.unstack(w);
                            app.apply();
                        }
                    }
                }
                C::Compact => app.on_hotkey(Hotkey::Compact.id()),
                C::SaveSnapshot => match crate::snapshots::save(app.desktop.snapshot(app.snapshot_windows())) {
                    Ok(id) => {
                        if let Some(c) = crate::control::current() {
                            c.snapshots_changed(&format!("Snapshot {id} saved"));
                        }
                    }
                    Err(e) => log::warn!("Snapshot: {e}"),
                },
                C::LoadSnapshot(id) => match crate::snapshots::Restore::new(&id, &app.snapshot_windows()) {
                    Ok(r) => app.restore = Some(r),
                    Err(e) => log::warn!("Snapshot: {e}"),
                },
            }
        }
        if app.drag.is_none() {
            app.restore_snapshot();
        }
    }
    kill_pump_timer(pump_timer_id);
    hotkeys::unregister_all();
    Ok(())
}

fn kill_pump_timer(timer: usize) {
    if timer != 0 {
        // SAFETY: `timer` is the id SetTimer returned for this thread.
        unsafe {
            let _ = KillTimer(None, timer);
        }
    }
}

impl App {
    fn app_identity(h: windows::Win32::Foundation::HWND) -> Option<AppIdentity> {
        if let Some(executable) = window::executable(h) {
            return Some(AppIdentity(executable.to_string_lossy().to_lowercase()));
        }
        let class = window::query(h).class_name.to_lowercase();
        (!class.is_empty()).then_some(AppIdentity(class))
    }

    fn has_same_app_window(&self, h: windows::Win32::Foundation::HWND, identity: &AppIdentity) -> bool {
        window::enumerate_tileable().into_iter().any(|other| {
            let id = window_id(other);
            other != h
                && (self.desktop.contains(id) || self.desktop.is_floating(id))
                && Self::app_identity(other).as_ref() == Some(identity)
        })
    }

    fn snapshot_windows(&self) -> Vec<crate::snapshots::AppWindow> {
        window::enumerate_tileable()
            .into_iter()
            .filter_map(|h| {
                let executable = window::executable(h);
                let info = window::query(h);
                let id = window_id(h);
                Some(crate::snapshots::AppWindow {
                    token: id.0.to_string(),
                    app: executable.as_ref().map(|p| p.to_string_lossy().to_lowercase()).unwrap_or(info.class_name),
                    title: info.title,
                    pid: 0,
                    executable,
                    rect: window::visible_rect(h)?,
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
                            let _ = window::place(hwnd(WindowId(id)), w.rect, false);
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
    fn bootstrap(&mut self) {
        self.desktop.sync_monitors(&monitors::enumerate());
        for h in window::enumerate_tileable() {
            // TODO(task 4): pass the window's app identity so a reopened app reclaims its slot.
            if self.desktop.window_appeared(window_id(h), monitors::monitor_of(h), None) {
                log::info!("tracking {}", window::describe(h));
            }
        }
        // SAFETY: plain query.
        let fg = unsafe { GetForegroundWindow() };
        if !fg.0.is_null() {
            self.desktop.focus_changed(window_id(fg));
        }
        self.apply();
    }

    /// Push the engine's placements to the real windows. Minimised, maximised and cloaked
    /// windows keep their slot but are left alone. A refused size is logged, never fought.
    fn apply(&mut self) {
        if crate::control::paused() {
            return;
        }
        let mut lost = Vec::new();
        for p in self.desktop.placements() {
            let h = hwnd(p.window);
            if !window::is_window(h) {
                lost.push(p.window);
                continue;
            }
            // Before `place`: a minimised window sits at -32000,-32000, so deriving frame insets
            // from it would produce nonsense. It keeps its slot regardless.
            if window::is_minimized(h) || window::is_maximized(h) {
                continue;
            }
            // A cloaked window sits on another virtual desktop. It keeps its slot -- that is
            // the spatial memory across desktops the README promises -- but moving it now is
            // invisible to the user and would still rewrite the Z-order of a desktop they are
            // not looking at. It is placed on the next apply after it is uncloaked.
            if window::is_cloaked(h) {
                log::debug!("cloaked (another virtual desktop), keeping its slot: {}", window::describe(h));
                continue;
            }
            if self.opts.dry_run {
                log::info!("dry-run: would place {} at {:?}", window::describe(h), p.rect);
                continue;
            }
            match window::place(h, p.rect, p.active) {
                Ok(()) => {
                    if let Some(got) = window::visible_rect(h) {
                        if !approx_eq(got, p.rect, 2) {
                            log::warn!(
                                "{} refused {:?}, got {:?} (minimum size or elevated?)",
                                window::describe(h),
                                p.rect,
                                got
                            );
                        }
                    }
                }
                Err(e) => log::warn!("place {} failed: {e}", window::describe(h)),
            }
        }
        for w in lost {
            log::info!("{w:?} no longer exists; freeing its slot");
            self.desktop.window_vanished(w);
        }
    }

    fn on_event(&mut self, ev: RawEvent) {
        // A window that vanishes mid-drag never sends MOVESIZEEND, so without this its drop
        // preview would stay on screen and its timer would tick for the life of the process.
        // Checked before the match because the `Hidden | Destroyed` arm below frees the slot
        // and re-applies, which must not run with a dead window still recorded as dragging.
        if matches!(ev.kind, EventKind::Destroyed | EventKind::Hidden)
            && self.drag.as_ref().map(|d| d.window) == Some(ev.window)
        {
            let _ = self.cancel_drag("its window was destroyed or hidden");
        }
        match ev.kind {
            EventKind::Shown | EventKind::Uncloaked | EventKind::Foreground => {
                // Track FIRST: `focus_changed` ignores a window the desktop does not know about,
                // so a late-styled window discovered at FOREGROUND would be tracked but never
                // focused, and the next new window would split the previously focused slot.
                let newly_tracked = self.try_track(ev.window);
                if ev.kind == EventKind::Foreground {
                    self.desktop.focus_changed(ev.window);
                    // Two independent reasons to re-apply; either alone is enough:
                    // - `try_track` already applied, but it did so before focus landed, so the
                    //   raise flag named the OLD active window;
                    // - a window sharing its slot with others has just become the active member,
                    //   and only `apply` (which passes raise=true for the active window) puts it
                    //   above its stack-mates.
                    // Both are idempotent. Anything else — a plain alt-tab between windows in
                    // slots of their own — would be pure churn, so it is deliberately skipped.
                    let stacked = self.desktop.is_stacked(ev.window);
                    // Logged with BOTH reasons because skipping the re-apply is the common case:
                    // without this line a stack member that failed to raise is indistinguishable
                    // from a foreground event that never arrived at all.
                    log::debug!("foreground {:?} (newly tracked: {newly_tracked}, in a stack: {stacked})", ev.window);
                    if newly_tracked || stacked {
                        self.apply();
                    }
                }
            }
            EventKind::Hidden | EventKind::Destroyed => {
                if self.desktop.window_vanished(ev.window) {
                    log::info!("vanished {:?}; its slot stays empty", ev.window);
                    self.apply();
                }
            }
            EventKind::Cloaked | EventKind::MinimizeStart => {}
            EventKind::MinimizeEnd => self.apply(),
            EventKind::MoveSizeStart => self.on_drag_start(ev.window),
            EventKind::MoveSizeEnd => self.on_drag_end(ev.window),
        }
    }

    fn on_drag_start(&mut self, w: WindowId) {
        if crate::control::paused() {
            return;
        }
        let h = hwnd(w);
        // Floating means left alone. No drag is recorded at all, so there is no preview timer,
        // and the MOVESIZEEND that follows finds no drag and does nothing -- the window stays
        // exactly where the user dropped it. Checked before the tileable test because a floating
        // window is usually still tileable-looking; it is the FLOAT that decides, not the styles.
        if self.desktop.is_floating(w) {
            log::debug!("drag of floating {} ignored; Win+Shift+F to tile it again", window::describe(h));
            return;
        }
        if !(self.desktop.contains(w) || window::is_tileable(&window::query(h))) {
            return;
        }
        let Some(start) = window::visible_rect(h) else { return };
        // A MOVESIZEEND can be lost (e.g. the window vanished mid-drag), which would otherwise
        // leak the previous drag's timer for the rest of the process's life.
        let _ = self.cancel_drag("drag restarted for another window");
        // SAFETY: NULL hwnd makes this a thread timer; the returned id is what KillTimer needs.
        let timer = unsafe { SetTimer(None, 0, PREVIEW_INTERVAL_MS, None) };
        self.drag = Some(Drag { window: w, start, timer });
        log::debug!("drag start {} from {:?}", window::describe(h), start);
    }

    /// The single teardown for an in-flight drag: kill its preview timer and hide the overlay,
    /// returning whatever was cancelled. Every way a drag can end — the button released, a new
    /// drag replacing it, its window destroyed — goes through here, so none of them can leave a
    /// ticking timer or a stuck preview behind. A no-op (and silent) when no drag is in flight.
    fn cancel_drag(&mut self, reason: &str) -> Option<Drag> {
        let drag = self.drag.take()?;
        // SAFETY: drag.timer came from SetTimer(None, ..) on this thread.
        unsafe {
            let _ = KillTimer(None, drag.timer);
        }
        if let Some(o) = &mut self.overlay {
            o.hide();
        }
        log::debug!("drag of {:?} cancelled: {reason}", drag.window);
        Some(drag)
    }

    fn on_drag_end(&mut self, w: WindowId) {
        let Some(drag) = self.cancel_drag("move/size end") else { return };
        if drag.window != w {
            log::warn!("move/size end for {w:?} but the drag was {:?}; re-applying", drag.window);
            self.apply();
            return;
        }
        let h = hwnd(w);
        let Some(after) = window::visible_rect(h) else { return };
        if is_pure_move(drag.start, after, RESIZE_TOLERANCE_PX) {
            let Some(at) = cursor_pos() else {
                self.apply();
                return;
            };
            let effect = self.desktop.drop_window(w, at, ctrl_held());
            log::info!("drop {} at {:?} -> {:?}", window::describe(h), at, effect);
            // An untracked window dropped on nothing stays where the user left it.
            if effect == DropEffect::Ignored && !self.desktop.contains(w) {
                return;
            }
        } else {
            let changed = self.desktop.window_resized(w, drag.start, after);
            log::info!("resize {} {:?} -> {:?} (ratio changed: {changed})", window::describe(h), drag.start, after);
        }
        self.apply();
    }

    /// Late-styled windows (Chrome, Electron) are untileable at SHOW and tileable by FOREGROUND,
    /// so every one of those events re-checks untracked windows. Returns true when this call is
    /// the one that started tracking `w`, so the caller can re-apply after focus lands.
    fn try_track(&mut self, w: WindowId) -> bool {
        if self.desktop.contains(w) || self.desktop.is_floating(w) {
            return false;
        }
        let h = hwnd(w);
        if !window::is_tileable(&window::query(h)) {
            return false;
        }
        // Cheap, and it is the only chance to notice a monitor plugged in since the last retile
        // *before* a window lands on it — otherwise the new window is assigned to a monitor the
        // engine has never heard of and gets no slot at all.
        self.desktop.sync_monitors(&monitors::enumerate());
        let identity = Self::app_identity(h);
        if self.opts.float_secondary_windows
            && identity.as_ref().is_some_and(|id| self.has_same_app_window(h, id))
            && self.desktop.window_appeared_floating(w, identity.clone())
        {
            log::info!("floating new secondary window {}", window::describe(h));
            return true;
        }
        if self.desktop.window_appeared(w, monitors::monitor_of(h), identity) {
            log::info!("tracking {}", window::describe(h));
            self.apply();
            return true;
        }
        false
    }

    fn on_hotkey(&mut self, id: i32) {
        let Some(hk) = Hotkey::from_id(id) else { return };
        if crate::control::paused() && hk != Hotkey::Quit {
            return;
        }
        // SAFETY: plain query.
        let fg = unsafe { GetForegroundWindow() };
        let fg_id = window_id(fg);
        log::info!("hotkey: {}", hk.label());
        match hk {
            // SAFETY: posts WM_QUIT to this thread's own queue.
            Hotkey::Quit => unsafe { PostQuitMessage(0) },
            Hotkey::Retile => {
                self.desktop.sync_monitors(&monitors::enumerate());
                self.apply();
            }
            Hotkey::Compact => {
                let target = cursor_pos().map(monitors::monitor_at).or_else(|| self.desktop.monitor_of(fg_id));
                // Both refusals name themselves: a hotkey that does nothing silently is
                // indistinguishable from one that never arrived.
                match target {
                    Some(m) => {
                        if self.desktop.compact(m) {
                            self.apply();
                        } else {
                            log::warn!("compact: {m:?} is not a monitor the engine knows about; nothing compacted");
                        }
                    }
                    None => log::debug!(
                        "compact: no target monitor (no cursor position, and the foreground window {fg_id:?} is not tracked)"
                    ),
                }
            }
            Hotkey::ToggleFloat => {
                if fg.0.is_null() {
                    log::debug!("toggle-float: no foreground window; nothing to float");
                    return;
                }
                let floating = self.desktop.toggle_float(fg_id, monitors::monitor_of(fg));
                log::info!("{} is now {}", window::describe(fg), if floating { "floating" } else { "tiled" });
                self.apply();
            }
            Hotkey::StackAtCursor => {
                if !fg.0.is_null() {
                    if let Some(at) = cursor_pos() {
                        let effect = self.desktop.stack_window(fg_id, at);
                        log::info!("stack {} at {:?} -> {:?}", window::describe(fg), at, effect);
                        if effect != DropEffect::Ignored {
                            self.apply();
                        }
                    }
                }
            }
            Hotkey::StackNext | Hotkey::StackPrev => {
                let delta = if hk == Hotkey::StackNext { 1 } else { -1 };
                if let Some(w) = self.desktop.cycle_stack(delta) {
                    self.apply();
                    // Raising steals focus, which is an ACT: --dry-run must observe only.
                    if self.opts.dry_run {
                        log::info!("dry-run: would raise {}", window::describe(hwnd(w)));
                    } else {
                        // SAFETY: w is a window we track.
                        unsafe {
                            let _ = SetForegroundWindow(hwnd(w));
                        }
                    }
                }
            }
        }
    }

    /// Runs every PREVIEW_INTERVAL_MS during a drag.
    fn on_tick(&mut self) {
        let Some(drag) = &self.drag else { return };
        // MOVESIZESTART fires for RESIZES too, and a resize is not a drop: previewing one
        // would promise the user a slot change that `on_drag_end` will never make (it routes
        // a non-pure-move to `window_resized`). Decided by the same `is_pure_move` test the
        // end handler uses, so the preview and the outcome cannot disagree.
        let (window, start) = (drag.window, drag.start);
        if let Some(now) = window::visible_rect(hwnd(window)) {
            if !is_pure_move(start, now, RESIZE_TOLERANCE_PX) {
                if let Some(o) = &mut self.overlay {
                    o.hide();
                }
                return;
            }
        }
        let Some(overlay) = &mut self.overlay else { return };
        match cursor_pos().and_then(|at| self.desktop.preview_rect(window, at, ctrl_held())) {
            Some(r) => overlay.show(r),
            None => overlay.hide(),
        }
    }
}

fn approx_eq(a: Rect, b: Rect, tol: i32) -> bool {
    (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol && (a.w - b.w).abs() <= tol && (a.h - b.h).abs() <= tol
}
