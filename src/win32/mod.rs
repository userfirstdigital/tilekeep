//! Thin Win32 shell. Conversions between engine types and Win32 handles/structs.

pub mod dpi;
pub mod events;
pub mod hotkeys;
pub mod monitors;
pub mod overlay;
pub mod window;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::geometry::{Point, Rect};
use crate::tree::WindowId;

pub fn window_id(h: HWND) -> WindowId {
    WindowId(h.0 as isize)
}

pub fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as *mut core::ffi::c_void)
}

pub fn rect_from(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

pub fn point_from(p: POINT) -> Point {
    Point { x: p.x, y: p.y }
}

pub fn cursor_pos() -> Option<Point> {
    let mut p = POINT::default();
    // SAFETY: p is a valid out-pointer for the duration of the call.
    match unsafe { GetCursorPos(&mut p) } {
        Ok(()) => Some(point_from(p)),
        Err(e) => {
            // Same disease as the enumerations: a caller cannot tell "no cursor" from "the call failed".
            log::debug!("GetCursorPos failed: {e}");
            None
        }
    }
}

pub fn ctrl_held() -> bool {
    // SAFETY: plain query with no pointers.
    (unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) }) < 0
}
