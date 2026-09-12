# Tilekeep — slot tiling for Windows and Linux

[Download the latest release](https://github.com/userfirstdigital/tilekeep/releases/latest).
The default development branch is `master`.

Tilekeep has a native tray menu for pause/resume, gap size, retile/compact, saved snapshots,
secondary-window behavior, login startup, and update status. Settings are saved per user. `wm`
remains the development binary name.
Use **Unstack active window** in the tray to separate a window sharing another window's slot.

Install the built/downloaded executable with `--install`, then run the installed copy. Linux:
`~/.local/share/tilekeep/bin/tilekeep`; Windows: `%LOCALAPPDATA%\Programs\Tilekeep\tilekeep.exe`.
Use the tray's **Start with Linux/Windows** checkbox to opt into login startup.

On Linux, from the folder containing the download:

```sh
chmod +x tilekeep-linux-x86_64
./tilekeep-linux-x86_64 --install
~/.local/share/tilekeep/bin/tilekeep
```

On Windows, from PowerShell in the download folder:

```powershell
.\tilekeep-windows-x86_64.exe --install
Start-Process "$env:LOCALAPPDATA\Programs\Tilekeep\tilekeep.exe"
```

Use the installed copy for automatic updates. Running the downloaded copy directly is supported,
but it does not replace itself. Login startup and loading a startup snapshot are separate choices.

**Snapshots:** choose **Save snapshot now**, then select it from **Load snapshot** or
**Snapshot at startup**. Saved layouts match existing windows and launch missing applications;
extra windows remain open. Exact placement requires the same monitors and available work area.
Snapshot menus show the name and original creation date/time in your local timezone, including
older snapshots. Use **Edit snapshots → [snapshot] → Rename… / Delete…** to manage them.
Renaming preserves the layout and startup selection. Delete asks for confirmation, disables
startup loading if needed, and moves the saved file to `snapshots/deleted` for recovery;
it never closes applications. Linux name/confirmation dialogs use `kdialog` (Plasma) or `zenity`.
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
LEFT│ ←    CENTER   → │RIGHT      Full tile → swap places
    │        ↓        │           Edge   → split the target, dragged window on that side
    └─────────────────┘
          BOTTOM
```

Closing a window leaves its slot **empty**; the next window you open takes the most recently
emptied slot, so nothing else moves. `Super+Shift+K` compacts when *you* decide.

On **Plasma Wayland**, hold **Ctrl while moving a tiled window by its title bar** to pull it out
of the layout. Its tile stays empty, every surrounding window keeps its exact size and position,
and the window stays floating where you release it. You can resize the surrounding windows into
that space manually when you want. Hold
Ctrl while moving that floating window again to see the regular tile shadows; drop it on a full,
half, quarter, or occupied target to make it tiled again. Moving a floating window without Ctrl
simply moves it, and Escape cancels either transition. `Super+Shift+F` remains the keyboard toggle
for floating, while `Super+Shift+G` creates a stack explicitly.

On **Plasma Wayland**, resizing follows the connected shared edge. Aligned windows immediately
below/above a vertical edge (or beside a horizontal edge) stay aligned, and windows across it
grow or shrink as you drag. Unrelated windows retain their rectangles: an empty stretch breaks
the connection, and matching coordinates or an old ancestor split alone do not link windows.
Hold **Ctrl while resizing** to leave same-side aligned windows in place. Only a window the moving
edge actually reaches yields space; shrinking the edge leaves reusable empty space.
Shrinking an unshared edge creates reusable empty space, including for a monitor-filling window.
Faint alignment guides and a **6-logical-pixel** snap range help match quarter/half/three-quarter
positions in the work area or available space, other window edges, and equal widths/heights.
Drag more than 6 pixels past a target to override it; Escape cancels the gesture. Snaps respect
application minimum sizes and panel work areas. When a collision or layout constraint prevents
further growth, the edge stops at the last valid local position rather than moving distant windows.
An already valid desktop layout is retained when the Plasma runtime restarts. A single overlapping
app is kept floating while the surrounding partition is adopted; if the arrangement cannot be
represented safely, all existing windows stay exactly where they are. Apps whose normal-window
metadata arrives late are enrolled within 750 ms instead of being missed at startup.
See [connected-edge verification](docs/verification-0.2.5.md) for behavior and test coverage.

Windows and X11 currently retain the original shared-divider resize behavior; these new local
resize and alignment-guide features are Plasma-specific.

## Run

```
cargo run --release -- [--gap 1] [--dry-run] [--list]
```

On Linux, both Plasma 6 Wayland and EWMH-compatible X11 desktops are supported. Plasma Wayland
uses a temporary KWin script because only the compositor is allowed to manage native Wayland
windows; the script is loaded when `wm` starts and unloaded when it quits. A small user-local
KWin effect reports Ctrl only during an active window move or resize, without raw input-device access;
it is also loaded and unloaded with Tilekeep. `qdbus6` (normally installed with Plasma) is
required. X11 uses the standard EWMH and RandR protocols directly.
Only one Plasma integration instance runs at a time. Starting a second does not disturb the first.
On Wayland, resize completion is asynchronous: sleeping clients are reconciled after they
resume drawing. If a monitor disconnects, its layout stays cached instead of being reassigned
to another monitor or KWin's temporary placeholder display. Stale placement requests are
cancelled; returning monitors and panel work areas must settle for two seconds before tiling
resumes. Apps opened or closed during the interruption are reconciled afterward. These caches
last for the running Plasma backend session and are matched by output name. The app does not
wake displays itself. See [wake-recovery verification](docs/verification-0.2.4.md).

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
| `Super/Win+Shift+G` | Stack the active window into the slot under the cursor |
| `Super+Shift+N` / `Super+Shift+B` | Next / previous window in the focused stack |
| `Super+Shift+Q` | Quit |

A floating window is otherwise left alone. On Plasma Wayland, Ctrl-title-bar-drag it onto a shaded
target to tile it; `Super+Shift+F` remains the direct keyboard toggle on every platform.

A chord another program owns is logged at start and skipped.

## What gets tiled

Visible, titled, movable and resizable top-level application windows that are not dialogs, tools,
panels, popups, or shell surfaces. Fixed-size dialogs are left alone. Minimised or maximised
windows, and windows on another virtual desktop, keep their slot and are not moved until they come back.
On Plasma, these inactive slots do not impose minimum-size limits on neighboring windows.

**Float new windows from existing apps** is enabled by default in the tray. Native dialogs already
float based on their window type; this setting also catches compose windows, pop-outs, and other
temporary windows that apps such as Electron expose as ordinary top-level windows. A normal window
created after Tilekeep starts floats at the app-selected size and position when another managed
window from the same app already exists. Windows present at startup are never reclassified, saved
snapshot placement takes priority, and changing the setting affects only windows opened afterward.
Because some apps do not publish an owner relationship, the portable fallback is app identity: a
later independent window from the same app floats too. Turn the tray setting off if an app's normal
multi-window workflow should always tile automatically; any window can still be toggled with
`Super+Shift+F` or docked with Ctrl-title-bar-drag on Plasma Wayland.

Empty-space drops use the center for the whole area, edge centers for halves, and corners for
quarters. The full-space hover target covers the central 70% of both width and height; on an
occupied tile that full target swaps the two tiles. The outer 15% bands select halves (or quarters
in empty space). The Plasma preview outlines the whole area, shows subdivision guides, and highlights
the selected portion. A quarter too small for the app falls back to a fitting half or the whole
area; an area that cannot fit the app is labeled and leaves the layout unchanged on release.
On Plasma, fit uses the application's reported minimum client size plus title bar/borders,
compared with the target dimensions after window gaps—not the window's current size.
Hidden occupants remain hidden and are retained in the source slot when possible.
On Plasma, the dragged window's own original slot also offers quarter/half/full placement;
another visible window in the same stack still counts as occupied.
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

For compositor lifecycle testing, prefer `node tests/plasma-isolated.cjs`. It starts a private
headless KWin with its own D-Bus session, runtime/configuration directories, and software
renderer; it does not use your desktop's display socket or inject global input. A development
live-test sequence did crash KWin and disrupt the desktop. See the
[crash and verification record](docs/verification-0.2.5.md). Run the live-desktop harnesses
below only in a disposable session; do not use them to stress-test a working desktop.

For real pointer-driven hover/drop checks in that private session, run
`node tests/plasma-isolated.cjs --native-drag` (add `--fractional-scale` for 125%).
This verifies corner quarters, edge halves, full-space centers, release geometry, and Escape,
including the dragged window's original slot. See [requirements and regression evidence](docs/verification-0.2.7.md).
Add `--control-drag` to load the modifier effect in the private OpenGL compositor and inject real
Ctrl press/release events. That mode also verifies tile-to-floating and floating-to-tile title-bar
drags, source-vacancy retention, occupied-destination yielding, modifier cleanup, and Ctrl edge resizing
without touching the login desktop. During an edge resize, Ctrl leaves windows aligned with the
resized window's side in place; only a window the moving edge actually reaches yields space.
Normal resizing continues to keep connected shared edges aligned.

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
The stack shortcut uses `Super+Shift+G`; Ctrl-drag is reserved for free placement. Plasma reserves
`Super+Shift+S` for Spectacle; testing only D-Bus action invocation misses this
kind of shortcut collision. Remote-input test connections must outlive the approval dialog.

The Plasma drag highlight uses its own persistent, click-through overlay. It does not use
KWin's shared snap outline, which KWin hides during ordinary drag processing. A live test of
300 pointer updates across hover zones verified one show, no intervening hides, and one hide
on cancellation; focus stayed on the dragged window.
