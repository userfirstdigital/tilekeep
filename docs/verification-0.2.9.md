# Tilekeep 0.2.9 verification

## Ctrl free placement

Holding Ctrl during a title-bar move collapses only the slot vacated by the dragged window.
Other empty slots remain available. Dropping into empty space retains the full/half/quarter
chooser; dropping onto an occupied edge or center still splits the destination instead of
swapping or stacking. A hidden stack member keeps the source slot alive. Explicit stacking is
available on every backend with `Super/Win+Shift+G`.

Plasma's ordinary KWin window script cannot see drag modifiers. Tilekeep installs two tiny
user-local JavaScript effects: an observer that receives KWin's modifier-bearing pointer signal,
and an inert marker loaded only while Ctrl and a window move are both active. The layout script
queries that compositor-local marker. Ordinary Ctrl shortcuts do not activate it, no raw input
device is opened, and both effects are unloaded when Tilekeep exits.

## Automated coverage

- Pure engine and tree tests compare normal source vacancies with targeted Ctrl collapse,
  occupied-target splitting, empty-target placement, exact preview geometry, and preservation
  of unrelated vacancies.
- The QML suite repeats those cases against the production backend and also covers hidden stack
  occupants and the Free preview state.
- `node tests/plasma-isolated.cjs --control-drag --native-drag --fractional-scale` runs a private
  KWin at 125%, sends real Ctrl and pointer events, proves ordinary Ctrl is inert, exercises 27
  existing hover/drop/Escape cases, then verifies both free-to-empty and free-to-occupied drops.
  The private compositor and both client windows survive, and the marker unloads on Ctrl release.

The release candidate passed:

- 111 Rust library tests, 2 command-line tests, and the snapshot-controller integration test.
- 92 production-QML behavior tests.
- Clippy with warnings denied and a full `x86_64-pc-windows-gnu` compile check.
- 40 repeated private KWin load/resize/overlay/unload cycles.
- The ordinary 27-case and Ctrl-extended 29-case real-pointer suites in private KWin at 125%.

No test in this verification sequence connects to or injects input into the login desktop.
