//! Linux desktop integration: a native KWin script on Plasma Wayland and standard
//! X11/EWMH protocols on X11 desktops.

mod kwin;
mod x11;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub dry_run: bool,
    pub gap: i32,
    pub float_secondary_windows: bool,
}

pub fn run(options: Options) -> Result<(), String> {
    if kwin::is_plasma_wayland() {
        kwin::run(options)
    } else if is_wayland() {
        Err(unsupported_wayland())
    } else {
        x11::run(options)
    }
}

pub fn list() -> Result<(), String> {
    if kwin::is_plasma_wayland() {
        kwin::list()
    } else if is_wayland() {
        Err(unsupported_wayland())
    } else {
        x11::list()
    }
}

fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
}

fn unsupported_wayland() -> String {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown compositor".into());
    format!(
        "native Wayland window management is compositor-specific; {desktop} is not supported yet (Plasma 6 and X11 sessions are supported)"
    )
}
