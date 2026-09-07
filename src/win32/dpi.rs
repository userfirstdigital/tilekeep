use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

/// Make every coordinate we read or write a physical pixel. Must run before any window is created.
pub fn enable_per_monitor_v2() {
    // SAFETY: no pointers; documented as safe to call once at process start.
    if let Err(e) = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
        log::warn!("DPI awareness not set ({e}); coordinates may be scaled on mixed-DPI setups");
    }
}
