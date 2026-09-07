use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};

use super::rect_from;
use crate::engine::MonitorId;
use crate::geometry::{Point, Rect};

/// Every monitor with its WORK area (taskbar excluded), in physical pixels.
pub fn enumerate() -> Vec<(MonitorId, Rect)> {
    let mut out: Vec<(MonitorId, Rect)> = Vec::new();
    // SAFETY: `collect` only dereferences the LPARAM we pass, which outlives the call.
    let ok = unsafe { EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut out as *mut _ as isize)) };
    if !ok.as_bool() {
        log::warn!("EnumDisplayMonitors failed; monitor list may be incomplete");
    }
    out
}

unsafe extern "system" fn collect(hmon: HMONITOR, _hdc: HDC, _r: *mut RECT, data: LPARAM) -> BOOL {
    let out = &mut *(data.0 as *mut Vec<(MonitorId, Rect)>);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(hmon, &mut info).as_bool() {
        out.push((MonitorId(hmon.0 as isize), rect_from(info.rcWork)));
    }
    BOOL(1)
}

pub fn monitor_of(h: HWND) -> MonitorId {
    // SAFETY: plain query.
    MonitorId(unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) }.0 as isize)
}

pub fn monitor_at(p: Point) -> MonitorId {
    // SAFETY: plain query.
    MonitorId(unsafe { MonitorFromPoint(POINT { x: p.x, y: p.y }, MONITOR_DEFAULTTONEAREST) }.0 as isize)
}
