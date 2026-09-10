# Tilekeep 0.2.10 verification

## Fractional-scale restart preservation

Version 0.2.9 exposed a startup-adoption defect on a real 125%-scaled Plasma desktop. KWin
reports native frames in subpixel increments, so logically adjacent one-pixel seams can differ by
fractions of a pixel. The exact-only reconstruction rejected that valid arrangement and the first
reconciliation tiled its windows again.

Startup reconstruction now allows a tightly bounded three-logical-pixel tolerance only while
adopting native frames. The first reconciliation carries the same one-shot tolerance, then all
later operations return to the normal stricter matching behavior. Real overlaps larger than that
tolerance remain invalid.

The read-only desktop capture/compare helper also has an explicitly guarded restore mode. It
matches KWin internal window IDs, restores only non-hidden windows, never launches or closes an
application, and requires `--allow-window-moves`.

## Verification

- The exact ten-visible-window, two-row fractional geometry that exposed the defect is a permanent
  production-QML regression test.
- All 93 production-QML behavior tests pass.
- Forty repeated private KWin load/resize/overlay/unload cycles pass.
- The 29-case Ctrl real-pointer suite passes in a private KWin compositor at 125% scaling.
- The login desktop was restored from its local pre-update capture and matched all 13 pre-existing
  window records and the output work area before further testing.

The final restart check must compare the login desktop against that same local capture. KWin itself
must retain its PID throughout installation.
