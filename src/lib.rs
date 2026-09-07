//! Slot-based tiling window manager. The engine modules are pure and unit-tested;
//! the platform modules are thin desktop-integration shells.

pub mod autostart;
pub mod control;
pub mod engine;
pub mod geometry;
pub mod settings;
pub mod snapshots;
pub mod tray;
pub mod tree;
pub mod updater;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod win32;

#[cfg(target_os = "linux")]
pub mod linux;
