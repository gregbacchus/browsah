# Split panes: two pages side by side in one tab

Date: 2026-11-21. Machine: the development laptop (GNOME on Wayland, GTK 4.22.4, libadwaita 1.9.1, CEF 156). Measured with the `split`
self-test section (synthetic input, the app's own calls: no real pointer); the picture was looked at once, in a plain window.

## Answer

A tab can show a second `BrowserView` to the right of its own (`src/app/split.rs`). It is a `gtk::Paned` in the tab's holder with two
rounded, clipped cards (`.browsah-pane`, 10 px or the window frame's radius) and the window colour showing in the 6 px divider, as in
the Paua terminal. At most two panes, always side by side. The tab bar and the address bar are the tab's; the panes are only the page.

- **What is a pane.** The left pane is the tab's own view (`Tab::view`, `Tab::state`: the tab's identity, title, favicon, saved
  back/forward lists, session). The right pane is a `Pane { view, state }` with a state of its own. `Tab::focused_view/focused_state`
  say which one the address bar, back/forward, reload/stop, find, zoom, print, DevTools, the star and `Shell::go` act on. The
  focus follows a capture-phase click or keyboard focus entering a card (`Shell::focus_pane`), and the focused card has a thin outline.
- **Events.** The right view's events come through `Shell::split_event_closure` and `handle_split`, never `handle`: `handle` writes
  `tab.state`, hides `tab.view()` on a crash, closes the tab on `Closed`, and starts the first-frame watchdog on `Ready`, all wrong
  for a second view. A pane keeps what is its own (address, title, loading, `Closed`: the pane goes, never the tab); popups,
  permissions, downloads, dialogs and new tabs go on to `handle` as for any page.
- **Closing.** *Close right pane* retires the right view (`retire`: its events reach nobody, then the browser closes) and puts the left
  widget back in the holder. *Close left pane* retires the tab's view and **promotes** the right view: its event handler is replaced
  by the tab's own (`set_event_handler`), its state is moved into `tab.state`, the saved back/forward lists are cleared (they led to the
  page that went), and the favicon is looked up again. Closing the tab, `close_all` and `fall_back` handle the second browser too
  (`Runtime::shutdown` waits for every counted browser).
- **Unfocused pane look.** `split.inactive` (`dim`, `saturation`, `lightness`, `hue_shift`, `fade_ms`; Paua's `panes.inactive`) is a per-window style sheet
  (`Shell::refresh_split_css`) on the card's `.inactive` class: `opacity: 1 - dim` over the window colour plus a GTK `filter: saturate() brightness() hue-rotate()`,
  with a transition. Measured: the dim and desaturation show in the self-test picture (a light blue page turned grey). The fade over time was not observed.
- **Where to open one.** The header's split button (`nav.split_button`; a web app has none): it opens the focused page in a new right
  pane and puts the cursor in the address bar; in a split tab it opens *Close left pane* / *Close right pane*. Links, images and media
  have *Open in split pane* (`open-split`, `open-image-split`), bookmarks have it in their right-click menu, and a split tab's page menu
  has *Close this pane* (`close-pane`). With a split already open these replace the right page: there is never a third pane.
- **Icon.** Paua's `split-right-symbolic` (same author, MIT), a filled 16 px drawing scaled onto the 24 px grid (`split-right.svg`).

## Not done, on purpose

- Not remembered across restarts (the tab comes back as its left page), and the right page is not written to the history.
- A split tab is never swept or put to sleep (`discard_tab` refuses, the sweep treats it as protected). A rendering fall-back ends the split.
- The back/forward long-press lists and the tab menu's *Copy address* still use the left page.
- No keyboard shortcut for splitting or moving between panes.
- The colour-frame variant of the divider and corner CSS (`workspace_css`) is written but was not looked at in a coloured workspace.
- Real pointer clicks in a pane (the focus gesture) were not exercised: the self-test calls `focus_pane` directly.

## Reproduce

```bash
scripts/test selftest split
BROWSAH_SHOTS=/tmp/shots BROWSAH_SELFTEST_ONLY=split ./target/release/browsah --selftest   # split.png, split-menu.png
scripts/test unit ctxmenu
```

(If the binary answers "this browsah installation differs from the codec runtime", an installed codec runtime is hashing the old binary:
set `BROWSAH_CODEC_ACTIVE=1` for the run instead of reinstalling.)
