# What does the browser do when it gets SIGTERM?

**Date:** 2026-11-19

**Short answer:** before this change it ended within a second with status 0 and none of the closing work: no final save of the tabs,
no screenshots of the tabs for next time, no ending of session cookies. (The status and speed suggest Chromium's own signal
handling ended the process; not investigated further.) It now handles `SIGTERM`, `SIGINT` and `SIGHUP` itself, so a session ending,
`kill`, systemd stopping the service, Ctrl+C in a terminal or the terminal closing quits the profile as the menu's *Quit profile* does,
without the confirmation question.

## What happens

A GLib Unix signal source (`on_signal` in `src/app/mod.rs`, over `g_unix_signal_add_full`; the glib crate has no wrapper) runs on
the GTK main loop. For each window it calls `Shell::finish_window_close` (the window's tabs are saved, a sleeping tab's screenshot
written, session cookies ended, browsers closed), then `Application::quit`, after which `main` shuts CEF down. The list of open
workspaces in `state.yaml` is left alone, so the next start reopens the same windows. A second signal while this is under way ends
the process at once (`128 + signal`). The journal says `SIGTERM: quitting the profile` (`browsah log --grep=SIG`).

## Measured (development machine: GNOME Wayland, CEF 156.5.0)

`scripts/test signals` starts a throwaway profile with a workspace window, sends each signal after 7 seconds and checks: exit status 0
within 5 seconds, the workspace's tabs are in the database, the workspace is still in the open list, and the journal line exists.
All three pass; the process ends in about 0.1 to 0.2 seconds. Not measured: `SIGKILL` (cannot be handled), a signal during start-up
before the main loop runs (the default action applies), a signal while a download or print is running, a signal with a page's
"leave this page?" dialog open (the quit skips the question, as it does for the menu's quit once confirmed).
