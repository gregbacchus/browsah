# Tabs in the real app

**Date:** 2026-11-17 · **Question:** can the tab model (one windowless CEF browser per tab) be built
in the GTK shell so hidden tabs stop rendering and switching is quick, and what broke on the way?

## Answer

Yes. One `BrowserView` per tab sits in an `adw::TabView` with an `adw::TabBar`. A view's widget being
mapped or unmapped drives `was_hidden`. In `--selftest` (67 checks, accelerated and readback paths,
repeated runs) a hidden tab received **0 frames over 1 s**, switching to it produced a fresh frame
and hid the previous tab within the 3 s check window, and tab titles, window title, address bar and
back/forward state followed the selected tab.

## Environment

AMD Radeon 890M, Mesa 26, GNOME/Mutter on native Wayland, display scale 2, GTK 4.22, libadwaita,
CEF 156.5.0. Release build, local test pages (`bench/pages/popup.html`, `a.html`, `b.html`).

## What was measured and what was not

- **Measured (scripted, this machine):** hidden tab frame count over 1 s (0); a window opened by a
  click becomes a tab; one opened by a timer without a gesture is blocked; a new tab navigates only
  itself; closing tabs leaves the first working.
- **Not measured:** wake time in the app (the check only needs a frame within 3 s; the 8-14 ms
  figure is from [cef-multi-tab-cost.md](cef-multi-tab-cost.md)); memory or power with many real
  tabs in the shell; favicons (not built); a frame-rate cap for visible but unfocused tabs (there is
  only ever one visible tab, so the policy from [cef-variable-frame-rate.md](cef-variable-frame-rate.md)
  is not needed yet); real keyboard, mouse and touch on the tab bar.

## Design

- Events carry no tab id: each tab has its own event closure holding a weak reference to its `Tab`.
- Shared chrome (address bar, back/forward/reload buttons, window title) is refreshed from the
  selected tab's `State` on every event for that tab and on every selection change.
- `window.open` and `target=_blank`: `on_before_popup` cancels the CEF popup (a windowless browser
  has no window for it) and opens the URL as a foreground tab, only if `user_gesture` is set. This
  loses `window.opener`, so pages that talk to their popup (some OAuth flows) will not work yet.
- Closing the last tab closes the window.

## Problems found

1. **A navigation sent to a just-opened tab was dropped** because its CEF browser did not exist yet.
   Fixed by queueing the URL (`pending_url`) and loading it in `on_after_created`.
2. **A tab focused before its browser exists never told CEF it had focus.** Fixed by sending the
   focus state when the browser becomes ready (`BrowserView::sync_focus`).
3. **The self-test became flaky** (cursor and hover checks failed in roughly 2 of 3 runs). Cause,
   shown by logging whether each motion event had a real event behind it: the real mouse pointer
   was resting over the window, and the tab bar's layout changes (title and spinner updates) made
   GTK re-deliver it, racing the emitted signals. Not a product bug, but a real mouse does the same
   to the page: the page sees the pointer where the mouse is, which is correct. The self-test now
   ignores real pointer events (`set_synthetic_pointer_only`); with that, 6 of 6 runs passed
   (2 readback, 4 dmabuf).
4. Starting views as hidden until mapped made no difference to (3); views now start visible and
   are hidden on unmap.

## Reproduce

```bash
cargo build --release
export LD_LIBRARY_PATH=$PWD/target/release
./target/release/browsah --selftest              # dmabuf
./target/release/browsah --selftest --mode=paint # readback
```
