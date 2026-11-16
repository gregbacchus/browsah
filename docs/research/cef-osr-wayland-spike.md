# CEF windowless rendering into a GTK4 window on Wayland

**Date:** 2026-11-16 · **Code:** [`src/bench/single.rs`](../../src/bench/single.rs), [`bench/`](../../bench/)
> **Code layout note (2026-11-16, M0 restructure).** This note describes the spike as measured. Since then the code moved: the single-browser benchmark is `--bench=single` ([`src/bench/single.rs`](../../src/bench/single.rs)), the multi-tab and frame-rate benchmarks are in [`src/bench/tabs.rs`](../../src/bench/tabs.rs), shared frame code is in [`src/frames.rs`](../../src/frames.rs), and the real browser is [`src/app/`](../../src/app/). The old pump mode, `--parent=wl` and the diagnostic `BROWSAH_NO_*` toggles were removed (the findings about them stand); `BROWSAH_KEEP_DCC` became `BROWSAH_NO_GPU_WORKAROUNDS` (or `gpu.workarounds: false` in the config).


## Question

browsah wants a custom Rust/GTK4 shell around Chromium, with **CEF not owning the window**
(window chrome, tabs and layout are ours). **Native Wayland is a hard requirement.** That means
CEF must render windowless (off-screen, "OSR") and hand us frames. How fast is that, and does it
work on Wayland?

## Answer

- **It works, at 60 fps, with zero copies**, using CEF's accelerated path (`OnAcceleratedPaint`,
  dmabuf planes) imported into a `GdkDmabufTexture` (GTK 4.14+). Frames were verified visually.
- **Two things are needed that are not obvious:**
  1. On AMD/Mesa, native Wayland delivers **no accelerated frames** unless the process
     environment has `AMD_DEBUG=nodcc` (see [Wayland failure](#wayland-no-frames-without-amd_debugnodcc)).
  2. On HiDPI displays CEF renders at **1x** regardless of `device_scale_factor`, which looks
     blurry; render at device resolution and compensate with zoom (see [HiDPI](#hidpi-blurry-frames-and-the-backing-scale-fix)).
- **The classic readback path (`OnPaint`) also works** and is the fallback. At 2880x1568 it
  copies about 1.1 GB/s and spends 1.7 ms of the 16.7 ms frame budget, versus 0 and 0.01 ms.
- **Unproven:** anything on other GPUs (NVIDIA, Intel), fractional scales, multiple browsers,
  input latency, and the cause of the Mesa problem.

## Environment

| | |
|---|---|
| Machine | Laptop, AMD Radeon 890M (radeonsi, RADV), Mesa 26.0.8 |
| Session | GNOME / Mutter, native Wayland, display scale 2 (1440x900 logical) |
| Toolkit | GTK 4.22.4, libadwaita 1.9.1, `gtk4` crate 0.11.5 |
| CEF | `cef` crate 156.5.0 (CEF 156.0.28), prebuilt binaries, Alloy runtime style |
| Rust | 1.99.0 |

## Approach

A GTK4 window containing one `GtkPicture`. CEF runs windowless with `external_message_pump`
(pumped from the GLib main loop) and `windowless_frame_rate = 60`. Two modes:

| `--mode=` | CEF callback | What reaches GTK |
|---|---|---|
| `paint` | `OnPaint` (GPU readback to CPU memory) | copy into `GdkMemoryTexture`; GSK uploads it |
| `accel` | `OnAcceleratedPaint` (shared texture) | dmabuf fds dup'd into a `GdkDmabufTexture`; no copy |

The test page [`bench/scroll.html`](../../bench/scroll.html) is tall, paint-heavy and scrolls
itself so every frame is a full-viewport change. [`bench/static.html`](../../bench/static.html)
is a fixed page of small text, 1px lines and borders for judging sharpness.

## Results

Native Wayland (`--ozone-platform=wayland`), 20 s per run, warm-up excluded, `AMD_DEBUG=nodcc`.

**At 1x (frame 1280x800), window 1280x800 logical:**

| mode | CEF paints/s | paint callback | copied | GTK frame interval p95 |
|---|---|---|---|---|
| `paint` | 60 | 0.44 ms | 271 MB/s | 16.7 ms |
| `accel` | 60 | 0.02 ms | 0 | 16.7 ms |

**At device resolution (frame 2880x1568), the real workload on this display:**

| mode | CEF paints/s | paint callback | copied | GTK frame interval p95 |
|---|---|---|---|---|
| `paint` | 60 | 1.70 ms | 1084 MB/s | 16.7 ms |
| `accel` | 60 | 0.01 ms | 0 | 16.7 ms |

- Both modes hold 60 fps with the same frame pacing here. The difference is CPU/memory
  bandwidth and headroom, which grows with pixel count (4K, several tabs).
- "paint callback" is time inside our callback only. In `paint` mode the texture upload happens
  later in GSK and appears as frame-interval jitter, not in that column.
- The first table understated readback cost 4x because it ran at 1x (see HiDPI).
- Not controlled: window size differed between a few early runs, so only compare numbers
  within one table.
- Pixel output: `paint` and `accel` produced byte-identical-size PNGs of the static page.

## Wayland: no frames without `AMD_DEBUG=nodcc`

**Symptom.** With `--ozone-platform=wayland` and `shared_texture_enabled=1`, neither
`OnAcceleratedPaint` nor `OnPaint` is ever called. The only log line is
`ERROR: components/viz/service/frame_sinks/video_capture/frame_sink_video_capturer_impl.cc:939]
Unable to allocate frame for first frame capture: OOM?`, while `video_capture_oracle` keeps
proposing capture sizes. With `--ozone-platform=x11` (XWayland) the same code delivers 60 fps.

**Workaround (measured).** Put `AMD_DEBUG=nodcc` in the environment of the CEF processes. The
browser sets it at start-up on AMD GPUs (`BROWSAH_NO_GPU_WORKAROUNDS=1` or `gpu.workarounds: false` disables that to reproduce the failure). Result:
60 paints/s, zero copy, correct pixels, with or without `--parent=wl`.

**Cause (suspected, not confirmed).** radeonsi gives the capture pool's scanout buffer a
DCC-compressed layout that Chromium's capturer cannot use, so allocation fails. Only the
workaround is verified; the mechanism is inferred from the reports below.

**Prior reports** (found with a multi-source web search, adversarially verified):

- [Lantharos/Sabine#3](https://github.com/Lantharos/Sabine/pull/3): same error on a Radeon 780M,
  Mesa 26.2.3, CEF 156; `AMD_DEBUG=nodcc` made the first GPU frame arrive. Single report.
- [Lantharos/Sabine#2](https://github.com/Lantharos/Sabine/pull/2),
  [moq-dev/studio#59](https://github.com/moq-dev/studio/pull/59),
  [#107](https://github.com/moq-dev/studio/pull/107): same error on other Mesa GPUs (Intel UHD
  630, Mesa 26.2.4). On the same project, NVIDIA proprietary on GNOME Wayland painted 9/9 tests.
- [moq-dev/studio#111](https://github.com/moq-dev/studio/pull/111): Intel equivalent,
  `INTEL_DEBUG=noccs` (avoids the two-plane CCS modifier); browser tests 3/14 to 14/14. **Not
  tested here.**
- **No CEF/Chromium maintainer statement or upstream bug was found**, and release builds strip
  the allocation-failure detail, so this may be a gap upstream.

### Things that did not fix it (all with `--ozone-platform=wayland`)

| Tried | Result |
|---|---|
| `--use-angle=gl`, `--use-angle=gl-egl`, `--use-gl=egl` | 0 frames (`--use-gl=egl` crashed the GPU process) |
| `--use-gl=angle --use-angle=vulkan --enable-features=Vulkan,DefaultANGLEVulkan,VulkanFromANGLE` (the flags purego-cef2gtk uses) | 0 frames |
| `--disable-gpu-sandbox`, `--in-process-gpu`, `--no-sandbox` | 0 frames |
| `--disable-features=WaylandLinuxDrmSyncobj`, `--disable-features=UseSkiaGraphite` | 0 frames |
| `--enable-native-gpu-memory-buffers`, `--disable-gpu-memory-buffer-video-frames` | 0 frames |
| `--enable-features=WaylandOverlayDelegation` | 0 frames |
| Passing GTK's `wl_surface` as CEF `parent_window` (`--parent=wl`), alone or with the Vulkan flags | 0 frames |
| ANGLE on SwiftShader | 0 frames |
| `--disable-gpu-compositing` | frames flow, but via CPU `OnPaint` (271 MB/s copied), so not zero-copy |
| `--ozone-platform=x11` | works zero-copy, but Chromium runs through XWayland, which breaks "native Wayland" |

`chrome://gpu` differs between the two backends (read over the DevTools port, see
[Reproduce](#reproduce)): Wayland uses ANGLE on OpenGL ES with `RGBA_8888` buffers; X11 uses
ANGLE on desktop OpenGL 4.6 with `BGRA_8888`. That explains different behaviour but not the
failure on its own.

## HiDPI: blurry frames and the backing-scale fix

**Symptom.** On the scale-2 display the output looked low-resolution. The static page reported
`devicePixelRatio 1` and the frame was 1440x784 px for a 1440x784 *logical* view: CEF rendered
at 1x and GTK stretched it 2x. This is independent of the paint path.

**Fix (on by default when scale > 1; `--backing=off` disables).** Tell CEF the view is
device-sized at 1x (`view_rect` = logical x scale; `screen_info.device_scale_factor` = 1), and
compensate with browser zoom, `zoom_level = ln(scale) / ln(1.2)` (CEF zoom factor is `1.2^level`).
Result: 2880x1568 frame for a 1440x784 logical view, `devicePixelRatio 2`, crisp text and 1px
lines, verified on a dumped PNG. The same technique is described by
[purego-cef2gtk](https://pkg.go.dev/github.com/bnema/purego-cef2gtk@v0.5.6) (its "OSR backing
scale"), which attributes it to CEF shared-texture builds emitting 1x/logical frames despite a
fractional scale factor.

**Caveats.** Zoom is re-applied every 30 GTK frames because navigation resets it (a real
browser needs proper per-site zoom handling). Only scale 2 was tested; fractional scales
(1.25, 1.5) are untested.

## Prior art: how other Chromium browsers handle windowing on Linux

None of the well-known browsers use CEF; they fork Chromium and use its own windowing:

| Browser | Approach |
|---|---|
| Chrome, Edge, Brave, Opera | Full Chromium tree plus patches, rebased every release. Window and widgets come from Chromium's Views/Aura with the Ozone platform layer (X11/Wayland). Chromium draws its own title bar, which is why they never look like GTK apps. |
| Brave | A `brave-core` overlay on Chromium (files replaced via `chromium_src` overrides). |
| Vivaldi | Chromium content layer, but the browser UI is itself a web app rendered by Chromium. |
| Qt WebEngine (qutebrowser, Falkon) | Native Qt UI around Chromium's content layer, composited into the Qt scene graph as GPU textures. The closest precedent for this project's design. |
| Epiphany (GNOME Web) | WebKitGTK, not Chromium. |

Forking costs a rebase and security-update treadmill; CEF (what browsah uses) tracks Chromium
releases at a lower maintenance cost but exposes less.

Related Wayland CEF-into-GTK project: [bnema/purego-cef2gtk](https://github.com/bnema/purego-cef2gtk)
(Go). It reports working Wayland dmabuf OSR into `GdkDmabufTextureBuilder`, defaults to ANGLE
on Vulkan, and passes the GTK surface as `parent_window`. Neither of those two settings was
needed here.

## Implications for browsah

1. **Architecture is viable:** GTK4 shell, windowless CEF, `OnAcceleratedPaint` into
   `GdkDmabufTexture`. CEF never owns a window. **Run CEF on its own thread**
   (`multi_threaded_message_loop`), not pumped from the GTK loop, or the main thread spins at
   100 % CPU; hand frames to GTK with `MainContext::invoke`.
2. **Plan for GPU-specific environment handling.** The Mesa workaround is AMD-specific here;
   Intel likely needs its own (`INTEL_DEBUG=noccs`, untested), NVIDIA appears to work without
   one. Needs a detection and per-vendor policy, plus a runtime fallback to `OnPaint` when no
   accelerated frame arrives (a watchdog on "no frame within N ms").
3. **Always render at device resolution** and own zoom/scale handling.
4. **Largest remaining risk is input,** not rendering: keyboard, IME, mouse/touch, scrolling
   feel, drag and drop, popups (`<select>`), cursors, accessibility. Input latency is not yet
   measured.

## Known gaps in the spike

- **Resolved:** the benchmark tables above were taken with CEF's pump on the GTK thread, which
  pins the main thread at 100 % CPU. Fixed by running CEF on its own thread; paint rate, frame
  pacing and pixels were unchanged in a re-run (60 paints/s, p95 16.7 ms). Cause, fix and the
  new CPU/power comparison (accelerated uses ~7x less main CPU and ~21 % less package power than
  readback) are in [cef-multi-tab-cost.md](cef-multi-tab-cost.md#resolved-the-100--main-thread-spin-its-cause-and-fix).
- No input, IME, popup widgets, drag and drop, or cursor handling.
- CEF sandbox disabled (`no_sandbox`); Linux sandbox setup not investigated.
- `external_begin_frame_enabled` off: Chromium paces itself rather than following GTK's frame
  clock. Likely worth testing for pacing and latency.
- dmabuf texture uses `coded_size`, which may include padding beyond the visible rect.
- Single browser, single view; multi-tab behaviour and buffer pool pressure not tested.
- Crude shutdown (50 message-loop iterations).

## Open questions

- Why exactly does the Mesa capture allocation fail, and is there a proper fix (modifier
  filtering) rather than disabling DCC for the whole process tree? Worth a minimal upstream
  CEF/Chromium bug report.
- Does it work without the workaround on newer Mesa/CEF, or on NVIDIA/Intel here?
- Fractional scale (125%/150%) and multi-monitor mixed-scale behaviour.
- Cost of `AMD_DEBUG=nodcc` on other GPU work in the same process tree (it also affects GTK/GSK
  if set globally; it is set before GTK starts in the spike).
- Input latency of OSR versus a native window.

## Reproduce

From the repo root (first build downloads about 300 MB of prebuilt CEF):

```bash
cargo build --release
export LD_LIBRARY_PATH=$PWD/target/release

# benchmark both paths (prints a TOTAL line)
./target/release/browsah --bench=single --mode=paint --seconds=20
./target/release/browsah --bench=single --mode=accel --seconds=20

# reproduce the Wayland failure (no frames), then the fix
BROWSAH_NO_GPU_WORKAROUNDS=1 ./target/release/browsah --bench=single --mode=accel --seconds=5
./target/release/browsah --bench=single --mode=accel --seconds=5

# sharpness check: saves the final frame as a PNG
BROWSAH_DUMP=/tmp/out.png ./target/release/browsah --bench=single --mode=accel --seconds=5 \
    --url=file://$PWD/bench/static.html
./target/release/browsah --bench=single --mode=accel --seconds=5 --backing=off   # the blurry 1x behaviour
```

Useful flags: `--ozone-platform=x11|wayland` (and any other Chromium switch), `--backing=off`. Chromium's GPU report can be read by
loading `chrome://gpu` with `--remote-debugging-port=9333` and evaluating the page text over the
DevTools WebSocket.

## Sources

- [bnema/purego-cef2gtk](https://pkg.go.dev/github.com/bnema/purego-cef2gtk@v0.5.6) and its
  [command-line setup](https://raw.githubusercontent.com/bnema/purego-cef2gtk/main/internal/cefadapter/command_line.go)
- [Lantharos/Sabine#2](https://github.com/Lantharos/Sabine/pull/2),
  [#3](https://github.com/Lantharos/Sabine/pull/3)
- [moq-dev/studio#59](https://github.com/moq-dev/studio/pull/59),
  [#107](https://github.com/moq-dev/studio/pull/107),
  [#111](https://github.com/moq-dev/studio/pull/111)
- [tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs) (the `cef` crate)
