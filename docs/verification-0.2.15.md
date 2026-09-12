# v0.2.15: new same-app windows float by default

Newly created normal windows float at their application-selected geometry when Tilekeep already
manages another window from the same app. This covers compose windows and Electron-style pop-outs
that do not advertise themselves as dialogs. The behavior is enabled by default and can be changed
live with **Float new windows from existing apps** in the tray.

Safety boundaries:

- Windows already present when Tilekeep starts are never reclassified.
- Native dialog, popup, tool, and shell window filtering is unchanged.
- Snapshot restoration takes priority over automatic secondary-window floating.
- Floating a secondary window does not consume an empty slot or resize its existing parent window.
- A setting change applies only to windows created afterward.
- Plasma handles `windowActivated` arriving before `windowAdded`, as well as late normal-window
  metadata, through one idempotent discovery path.

Verification performed before release:

- The compositor-side JavaScript suite covers same-app matching, startup/disabled/different-app
  exclusions, unchanged tile geometry, and vacancy preservation.
- The Rust suite verifies floating enrollment leaves the layout tree byte-for-byte unchanged and
  cannot be followed by accidental automatic tiling.
- A private KWin Wayland compositor at 125% scale creates real same-process top-level windows after
  startup. It verifies the compose window floats, existing tiles do not move, the live setting can
  be disabled, and the next same-app window then tiles normally.
- The existing private compositor lifecycle and native-drag suites are rerun to guard script
  teardown, Ctrl floating transitions, occupied swaps, and empty full/half/quarter placement.
- Linux native checks and the Windows cross-target check cover both non-Plasma implementations.
