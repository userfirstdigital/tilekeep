# v0.2.4: preserve Plasma layouts across monitor loss

## Failure and fix

KWin can replace the last physical output with `Placeholder-1` while a monitor is powered
off. Previously, Tilekeep deleted the real output's tree and re-enrolled its windows on the
placeholder, then repeated that process when the real output returned. This changed slot
membership and geometry even without user interaction.

The Plasma backend now retains offline trees by output name, ignores placeholder, disabled,
and invalid outputs, and cancels in-flight placement requests on topology changes. Placement
and interaction entry points also check the current output list, covering callbacks that
arrive before `screensChanged`. A display epoch invalidates interrupted drag/resize gestures.

Returning output identities and work areas must remain unchanged for four 500 ms intervals
before placement resumes. A late panel reservation resets this settling period. Newly opened
windows wait for a usable monitor; closed windows are removed from retained trees. Pause state
is preserved. Snapshot requests wait for recovery; offline snapshot trees cannot also be
assigned to an unmatched active monitor.

## Verification

- 101 Rust library tests, 2 CLI tests, and 71 compositor-function tests passed.
- Regression cases include the observed real-output → placeholder → replacement-output
  sequence, 12 repeated reconnect cycles, two-output removal/reconnection, late panel areas,
  invalid outputs, startup without a real output, queued client creation/removal, minimized
  and floating clients, snapshot deferral, and interrupted resize callbacks.
- Linux and Windows-target Clippy passed with warnings treated as errors; formatting passed.
- The live resize harness passed local resize isolation, native keyboard resize, visible snap
  capture and escape, focus preservation, final placement, and guide dismissal.
- The live display replay harness used real KWin client windows and production recovery and
  placement functions. Only its display samples and test work area were substituted. It
  displaced the test clients while disconnected, verified that no placement occurred, changed
  the returning panel area before settling, and verified restoration of the original client
  frames and tree. All ten preexisting user windows retained their geometry and visibility.
- A DPMS off/on check on Plasma 6.7.4 Wayland at 125% scale preserved all ten user windows and
  the panel work area. The requested pre-incident arrangement was separately restored, with
  newer apps placed in available space; no user applications were closed or relaunched.

The only-output disable command was refused by Plasma. DPMS testing is therefore **not** a
claim of physical cable-unplug verification. The placeholder sequence is covered by automated
backend tests, and the no-real-output recovery is replayed against real client frames.

## Repeating live checks

Only run on an explicitly authorized desktop. Stop the normal Tilekeep runtime, then choose
a clear logical-pixel region at least 420×500:

```sh
node tests/plasma-display-live.cjs --allow-test-windows --area X,Y,WIDTH,HEIGHT
node tests/plasma-resize-live.cjs --allow-test-windows --area X,Y,WIDTH,HEIGHT
```

Restart Tilekeep afterward. Both harnesses terminate only their directly spawned test process;
neither sends a global close shortcut. The display replay needs `qml6`; native resize additionally
needs Python `evdev` and access to `/dev/uinput`.

For read-only before/after verification, with either runtime state:

```sh
node tests/kwin-desktop.cjs --capture /PRIVATE/PATH/before.json
node tests/kwin-desktop.cjs --compare /PRIVATE/PATH/before.json
```

Capture files contain private application identifiers and titles; do not commit them. Geometry
comparison permits one logical pixel for fractional-scale native-client rounding. Probe data
also appears in the local compositor journal, as do ordinary Tilekeep placement logs.

## Boundaries

The topology cache lasts for the current backend process, not across a process/compositor
restart. Monitor identity uses the connector name; moving a display to a different connector
is not treated as reconnecting the same output. Permanently removed monitors retain their
trees until the user moves their windows or ends the session. Floating windows remain under
the compositor's placement policy. This change is specific to Plasma Wayland; Windows/X11
monitor-removal behavior and physical multi-monitor hardware have not been live-tested here.
