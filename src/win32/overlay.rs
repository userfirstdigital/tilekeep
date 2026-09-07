//! A translucent, click-through, always-on-top rectangle showing where a drop would land.
//! The class background brush paints it, so there is no WM_PAINT code.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::CreateSolidBrush;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WNDCLASSEXW, WNDCLASS_STYLES,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::window::OVERLAY_CLASS;
use crate::geometry::Rect;

/// COLORREF is 0x00BBGGRR; this is a mid blue.
const FILL: COLORREF = COLORREF(0x00D0_8A2E);
const ALPHA: u8 = 110;

pub struct Overlay {
    hwnd: HWND,
    shown: Option<Rect>,
}

unsafe extern "system" fn wnd_proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: every argument is supplied by Windows itself and is only forwarded, unread and
    // undereferenced, to DefWindowProcW. This proc dereferences nothing of its own.
    DefWindowProcW(h, msg, w, l)
}

impl Overlay {
    pub fn create() -> Result<Overlay, String> {
        let class_name: Vec<u16> = OVERLAY_CLASS.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: all pointers passed outlive the calls; the class name buffer lives until return.
        unsafe {
            let hmodule = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let hinstance = HINSTANCE(hmodule.0);
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: WNDCLASS_STYLES(0),
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                hbrBackground: CreateSolidBrush(FILL),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            if RegisterClassExW(&wc) == 0 {
                return Err("RegisterClassExW failed for the overlay".into());
            }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                10,
                10,
                None,
                None,
                Some(hinstance),
                None,
            )
            .map_err(|e| e.to_string())?;
            SetLayeredWindowAttributes(hwnd, COLORREF(0), ALPHA, LWA_ALPHA).map_err(|e| e.to_string())?;
            Ok(Overlay { hwnd, shown: None })
        }
    }

    pub fn show(&mut self, r: Rect) {
        if self.shown == Some(r) {
            return;
        }
        // SAFETY: hwnd is our own window.
        unsafe {
            match SetWindowPos(self.hwnd, Some(HWND_TOPMOST), r.x, r.y, r.w, r.h, SWP_NOACTIVATE | SWP_SHOWWINDOW) {
                // `shown` is a cache of where the window actually IS. Recording a rect the call
                // never applied would make the guard above suppress every retry, so one failed
                // move would freeze the preview for the rest of the drag, silently.
                Ok(()) => self.shown = Some(r),
                Err(e) => log::warn!("overlay SetWindowPos {r:?} failed: {e}"),
            }
        }
    }

    pub fn hide(&mut self) {
        if self.shown.is_none() {
            return;
        }
        // SAFETY: hwnd is our own window.
        unsafe {
            // Unlike SetWindowPos above, ShowWindow's BOOL is the window's PREVIOUS visibility,
            // not success or failure, so there is nothing here to check and nothing to log.
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.shown = None;
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // SAFETY: hwnd is our own window, destroyed once.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
