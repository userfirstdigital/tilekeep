#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let console = args
        .iter()
        .any(|a| matches!(a.as_str(), "--help" | "-h" | "--version" | "-V" | "--list" | "--dry-run" | "--install"));
    #[cfg(windows)]
    if console {
        unsafe {
            let _ =
                windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
    } // SAFETY: attaches only this process to its parent's console.
    let mut logger = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if !console {
        if let Ok(dir) = windowmanager::settings::config_dir() {
            if std::fs::create_dir_all(&dir).is_ok() {
                let path = dir.join("tilekeep.log");
                if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
                    let backup = dir.join("tilekeep.log.1");
                    let _ = std::fs::remove_file(&backup);
                    let _ = std::fs::rename(&path, backup);
                }
                if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    logger.target(env_logger::Target::Pipe(Box::new(file)));
                }
            }
        }
    }
    logger.init();
    #[cfg(windows)]
    if args.first().is_some_and(|s| s == "--apply-update") {
        let result = args
            .get(1)
            .and_then(|s| s.parse().ok())
            .ok_or("Invalid parent PID".into())
            .and_then(windowmanager::updater::windows_helper);
        if let Err(e) = result {
            log::error!("Update helper: {e}");
            std::process::exit(1);
        }
        return;
    }
    if args == ["--install"] {
        match windowmanager::settings::install() {
            Ok(p) => println!("Installed Tilekeep at {}", p.display()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("wm {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = validate_args(&args) {
        eprintln!("{e}\nTry 'wm --help' for usage.");
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--list") {
        list();
        return;
    }
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let controller = if dry_run {
        None
    } else {
        match windowmanager::control::Controller::start(args.iter().any(|a| a == "--gap").then(|| gap_arg(&args))) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    };
    if controller.as_ref().is_some_and(|c| c.state().settings.automatic_updates) {
        match windowmanager::updater::apply_on_launch() {
            Ok(true) => return,
            Ok(false) => (),
            Err(e) => log::warn!("Pending update: {e}"),
        }
    }
    let gap = controller.as_ref().map(|c| c.state().settings.gap).unwrap_or_else(|| gap_arg(&args));
    let float_secondary_windows =
        controller.as_ref().map(|c| c.state().settings.float_secondary_windows).unwrap_or(true);
    if let Some(c) = controller {
        if !args.iter().any(|a| a == "--no-tray") {
            if let Err(e) = windowmanager::tray::start(c.clone()) {
                log::warn!("Tray unavailable: {e}");
            }
        }
        windowmanager::updater::schedule(c);
    }
    log::info!("wm {} starting (dry_run={dry_run}, gap={gap})", env!("CARGO_PKG_VERSION"));
    if let Err(e) = run(dry_run, gap, float_secondary_windows) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    windowmanager::tray::stop();
}

const DEFAULT_GAP: i32 = 1;

fn print_help() {
    println!(
        "Tilekeep {}\n\nSlot-based tiling window manager for Windows and Linux.\n\nUsage: wm [OPTIONS]\n\nOptions:\n  --gap N       Override saved gap (default: {DEFAULT_GAP})\n  --install     Install this binary for the current user\n  --no-tray     Run without the tray icon\n  --autostart   Login-startup launch (uses saved settings)\n  --dry-run     Log placements without moving windows\n  --list        List manageable windows (Windows and X11)\n  -h, --help    Print help\n  -V, --version Print version",
        env!("CARGO_PKG_VERSION")
    );
}

fn validate_args(args: &[String]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" | "--list" | "--help" | "-h" | "--version" | "-V" | "--autostart" | "--no-tray" => i += 1,
            "--gap" => i += 2,
            unknown => return Err(format!("unknown option: {unknown}")),
        }
    }
    Ok(())
}

/// `--gap N`. Anything that is not a non-negative number falls back to the default and says
/// so: a negative gap makes `Rect::inset` grow the tiling area instead of shrinking it, so
/// every tile lands partly outside the work area and overlaps its neighbours -- a layout the
/// user cannot have wanted, produced silently. Observed and named, never obeyed.
fn gap_arg(args: &[String]) -> i32 {
    let Some(i) = args.iter().position(|a| a == "--gap") else {
        return DEFAULT_GAP;
    };
    let Some(raw) = args.get(i + 1) else {
        log::warn!("--gap needs a value; using {DEFAULT_GAP}");
        return DEFAULT_GAP;
    };
    match raw.parse::<i32>() {
        Ok(g) if g >= 0 => g,
        Ok(g) => {
            log::warn!("--gap {g} is negative, which would overlap tiles outside the work area; using {DEFAULT_GAP}");
            DEFAULT_GAP
        }
        Err(e) => {
            log::warn!("--gap {raw:?} is not a whole number ({e}); using {DEFAULT_GAP}");
            DEFAULT_GAP
        }
    }
}

#[cfg(windows)]
fn run(dry_run: bool, gap: i32, float_secondary_windows: bool) -> Result<(), String> {
    windowmanager::app::run(windowmanager::app::Options { dry_run, gap, float_secondary_windows })
}

#[cfg(target_os = "linux")]
fn run(dry_run: bool, gap: i32, float_secondary_windows: bool) -> Result<(), String> {
    windowmanager::linux::run(windowmanager::linux::Options { dry_run, gap, float_secondary_windows })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn run(_dry_run: bool, _gap: i32, _float_secondary_windows: bool) -> Result<(), String> {
    Err("wm supports Windows and Linux/X11".into())
}

#[cfg(windows)]
fn list() {
    use windowmanager::win32::{dpi, monitors, window};
    dpi::enable_per_monitor_v2();
    println!("Monitors (work areas):");
    for (id, r) in monitors::enumerate() {
        println!("  {:?} @ {},{} {}x{}", id, r.x, r.y, r.w, r.h);
    }
    println!("Tileable windows:");
    for h in window::enumerate_tileable() {
        println!("  {:?} {}", monitors::monitor_of(h), window::describe(h));
    }
}

#[cfg(target_os = "linux")]
fn list() {
    if let Err(e) = windowmanager::linux::list() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn list() {
    eprintln!("--list is available on Windows and Linux/X11");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn gap_arg_accepts_a_non_negative_number_and_refuses_everything_else() {
        assert_eq!(gap_arg(&args(&["--gap", "12"])), 12);
        assert_eq!(gap_arg(&args(&["--gap", "0"])), 0, "zero is a legal gap");
        assert_eq!(gap_arg(&args(&[])), DEFAULT_GAP, "absent");
        assert_eq!(gap_arg(&args(&["--dry-run"])), DEFAULT_GAP);
        assert_eq!(gap_arg(&args(&["--gap"])), DEFAULT_GAP, "no value");
        assert_eq!(gap_arg(&args(&["--gap", "-4"])), DEFAULT_GAP, "negative gaps overlap tiles");
        assert_eq!(gap_arg(&args(&["--gap", "six"])), DEFAULT_GAP, "unparsable");
        assert_eq!(gap_arg(&args(&["--gap", "6.5"])), DEFAULT_GAP, "not a whole number");
        assert_eq!(gap_arg(&args(&["--gap", ""])), DEFAULT_GAP, "empty");
        assert_eq!(gap_arg(&args(&["--dry-run", "--gap", "8"])), 8, "value follows the flag, not position 0");
    }

    #[test]
    fn argument_validation_accepts_documented_options_and_rejects_unknown_ones() {
        assert!(validate_args(&args(&["--dry-run", "--gap", "8"])).is_ok());
        assert!(validate_args(&args(&["--list"])).is_ok());
        assert!(validate_args(&args(&["--wat"])).is_err());
    }
}
