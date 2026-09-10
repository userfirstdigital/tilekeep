# Tilekeep 0.2.11 verification

## Ctrl free resizing

Ctrl free placement originally applied only to title-bar moves. Holding Ctrl while dragging a
window edge therefore looked identical to an ordinary connected-edge resize.

On Plasma Wayland, Ctrl now also selects free behavior for interactive edge resizing. Windows
aligned on the resized window's side remain fixed. A visible window only yields once the moving
edge reaches it, including across a deliberate vacancy. Ordinary resizing continues to propagate
the connected shared edge, and all results remain bounded by client minimum sizes and the panel
work area.

The compositor modifier observer recognizes KWin's move and resize states. It remains inert for
ordinary Ctrl shortcuts and unloads its marker immediately when Ctrl is released. The last Ctrl
state observed during the gesture is retained through KWin's asynchronous mouse-release callback,
so releasing Ctrl immediately after the mouse button cannot turn a free gesture into a normal one.

The guarded live install check also exposed a separate startup edge case: one intentionally
overlapping window made the otherwise valid native partition unreconstructable, so the stale
startup tree was replayed. Startup now adopts the representable partition and preserves a single
overlapping client as floating. If the desktop still cannot be represented, every existing window
is left untouched rather than being rearranged.

The live history showed the overlapping DevManager process started after Tilekeep but never
reached the normal-window criteria during its initial `windowAdded` callback. A lightweight
eligibility scan now enrolls such late-ready applications within 750 ms. It does not re-enroll
windows explicitly toggled to floating.

## Verification

- All 100 production-QML behavior tests pass, including direct, vacant-strip, interactive preview,
  release, minimum-size, normal connected-edge, and unrelated-window cases.
- All 114 Rust and integration tests pass, Clippy passes with warnings denied, and the Windows GNU
  target compiles successfully.
- A private KWin compositor at 125% scaling receives real Ctrl input during KWin's native resize,
  while Ctrl outside an interactive move or resize remains inert.
- The private 29-case native pointer suite covers all quarter, half, center, Escape, empty-space,
  source-space, and occupied-destination paths.
- Forty repeated private KWin load/resize/overlay/unload cycles complete without a compositor or
  client exit.

The guarded login-desktop check captured all 13 existing windows by KWin ID. A Ctrl resize widened
Dolphin from 656 to 838.4 logical pixels, moved and narrowed the directly crossed Kate window from
870.4 to 690.4 pixels, and kept the same-side Konsole at its original 655.2-pixel width. The exact
pre-test arrangement and T3 focus were then restored. The final installation restart and the test
both preserved KWin PID 1963850; no application was launched or closed by the live check.
