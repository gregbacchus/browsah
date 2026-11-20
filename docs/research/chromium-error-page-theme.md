# Light and dark Chromium connection errors

**Date:** 2026-11-20

Chromium's built-in connection error document has styles for both appearances. Its
`prefers-color-scheme` value comes from the same per-browser emulation as an ordinary web page.

On the GNOME Wayland development machine with CEF 156.5.0, a connection to a closed loopback port
produced Chromium's own error page. In a normal browser window it painted RGB (32, 33, 36) in
dark mode and RGB (255, 255, 255) in light mode. A new tab whose first navigation failed also
reported dark when the desktop appearance was forced dark. The reported light-page mismatch
was not reproduced. No runtime theme change was made.

```bash
test_dir="$(mktemp -d)"
XDG_CONFIG_HOME="$test_dir/config" XDG_DATA_HOME="$test_dir/data" \
XDG_CACHE_HOME="$test_dir/cache" BROWSAH_SHOTS="$test_dir/shots" scripts/test selftest theme
```
