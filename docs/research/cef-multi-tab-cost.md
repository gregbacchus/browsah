# Cost of many tabs, and tab-sleeping wake times (CEF windowless, GTK4, Wayland)

**Date:** 2026-11-16 · **Code:** [`src/bench/tabs.rs`](../../src/bench/tabs.rs), [`bench/tab.html`](../../bench/tab.html) ·
Builds on [cef-osr-wayland-spike.md](cef-osr-wayland-spike.md) (same machine and versions).

## Question

With one windowless CEF browser per tab rendering into a GTK4 window: what do many tabs cost
in memory, CPU, GPU and power, can tabs sleep, and how long does a sleeping tab take to wake?

## Answer (for this machine and these test pages; see caveats)

Measured with CEF on its own UI thread (the spin-free mode, see [below](#resolved-the-100--main-thread-spin-its-cause-and-fix)),
10 tabs, 2880x1568, native Wayland, zero-copy.

- **Hidden tabs are free.** Nine hidden tabs, even busy ones (animations plus a 100 ms timer),
  added about **0 W and ~0.3 % of a core each**. Idle tabs add nothing.
- **Visible tabs are what costs power.** Ten busy tabs all visible: **33.5 W package power vs
  9.2 W with one visible**, about **+2.7 W per extra visible busy tab**, ~43 % of a core and
  ~112 MB of GPU memory each, GPU 99 % busy. A single full-screen busy tab was +5.6 W over an
  empty window.
- **Memory is the other cost:** about **44-48 MB PSS per extra tab** for these small pages
  (real sites will be much larger), one renderer process per tab. Hiding does not free it.
- **Wake times are short:** hidden to a fresh frame **~8-9 ms**, frozen **~8-14 ms**, discarded
  (new browser, local page) **~70-80 ms to first frame**. Real network pages will be slower.
- **Discarding frees ~42 MB per tab** of PSS. **Freezing gained nothing measurable** over
  hiding in CPU, power or wake time, and its memory effect was inconsistent (see below).
- **An idle visible tab costs 0.8-1.0 % main CPU and ~3 W package power**, the same as an empty
  GTK window, so an idle browser itself is cheap.

## Resolved: the 100 % main-thread spin, its cause and fix

**Symptom.** Whenever CEF was initialised alongside the GTK main loop, the browsah main thread
used a full core, even idle, even on a static page, even with no browser created. With CEF
skipped it idled at 0.3 %.

**Cause (established).** Chromium's browser-process message pump attaches its GLib sources to
the process's default GLib main context, the same one GTK iterates. Dispatched by GTK's loop
rather than by a Chromium run loop, one of those sources is permanently "ready":
`strace` showed the main thread calling `ppoll` on the GLib wakeup eventfd, a pipe (Chromium's
pump wakeup) and a unix socket, with a **zero timeout, about 100 000 times a second, and no
other syscalls**: a busy poll with nothing to do. The main thread almost never blocks (about 190
voluntary context switches in 7 s).

**Fix.** Run CEF on its own thread: `Settings.multi_threaded_message_loop = true`
(`external_message_pump` off, no `do_message_loop_work` calls). Chromium's pump then never
touches the GTK context. Main thread idle CPU: **0.3 %** (see table below).

What it requires (done in [`src/app/view.rs`](../../src/app/view.rs) for the browser and [`src/bench/tabs.rs`](../../src/bench/tabs.rs) for the benchmark, with shared code in [`src/frames.rs`](../../src/frames.rs) and [`src/cef_rt.rs`](../../src/cef_rt.rs)):

- CEF callbacks (`view_rect`, `screen_info`, `on_paint`, `on_accelerated_paint`,
  `on_after_created`) now run on CEF's UI thread, not the GTK thread. Shared state must be
  thread-safe: view size/scale in a `Mutex`, paint counters as atomics, the `BrowserHost` handle
  in a mutex (set from `on_after_created`).
- Frames cross to the GTK thread with `glib::MainContext::invoke`: `on_paint` copies into a
  `glib::Bytes`; `on_accelerated_paint` dups the dmabuf fds into a plain-data `DmabufFrame`
  (fds, offsets, strides, modifier). The `GdkTexture` is built on the GTK thread. (`invoke`
  runs inline when already on that thread, so the same code works in both pump modes.)
- The browser is created with the asynchronous `browser_host_create_browser`; the host handle
  arrives in a `LifeSpanHandler`.
- My first multi-threaded attempt crashed with heap corruption (`malloc(): corrupted top
  size`) because the handlers still read GTK-thread-local state, so CEF got an empty view
  rect. Rendering was checked visually afterwards (2880x1568, sharp, correct scroll position).

**Ruled out first** (each left the spin at ~100 %; the diagnostic toggles used for this, and the old pump mode, were removed in the M0 restructure): the GSK renderer (default/ngl/vulkan/cairo),
the GTK tick callback, my pump timer and on-demand scheduling, `external_message_pump` off,
the Wayland backend and the GPU (`--ozone-platform=x11`, `--disable-gpu`), CEF's signal
handlers, and giving CEF a private GLib context during `initialize`.

### Effect on idle and active cost (10 s windows after 7 s warm-up, 2880x1568)

Main-process CPU is all threads of the main process (GTK plus CEF's UI thread); 100 % is one
core. Package power is the amdgpu hwmon reading (whole APU, laptop on charger, other work
running): use it to compare rows measured back to back, not as an absolute.

| Scenario | Main CPU | Children CPU | Package power |
|---|---|---|---|
| GTK window only (no CEF) | 0.4 % | 0.0 % | 3.8 W |
| **CEF on its own thread**, idle static page | **0.5 %** | 0.0 % | **3.5 W** |
| CEF pump on GTK thread (old), idle static page | 100.6 % | 0.0 % | 13.9 W |
| **Accelerated (dmabuf)**, scrolling page, own thread | **6.2 %** | 50.9 % | **10.8 W** |
| **Readback (`OnPaint`)**, scrolling page, own thread | **41.1 %** | 60.1 % | **13.6 W** |
| Accelerated, scrolling, pump on GTK thread (old) | 102.2 % | 52.1 % | 23.2 W |
| Readback, scrolling, pump on GTK thread (old) | 101.6 % | 58.2 % | 23.8 W |

- The spin was costing about **10 W of package power at idle** and roughly doubling it while
  rendering. A browser that did this would have been unshippable.
- With the spin gone, the benefit of zero-copy is clearly visible: about **7x less main-process
  CPU (6 % vs 41 %), 9 percentage points less child CPU (51 % vs 60 %) and about 2.8 W (21 %) less package power** while
  scrolling at 60 fps on a 2880x1568 view. Earlier benchmarks only showed 0 vs 1.1 GB/s copied
  and a 1.7 ms callback; this is the system-level effect.
- Single run per row, one machine, no repeats.

## Method

`--bench=tabs --tabs=10`, native Wayland, accelerated (dmabuf) mode, 2880x1568 frames
(scale 2), AMD Radeon 890M, CEF on its own UI thread. Each scenario settles for 3-8 s, then is
sampled for 5 s.

- **PSS** summed over the whole process tree from `/proc/*/smaps_rollup` (shared pages counted
  proportionally).
- **CPU** from `/proc/*/stat`, split into the main process (GTK plus CEF's UI thread) and its
  children (renderers, GPU and utility processes). 100 % = one core.
- **Power / GPU busy / VRAM / GTT** from amdgpu sysfs and hwmon: package power of the whole APU
  (CPU and GPU), so it includes everything else running on the machine. GTT and GPU busy are
  system-wide counters; on this APU the GPU buffers show up in GTT, not VRAM.
- **Wake time** is from the call (`was_hidden(false)`, DevTools unfreeze, or creating the
  browser) to the first `OnAcceleratedPaint` callback for that tab, timestamped on CEF's thread.
  It measures delivery to our code, not photons on screen.
- **Tiers:** *hidden* = `was_hidden(true)`. *Frozen* = hidden plus DevTools
  `Page.setWebLifecycleState` `frozen` (CEF accepted the call for all nine tabs; the effect was
  inferred from CPU/memory, not confirmed in the page). *Discarded* = browser closed, recreated
  on wake.
- **Pages:** `live` = 360 animated cells plus a 100 ms timer and a 200k-object heap; `static` =
  same DOM and heap, no animation or timer.

## Results

### Live (busy) pages

| Scenario | Renderers | PSS MB | Main CPU % | Children CPU % | Power W | GPU busy % | GTT MB |
|---|---|---|---|---|---|---|---|
| App only, no tabs | 1* | 273 | 1.0 | 0.0 | 4.1 | 1 | 2892 |
| 1 tab, visible | 3 | 495 | 6.8 | 32.8 | 9.7 | 24 | 2998 |
| 10 tabs: 1 visible, 9 hidden | 12 | 927 | 6.2 | 34.0 | 9.2 | 24 | 3176 |
| 10 tabs: all visible | 12 | 1329 | 13.8 | **417.6** | **33.5** | **99** | 4186 |
| 9 hidden + frozen | 12 | 968 | 5.8 | 34.6 | 9.9 | 24 | 3654 |
| 9 discarded (tab 0 alive) | 3 | 547 | 6.2 | 39.0 | 9.6 | 24 | 3282 |

### Static (idle) pages

| Scenario | Renderers | PSS MB | Main CPU % | Children CPU % | Power W | GPU busy % | GTT MB |
|---|---|---|---|---|---|---|---|
| App only, no tabs | 1* | 273 | 0.8 | 0.0 | 4.4 | 1 | 2892 |
| 1 tab, visible | 3 | 442 | 1.0 | 0.2 | 3.0 | 0 | 2996 |
| 10 tabs: 1 visible, 9 hidden | 12 | 842 | 0.8 | 0.0 | 3.0 | 0 | 3177 |
| 10 tabs: all visible | 12 | 841 | 0.8 | 2.6 | 3.1 | 0 | 3765 |
| 9 hidden + frozen | 12 | 770 | 0.8 | 0.0 | 3.0 | 0 | 3812 |
| 9 discarded (tab 0 alive) | 3 | 465 | 0.8 | 0.0 | 3.0 | 0 | 3539 |

\* The renderer column is a heuristic (children of the non-GPU zygote) and includes a couple of
non-tab helpers: about one renderer per tab plus 2.

### Derived per-tab costs (live / static)

| | Live | Static |
|---|---|---|
| Extra hidden tab, PSS | 48 MB | 44 MB |
| Extra hidden tab, CPU | ~0.1 % of a core | ~0 |
| Extra hidden tab, power | ~0 W (within noise) | ~0 W |
| Extra *visible* tab, PSS over hidden | 45 MB | ~0 |
| Extra visible tab, children CPU | ~43 % of a core | ~0.3 % |
| Extra visible tab, power | ~2.7 W | ~0 W |
| Extra visible tab, GTT (GPU) memory | ~112 MB | ~65 MB |
| Freed by discarding | 42 MB | 42 MB |

### Wake time to first fresh frame (5 tabs each)

| From | Live median (max) | Static median (max) |
|---|---|---|
| Hidden | 9 ms (20) | 8 ms (11) |
| Frozen | 14 ms (15) | 8 ms (14) |
| Discarded, to first frame | 82 ms (100) | 71 ms (74) |
| Discarded, to load complete | 71 ms (76) | 64 ms (66) |

### Earlier run (CEF pumped from the GTK thread, main thread spinning)

The same benchmark taken before the fix gave the same child CPU, PSS, GPU busy and wake times
within noise (for example 10 tabs all visible: 487 % children CPU, PSS 1267 MB; hidden wake
14 ms live; discarded 85 ms live), but main CPU was pinned at ~100 % and the power reading was
flat at 17.0-17.3 W in every live scenario. The spin was swamping or saturating the power
sensor, which is why those power numbers were discarded.

## Interpretation

1. **Background tabs cost memory only.** Chromium stops frame production for hidden pages and
   throttles their timers, so 9 hidden busy tabs added no measurable CPU or power.
2. **Power scales with visible, animating tabs.** About 2.7 W per extra visible busy tab here,
   dominated by GPU and compositing work. Split view or tiling many live tabs is where a custom
   shell would feel it. Keep non-visible tabs hidden, and consider capping the frame rate of
   small or unfocused visible panes.
3. **Idle pages are nearly free even when visible:** 10 visible static tabs cost 3.1 W package
   power and 2.6 % child CPU, the same as one.
4. **Tab sleeping is worth it for memory only.** Discarding recovers ~42 MB per tab here at a
   cost of a ~70-80 ms wake for a local page. Freezing bought nothing over hiding: CPU, power
   and wake time were all within noise, and PSS moved inconsistently (+41 MB on live pages,
   -72 MB on static pages, in both the old and new runs), which I cannot explain.
5. **A discarded tab can keep its last frame as a placeholder** for an instant-feeling switch,
   but the reload itself will be network-bound for real sites.
6. **Per-tab memory here is a floor.** These pages are tiny; real pages are typically 5-20x
   larger, so tab discarding matters more than these numbers suggest.

## Caveats (important)

- **Single machine, single run per scenario, no repeats or error bars.** Power noise looks
  like roughly +/-1 W (for example 4.4 W for the empty window vs 3.0 W for the same window with
  an idle tab), so only large differences are meaningful. Frozen vs hidden is within noise.
- **Power is whole-APU package power** from one sensor, with the laptop on the charger and
  other software running. It is a relative measure between back-to-back rows, not an absolute
  figure for the browser. A second source (battery discharge rate, `powertop`, an external
  meter) has not been used to confirm it.
- **GTT and GPU busy are system-wide** and move with other applications. GTT did not fall after
  discarding or freezing, so freed GPU memory is not demonstrated.
- **Hidden for only ~5-8 s.** Chromium's stronger background throttling (intensive timer
  throttling after about 5 minutes) is untested, as are tabs swapped out by the OS under memory
  pressure, which would make wake slower.
- **Local `file://` pages, warm cache, tiny heaps.** No network, no heavy JS, no video or WebGL.
- **Frozen effect unconfirmed:** CDP accepted the call, but I did not verify page execution had
  stopped.

## Reproduce

```bash
cargo build --release
export LD_LIBRARY_PATH=$PWD/target/release
./target/release/browsah --mode=accel --bench=tabs --tabs=10 --tab-mode=live
./target/release/browsah --mode=accel --bench=tabs --tabs=10 --tab-mode=static
```

Takes about three minutes per run and prints the tables above. Needs an AMD GPU for the
VRAM/GTT/power columns (other columns work anywhere).

## Open questions

- Repeat runs and a second power source (battery discharge, `powertop`, an external meter), on
  battery rather than the charger.
- Real-site pages (heavy JS, video, WebGL, ads) and many more tabs (50-100).
- Does `Page.setWebLifecycleState` frozen actually stop execution under OSR, and why does it
  raise PSS on live pages and lower it on static ones? Compare with Chromium's own discard and
  with `Page.enable` first.
- GPU memory per visible tab: does the capture buffer pool shrink for hidden tabs, and can the
  frame rate be lowered for visible-but-unfocused panes to cut the ~2.7 W per visible busy tab?
- Memory-pressure behaviour and wake latency when a tab has been swapped out.
