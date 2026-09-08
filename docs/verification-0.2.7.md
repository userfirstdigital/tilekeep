# v0.2.7: quarter/half hover targets inside the source slot

The source window was counted as an occupant of its original slot throughout a drag.
Consequently, a corner hover in that space used the occupied-slot rules: the preview
remained full-size and release did not split the space. The fractional placement code
was still present for separately vacant destinations, but this source-slot case was
missing from the old tests.

Drop/preview hit testing now ignores the dragged window itself when deciding whether
a slot is vacant. Another visible stack member still counts as occupied. Corners select
quarters, edge regions select halves, and the center selects the full space, subject to
the application's minimum dimensions. Hidden members remain hidden and are not duplicated.
The existing resize, connected-edge, snapshot, monitor-recovery, and lifecycle code is unchanged.

Three regression tests cover all source-slot fractions, native callback hover updates
and Escape, and hidden/visible stack members. The original nine empty-destination tests
remain enabled. They run on every CI and release build.

The new native integration harness uses a private headless KWin and real Wayland pointer,
button, and Escape events. It never invokes the handlers directly. The input helper checks
the socket's peer PID against the private compositor PID and refuses normal desktop sockets.
Its Wayland interface permission is declared only in the temporary session's desktop entry;
the real session's security configuration is not changed.

```sh
# Reproduce the old full-size shadow; expected to fail the first corner assertion:
node tests/plasma-isolated.cjs --native-drag --baseline
# Exercise production code with real pointer events:
node tests/plasma-isolated.cjs --native-drag
node tests/plasma-isolated.cjs --native-drag --fractional-scale
```

The baseline is pinned to v0.2.6. The harness checks 18 native hover/drop targets (nine
inside the source slot, nine in a separate vacancy), plus Escape, final geometry,
neighbor isolation, focus, and preview dismissal. It captures the first real preview
surface as `hover.png` in its printed diagnostic directory. The baseline image showed
a full-size rectangle; the fixed image showed quarter selection and full-area guides.

Verification passed: 107 Rust library tests, 2 CLI tests, the snapshot-controller test,
and 87 JavaScript regressions; Linux and Windows-target Clippy; 19 native cases each
at 100% and 125% (with output scale independently checked through kscreen-doctor);
and 40 isolated lifecycle cycles. The original full-size-shadow baseline failed as
expected. No real-desktop pointer injection or monitor cycling was used.

Native testing requires Plasma 6, qml6, qdbus6, kbuildsycoca6, a C++ compiler, pkg-config,
kscreen-doctor for fractional scale, and the Qt6 Core, KWaylandClient, and Wayland client development packages. The regular
JavaScript/Rust test suites do not require a compositor or these native-test dependencies.
