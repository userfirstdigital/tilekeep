// KWin effects receive modifier state with pointer events. Keep only a tiny,
// queryable marker effect loaded while Ctrl is held; Tilekeep's ordinary KWin
// script can ask /Effects about that marker without global input permissions.
const controlModifier = 0x04000000;
const marker = "tilekeep-control-marker";
let marked = effects.isEffectLoaded(marker);

function updateControl(_position, _oldPosition, _buttons, _oldButtons, modifiers) {
    const control = (Number(modifiers) & controlModifier) !== 0;
    // Do no work for ordinary Ctrl shortcuts. Once a drag has activated the
    // marker, retain it until Ctrl is released so the drop callback can query
    // the exact release gesture after KWin clears `window.move`.
    const moving = effects.stackingOrder.some(window => window.move);
    const next = control && (marked || moving);
    if (next === marked) {
        return;
    }
    marked = next;
    if (next) {
        effects.loadEffect(marker);
    } else {
        effects.unloadEffect(marker);
    }
}

effects.mouseChanged.connect(updateControl);
