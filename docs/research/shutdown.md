# Shutdown: the fixed half-second wait, and the sqlite "Failed to post task" warning

**Date:** 2026-11-18. **Environment:** Ubuntu with GNOME on native Wayland, AMD GPU, CEF 156.0.28, release build (the self-test
sections `basic`, `tabs`, `storage`, `cookies`, `picker`). The slow close and the log lines were reported from another machine (a
Homebrew install, 0.1.4 for the log lines); the measurements below are from this one, where both reproduce.

## Reports

1. Closing the window took about half a second.
2. `cef.log` ends with two warnings: `sqlite_persistent_store_backend_base.cc:182 Failed to post task from
   FlushAndNotifyInBackground … to client_task_runner_`.

## 1. The slow close: a fixed wait (fixed)

`Runtime::shutdown` slept a fixed 500 ms before shutting CEF down. **Measured:** by then the browsers had already reported closed
(counted by the life-span handler: 0 left after a few microseconds in `basic`, `tabs` and `storage`), so the whole wait was spare.
Shutdown now waits until every counted browser has closed (`cef_rt::browser_created/closed`, at most 2 s). The benchmarks create
browsers without counting them and keep the 500 ms wait. CEF's own `shutdown()` then takes 35 to 60 ms (`BROWSAH_DEBUG=1` prints it).

## 2. The warning: Chromium's own teardown, harmless (not changed)

It appears in the `selftest` profile's `cef.log` after even the trivial `basic` section (two lines every run), so it needs no busy page.
What was tried, all on this machine:

| Attempt | Result |
|---|---|
| An explicit `CookieManager::flush_store` of every store (global and isolated), waited for (about 10 ms), before shutdown | warning unchanged |
| A delay of 300 ms and of 1000 ms between the browsers closing and `shutdown()` | warning unchanged (2 lines at each) |
| Run the `cookies` section with the explicit flush and without it, from an empty cookie database | the cookie written last (`localhost:ck`) is in the database in **both** cases |

So the explicit flush did nothing measurable and was **removed** (an earlier note here said the warning might mean lost writes; that
was a guess and was wrong). **Measured on the cookie database after the runs:** `PRAGMA integrity_check` is `ok`, the journal file is
empty, and the file's time is that of the last run, so writes land. **Suspected, not confirmed in Chromium's source:** the stores are
destroyed during the network service's teardown, their final commit runs, and the completion notice cannot be posted back to a thread
that has already stopped; the log line is that notice failing. Nothing in browsah can reorder that, so it is left alone.

## Reproduce

```bash
export LD_LIBRARY_PATH=$PWD/target/release
BROWSAH_SELFTEST_ONLY=basic ./target/release/browsah --selftest >/dev/null 2>&1
grep -c 'Failed to post task' ~/.local/share/browsah/profiles/selftest/cef/cef.log      # 2
BROWSAH_DEBUG=1 BROWSAH_SELFTEST_ONLY=storage ./target/release/browsah --selftest 2>&1 | grep -E 'browsers closed|CEF shut'
sqlite3 -readonly ~/.local/share/browsah/profiles/selftest/cef/Default/Cookies 'pragma integrity_check'
```
