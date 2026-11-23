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

## Update 2026-11-23: Not Responding "all the time" (the `home` profile, same machine)

**Measured** (`journalctl --user -t browsah`, 21-23 November): the main-loop watch logged only three stalls in two days, all of the
whole process, none of them short:

| When | Blocked | Main thread when first seen |
|---|---|---|
| 22 Nov 08:29 | 29 s | running (state R) |
| 22 Nov 18:20 | 14 s | asleep in `ppoll` |
| 23 Nov 07:26 | 12 s | asleep in `ppoll` |

In the last two the loop was *idle* (in `poll`, nothing to run) while its own 100 ms timer and the watch thread's 250 ms sleep were
both late by 12-14 s and both ran again within about 50 ms of each other. That is the whole process (or machine) not being
scheduled, not a handler that blocked the loop: our code cannot hold a main loop that is asleep in `poll`. The sweep pass that
follows each line is a consequence (its 30 s timer fires when the process wakes), not a cause. No other stall of 750 ms or more was
logged, so the many Not Responding dialogs are not explained by a blocked GTK thread in this process.

**Not found / not measured:** what froze the process. `/proc/pressure` was clean when looked at afterwards (memory `some` 16 µs in
total since boot), so memory pressure is not shown for the 23 Nov freeze; the journal has systemd "user.slice: Unit now frozen" lines
at other times (suspend), none at these three. `at-spi2-registryd` also logged "Disabling unresponsive app" for two pids on
21 and 22 Nov (accessibility D-Bus calls unanswered; not checked whether they were browsah's), a separate path from the compositor ping.

**Change:** the stall line now also carries the machine's CPU, memory and I/O pressure (`/proc/pressure`, `some`, 10 s average), so the
next freeze says whether the machine was short of one of them. If dialogs keep coming with no stall line, the cause is not this
process's main loop: look at `journalctl -b -p warning` around the time (gnome-shell, mutter) for the window that was flagged.

### Same day, later: the watch and the dialogs disagree

The user counts about ten GNOME "Not Responding" dialogs on 23 Nov (the name varies with the frozen window; once `Browsah (Work)`, around
activity such as returning to a window or a notification; one around a Teams web app) while the journal has one stall line for the
day (07:26, above) and none since the 13:14 boot. So the dialogs are not (all) a main loop that stopped for 0.75 s or more. Measured:
`check-alive-timeout` is the default 5000 ms; no gnome-shell or mutter line about a ping in the journal; only the `home` profile is
running (the `work` profile has no process and no log lines today).

Not yet known: whether the compositor's ping reaches us late or our pong leaves late, or whether the dialog is about something else.
`scripts/ping-watch.py` answers the first part: `WAYLAND_DEBUG=client browsah --profile=home 2>&1 | scripts/ping-watch.py` prints every
`xdg_wm_base` ping answered later than a second (or never) with the client's own timestamps. The script was checked on made-up input and
on the real debug line format (registry lines only; a real ping was not seen), not against a live dialog.

**Resolved for the Work machine:** the stalls there are disk waits in `fsync` on the GTK thread, see [2026-11-23-fsync-stalls.md](2026-11-23-fsync-stalls.md). The reading above (and the "same day, later" section) is about the Home machine, whose three freezes are still unexplained.
