# Tilekeep 0.2.12 verification

## Ctrl title-bar transition between tiled and floating

On Plasma Wayland, Ctrl title-bar dragging now changes mode in either direction:

- A tiled window is detached from the layout, a directly connected neighbor fills its source hole
  without rescaling unrelated windows, and its native drop rectangle remains floating.
- A floating window displays Tilekeep's existing full, half, quarter, unavailable, and occupied
  target previews. A valid drop removes it from the floating set and enrolls it in that tile.
- A floating window moved without Ctrl remains floating and displays no tile target.
- Escape restores the starting rectangle and cancels either transition.

Ctrl edge resizing remains independent: it leaves same-side aligned windows fixed and only makes
a window across the moving edge yield space. Normal title-bar dragging, connected-edge resizing,
explicit `Super+Shift+F` floating, and `Super+Shift+G` stacking retain their existing behavior.

## Verification

- Production-QML tests cover tiled-to-floating source collapse (including nested vacancies and
  minimized stack members), exact native floating geometry,
  floating-to-tiled occupied drops, all nine empty-space full/half/quarter targets, ordinary
  floating movement, Escape in both directions, and the existing Ctrl-resize behavior.
- A private KWin compositor at 125% scaling performs both transitions with native pointer, button,
  and Ctrl events. It verifies the absence of a misleading tile preview while pulling a tile out,
  the shaded target while putting the floating window back, exact final membership, source-hole
  collapse, destination yielding, and modifier cleanup.
- The full 106-case production-QML suite, 40 private compositor lifecycle cycles, all 114 Rust
  tests, Clippy with warnings denied, and the Windows GNU cross-compile passed before release.
- The guarded live check on DP-3 at 125% scaling captured 13 windows. Ctrl-drag moved Dolphin from
  `{x:1,y:1,w:656,h:914.4}` to `{x:301,y:151,w:656,h:914.4}` as a floating window. Only its directly
  connected Kate tile expanded, from `{x:658,y:1,w:870.4,h:912}` to
  `{x:1,y:1,w:1527.2,h:914.4}`; Edge and every unrelated logical rectangle stayed fixed (two clients
  repainted by less than one logical pixel at fractional scale).
- Ctrl-dragging that actual floating Dolphin onto Kate's left shaded target enrolled it again as
  `{x:1,y:1,w:763.2,h:914.4}`, with Kate yielding the matching right half at
  `{x:765,y:1,w:763.2,h:914.4}`.
- The saved arrangement was then restored exactly: all 13 windows and the DP-3 work area matched,
  Edge focus was restored, Tilekeep was active, no application was closed, and KWin retained PID
  `1963850` for the entire check.
