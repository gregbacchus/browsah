# Can a workspace pin show its title without losing native pin behaviour?

**Date:** 2026-11-18

Libadwaita lays out its native pinned tabs in a separate compact strip.
A fixed-width titled pin measured 118 px when an ordinary tab measured 626 px
in an 800 px test window. For `size: full`, use an ordinary tab in libadwaita's
regular strip so it shares the same layout and resizing as the other tabs.
Browsah keeps the pin's scope, ordering and close protection. The tab bar does
not expose a public per-page close button API, so hide that button on the
matching internal tab widget. Compact pins use libadwaita's native pin strip
and appear before full pins.

On the development machine (GNOME Wayland, libadwaita 1.9.1, CEF 156.5.0),
the `locked` self-test compares both pin styles with an ordinary tab before
and after a resize, checks the close button is hidden
and exercises close rejection and scope navigation. Other libadwaita versions
and real drag gestures were not checked.

To reproduce with a temporary profile and save the tab strip:

```bash
run_dir=$(mktemp -d)
XDG_DATA_HOME="$run_dir/data" XDG_CONFIG_HOME="$run_dir/config" \
  XDG_CACHE_HOME="$run_dir/cache" BROWSAH_SHOTS="$run_dir/shots" \
  scripts/test selftest locked
```
