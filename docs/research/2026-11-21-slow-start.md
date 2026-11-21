# Slow start and "Not Responding" (2026-11-21)

**Question:** sometimes browsah starts slow and GNOME shows its Not Responding dialog.

**Environment:** Ubuntu, GNOME Wayland, AMD GPU, 24 cores, 30 GB RAM (10 GB swap in use), the `home` profile (4 workspaces, 2 web apps, 1Password extension, about 21 remembered tabs).

## Measured

- Journal (`journalctl -t browsah`, plus the systemd scope lines for `be.geee.browsah.home`) for 19-21 November: every run that logged
  `no accelerated frame within 3s; falling back to readback rendering` used far more CPU than the others, for example 9m58s CPU in
  10m20s wall (08:31 on 20 Nov), 9m21s in 46m, 3m10s in 3m54s. Runs without the fallback: 13s in 13m, 1m in 17m.
  Readback costs about 7x the main-process CPU (`docs/config/options.md`, `rendering.mode`).
- One fallback was logged six minutes into a run, in a session whose other tabs were already showing accelerated frames. So the
  3 s watchdog was switching the whole process to readback because **one tab** was slow to paint, not because the GPU path was broken.
- Not reproduced: a copy of the profile started with a warm cache, with Mesa's shader cache disabled, and against a server that
  never answers, showed no fallback and no main-loop stall (about 1.5 s of main-thread CPU in the first 20 s).
  A cold start (cold disk cache, a busy system at login) could not be tried here.
- Nothing on the GTK thread at start-up blocks on I/O or on CEF (read through `src/app` for waits, joins, child processes, large reads);
  the 21 screenshots are 3.4 MB in total.

## Suspected (inferred from the journal, not reproduced)

A slow first frame (busy start, cold caches, GPU process still starting) tripped the watchdog, which is irreversible for the process
(`Shell::fall_back` sets `Mode::Paint` "for every later tab"), so the browser crawled and the compositor's ping went unanswered.

## Change

- The watchdog no longer falls back once any view of the process has received an accelerated frame
  (`view::accelerated_path_works`); it logs an info line instead. With no accelerated frame at all it falls back as before
  (checked: `BROWSAH_NO_GPU_WORKAROUNDS=1` self-test `basic` still falls back; without it `basic` does not).
- New: a main-loop watch (`src/app/stall.rs`). When the GTK thread does not run for 750 ms the journal gets a warning with the
  thread's state and system call (`journalctl -t browsah -p warning`, or `browsah log`). If Not Responding comes back, that line says
  whether the thread was running (our code or drawing) or waiting (a lock or another thread).

## Reproduce

`BROWSAH_NO_GPU_WORKAROUNDS=1 BROWSAH_SELFTEST_ONLY=basic ./target/release/browsah --selftest` shows the fallback;
`XDG_*` pointed at a temporary folder with a copy of a profile (renamed, so the application id differs from a running one) shows the start.
