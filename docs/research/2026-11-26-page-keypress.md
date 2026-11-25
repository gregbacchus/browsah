# Plain-letter shortcuts and CEF keypress

Browsah now keeps the virtual key code when it sends a character event to CEF. The
typed character stays in CEF's `character` field. This fixes the browser-side
event mismatch found while investigating Gmail's `e` (Archive) shortcut.

## Measurements

Ubuntu 26.04.1, AMD Strix with Mesa 26.0.8, headless GNOME Shell 50.1 on Wayland,
GTK 4.22.4, libadwaita 1.9.1, CEF 156.0.28, readback rendering. Both builds used
throwaway profiles and local pages. A virtual keyboard attached to the compositor
delivered native Wayland events.

| Event for plain `e` with no text field focused | Before, `event.key` | After, `event.key` |
|---|---|---|
| `keydown` | `e` | `e` |
| `keypress` | `5` | `e` |
| `keyup` | `e` | `e` |

`event.code` was `KeyE` throughout and `keypress.which` stayed 101. The old
browser bridge replaced the virtual key (`0x45`) with the Unicode value
(`0x65`) on the CEF `CHAR` event. CEF on Linux interpreted `0x65` as
`VK_NUMPAD5` when determining the DOM key. The text in a focused input field
was `e` in both builds. Shift+E produced `E` in both builds.

The focused headless input self-test passed 40 of 40 checks, including the
shortcut, typing, composed text commits and clipboard. Live Gmail and real
IME composition were outside these measurements.

## Reproduce

Run the input section on Wayland with its window focused. The local fixture at
`bench/pages/input.html` records the full event sequence for plain `e`.

```bash
scripts/test selftest input
# Force the readback path after building:
LD_LIBRARY_PATH="$PWD/target/release" BROWSAH_SELFTEST_ONLY=input \
  ./target/release/browsah --selftest --mode=paint
```
