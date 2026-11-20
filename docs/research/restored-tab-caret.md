# No caret in a text field after a tab is restored (2026-11-20)

**Answer (suspected, not measured):** the page of a woken (restored or discarded) tab is not
reliably given CEF focus, so `document.hasFocus()` is false and Chromium does not blink the caret,
although key events still reach the field (typing works). The fix re-sends focus at the three
moments where the single focus call can be lost. It was **not reproduced** here.

## Environment

Code reading only, in a sandbox with no display, no compositor session and no built CEF
(`target/` absent, CEF download not attempted). Nothing below was measured on a machine.

## What the code did

- The only focus calls were `set_focus(1)` on the GTK focus controller's *enter*, and one
  `sync_focus` on `ViewEvent::Ready` (browser created).
- A woken tab (`BrowserView::wake`, from `Shell::on_selected`) is focused (`grab_focus`) *before*
  its browser exists and is mapped (`set_hidden(false)`, `was_hidden`) around the same time.
  Chromium keeps focus only on a visible widget with a live renderer; the `Ready` call can come
  before the view is shown and before the first navigation's renderer exists.
- A click calls `grab_focus`, which sends no focus-enter when the widget already has focus, so the
  click never repaired it. The `input` section never saw this because a normal tab is focused
  after its browser exists.

## Change

- `BrowserView::refocus` (sends `set_focus(1)` only when the widget has GTK focus, never takes it
  away), called on a press in the page and when a load finishes.
- `set_hidden(false)` sends `set_focus(1)` after `was_hidden(false)` on CEF's UI thread, in order.
- Self-test `restore`: after waking a sleeping tab, load `input.html`, click the text area and
  require `document.hasFocus()` (`hf`) and the focused element. A picture `restored-focus` is
  saved with `BROWSAH_SHOTS` for looking at the caret.

## Unverified

- That this is the actual cause on the reporter's machine (GNOME/Wayland): the self-test drives
  GTK signals, not real devices, and `hasFocus()` is a proxy for the caret blinking.
- The new check and the code were not compiled or run here.

Reproduce: `scripts/test selftest restore`, and open the picture from `BROWSAH_SHOTS=/some/dir`.
