# v0.2.14: full occupied drops swap by default

Dragging a tiled window over the full target of another occupied tile swaps their places. The
full target now uses the same central 70% hit area as full empty-space placement, so it is the
default selection rather than a small center bullseye. Deliberate hovers in the outer 15% bands
still split the occupied target and place the dragged window in the selected half.

Verification performed before release:

- The 107-case compositor-side JavaScript suite covers full-target slot swaps, exact preview
  geometry, stacked slot contents, retained source vacancies, and occupied-edge splitting.
- The Rust suite covers the same 70% platform-neutral hit testing and verifies a 20%-inset drop
  swaps rather than splitting, keeping Windows and Linux/X11 consistent with Plasma Wayland.
- A private KWin Wayland compositor at 125% scale received real pointer and title-bar input for
  31 drag cases. It verified the full occupied swap, occupied edge split, empty full/half/quarter
  choices, Escape, and both Ctrl floating transitions without touching the user's desktop.
- The private 40-cycle load, resize, overlay, and unload stress suite checks that KWin and the
  test clients survive repeated script teardown.
