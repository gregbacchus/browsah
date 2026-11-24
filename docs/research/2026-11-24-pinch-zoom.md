# Pinch to zoom (2026-11-24, redone)

**Problem:** a two-finger pinch did nothing. A first attempt (a `GtkGestureZoom` that changed browsah's page zoom, then
faked `Ctrl`+wheel for pages like Google Maps) was thrown away: on real hardware it reflowed the page around its top-left corner, was
3x too sensitive, zoomed away from the pointer, and left the mouse button stuck down.

**Decision (asked of the user):** a pinch is Chrome's view zoom (visual viewport): anchored at the fingers, no reflow, no zoom
indicator, not remembered per site, reset by navigation. It is not the `Ctrl`+`+` page zoom. `Ctrl+0` also resets it
(`Emulation.setPageScaleFactor`).

**Touch screen:** raw GDK touch events (an `EventControllerLegacy` in the capture phase) go to `BrowserHost::send_touch_event`.
Chromium recognises tap, scroll, fling, long press and the pinch itself, and pages get real `touchstart`/`touchmove` (and may cancel
them, as maps do). The click and motion controllers ignore touch events, so a page never also gets fake mouse input.
Measured: two touch points spreading make `visualViewport.scale` 1 -> >1.3 in windowless mode and the frame shows it (picture looked at).

**Touchpad:** CEF has no pinch API. Chromium's own route on other platforms (`TouchpadPinchEventQueue`) sends synthetic `Ctrl`+wheel
events with `deltaY = 100 * ln(scale ratio)` at the pointer, and pinches the page only if none was cancelled. We do the same:
wheel events at the pointer (`WHEEL_HOOK` reports per frame whether the page called `preventDefault`); a cancelling page owns the
whole pinch (spreading is `deltaY < 0`); otherwise the pinch becomes two synthetic touch points around the pointer that spread with the
fingers, which Chromium turns into the same view pinch as on the touch screen. Unhandled `Ctrl`+wheel does not zoom the page by itself
(checked: `innerWidth` and `user_zoom` unchanged).

**Stuck mouse button:** every press we send now has a release: `GestureClick::cancel` (another gesture took the press) sends it, as does
a pointer moving with button 1 not held while one is recorded down.

**Measured (this machine, synthetic):** `scripts/test selftest pinch` (touch points through `send_touch_event`, touchpad pinch phases
through `pad_pinch`, a cancelling page, a cancelled press) and `input,zoom,scroll` still pass. **Not covered:** real touch hardware and a
real touchpad (GDK's `TouchpadPinch` events are emitted by hand), real Google Maps. Set `BROWSAH_DEBUG_PINCH=1` to log touchpad pinch phases.

Reproduce: `scripts/test selftest pinch`
