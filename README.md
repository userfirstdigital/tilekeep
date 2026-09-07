# Tilekeep — slot tiling for Windows and Linux

[Download the latest release](https://github.com/userfirstdigital/tilekeep/releases/latest).
The default development branch is `master`.

Tilekeep has a native tray menu for pause/resume, gap size, retile/compact, saved snapshots,
login startup, and update status. Settings are saved per user. `wm` remains the development binary name.
Use **Unstack active window** in the tray to separate a window sharing another window's slot.

Install the built/downloaded executable with `--install`, then run the installed copy. Linux:
`~/.local/share/tilekeep/bin/tilekeep`; Windows: `%LOCALAPPDATA%\Programs\Tilekeep\tilekeep.exe`.
Use the tray's **Start with Linux/Windows** checkbox to opt into login startup.

**Snapshots:** choose **Save snapshot now**, then select it from **Load snapshot** or
**Snapshot at startup**. Saved layouts match existing windows and launch missing applications;
extra windows remain open. Exact placement requires the same monitors and available work area.
Document contents/browser tabs are the application's responsibility, not included in snapshots.
Missing apps are relaunched from their saved executable, without replaying command-line arguments.
Apps that require a launcher or do not reopen multiple windows themselves may need manual reopening.

**Updates:** signed releases download silently in the background and install on the next launch.
Development builds without a release verification key clearly show that updates are unavailable.
Release publication and trust setup are described in [the release checklist](docs/release-checklist.md).
Official release builds include the trusted verification key published in `release.pub`.

Configuration, snapshots, and `tilekeep.log` live in `~/.config/tilekeep` on Linux (respecting XDG paths)
and the user's configuration directory on Windows. `--no-tray` runs without the tray; `--dry-run`
does not create a tray, startup entry, settings, or updater.

Every visible rectangle is a **slot**. Drag a window onto another window:

```
           TOP
    ┌─────────────────┐
    │        ↑        │
LEFT│ ←    CENTER   → │RIGHT      Center → swap (Ctrl+Center → stack)
    │        ↓        │           Edge   → split the target, dragged window on that side
    └─────────────────┘
          BOTTOM
```

Closing a window leaves its slot **empty**; the next window you open takes the most recently
emptied slot, so nothing else moves. `Super+Shift+K` compacts when *you* decide. Resizing any
window with the mouse moves the boundary it shares with its neighbours; everything nested on
either side follows.

## Run

```
cargo run --release -- [--gap 1] [--dry-run] [--list]
```

On Linux, both Plasma 6 Wayland and EWMH-compatible X11 desktops are supported. Plasma Wayland
uses a temporary KWin script because only the compositor is allowed to manage native Wayland
windows; the script is loaded when `wm` starts and unloaded when it quits. `qdbus6` (normally
installed with Plasma) is required. X11 uses the standard EWMH and RandR protocols directly.
Only one Plasma integration instance runs at a time. Starting a second does not disturb the first.
On Wayland, resize completion is asynchronous: when a display sleeps, pending placements are
retained and reconciled after clients resume drawing. The app does not wake displays itself.

The first build downloads and compiles the platform crate and can take a few minutes; subsequent
builds are fast.

- `--list` prints monitors and the windows wm would manage, then exits (Windows and X11).
- `--dry-run` logs placements without moving anything.
- `RUST_LOG=debug` for drag/resize detail.

On Plasma Wayland, KWin script output—including `--dry-run` placements—is available with:

```
journalctl -f _COMM=kwin_wayland | grep Tilekeep
```

## Hotkeys

| Keys | Action |
|---|---|
| `Super+Shift+K` | Compact the monitor under the cursor (remove empty slots) |
| `Super+Shift+L` | Re-read monitors and re-apply the layout |
| `Super+Shift+F` | Toggle the foreground window between tiled and floating |
| `Super+Shift+G` | Plasma Wayland: stack the active window into the slot under the cursor |
| `Super+Shift+N` / `Super+Shift+B` | Next / previous window in the focused stack |
| `Super+Shift+Q` | Quit |

A floating window is left alone: wm neither previews nor re-tiles its drags; press `Super+Shift+F`
again to put it back.

A chord another program owns is logged at start and skipped.

## What gets tiled

Visible, titled, movable and resizable top-level application windows that are not dialogs, tools,
panels, popups, or shell surfaces. Fixed-size dialogs are left alone. Minimised or maximised
windows, and windows on another virtual desktop, keep their slot and are not moved until they come back.
On Plasma, these inactive slots do not impose minimum-size limits on neighboring windows.
Empty-space drops use the center for the whole area, edge centers for halves, and corners for
quarters. The Plasma preview outlines the whole area, shows subdivision guides, and highlights
the selected portion. A quarter too small for the app falls back to a fitting half or the whole
area; an area that cannot fit the app is labeled and leaves the layout unchanged on release.
Hidden occupants remain hidden and are retained in the source slot when possible.
New Plasma windows use visible free space (including slots held by minimized windows) before
splitting occupied slots. A cramped focused slot is skipped when another slot can fit a split.
The default gap is 1 logical pixel. Resizing fully into a vacancy absorbs its leftover strip
and extra gap, retaining hidden occupants for restoration. Plasma's panel-reserved work area
is refreshed automatically, including panels on the top, bottom, left, or right edge.

## Known limitations

- **Elevated windows** cannot be moved by a non-elevated wm (UIPI). Run wm elevated if you need them tiled.
- **Virtual desktops** are not modelled: a window on another desktop keeps its slot (cloaked windows are skipped, not removed).
- **Minimum sizes**: Plasma reserves each visible tiled application's minimum size, including decorations,
  when allocating split boundaries. If a new split cannot fit, its windows share the target
  slot as a stack; the drag outline previews that fallback. An application larger than the
  entire work area still needs to be floated. X11 logs refused placements; Plasma keeps
  delayed placements pending, because a sleeping display can also delay a resize.
- **Other Wayland compositors** are not yet supported: Wayland intentionally has no generic API
  for managing other applications' windows. Plasma 6 is supported through KWin; use an X11
  session on GNOME, Sway, Hyprland, and other compositors for now.
- **Plasma stacking modifier**: KWin scripts do not expose the keyboard modifiers held during a
  title-bar drag. Use `Super+Shift+G` with the pointer over the destination slot instead of
  Ctrl+Center to create a stack on Plasma Wayland. Center-drop still swaps.
- Unless a startup snapshot is selected, layout is rebuilt from open windows at launch. No tab strip for stacks or per-app rules.
- On Windows, `Win+Shift+C`, `R` and `P` are registered by the OS itself, which is why wm uses `K`, `L` and `B` instead; any chord another program already owns is logged at start and skipped.
- During a title-bar drag, Windows clamps the cursor to the work area, so a window cannot literally be dropped on the taskbar; releasing at the bottom edge of the work area outside any slot snaps it back.
- Some apps (Task Manager, some web-app hosts) refuse the height or width they are given; wm logs `refused ... got ...` once per placement and leaves them at the size they accepted, which can overlap a neighbour.

## Layout

`src/geometry.rs`, `src/tree.rs`, `src/engine.rs` are the pure, unit-tested engine.
`src/win32/*` and `src/app.rs` are the Win32 shell; `src/linux/x11.rs` is the X11 shell and
`src/linux/kwin.qml` is the compositor-side Plasma Wayland shell. `cargo test` does not touch the desktop.

## Verification

```
cargo test
cargo clippy --all-targets -- -D warnings
node --test tests/kwin.test.cjs
```

The JavaScript tests execute the actual QML backend functions with a mock compositor, covering
sleep/wake reconciliation, asynchronous and synchronous resize completion, interrupted drags,
nested splits, window removal, and windows that should be left alone.

With Tilekeep running on Plasma, `python tests/tray-live.py --allow-settings-changes` exercises
real tray callbacks for pause, gap, login startup, and snapshot save/load/startup selection.
It changes settings and layouts, leaves the gap at 1 px and startup snapshot disabled, and
keeps the snapshot it creates. It never closes application windows. Requires Python `dbus`.

For an explicitly authorized **live Plasma desktop** (wakes/repaints must be enabled):

```
node tests/plasma-live.cjs --allow-window-moves
```

This opt-in test stops any running Tilekeep integration, rearranges existing windows, exercises
the registered shortcuts and layout operations, checks resulting frame geometry, and restores
the initial test layout. It never closes windows. It needs `qdbus6`, `journalctl`, and at least two
visible resizable windows. Start `wm` again afterward. Actual pointer injection separately
requires Plasma's Remote Control approval; the test exercises drop/resize logic directly.

Live-tested on Plasma/KWin 6.7.4 Wayland at 125% scale: initial tiling, float/unfloat, re-tile,
center swap, edge split, nested resize, stacking, next/previous stack shortcuts, compaction,
and layout restoration. X11 and Windows live desktop behavior is not covered by this Plasma test.

The opt-in `node tests/plasma-zones-live.cjs --allow-window-moves --snapshot PATH` uses a saved
snapshot for restoration, checks all nine empty-space targets, repeated preview updates, focus,
and actual client geometry. It briefly rearranges windows but never closes them. Add
`--screenshot /tmp/tilekeep-preview.png` to capture the guide for visual inspection. Screenshots
may include personal desktop contents: keep them local. Restart Tilekeep after the live tests.

Physical mouse/keyboard testing also covers title-bar swapping, edge drops, border resizing,
Escape cancellation, floating-window drags, stacking, stack cycling, compaction, and re-tiling.
The stack shortcut uses `Super+Shift+G` because
Plasma reserves `Super+Shift+S` for Spectacle; testing only D-Bus action invocation misses this
kind of shortcut collision. Remote-input test connections must outlive the approval dialog.

The Plasma drag highlight uses its own persistent, click-through overlay. It does not use
KWin's shared snap outline, which KWin hides during ordinary drag processing. A live test of
300 pointer updates across hover zones verified one show, no intervening hides, and one hide
on cancellation; focus stayed on the dragged window.
