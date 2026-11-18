# Findings summary and decisions

**Date:** 2026-11-16 · One-page roll-up of the research notes. Evidence, method and caveats are in
the linked notes; this page is what to read first.

**Scope of the evidence:** one machine (AMD Radeon 890M, Mesa 26, GNOME/Mutter on native
Wayland, display scale 2, GTK 4.22, CEF 156.5.0 via the `cef` crate), single runs, synthetic local
test pages. Nothing here is verified on other GPUs, other compositors, fractional scales, or
real websites.

## The approach

A Rust GTK4 application owns the window and all chrome. CEF renders each tab **windowless**
(off-screen) and hands us frames; we show them in a GTK widget. CEF never owns a window. Native
Wayland is a hard requirement and is met.

## Decisions (what to build on)

| Decision | Why | Evidence |
|---|---|---|
| **Windowless CEF into a GTK4 widget, native Wayland** | Only way to get a fully custom window with CEF on Wayland; works at 60 fps | [spike](cef-osr-wayland-spike.md) |
| **Use `OnAcceleratedPaint` (dmabuf into `GdkDmabufTexture`) as the main path**, keep `OnPaint` (readback) as a fallback | Zero copy; at 2880x1568 readback copies 1.1 GB/s and uses ~7x the main-process CPU and ~21 % more package power | [spike](cef-osr-wayland-spike.md), [multi-tab](cef-multi-tab-cost.md) |
| **Run CEF on its own UI thread** (`multi_threaded_message_loop`); never pump it from the GTK loop | Pumping from GTK pins the main thread at 100 % CPU (~10 W at idle) | [multi-tab](cef-multi-tab-cost.md#resolved-the-100--main-thread-spin-its-cause-and-fix) |
| **Marshal frames to the GTK thread** with `MainContext::invoke`; keep shared view state behind a mutex | CEF callbacks arrive on CEF's thread in the above mode | [multi-tab](cef-multi-tab-cost.md) |
| **Render at device resolution** (logical view + the real device scale factor, with `--disable-features=WaylandFractionalScaleV1`) | CEF paints at the scale itself: sharp, `devicePixelRatio` right, scrollbars right. (The first workaround, zoom by the scale at a 1x view, left scrollbars half size and saved zoom levels.) | [hidpi-scale.md](hidpi-scale.md) |
| **Text scaling is an extra device scale factor** (`text-scaling-factor` from GNOME's Large Text: CEF gets display scale x text scale and a window-divided-by-text view) | The page follows the desktop's text size, sharp, with scrollbars and input right; GTK itself only scales fonts. Measured with a forced factor, not yet on a Large Text machine. | [text-scale.md](text-scale.md) |
| **Bridge the clipboard through GTK** (copy/cut from `on_text_selection_changed`, paste via `ime_commit_text`) | Windowless Chromium's own clipboard never reaches the Wayland clipboard | [input](cef-input.md) |
| **Page copy buttons: a per-frame script reports the text** (`writeText`, `execCommand('copy')`, `copy` handlers), used only within 5 s of input | Chromium's own clipboard never reaches Wayland; see [page-copy-buttons.md](page-copy-buttons.md) |
| **Use Lucide icons: drawn from a 24 px grid, 1.5 px stroke, shown at the usual 16 px** | One consistent outline family with a permissive licence; GTK fills symbolic icons, so they are installed as ordinary icons per style (see [icons-lucide.md](icons-lucide.md)) |
| **One CEF browser per tab; hide background tabs (`was_hidden`)** | Hidden tabs cost ~0 W and ~0 CPU; only memory (~45 MB each here) | [multi-tab](cef-multi-tab-cost.md) ; built in the app ([tabs](browser-tabs.md)) |
| **Use an ordinary tab to display full workspace pins** | Share libadwaita's normal tab layout and keep pin behaviour in the workspace | [pinned tab size](pinned-tab-size.md) |
| **Preserve open workspaces on profile quit** | Keep the restore list when quitting the profile and use it on cold launches that name a workspace | [workspace restart](workspace-restart.md) |
| **Cap the frame rate of visible, unfocused tabs** (about 15 fps, 5 for tiny panes), raise on input | 10 busy tabs: 28 W at 60 fps vs 22 W (focused 60 + 9 at 15) vs 15 W (+9 at 5); ramp back to 60 fps in 3-110 ms | [frame rate](cef-variable-frame-rate.md) |

## Workarounds that are required

| Problem | Workaround | Status |
|---|---|---|
| Native Wayland delivers **no accelerated frames** on AMD/Mesa (`Unable to allocate frame for first frame capture: OOM?`) | `AMD_DEBUG=nodcc` in the environment of the CEF processes (set by the app at start-up) | Measured fix on this GPU; cause suspected (DCC-compressed buffer) not confirmed. Intel: `INTEL_DEBUG=noccs` is applied too, since it removed the same failure on an Intel + NVIDIA laptop ([intel-noccs.md](intel-noccs.md); only "no fallback" observed, not performance); NVIDIA reported to work without one |
| HiDPI blur | Real device scale factor to CEF; Wayland fractional-scale feature off | Verified at native scale 2; 1.5 and 1 only by forcing the scale on the same screen; mixed-DPI monitors untried |
| Frames never arrive (rare driver cases) | Fall back to `OnPaint` | Built: no accelerated frame within 3 s replaces the view with a readback one ([tabs note](browser-tabs.md#dmabuf-watchdog)); tested only by removing the AMD workaround |

Things that were tried and did **not** matter on Wayland: ANGLE backend choice (GL, EGL, Vulkan),
GPU sandbox flags, `--in-process-gpu`, passing GTK's `wl_surface` as `parent_window`, and
several `--disable-features`/`--enable-features` toggles (full list in the spike note).
`--ozone-platform=x11` also works but runs Chromium through XWayland, which breaks the
native-Wayland requirement.

## Measured numbers worth remembering

All at 2880x1568, native Wayland, zero-copy, single run each.

- **Idle window:** ~0.5-1 % main CPU and ~3 W package power, the same as an empty GTK window.
- **Hidden tab:** ~0 W, ~0.1 % CPU, ~44-48 MB memory. One renderer process per tab.
- **Visible busy tab:** about +2.7 W, ~43 % of a core, ~112 MB GPU memory each. Ten visible busy
  tabs saturate the GPU (39 fps achieved at a 60 fps cap).
- **Wake time to first fresh frame:** hidden ~8-9 ms, frozen ~8-14 ms, discarded (new browser,
  local page) ~70-80 ms. Real network pages will be slower.
- **Discarding a tab frees ~42 MB** (small test pages; real pages are larger). Freezing gave no
  measurable benefit over hiding.
- **Frame-rate ramp:** 3 ms from 30 fps, 45 ms from 15 fps, 110 ms from 5 fps.
- **Input latency** (event to next frame at GTK, excluding GTK drawing and scan-out): ~7-8 ms
  dmabuf, ~11 ms readback; independent of the frame cap for isolated inputs after idle.

## What is not known (biggest risks first)

1. **Input, verified only with emitted signals.** Mouse, wheel, keyboard, IME commit, focus,
   cursors, tooltips, `<select>` popups, clipboard and a context menu are built and pass 46
   scripted checks, and input to the next frame at GTK is ~7-8 ms (dmabuf). **Not verified:** real
   device input, a real IME (preedit, candidate window), drag selection, drag and drop,
   kinetic scrolling, accessibility ([input](cef-input.md)).
2. **Other hardware.** AMD/Mesa on one GNOME session, and one Intel + NVIDIA laptop for the Intel fix (see [intel-noccs.md](intel-noccs.md)). NVIDIA rendering, other compositors
   (KDE, Sway), and fractional scales (125 %, 150 %) are untested; the Mesa workaround is
   GPU-specific.
3. **Real websites.** Everything was measured on tiny local pages: no network, video, WebGL,
   heavy JS, or ads.
4. **Power ground truth.** One whole-APU sensor with about +/-1 W noise, on the charger. Not
   cross-checked with battery discharge or an external meter.
5. **Sandbox and packaging.** The CEF sandbox is disabled in the spike; CEF's ~300 MB runtime
   needs a distribution plan (deb, Flatpak, tarball).
6. **Upstream.** No CEF/Chromium maintainer statement or bug exists for the Mesa/Wayland
   failure; a minimal bug report is worth filing.

## Planned direction

Multiple profiles (one process and app id each), workspaces (a window of tabs) and web apps (own
app id, defaults from the web manifest), per-profile URL routing, cascading config and tab sweeping
are designed in [profiles-workspaces-design.md](profiles-workspaces-design.md). Measured there:
this CEF build plays VP8/VP9/AV1/Opus/MP3 but not H.264 or AAC.
The original capability probe did not play media. A third-party prebuilt CEF subsequently decoded
H.264 and AAC in an isolated test, but is older than browsah's bindings and is not yet verified
as a release dependency ([codec options](codec-options.md)).
Building a codec-enabled CEF may fit below upstream's 200 GB recommendation, but Linux peak space
and build time have not been measured here ([build resources](cef-build-resources.md)).

## Where things are

| | |
|---|---|
| Browser code | [`src/app/`](../../src/app/) (shell and `BrowserView`), [`src/cef_rt.rs`](../../src/cef_rt.rs), [`src/frames.rs`](../../src/frames.rs), [`src/gpu.rs`](../../src/gpu.rs), [`src/config.rs`](../../src/config.rs) |
| Benchmark code | [`src/bench/`](../../src/bench/): `--bench=single`, `--bench=tabs`, `--bench=fps` |
| Test pages | [`bench/`](../../bench/) |
| Notes | [spike](cef-osr-wayland-spike.md), [multi-tab cost](cef-multi-tab-cost.md), [frame rate](cef-variable-frame-rate.md), [input](cef-input.md) |
| Next steps | [`docs/roadmap.md`](../roadmap.md) |
