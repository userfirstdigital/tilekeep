//! `SetWinEventHook` plumbing. Callbacks run on this thread inside the message pump, so
//! they push onto a static queue that the loop drains after each message.

use std::collections::VecDeque;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    PostThreadMessageW, CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW,
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_APP,
};

use super::window_id;
use crate::tree::WindowId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Shown,
    Hidden,
    Destroyed,
    Cloaked,
    Uncloaked,
    Foreground,
    MoveSizeStart,
    MoveSizeEnd,
    MinimizeStart,
    MinimizeEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: EventKind,
    pub window: WindowId,
}

static QUEUE: Mutex<VecDeque<RawEvent>> = Mutex::new(VecDeque::new());

unsafe extern "system" fn hook_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    idobject: i32,
    idchild: i32,
    _thread: u32,
    _time: u32,
) {
    if idobject != OBJID_WINDOW.0 || idchild != CHILDID_SELF as i32 || hwnd.0.is_null() {
        return;
    }
    let kind = match event {
        EVENT_OBJECT_SHOW => EventKind::Shown,
        EVENT_OBJECT_HIDE => EventKind::Hidden,
        EVENT_OBJECT_DESTROY => EventKind::Destroyed,
        EVENT_OBJECT_CLOAKED => EventKind::Cloaked,
        EVENT_OBJECT_UNCLOAKED => EventKind::Uncloaked,
        EVENT_SYSTEM_FOREGROUND => EventKind::Foreground,
        EVENT_SYSTEM_MOVESIZESTART => EventKind::MoveSizeStart,
        EVENT_SYSTEM_MOVESIZEEND => EventKind::MoveSizeEnd,
        EVENT_SYSTEM_MINIMIZESTART => EventKind::MinimizeStart,
        EVENT_SYSTEM_MINIMIZEEND => EventKind::MinimizeEnd,
        _ => return,
    };
    if let Ok(mut q) = QUEUE.lock() {
        q.push_back(RawEvent { kind, window: window_id(hwnd) });
    }
    // The hook runs inside GetMessageW without it returning, so wake the pump immediately
    // rather than waiting for the fallback timer to drain the queue we just pushed onto.
    // SAFETY: posts to this thread's own queue; a full queue is the only failure mode.
    if let Err(e) = unsafe { PostThreadMessageW(GetCurrentThreadId(), WM_APP, WPARAM(0), LPARAM(0)) } {
        log::debug!("PostThreadMessageW wake failed: {e}");
    }
}

/// Installed hooks; dropping unhooks.
pub struct EventHooks(Vec<HWINEVENTHOOK>);

impl EventHooks {
    pub fn install() -> Self {
        let ranges = [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE), // DESTROY, SHOW, HIDE are contiguous
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ];
        let mut hooks = Vec::new();
        for (lo, hi) in ranges {
            // SAFETY: hook_proc has the documented WINEVENTPROC signature; out-of-context needs no module.
            let h = unsafe {
                SetWinEventHook(lo, hi, None, Some(hook_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)
            };
            if h.0.is_null() {
                log::warn!("SetWinEventHook({lo}..{hi}) failed; those events will not be seen");
            } else {
                hooks.push(h);
            }
        }
        EventHooks(hooks)
    }
}

impl Drop for EventHooks {
    fn drop(&mut self) {
        for h in self.0.drain(..) {
            // SAFETY: h came from SetWinEventHook on this thread.
            unsafe {
                let _ = UnhookWinEvent(h);
            }
        }
    }
}

/// Take every queued event.
pub fn drain() -> Vec<RawEvent> {
    QUEUE.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_empties_the_queue() {
        QUEUE.lock().unwrap().push_back(RawEvent { kind: EventKind::Shown, window: WindowId(5) });
        let got = drain();
        assert_eq!(got, vec![RawEvent { kind: EventKind::Shown, window: WindowId(5) }]);
        assert!(drain().is_empty());
    }
}
