# Shutdown: the fixed half-second wait and the cookie database flush

**Date:** 2026-11-18. **Environment:** Ubuntu with GNOME on native Wayland, AMD GPU, CEF 156.0.28, run from the release build (the
self-test sections `basic`, `tabs`, `storage`). The slow close and the log lines were reported from another machine (a Homebrew
install); the timings below are from this one.

## Reports

1. Closing the window took about half a second.
2. A real session's `cef.log` ended with two warnings: `sqlite_persistent_store_backend_base.cc:182 Failed to post task from
   FlushAndNotifyInBackground … to client_task_runner_`.

## Causes

1. `Runtime::shutdown` slept a fixed 500 ms ("CEF's own thread finishes the closes") before shutting CEF down, whether or not
   anything was still closing. **Measured:** by then the browsers had already reported closed (counted by the life-span handler:
   0 left after a few microseconds in `basic`, `tabs` and `storage`), so the whole wait was spare.
2. **Suspected, not reproduced:** Chromium writes its SQLite stores (cookies, and others) in the background and the process was
   torn down while a write was queued, so the store's task runner was gone when the flush was posted. The warning did not appear
   in three `cookies` self-test runs or in `basic`/`tabs`/`storage`; it needs a busy real page at close, which the tests do not
   have. So it is **not shown that the warning is gone**, only that the stores are now flushed and waited for.

## Changes

- Shutdown waits until every counted browser has closed (`cef_rt::browser_created/closed`, at most 2 s) instead of a fixed sleep.
  The benchmarks create browsers without counting them and keep the 500 ms wait.
- Before CEF shuts down the cookie stores (the global one and each isolated workspace's) are flushed and waited for
  (`view::flush_cookie_stores`, at most 3 s, from `app::flush_stores_on_exit`).

## Measured (this machine)

| step | time |
|---|---|
| flush of the global and an isolated store (`storage` section) | about 10 ms |
| browsers closed (counted) when shutdown starts | a few microseconds |
| CEF's own `shutdown()` | 35 to 60 ms |
| the old fixed wait | 500 ms |

`BROWSAH_DEBUG=1` prints the last three. Other stores (localStorage, IndexedDB, caches) are written by Chromium's own close.

## Reproduce

```bash
export LD_LIBRARY_PATH=$PWD/target/release
BROWSAH_DEBUG=1 BROWSAH_SELFTEST_ONLY=storage ./target/release/browsah --selftest 2>&1 | grep -E 'browsers closed|CEF shut|flush'
```
