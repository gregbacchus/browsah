# Pasting a screenshot into a page (Claude web, chat boxes)

**Answer:** Ctrl+V gave the page an empty paste whenever the clipboard held only a picture (what a screenshot tool leaves there), so
claude.ai showed nothing. Only text was bridged from GTK's clipboard (`clipboard_shortcut`). Now, when the clipboard has a picture and
no text, `BrowserView::paste_image` reads it through GTK (`read_texture_async`), encodes a PNG and has the focused element receive a
`paste` event whose `clipboardData.files` holds it (`ClipboardEvent` built in the page through DevTools `Runtime.evaluate`).

## Evidence

- Measured on this machine (GNOME, Wayland, release build): before the change the `pasteimage` self-test section got
  `paste:files=0 type=- size=0 text=0` from `bench/pages/paste.html` after Ctrl+V with a 40x30 picture on the clipboard; after, one
  `image/png` file of a few hundred bytes. Text still pastes as text (same section).
- Why it failed: windowless Chromium has no surface to take Wayland clipboard focus, so its own paste reads nothing (see
  [cef-input.md](cef-input.md)).
- Not measured: Claude's or Teams' real chat boxes (no sign-in in tests), a screenshot from a real screenshot tool (the test puts a
  texture on the clipboard the same way a tool offers `image/png`), a picture copied from a web page that also offers text (text wins,
  as before), why Teams already accepted a pasted screenshot for the person who reported this.

## Limits

- The event is created by the page's own script, so `isTrusted` is false; a site that checks it ignores the paste. A site whose
  paste handler does not read `clipboardData.files` is unaffected.
- Only the picture is handed over, not the other formats. Over 20 MB is not sent. The context menu's *Paste* uses the same path, but
  CEF may show it greyed out when only a picture is on the clipboard (not checked).
- Dropping files from a file manager onto the page is a separate gap.

Reproduce: `scripts/test selftest pasteimage`.
