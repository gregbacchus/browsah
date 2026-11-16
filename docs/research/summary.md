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
| **Render at device resolution** (view = logical x scale at 1x, zoom by the scale) | CEF otherwise renders 1x on HiDPI and GTK upscales it, which looks blurry | [spike](cef-osr-wayland-spike.md#hidpi-blurry-frames-and-the-backing-scale-fix) |
| **Bridge the clipboard through GTK** (copy/cut from `on_text_selection_changed`, paste via `ime_commit_text`) | Windowless Chromium's own clipboard never reaches the Wayland clipboard | [input](cef-input.md) |
| **One CEF browser per tab; hide background tabs (`was_hidden`)** | Hidden tabs cost ~0 W and ~0 CPU; only memory (~45 MB each here) | [multi-tab](cef-multi-tab-cost.md) ; built in the app ([tabs](browser-tabs.md)) |
| **Cap the frame rate of visible, unfocused tabs** (about 15 fps, 5 for tiny panes), raise on input | 10 busy tabs: 28 W at 60 fps vs 22 W (focused 60 + 9 at 15) vs 15 W (+9 at 5); ramp back to 60 fps in 3-110 ms | [frame rate](cef-variable-frame-rate.md) |

## Workarounds that are required

| Problem | Workaround | Status |
|---|---|---|
| Native Wayland delivers **no accelerated frames** on AMD/Mesa (`Unable to allocate frame for first frame capture: OOM?`) | `AMD_DEBUG=nodcc` in the environment of the CEF processes (set by the app at start-up) | Measured fix on this GPU; cause suspected (DCC-compressed buffer) not confirmed; Intel analogue `INTEL_DEBUG=noccs` reported by others, untested here; NVIDIA reported to work without one |
| HiDPI blur | Device-resolution view plus zoom compensation | Verified at scale 2 only |
| Frames never arrive (rare driver cases) | Fall back to `OnPaint` | **Not built.** Needs a watchdog ("no accelerated frame within N ms") |

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
2. **Other hardware.** Only AMD/Mesa on one GNOME session. Intel, NVIDIA, other compositors
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

## Where things are

| | |
|---|---|
| Browser code | [`src/app/`](../../src/app/) (shell and `BrowserView`), [`src/cef_rt.rs`](../../src/cef_rt.rs), [`src/frames.rs`](../../src/frames.rs), [`src/gpu.rs`](../../src/gpu.rs), [`src/config.rs`](../../src/config.rs) |
| Benchmark code | [`src/bench/`](../../src/bench/): `--bench=single`, `--bench=tabs`, `--bench=fps` |
| Test pages | [`bench/`](../../bench/) |
| Notes | [spike](cef-osr-wayland-spike.md), [multi-tab cost](cef-multi-tab-cost.md), [frame rate](cef-variable-frame-rate.md), [input](cef-input.md) |
| Next steps | [`docs/roadmap.md`](../roadmap.md) |
