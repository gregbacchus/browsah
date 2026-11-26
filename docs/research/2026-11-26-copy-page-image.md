# Copying images and structured data from a page

Miro’s [Copy as image](https://help.miro.com/hc/en-us/articles/4413451973266-Copy-as-text-or-as-an-image)
produces a PNG. Browsah’s `navigator.clipboard.write` bridge resolved the
promise for an image-only item while leaving the system clipboard untouched.
The `navigator.clipboard.write()` bridge publishes images with their original
MIME types and bytes, plain text as supplied, and HTML processed with Chromium's
HTML Sanitizer API. It publishes other types using Chromium's Linux web
custom-format map and numbered binary slots. Pages can use either
`web application/json` or `application/json` to write a custom representation.
That representation is advertised to Chromium readers as `web application/json`.
The clipboard item can include up to 16 representations and 16 MiB total.
The five-second recent-input check applies when GTK receives the write.

On Ubuntu 26.04.1, headless GNOME 50.1 on Wayland with GTK 4.22.4 and CEF
156.0.28, a local page wrote a 1024 × 1024 canvas PNG using a promise-backed
`ClipboardItem`. The page reported success before the fix, but GTK could
only read the previous clipboard text. Afterward GTK read the full-size
image. The self-test reads PNG, JPEG, WebP and SVG from the clipboard using
their advertised MIME types. It also checks exact plain text, a sanitized HTML
table, and exact JSON and arbitrary binary bytes in their custom slots.
The test uses a local page and emitted GTK input signals. The live Miro board and
cross-application paste were outside the test. Reading custom formats through
Browsah's page paste or `navigator.clipboard.read()` is outside this bridge.

Run `scripts/test selftest copybutton,pasteimage` to reproduce.
