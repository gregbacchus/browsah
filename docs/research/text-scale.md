# Text scaling ("Large Text"): an extra scale factor for the page

**Date:** 2026-11-18. **Environment:** Ubuntu with GNOME on native Wayland, a display at scale 2 (this machine's own), CEF 156.0.28,
GTK 4.22. **Not measured on a machine with Large Text on:** the report came from another computer with GNOME's accessibility
*Large Text* enabled; this machine has `text-scaling-factor` 1.0. Everything below at 1.25 and 1.5 used `BROWSAH_FORCE_TEXT_SCALE`
(the same code path, the value only comes from the environment instead of the setting).

## Question

With *Large Text* on, GTK's own widgets (tabs, address bar, menus) get bigger text but the web page did not change. How should the
page follow, and where does it fit with the device-scale handling in [hidpi-scale.md](hidpi-scale.md)?

## What the setting is

GNOME's *Large Text* switch (Accessibility) and the Tweaks "Font scaling" slider both write
`org.gnome.desktop.interface text-scaling-factor` (1.25 for Large Text; the range GNOME offers is 0.5 to 3). GTK applies it to font
sizes only; the surface scale (`gdk::Surface::scale`, the display's) is untouched. browsah read only the surface scale, so the page
never saw it. Chromium on Wayland does not apply it to a page by itself either.

## Decision: treat it as part of the device scale factor

The page is given the **display scale times the text scale** as its device scale factor and a logical view that is the window's size
**divided by the text scale** (`frames::View`: `dsf()`, `device_size()`, `k()`). The frame CEF paints is then the window's size in
device pixels (logical x text x scale), exactly as before, so the picture still fits the widget without stretching. Pointer
coordinates, popup rectangles, context-menu positions and scroll distances are converted by `k() = 1 / text`; they already went
through `k` (the old zoom workaround), so nothing else changed.

Why not page zoom: the same reason as for the display scale. Zoom leaves Chromium's scrollbars at the old size, and it would mix with
the person's own per-site zoom (`Ctrl+Plus`), which stays separate and multiplies on top.

The value is read on every size/scale check (the picture's tick callback) from `gio::Settings`, so changing the setting while a window
is open should take effect on the next frame (**not tried live**: changing the desktop's settings was not done from here). Without GNOME's schema (other desktops) the factor is 1.

## Measured (this machine, scale 2, forced text scale)

`hidpi` section, 1440x735 view, `BROWSAH_FORCE_TEXT_SCALE`:

| text scale | `devicePixelRatio` | page `innerWidth` (CSS px) | scrollbar (device px) | frame (px) |
|---|---|---|---|---|
| 1.0 | 2 | 1440 | 30 | 2880x1470 |
| 1.25 | 2.5 | 1152 | 37.5 | 2880x1470 |
| 1.5 | 3 | 960 | 45 | 2880x1470 |

The section also clicks a window point and reads `clientX/Y` in the page: it is the window point divided by the text scale (checked at
1.25 and 1.0). Unit tests (`frames::tests`) cover the geometry arithmetic.

## Caveats

- **Scrolling distance is visual, not in CSS px:** a wheel notch moves the page the same distance on screen, so in CSS px it is 1/text
  as far (the `scroll` self-test, which counts CSS px, reports 1600 for 2000 at 1.25; that is the test's assumption, not a fault).
- Other sections (`input`, `menu`) click fixed window coordinates and so only pass at text scale 1 (verified: 45 of 45 pass at 1; at 1.25
  nine checks fail because the page layout moved under the fixed coordinates). The `<select>` popup's position at a text scale other
  than 1 was **not checked by eye**.
- `--backing=off` (diagnostic) renders at 1x and stretches, as it does for the display scale.
- Rounding: the logical size is a whole number, so the frame can be a pixel or two off the window (the picture is stretched by that).

## Reproduce

```bash
export LD_LIBRARY_PATH=$PWD/target/release
BROWSAH_FORCE_TEXT_SCALE=1.25 BROWSAH_SELFTEST_ONLY=hidpi ./target/release/browsah --selftest
BROWSAH_FORCE_SCALE=2 BROWSAH_FORCE_TEXT_SCALE=1.5 BROWSAH_SELFTEST_ONLY=hidpi ./target/release/browsah --selftest
scripts/test unit frames
```
