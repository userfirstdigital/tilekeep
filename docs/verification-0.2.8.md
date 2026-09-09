# v0.2.8: easier full-space hover selection

The full-space hover target now spans 15%–85% of each dimension, including its boundary,
instead of 25%–75%. It occupies 49% of the area instead of 25% (1.96 times larger).
The outer 15% bands retain half/quarter placement. This changes empty-space selection on
Plasma and Windows/X11; occupied-window swap/split zones remain unchanged.

Plasma's fit rule is unchanged: reported application minimum client dimensions plus frame
decorations, rounded up, compared with the candidate rectangle after gaps. Current window
size does not determine fit. Undersized quarters fall back to a fitting half, then full;
if the full rectangle cannot fit, the drop is rejected without a layout change.

JavaScript/Rust tests cover the expanded center, exact 15%/85% boundaries, and adjacent
edge/corner regions. A new JavaScript test verifies that a currently large window can fit
a quarter when its minimum allows, including decoration and gap accounting.

The isolated native input harness now includes eight off-center full-space targets which
previously selected quarters, across both the source slot and a separate vacancy. Together
with the earlier cases it verifies 26 hover/drop targets plus Escape, using real pointer
events, actual preview surfaces, and final window geometry. It checks that the neighboring
test window stays fixed. `--fractional-scale` independently verifies 125% output scaling.
The real desktop is not used for input testing.

Run: `node tests/plasma-isolated.cjs --native-drag --fractional-scale`.
