# Why did a site's "copy" button do nothing?

**Date:** 2026-11-19

**Short answer:** Windowless Chromium cannot reach the Wayland clipboard, so every way a page can copy failed or stayed inside the
browser. Ctrl+C and the context menu were already bridged through GTK (`cef-input.md`), but a page's own copy button was not.
browsah now runs a small script in every frame that makes the page's copy calls succeed and reports the text to the shell, which
puts it on the system clipboard, but only if the person used the page in the last 5 seconds.

## Measured (development machine: GNOME Wayland, CEF 156.5.0)

The `copybutton` self-test section loads `bench/pages/copy.html` (four buttons) and clicks each with emitted pointer events:

| How the page copies | Before | After |
|---|---|---|
| `navigator.clipboard.writeText` | rejected with `NotAllowedError`; system clipboard unchanged | resolves, text on the system clipboard |
| `document.execCommand('copy')` on a selected textarea | returned `false` | returns `true`, text on the clipboard |
| a `copy` event handler calling `clipboardData.setData` | `false`, nothing | text on the clipboard |
| `navigator.clipboard.write` with a `text/plain` item | `NotAllowedError` | text on the clipboard |
| a page that calls `writeText` with no recent use | n/a | clipboard untouched (check passes) |

Sites therefore showed "copy failed" or silently did nothing.

## How it works (and what did not)

- **Not usable:** the DevTools route (`Page.addScriptToEvaluateOnNewDocument` plus a `Runtime.addBinding` function) installed fine
  (calls succeeded) but the script never ran in the page, even in a page loaded after it, so the page could not call the binding.
  Not understood; not investigated further.
- **Used:** `LoadHandler::on_load_start` runs `COPY_HOOK` with `Frame::execute_java_script` in every frame, including cross-origin
  iframes, each time it starts loading. The hook replaces `Clipboard.prototype.writeText` and `write`, wraps `Document.execCommand`
  for `copy` and `cut` (it reads the selection, including a textarea's, or the text a `copy` handler set through
  `DataTransfer.setData`) and reports the text with `console.debug` and a private prefix. `DisplayHandler::on_console_message`
  takes those messages (and does not log them) and sends `ViewEvent::ClipboardWrite`. The shell sets the clipboard only if
  `BrowserView::used_within(5 s)`: any key, click or scroll sent to the page, like a browser's user-activation window. Modals do the
  same.
- Paste buttons (`navigator.clipboard.readText`) are not handled; Ctrl+V and the context menu's Paste still work. Copying images
  (`write` with `image/png`) is not handled: only `text/plain`.
- A page can fake the console message, which is the same power it has with `writeText` after a click; the 5 second rule is what
  stops it rewriting the clipboard unprompted. Not measured: a real site (the self-test uses a local page), a page that copies after
  an `await` of more than 5 seconds (it would be ignored), pages that replace the hook's functions again.

## A related fix found on the way

DevTools message ids must increase in the order calls run, and browsah took ids on the GTK thread and ran the calls on CEF's thread.
When two callers raced, CEF renumbered one call and the caller read another call's result (seen while installing the first version
of the hook: a probe's result was the previous call's). `Shared::post_cdp` now takes the id and posts the call under one lock.
Sweep probes, the colour scheme and `cdp_eval` use it. The race was observed once, not shown to affect a sweep.

Reproduce:

```bash
run_dir=$(mktemp -d)
XDG_DATA_HOME="$run_dir/data" XDG_CONFIG_HOME="$run_dir/config" \
  XDG_CACHE_HOME="$run_dir/cache" scripts/test selftest copybutton
```
