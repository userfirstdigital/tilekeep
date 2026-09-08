# v0.2.3: local Plasma resizing and gentle alignment

## Behavior

Plasma edge resizing rebuilds local free space from fixed occupied rectangles rather than
rescaling ancestor dividers. Only directly touching neighbors can share the resized edge;
unrelated windows stay fixed. Explicitly stacked windows still share a slot.

Quarter, half, three-quarter, work-area, other-window-edge and equal-size targets capture
within six logical pixels. A thin, non-focusable, click-through guide appears while aligned.
Moving beyond the capture range releases the snap without a modifier key. Pointer deltas are
measured from the start of the gesture, avoiding cumulative drift or sticky snapped geometry.

The first native Wayland resize step can report an old committed frame. Edge detection also
uses pointer movement near the starting frame, so that stale frame is not sent back to the
client. A corner can acquire its second moving edge later in the gesture. Escape discards
the resize. Collision/minimum-size/layout constraints stop at the last valid local position.
Native KWin tile associations are released even for windows already at their target geometry,
so the compositor's own tile tree cannot retain a second resize association.

Intentional small vacancies survive snapshot save/load. Restarting the Plasma runtime adopts
valid, non-overlapping actual window frames instead of rearranging an already tiled desktop.
The implementation uses the [KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/).

## Verification

- 101 Rust library tests and 2 CLI tests passed.
- 57 compositor-function tests passed, including the existing 180 randomized drop cases.
- New regressions cover all four resize directions across ancestor dividers, direct-neighbor
  isolation, resizing a monitor-filling window, small vacancies, snapshot round trips,
  quarter/half/three-quarter capture and escape, equal-size alignment, minimum-size rejection,
  delayed first frames, corner edges, Escape cancellation, and startup geometry adoption.
- Linux and Windows-target Clippy passed with warnings treated as errors.
- The live Plasma harness passed local resizing against real client frames, a real guide
  surface, native KWin resize input, snap capture, escape, final placement, and guide dismissal.
  Preexisting user windows were checked for unchanged geometry. Only the two disposable
  test windows were terminated, using their directly spawned child process.
- A production dry run predicted the existing desktop rectangles. The installed v0.2.3
  runtime then started with those same placements and no new runtime QML errors observed.

The live harness uses a temporary virtual keyboard through `/dev/uinput` for native KWin
resize input; it is not a claim of exhaustive physical-mouse testing. It does not send close
shortcuts. Run only on an authorized desktop, with the normal Tilekeep runtime stopped:

```sh
node tests/plasma-resize-live.cjs --allow-test-windows --area X,Y,WIDTH,HEIGHT
```

Choose a clear region at least 420×500 logical pixels; `qml6`, Python `evdev`, and access to
`/dev/uinput` are required. Restart the normal runtime afterward. Desktop screenshots,
window titles, process identifiers, and user snapshots are not published.

## Boundaries

The new resize policy and guides apply to Plasma Wayland. Windows and X11 still use shared
ancestor-divider resizing. Arbitrary overlapping/non-slicing arrangements, app minimum sizes,
and screen boundaries can restrict a local resize; the implementation does not move distant
windows to force it. Windows desktop interaction has not been live-tested here.
