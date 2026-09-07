# v0.2.2 verification

Tested on Plasma/KWin 6.7.4 Wayland at 125% display scaling. Desktop screenshots,
window titles, process identifiers and snapshots remain local and are not published.

## Automated checks

- 100 library tests plus 2 command-line tests passed.
- 44 compositor-backend tests passed, including 180 deterministic randomized vacancy/drop cases.
- Clippy passed for Linux and Windows targets, with warnings treated as errors.
- [CI on Linux and Windows](https://github.com/userfirstdigital/tilekeep/actions/runs/34167988520) passed.

Regression coverage includes hidden/minimized vacancies, collapsed placeholders, all nine
empty-space targets, minimum-size fallback and rejection, preview/commit agreement, preservation
of existing windows, focus-safe layout reconciliation, floating windows, stack cycling, panel
work areas on every edge, asynchronous resize recovery, and snapshot restoration.

## Live Plasma checks

- The 13-operation tiling test passed: float/unfloat, retile, swaps, splits, nested resize,
  stacking/cycling, compaction, collapsed-space cleanup and restoration.
- All nine empty-space targets passed against actual client frame geometry, followed by
  snapshot restoration. Repeated hover updates retained visibility and did not steal focus.
- The full-area/quarter guide was inspected in a desktop screenshot.
- 14 real tray callback checks passed after the update, including startup toggles, saved gap,
  snapshot save/load/startup selection, and a successful signed update check.
- A real calculator launch filled a minimized file-manager vacancy exactly, without moving
  visible neighbors. The test calculator was closed and the file manager restored.
- No preexisting application windows were closed. Login startup and startup-snapshot selection
  were returned to their original disabled state; the window gap remains 1 logical pixel.

These are actual compositor/geometry and tray tests, not a claim that every possible interaction
or third-party application has been verified. The nine-zone harness invokes the compositor's
drop logic directly; it does not inject a physical mouse drag.

## Signed release and updater

The [release workflow](https://github.com/userfirstdigital/tilekeep/actions/runs/34167989279)
built and signed Windows/Linux binaries and the update manifest, then published
[v0.2.2](https://github.com/userfirstdigital/tilekeep/releases/tag/v0.2.2).

The installed Linux v0.2.1 build downloaded v0.2.2 through its real updater. Both staged
manifest and payload signatures were independently verified. The next launch installed the
update and retained the previous executable. The installed binary matched the published asset's
SHA-256 digest, the tray reported v0.2.2, and application processes/positions were preserved.
A subsequent update check reported up to date.

## Remaining verification boundaries

Windows builds and tests pass on a real Windows CI runner, but Windows desktop interactions,
login behavior and update-helper replacement have not been live-tested here. X11 desktop
behavior is likewise not covered by the Plasma harness. Snapshots do not restore unsaved
documents, browser tabs, or minimized/maximized state; some app launchers/singleton apps require
manual reopening of additional windows. Exact geometry depends on app minimum sizes and the
monitor/work-area configuration. Keep offline backups of the release-signing key and password.
