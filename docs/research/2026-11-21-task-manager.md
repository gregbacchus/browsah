# Task manager: per-tab CPU and memory (2026-11-21)

**Answer:** `Shift+Esc` (or the main menu) opens a task manager. Per tab it shows the page's main-thread CPU, script memory and element
count (from DevTools `Performance.getMetrics`); per process it shows CPU and memory (from `/proc`). Tabs can be put to sleep or closed from
their rows. Nothing is sampled while the window is closed.

**Environment:** Ubuntu, GNOME Wayland, AMD GPU, CEF 156 (Chromium 156.0.8078.88), this program's `--selftest`.

## Measured

- `Performance.enable` + `Performance.getMetrics` answer through `execute_dev_tools_method` on a windowless CEF browser. A page that
  spins its main thread about 70% of the time (`bench/pages/busy.html`: 70 ms busy, 30 ms idle) reads 70-71% (`taskmanager` self-test).
  A quiet page reads 0-0.1%. `TaskDuration` over the page's own `Timestamp` clock gives the share.
- `TaskDuration` is the **main thread's task time only**: not web workers, not the compositor, not network or the GPU. The column is
  therefore called "CPU" under a heading that says main thread, and the process list is shown separately.
- Processes: Chromium forks the page renderers, the GPU process and most utilities **from a zygote and they keep the zygote's command line**
  (`--type=zygote`), so `--type=` cannot tell them apart. Their threads can: the GPU process has `GpuWatchdog` and `VizCompositorTh`, a page
  renderer has a `Compositor` thread (read from `/proc/PID/task/*/comm`). Zygotes themselves are direct children of the browser process and
  are left out. The network service is started directly and does carry `--type=utility --utility-sub-type=network.mojom.NetworkService`.
  1Password's native messaging host is a child of the browser and is listed as a helper program.
- Memory is the proportional set size (`smaps_rollup` `Pss`: the shared CEF library is split between the processes that map it), else `VmRSS`.

## Not measured / decisions

- **A tab's own process cost.** Chromium shares one renderer between same-site tabs and swaps it on cross-site navigation, so a pid per tab
  is unreliable; the per-tab numbers come from the page instead, and the process list says a renderer can serve several tabs.
- What the counters do when a page navigates (they should start again, so the delta goes negative): `tab_cpu_percent` treats that as a new
  baseline and is unit tested, but a real navigation was not watched.
- Out-of-process iframes are not part of the page's counters. Modal popup views are not listed. Split tabs list each pane.
- The process scan reads `smaps_rollup` for about 10-40 processes; it runs on a worker thread. Its cost was not measured.
- No config option was added (the refresh beat is fixed at 1.5 s).

## Reproduce

```bash
BROWSAH_SHOTS=/tmp/shots BROWSAH_SELFTEST_ONLY=taskmanager ./target/release/browsah --selftest   # /tmp/shots/task-manager.png
cargo test --release taskman
```
