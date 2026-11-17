# HiDPI: why the device scale factor was ignored, and the fix

**Date:** 2026-11-17. **Environment:** Ubuntu with GNOME on native Wayland, a display at scale 2 (the machine's own; GTK reported
`surface.scale() == 2`), AMD GPU (`AMD_DEBUG=nodcc` applied), CEF 156.0.28 with `cef` crate 156.5.0, GTK 4.22, libadwaita 1.x.
Scales 1.5 and 1 were produced with `BROWSAH_FORCE_SCALE` on the same screen, not on real displays of that scale.

## Question

A user's picture of a coloured workspace window showed the page's scrollbar at about half its proper size. Why, and what is the right
fix, given that the HiDPI "backing scale" workaround (see [the spike](cef-osr-wayland-spike.md#hidpi-blurry-frames-and-the-backing-scale-fix))
renders the page at device resolution?

## What the old workaround did, and why scrollbars broke

CEF was told the view was device-sized at scale 1 and the page was **zoomed** by the scale (`zoom_level = ln(scale) / ln(1.2)`), so
text and layout came out the right size and sharp. Chromium's own scrollbars do not follow page zoom: they are 15 px at the
device scale factor, which the page believed was 1. On a scale-2 display that is half the right thickness (**measured**: 16 device
px, wanted 30).

## Root cause of "CEF renders at 1x whatever `device_scale_factor` says"

The spike concluded that CEF ignores `CefScreenInfo.device_scale_factor` on Linux and rendered at 1x. That is **true only with
Chromium's Wayland fractional-scale feature on** (the default). Measured, same binary, same page, scale forced to 2,
`--backing=off` (CEF is given a logical view and a scale factor of 2):

| Chromium configuration | frame (logical 1440x735) | `devicePixelRatio` | scrollbar |
|---|---|---|---|
| `--ozone-platform=wayland` (default) | 1440x735 (1x) | 1 | 15 device px |
| wayland, `--disable-features=WaylandPerSurfaceScale` | 1x | 1 | 15 |
| wayland, `--disable-features=WaylandUiScale` | 1x | 1 | 15 |
| wayland, `--force-device-scale-factor=2` | 1x | 1 | 15 |
| wayland, **`--disable-features=WaylandFractionalScaleV1`** | **2880x1470** | **2** | **30** |
| `--ozone-platform=x11` | 2880x1470 | 2 | 30 |
| `--ozone-platform=headless` | 2880x1470 | 2 | 30 (but accelerated frames fall back to readback) |

The same 1x result came from the software (`OnPaint`) and accelerated (dmabuf) paths, so it is not about the paint path. CEF **does**
read our screen info (a deliberately wrong `rect` showed up as `screen.width` in the page), so the screen-info callback is wired
correctly; only the scale factor is overridden. **Suspected, not confirmed:** with `WaylandFractionalScaleV1` enabled the Wayland
platform pins the scale Chromium uses; the switch name was found by trying the Wayland scale features one at a time, not from
documentation or a bug report (none was found for CEF; searched the CEF forum, issue tracker and source).

## The fix

1. Start Chromium with `--disable-features=WaylandFractionalScaleV1` in addition to `--ozone-platform=wayland`
   (`cef_rt::with_default_flags`; merged into a user's own `--disable-features`, left alone for other platforms).
2. Tell CEF the **real** scale factor and a **logical** view size (`View::dsf`, `View::device_size`). CEF then paints at device
   resolution itself, reports `devicePixelRatio == scale`, scales scrollbars, and works in logical (DIP) coordinates, so the zoom
   compensation, the `k` conversions of input coordinates and the scrollbar workaround are gone (`View::k()` is now 1).
3. `notify_screen_info_changed` is sent when the browser is created and when the scale changes.

`rendering.device_resolution: false` now means "tell CEF scale 1", i.e. render at 1x and stretch (blurry, cheaper).

## Verified

`--selftest` section `hidpi` (needs no flag; `BROWSAH_FORCE_SCALE=<n>` pretends a display scale): `devicePixelRatio` equals the scale,
the page's layout width is the window's logical width, the frame is `logical x scale` pixels, the scrollbar is 15 CSS px (so
15 x scale device px), another site (another process) gets the same, and a picture of the window and of the raw frame were looked
at (text sharp at device resolution). Whole self-test **310 of 310** at scale 2; input, scrolling, zoom, find, suggestions, groups, menu
and dialogs also pass at 1.5 and 1; unit tests 131; `--bench=single` still 60 fps zero-copy.

## The developer tools were twice as large: saved zoom levels

After the change above the user reported the developer tools window "really large". Measured with `scripts/devtools-probe.py`
(connects to Chromium's remote debugging port and asks the DevTools frontend itself; run the self-test section `devtools` with
`BROWSAH_DEVTOOLS_HOLD_MS=9000 --remote-debugging-port=9231`): the frontend had `devicePixelRatio` 4 and a layout width of
304 CSS px in a 640 px window. In a **fresh profile** it was `devicePixelRatio` 2 and 608 CSS px (correct), and the protocol log
showed the window itself identical either way (640x640 logical, 1280 px buffer), so the cause was in the profile: Chromium's
`Preferences` had `partition.per_host_zoom_levels` with `devtools` and `chromewebdata` (error pages) at 3.80, which is
exactly `ln(2)/ln(1.2)`, the zoom the old HiDPI workaround set. Chromium saves a zoom level per host; browsah overwrote the level
of every site it loaded, but never of the developer tools or error pages, so those kept the old zoom on top of the (now
correct) scale. A first guess that the tools double-counted our reported scale was wrong (measured: same result at forced scales
1, 1.5, 2 and with `--backing=off`).

Fix: `cef_rt::forget_saved_zoom` removes the saved levels from `Default/Preferences` before Chromium starts
(`without_saved_zoom`, unit tested). Nothing is lost: browsah applies a page's zoom itself on every load, from its own per-site
zoom in the profile database. Checked on the old self-test profile that had the 3.8 entries: DevTools went from 4 to 2.
**Not checked:** the user's real profile (not touched in testing); `--ozone-platform=x11` gives the frontend `devicePixelRatio` 2
in a window half as wide in logical pixels, so that path was not compared further; the DevTools window's own size and
placement (default size, set by Chromium).

## Things that were tried first and are wrong (worth not repeating)

- **DevTools device emulation** (`Emulation.setDeviceMetricsOverride`): `devicePixelRatio` and scrollbar CSS size looked right in
  numbers, but the page was painted at 1x into a frame twice as large (content in the top-left quarter, text half size). Numeric
  proxies (`devicePixelRatio`, `innerWidth`) passed while the picture was wrong: **look at the picture and the frame size**.
- **Injecting `::-webkit-scrollbar` CSS** sized for the zoom: works, but restyles every page's scrollbars; unnecessary now.

## Not measured

A real display at a fractional scale (1.25, 1.5) or two monitors of different scale (the scale is read per window, and `notify_screen_info_changed`
is sent when it changes, but moving a window between them was not tried); behaviour if a future Chromium renames or removes
`WaylandFractionalScaleV1` (the flag would then do nothing and the symptom would come back, which the `hidpi` section detects);
power or latency cost of rendering at device resolution through CEF's own scaling versus the old zoom (the benchmark shows 60 fps
zero-copy).

## Reproduce

```bash
export LD_LIBRARY_PATH=$PWD/target/release
# the fix is on by default (the `hidpi` section passes). BROWSAH_NO_SCALE_FIX=1 leaves the Wayland feature on: the section then fails
# (devicePixelRatio 1, frame 1x, scrollbar 15 device px)
BROWSAH_NO_SCALE_FIX=1 BROWSAH_FORCE_SCALE=2 BROWSAH_SELFTEST_ONLY=hidpi ./target/release/browsah --selftest
BROWSAH_FORCE_SCALE=2 BROWSAH_SELFTEST_ONLY=hidpi ./target/release/browsah --selftest --ozone-platform=x11     # x11 row
BROWSAH_SHOTS=/tmp/shots BROWSAH_SELFTEST_ONLY=hidpi ./target/release/browsah --selftest                       # pictures: hidpi-tall-page, hidpi-frame, hidpi-crop
```
