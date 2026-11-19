# The log

browsah logs to the **system journal**, so "why did that happen?" (why was that tab swept, why did rendering fall back) can be
answered afterwards with `journalctl`, without having started the browser from a terminal.

Every entry has the identifier `browsah`, a priority, and the fields `BROWSAH_PROFILE` (the profile that wrote it) and
`CODE_TARGET` (the part of the program, e.g. `app::sweep`).

```bash
journalctl --user -t browsah -e                          # the end of the log (drop --user if your entries are in the system journal)
journalctl -t browsah BROWSAH_PROFILE=default -g sweep   # one profile, lines mentioning "sweep"
journalctl -t browsah -p warning --since today           # warnings and errors from today
journalctl -t browsah -f                                 # follow
```

`browsah log` runs the same thing for you: `browsah log --grep=sweep --lines=100`, `--profile=ID` (without it every profile is shown, and
each line starts with `[profile]`), `--level=warn`, `--since="1 hour ago"`, `--follow`.

A line, as `journalctl` prints it:

```
2026-11-19T10:12:03+13:00 host browsah[4242]: sweep Some("work"): rule "Stale" Close https://example.com/a (title "A", idle 2h5m, pinned false, awake)
2026-11-19T10:12:03+13:00 host browsah[4242]: tab closed: https://example.com/a (workspace Some("work"), reason sweep:Stale, 4 left)
```

Journal entries are kept (and rotated) by journald according to the system's own settings (`journald.conf`). Where there is no
journal (a system without systemd) nothing is stored; warnings and errors still print on the terminal.

## What is logged

- `info` (the default): start-up, every tab a sweep closes or puts to sleep (rule, address, title, idle time, whether it was
  pinned or already asleep), the summary of each sweep pass, sweeps of closed workspaces, every tab that closes (and why: `user`,
  `window`, `sweep:<rule>`), rendering fall-backs, and [Chrome extensions](extensions.md): each extension loaded at start-up (name, version,
  id, folder), each native messaging host copied, every install, update check (what is installed and what the store has), update and removal
  (also when run as `browsah extension ...`, under the `--profile` it was given), with the download address and the package check.
  A warning says when an extension asks for native messaging but no host manifest names it (1Password without its desktop app installed).
- `debug` adds why a sweep **kept** each tab (the selected tab, unsaved form input, playing media, still loading, no answer from
  the page, the last tab of the window, or what each rule said: `Stale: idle 5m0s < 1h0m, score 0.31 (limit 0.5)`; the score is that rule's, by its `half_life`). A line for a tab a rule acts on (info level) also ends its idle time and score.
- `warn` and `error` are always also printed on the terminal (a failed `browsah extension add` prints its reason once, and logs it).

To find out why a sweep did something, set `logging.level: debug`, restart, and look at the sweep lines:

```bash
browsah log --grep=sweep --lines=100
```

`BROWSAH_LOG=debug` raises the level for one run without changing the configuration; `BROWSAH_DEBUG=1` also prints every line on the terminal.

## Options

See [`logging`](options.md#logging). They are fixed per process: they apply at the next start and are ignored in a workspace's or
app's file.
