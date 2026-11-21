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
- **Icon.** `split-right.svg`: Lucide's `columns-2` (rounded square, divider) with a small filled block in the right pane, drawn with the same 1.5 stroke as every other icon. It was first Paua's `split-right-symbolic`, a filled 16 px drawing whose outline came out 1.9 px thick on the 24 px grid, visibly heavier than the Lucide strokes beside it; the block is kept, the lines are now 1.5 (changed 2026-11-21).

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

## 2026-11-21: the focus ring showed only in the gap

The focused pane's ring was a `box-shadow` outside its card. The panes fill the tab area, so the part of the shadow that fell outside the window (top, bottom and the outer side) was cut off and only the strip in the gap between the panes showed. The ring is now the card's own 2 px border (transparent on the unfocused pane, so nothing moves when focus changes), drawn inside the card on all four sides. Checked in the `split` section's picture (`split.png`: the right pane's ring runs all round); 43 of 43 checks. A ring drawn outside a widget that fills its parent will always be clipped: draw it inside.

Follow-up, same day: **the frame stays `b` thick round and between the panes.** With the ring (`r` = 2 px, `split::RING`) inside the cards, the outer padding of a split tab is `b - r` (class `split-active` on the frame, set in `refresh_split_button`) and the separator is `b - 2r`, never below 0, so ring + padding and ring + separator + ring are both `b` (`window.frame.thickness`, default 4: padding 2, separator 0). Measured in the `frame` self-test with the 4 px frame: the page is 4 px from the window's edges and 4 px from the other pane (66 of 66 checks, and `frame-split.png` looked at); closing the split gives the full 4 px back. Without a frame (no workspace colour) the separator is 0 and the gap is the two rings, 4 px. **Not checked:** dragging the divider by hand with a 0 px separator (GTK may or may not give the handle extra grab area); check on real input.
