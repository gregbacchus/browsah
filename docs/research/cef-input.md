# Input to a windowless CEF page, and its latency (GTK4, native Wayland)

**Date:** 2026-11-16 · **Code:** [`src/app/view.rs`](../../src/app/view.rs) (handlers and controllers),
[`src/app/input.rs`](../../src/app/input.rs) (mapping, unit tested),
[`src/app/latency.rs`](../../src/app/latency.rs) (`--latency`), [`src/app/selftest.rs`](../../src/app/selftest.rs)
(`--selftest`, input section), pages in [`bench/pages/`](../../bench/pages/). Same machine and versions as
[cef-osr-wayland-spike.md](cef-osr-wayland-spike.md).

## Question

With CEF rendering windowless into a GTK4 widget, can we give pages all the input a browser needs
(mouse, wheel, keyboard, IME text, focus, cursors, tooltips, `<select>` popups, clipboard, context
menu), and is the input latency acceptable? This was the largest unknown for the design.

## Answer

- **Yes, all of those work**, verified by 46 scripted checks that drive the real GTK input
  controllers and read the result back from a test page. They pass on both the dmabuf and the
  readback paths.
- **Latency is low and tight: about 7-8 ms** from an input event to the next frame arriving at
  GTK on the dmabuf path (p95 under 9-11 ms), and about 11 ms on the readback path. It did
  **not** depend on the frame-rate cap (30, 60 and 120 fps gave the same) for isolated inputs after
  idle.
- **Clipboard needs a bridge.** Chromium's own copy and paste work inside the page but never
  reach the system clipboard on Wayland, so copy, cut and paste are routed through GTK's clipboard.
- **Not verified:** real device input (as opposed to emitted GTK signals), a real IME (ibus,
  fcitx), touchpad scrolling feel, drag selection and drag-and-drop. See Caveats.

## Design

CEF runs on its own thread (see [multi-tab note](cef-multi-tab-cost.md#resolved-the-100--main-thread-spin-its-cause-and-fix)),
so input goes from the GTK thread to CEF through `BrowserHost` methods that are safe to call from
any thread. GTK controllers sit on the page widget (a `GtkOverlay`, so popups can draw above the
page):

| Input | GTK source | CEF call | Notes |
|---|---|---|---|
| Pointer | `EventControllerMotion` | `send_mouse_move_event` | Coordinates are logical x render scale (`View::k`), because the view is device-sized |
| Buttons | `GestureClick` (all buttons) | `send_mouse_click_event` | Press focuses the page; click count passed through |
| Wheel / touchpad | `EventControllerScroll` | `send_mouse_wheel_event` | One notch = 53 px (as Chromium), sign flipped for CEF, multiplied by the render scale; touchpad deltas are already pixels |
| Keys | `EventControllerKey` | `send_key_event` | RAWKEYDOWN then CHAR for printable keys and Enter, KEYUP on release; Windows key codes (US-layout punctuation) |
| Text from an input method | `IMMulticontext` `commit` / `preedit-changed` | `ime_commit_text`, `ime_set_composition` | See below |
| Focus | `EventControllerFocus` | `set_focus` | Plus `focus_in`/`focus_out` on the input method |
| Cursor | `on_cursor_change` | `set_cursor_from_name` | Mapped from CEF's cursor type |
| Tooltip | `on_tooltip` | GTK tooltip on the widget | |
| `<select>` etc. | `on_popup_show`, `on_popup_size`, popup paint element | second `GtkPicture` in a `GtkFixed` layer | See bugs below |
| Selection | `on_text_selection_changed` | stored | Feeds the clipboard bridge |
| Context menu | `ContextMenuHandler::run_context_menu` | `GtkPopover` | Only commands that work here (back, forward, reload, edit commands) |

**Input-method handling.** Every key goes to the input method first. Ordinary characters come back
as a commit of exactly the key's own character; those are sent to the page as normal key events,
so pages see `keydown`/`keypress` (single-key shortcuts and games work). Anything else the input
method produces (compose sequences, accents, IME text) is inserted with `ime_commit_text`.
Preedit text is passed to `ime_set_composition`. **Not done:** placing the IME candidate window
at the caret (needs `on_ime_composition_range_changed`).

**Shell shortcuts** (`Ctrl+L`, `Ctrl+R`, `F5`, `F6`, `Alt+Left/Right`) are not sent to the page.

## Latency

`--latency` loads a static page (nothing animates, so Chromium produces no frames until something
changes), waits for quiet, sends 60 inputs of each kind through the real GTK controllers, and times
from the call to the frame hook firing on the GTK thread. 2880x1568 view, display 120 Hz, release
build.

| Path / cap | Key mean (p95) | Click mean (p95) | Wheel mean (p95) |
|---|---|---|---|
| dmabuf, 60 fps cap | 7.3 (7.8) ms | 7.9 (8.9) ms | 6.8 (11.3) ms |
| dmabuf, 120 fps cap | 8.2 (9.4) ms | 8.3 (9.4) ms | 6.2 (8.6) ms |
| dmabuf, 30 fps cap | 8.1 (9.3) ms | 7.8 (8.8) ms | 5.5 (6.7) ms |
| readback, 60 fps cap | 12.0 (14.3) ms | 11.1 (12.4) ms | 12.1 (20.2) ms |
| readback, 120 fps cap | 11.3 (13.6) ms | 11.1 (12.8) ms | 11.3 (15.0) ms |

- The readback path costs about 3-4 ms more and has a longer tail (max 26 ms in one run).
- The frame-rate cap does not change the first frame after idle: Chromium starts a frame as soon
  as there is something to draw. The cap does govern the spacing of *consecutive* frames (held
  keys, continuous scrolling), which this test does not measure. The
  [frame-rate note](cef-variable-frame-rate.md) measured ramp-up from a low cap while animating.
- **What this excludes:** GTK drawing the frame (up to one GTK frame, about 8 ms at 120 Hz), the
  compositor, and display scan-out; and the input device and compositor queueing before the event
  reaches GTK. Expect roughly one to two display frames more for input-to-photon. No native-window
  comparison was made, so "acceptable" is a judgement from the absolute numbers, not a measured
  difference from a native app.

## Findings and bugs worth remembering

1. **Wayland clipboard: copy never reaches the system.** Selecting text and pressing Ctrl+C left
   the system clipboard unchanged, while Ctrl+V inside the page pasted the copied text (from
   Chromium's private clipboard). A windowless browser has no surface of its own to take
   clipboard focus. Fix: Ctrl+C/X also write `on_text_selection_changed`'s text to GTK's
   clipboard; Ctrl+V reads GTK's clipboard and inserts with `ime_commit_text` instead of sending
   the key (otherwise the page would also paste its private copy). Verified both ways, including
   text that came from outside the browser. Non-text clipboard content (images, files) is not
   handled.
2. **Popup sized wrongly.** The `<select>` popup first drew at twice its size: a `GtkPicture`
   takes its natural size, which is the texture's pixel size, so a device-pixel texture was shown
   in logical units. Fix: place it in a `GtkFixed` with an exact size request. CEF's reported
   popup rectangle (view pixels) was correct.
3. **Context menu ordering.** Closing the `GtkPopover` fires its `closed` signal, which cancels
   CEF's callback; the chosen command must be answered *before* closing, or it silently does
   nothing.
4. **`GtkEntry` focus** lives in an inner text widget, so `entry.has_focus()` is false even when it
   is focused; look for focus inside the entry.
5. **`GestureClick.current_button()` is 0 for a signal emitted by hand**, so 0 is treated as the
   left button (real events always carry 1+).

## Caveats

- **Emitted signals, not real devices.** The self-test emits the GTK controllers' signals. That
  tests my translation to CEF and CEF's handling, but not GDK's own event generation, the input
  method on a real key press (`filter_keypress` with a real event is skipped for emitted
  signals), kinetic touchpad scrolling, or held-button state during drags (a real motion event
  carries the button mask; an emitted one does not, so **drag selection is untested**). No input
  injection tool was available (`/dev/uinput` is root-only, no `ydotool`/`wtype`).
- **IME:** commit is tested with an emitted `commit`; composition (preedit) and candidate-window
  placement are not verified against a real IME.
- **Keyboard layouts:** key codes assume a US-style layout; text comes from the character, so
  typing is right on other layouts, but `KeyboardEvent.code` may differ.
- **One machine, one compositor, one page type.** The latency page is trivial; heavy pages could
  add main-thread time in the renderer before the frame.
- **Latency is the first frame after idle,** with the machine otherwise quiet.
- Tooltips, cursors and popups were verified for a few cases, not exhaustively. Links open in the
  same tab only (popups and `window.open` come with tabs).

## Reproduce

```bash
export LD_LIBRARY_PATH=$PWD/target/release
./target/release/browsah --selftest                     # 46 checks: navigation and input
./target/release/browsah --selftest --mode=paint        # the same on the readback path
BROWSAH_SCREENSHOT=/tmp/shell.png ./target/release/browsah --selftest   # also saves window images
./target/release/browsah --latency [--mode=paint] [--config=PATH]       # input latency
```

## Open questions

- Real-device verification: drag selection, drag and drop, kinetic scrolling, a real IME.
- IME candidate-window placement via `on_ime_composition_range_changed`.
- Latency of consecutive inputs (held key, continuous scroll) and with heavy pages; and a
  native-window baseline for comparison.
- Middle-click primary-selection paste, and accessibility. (A pasted picture is handled: [2026-11-23-paste-image.md](2026-11-23-paste-image.md).)
