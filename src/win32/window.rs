//! Per-window queries and placement. `is_tileable` is pure so the filter is unit-tested.

use core::ffi::c_void;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, GW_OWNER, HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER,
    WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};

use super::rect_from;
use crate::geometry::Rect;

pub const OVERLAY_CLASS: &str = "UserFirstWmOverlay";
pub fn executable(h: HWND) -> Option<std::path::PathBuf> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    };
    let mut pid = 0;
    let mut buffer = vec![0u16; 32768];
    let mut len = buffer.len() as u32;
    // SAFETY: query-only process handle and correctly sized UTF-16 output buffer.
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let result =
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buffer.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        result.ok()?;
    }
    Some(String::from_utf16_lossy(&buffer[..len as usize]).into())
}

/// Shell surfaces and helpers that must never be tiled.
const DENIED_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
    "ForegroundStaging",
    "MultitaskingViewFrame",
    "Windows.Internal.Shell.TabProxyWindow",
    OVERLAY_CLASS,
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    pub style: u32,
    pub ex_style: u32,
    pub class_name: String,
    pub title: String,
    pub has_owner: bool,
    pub visible: bool,
    pub cloaked: bool,
}

/// A top-level, visible, resizable, titled, unowned window that is not a shell surface.
pub fn is_tileable(i: &WindowInfo) -> bool {
    if !i.visible || i.cloaked || i.has_owner || i.title.is_empty() {
        return false;
    }
    if i.style & WS_CHILD.0 != 0 || i.style & WS_THICKFRAME.0 == 0 {
        return false;
    }
    if i.ex_style & WS_EX_TOOLWINDOW.0 != 0 || i.ex_style & WS_EX_NOACTIVATE.0 != 0 {
        return false;
    }
    !DENIED_CLASSES.contains(&i.class_name.as_str())
}

pub fn query(h: HWND) -> WindowInfo {
    // SAFETY: every call takes a valid HWND or a correctly sized out-buffer.
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let mut cbuf = [0u16; 256];
        let n = GetClassNameW(h, &mut cbuf).max(0) as usize;
        let class_name = String::from_utf16_lossy(&cbuf[..n]);
        let mut tbuf = [0u16; 512];
        let n = GetWindowTextW(h, &mut tbuf).max(0) as usize;
        let title = String::from_utf16_lossy(&tbuf[..n]);
        let has_owner = GetWindow(h, GW_OWNER).map(|o| !o.0.is_null()).unwrap_or(false);
        let visible = IsWindowVisible(h).as_bool();
        let mut cloaked_val: u32 = 0;
        let cloaked = match DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked_val as *mut u32 as *mut c_void, 4) {
            Ok(()) => cloaked_val != 0,
            Err(e) => {
                // Treated as not cloaked, but say so: a window we cannot ask about is not
                // the same fact as a window that answered "visible".
                log::debug!("DWMWA_CLOAKED unreadable for [{class_name}] \"{title}\": {e}; assuming not cloaked");
                false
            }
        };
        WindowInfo { style, ex_style, class_name, title, has_owner, visible, cloaked }
    }
}

/// The rect Windows reports, including the invisible resize border.
pub fn window_rect(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    // SAFETY: r is a valid out-pointer.
    if let Err(e) = unsafe { GetWindowRect(h, &mut r) } {
        // Debug, not warn: this fires routinely for windows being destroyed.
        log::debug!("GetWindowRect failed for {h:?}: {e}");
        return None;
    }
    Some(rect_from(r))
}

/// The rect the user sees (DWM extended frame bounds), falling back to `window_rect`.
pub fn visible_rect(h: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    // SAFETY: r is a valid out-pointer of the size we pass.
    let got = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    match got {
        Ok(()) => Some(rect_from(r)),
        Err(e) => {
            // Do not call describe() here: describe() calls visible_rect().
            log::debug!("DWMWA_EXTENDED_FRAME_BOUNDS failed for {h:?}: {e}; falling back to GetWindowRect");
            window_rect(h)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// How far the reported rect extends beyond the visible frame on each side.
pub fn frame_insets(outer: Rect, visible: Rect) -> Insets {
    Insets {
        left: visible.x - outer.x,
        top: visible.y - outer.y,
        right: outer.right() - visible.right(),
        bottom: outer.bottom() - visible.bottom(),
    }
}

/// Grow a desired VISIBLE rect by the insets to get what `SetWindowPos` needs.
pub fn outer_target(visible_target: Rect, i: Insets) -> Rect {
    Rect::new(
        visible_target.x - i.left,
        visible_target.y - i.top,
        visible_target.w + i.left + i.right,
        visible_target.h + i.top + i.bottom,
    )
}

/// Move/size the window so its VISIBLE frame fills `target`. `raise` puts it on top of its siblings.
pub fn place(h: HWND, target: Rect, raise: bool) -> Result<(), String> {
    // The hwnd as well as describe(): a window that has just died describes as `[] ""`,
    // so describe() alone leaves the failure unattributable.
    let outer = window_rect(h).ok_or_else(|| format!("GetWindowRect failed for {h:?} {}", describe(h)))?;
    let vis = visible_rect(h).unwrap_or(outer);
    let t = outer_target(target, frame_insets(outer, vis));
    let mut flags = SWP_NOACTIVATE;
    if !raise {
        flags |= SWP_NOZORDER;
    }
    // SAFETY: h is a window handle we were given by the system; flags are valid.
    unsafe { SetWindowPos(h, if raise { Some(HWND_TOP) } else { None }, t.x, t.y, t.w, t.h, flags) }
        .map_err(|e| format!("SetWindowPos({},{} {}x{}) failed for {h:?} {}: {e}", t.x, t.y, t.w, t.h, describe(h)))
}

pub fn is_window(h: HWND) -> bool {
    // SAFETY: plain query.
    unsafe { IsWindow(Some(h)) }.as_bool()
}

pub fn is_minimized(h: HWND) -> bool {
    // SAFETY: plain query.
    unsafe { IsIconic(h) }.as_bool()
}

pub fn is_maximized(h: HWND) -> bool {
    // SAFETY: plain query.
    unsafe { IsZoomed(h) }.as_bool()
}

/// True when DWM says the window is cloaked — almost always because it lives on another
/// virtual desktop. It keeps its slot (we do not model desktops), but placing it is pointless
/// and re-orders a Z-order the user cannot see. An unreadable attribute reads as NOT cloaked,
/// which `query` logs, so a window we cannot ask about is still placed rather than dropped.
pub fn is_cloaked(h: HWND) -> bool {
    query(h).cloaked
}

/// Every currently tileable top-level window, in Z order (topmost first).
pub fn enumerate_tileable() -> Vec<HWND> {
    let mut out: Vec<HWND> = Vec::new();
    // SAFETY: `collect` only dereferences the LPARAM we pass, which outlives the call.
    if let Err(e) = unsafe { EnumWindows(Some(collect), LPARAM(&mut out as *mut _ as isize)) } {
        log::warn!("EnumWindows failed: {e}; window list may be incomplete");
    }
    out
}

unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
    let out = &mut *(data.0 as *mut Vec<HWND>);
    if is_tileable(&query(h)) {
        out.push(h);
    }
    BOOL(1)
}

/// `[class] "title" @ x,y wxh` for logs.
pub fn describe(h: HWND) -> String {
    let i = query(h);
    let r = visible_rect(h).unwrap_or_default();
    format!("[{}] \"{}\" @ {},{} {}x{}", i.class_name, i.title, r.x, r.y, r.w, r.h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_EX_APPWINDOW, WS_VISIBLE};

    fn app_window() -> WindowInfo {
        WindowInfo {
            style: WS_VISIBLE.0 | WS_CAPTION.0 | WS_THICKFRAME.0,
            ex_style: WS_EX_APPWINDOW.0,
            class_name: "Notepad".into(),
            title: "Untitled - Notepad".into(),
            has_owner: false,
            visible: true,
            cloaked: false,
        }
    }

    #[test]
    fn ordinary_resizable_app_window_is_tileable() {
        assert!(is_tileable(&app_window()));
    }

    #[test]
    fn rejects_invisible_cloaked_owned_and_untitled() {
        assert!(!is_tileable(&WindowInfo { visible: false, ..app_window() }));
        assert!(!is_tileable(&WindowInfo { cloaked: true, ..app_window() }));
        assert!(!is_tileable(&WindowInfo { has_owner: true, ..app_window() }));
        assert!(!is_tileable(&WindowInfo { title: String::new(), ..app_window() }));
    }

    #[test]
    fn rejects_fixed_size_dialogs_children_and_tool_windows() {
        let fixed = WindowInfo { style: WS_VISIBLE.0 | WS_CAPTION.0, ..app_window() };
        assert!(!is_tileable(&fixed), "no WS_THICKFRAME");
        let child = WindowInfo { style: app_window().style | WS_CHILD.0, ..app_window() };
        assert!(!is_tileable(&child));
        let tool = WindowInfo { ex_style: WS_EX_TOOLWINDOW.0, ..app_window() };
        assert!(!is_tileable(&tool));
        let noactivate = WindowInfo { ex_style: WS_EX_NOACTIVATE.0, ..app_window() };
        assert!(!is_tileable(&noactivate));
    }

    #[test]
    fn rejects_shell_classes_and_our_own_overlay() {
        for class in ["Progman", "WorkerW", "Shell_TrayWnd", "Windows.UI.Core.CoreWindow", OVERLAY_CLASS] {
            assert!(!is_tileable(&WindowInfo { class_name: class.into(), ..app_window() }), "{class}");
        }
    }

    #[test]
    fn frame_insets_measure_the_invisible_border() {
        // Typical Windows 11: 7px invisible border left/right/bottom, none on top.
        let outer = Rect::new(93, 100, 814, 620);
        let visible = Rect::new(100, 100, 800, 613);
        let insets = frame_insets(outer, visible);
        assert_eq!(insets, Insets { left: 7, top: 0, right: 7, bottom: 7 });
        assert_eq!(outer_target(Rect::new(0, 0, 500, 500), insets), Rect::new(-7, 0, 514, 507));
        assert_eq!(frame_insets(outer, outer), Insets::default());
    }
}
