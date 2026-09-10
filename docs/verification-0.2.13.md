# Tilekeep 0.2.13 verification

## Non-destructive Ctrl pull-out

On Plasma Wayland, Ctrl-title-bar-dragging a tiled window now detaches only that window. Its
original tile remains vacant and every surrounding tiled window keeps its rectangle. The floating
window retains the native drop rectangle, and the user can resize adjacent windows into the vacancy
manually. Ctrl-dragging the floating window onto a shaded target still enrolls it into that tile and
makes an occupied destination yield space. Escape cancels either transition.

## Verification

- All 105 production-QML cases pass. Pull-out tests require the tree, vacant source rectangle,
  neighboring rectangles, and minimized source-slot occupants to remain unchanged.
- The private KWin compositor at 125% scaling injects real Ctrl and pointer events, verifies that
  the source slot remains vacant and the neighbor remains exact, then docks the floating window
  into an occupied shaded target. All 29 pointer cases and 40 load/resize/overlay/unload cycles pass.
- All 114 Rust tests, Clippy with warnings denied, and the Windows GNU cross-compile pass.
- The guarded login-desktop check captured 13 windows. Dolphin moved from
  `{x:1,y:1,w:656,h:914.4}` to `{x:266,y:133.5,w:656,h:914.4}` while every one of the other 12
  windows retained its exact fractional geometry. Two newly launched Dolphin windows then claimed
  and split the vacant source tile, demonstrating that the hole remains reusable. The attempted
  re-dock was cancelled so those new windows were not disturbed. No window was closed, Tilekeep
  remained active, and KWin retained PID `1963850`.
