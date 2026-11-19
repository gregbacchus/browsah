# DevTools window appearance on Wayland

**Date:** 2026-11-20

CEF creates DevTools as a Chrome-style native window. Browsah supplies the profile's Wayland app
ID as Chromium's `--class` default, so the compositor can find the profile's launcher and icon.
An explicit Chromium class takes precedence.

When the inspector opens, Chromium's request context gets a palette seeded from the profile tint
and the current desktop light or dark appearance. Profiles without a tint get a neutral palette.
Chromium generates the surrounding colours from the seed. The palette belongs to the request
context and can be shared by several workspace windows.

On GNOME with CEF 156.5.0, the temporary profile's DevTools window advertised
`be.geee.browsah.selftest` in the Wayland trace and appeared with a purple-tinted title bar in a
headless GNOME Wayland capture. The small in-frame icon was still Chromium's. It is drawn by
Chromium's [native frame](https://chromium.googlesource.com/chromium/src/+/main/chrome/browser/ui/views/tab_icon_view.cc).
CEF's name-based resource lookup returned no match for that frame's `IDR_PRODUCT_LOGO_16`
resource in the bundled build. CEF's [DevTools creation path](https://github.com/chromiumembedded/cef/blob/master/libcef/browser/chrome/chrome_browser_delegate.cc)
also excludes windowless DevTools from this path. With a temporary desktop entry installed in
the headless session, GNOME matched the window to the Browsah launcher and resolved its icon
name. The dock itself was not captured.

```bash
test_dir="$(mktemp -d)"
XDG_CONFIG_HOME="$test_dir/config" XDG_DATA_HOME="$test_dir/data" \
XDG_CACHE_HOME="$test_dir/cache" scripts/test selftest devtools
```
