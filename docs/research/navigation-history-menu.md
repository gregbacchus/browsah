# Jumping through tab history from Back and Forward

**Date:** 2026-11-20

A long press claims the GTK click sequence and opens a list of entries in the chosen direction,
nearest first. **More history…** opens the profile History window. A normal button click still
goes one step.

Chromium entries are visited by their DevTools navigation IDs, preserving the browser's stack
even when different entries share a URL. A target beyond the native list uses the mechanism
for [saved tab history](tab-history.md). The shell folds the native entries into the saved
stacks, moves the intervening pages to the opposite stack, resets Chromium's list, and replaces
the current page. The saved target is loaded by URL, so restored entries have the same limitations
as ordinary Back and Forward through the saved stacks.

The focused GNOME Wayland self-test exercises the menu with GTK signals. It covers nearest-first
ordering, a native jump, saved jumps, a mixed list, the History link, and a regular click.
It does not generate a physical press, hold, and release.

```bash
test_dir="$(mktemp -d)"
XDG_CONFIG_HOME="$test_dir/config" XDG_DATA_HOME="$test_dir/data" \
XDG_CACHE_HOME="$test_dir/cache" BROWSAH_SHOTS="$test_dir/shots" scripts/test selftest navhistory
```
