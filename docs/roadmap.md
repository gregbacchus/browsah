# Roadmap: from spike to the simplest usable browser

**Date:** 2026-11-16 · Based on [docs/research/summary.md](research/summary.md). Effort figures are
rough guesses by the author, not measurements; treat them as relative sizes.

## Where we are

**M0, M1, M2 and M3 are done** (2026-11-16, tabs 2026-11-17). The code is split into modules, the benchmarks live in
`src/bench/` and still run, and there is a real window: libadwaita header bar with back, forward,
reload/stop and an address bar (URL, host name or search), page title in the window title, a
YAML config with docs and tests, and `--selftest`. M2 added input to the page: mouse, wheel,
keyboard, input-method text, focus, cursors, tooltips, `<select>` popups, a clipboard bridge, and
a context menu. `--selftest` now runs 46 scripted checks of the real window (navigation and
input), passing on both the dmabuf and readback paths, and `--latency` measures input latency
(about 7-8 ms to the next frame at GTK on the dmabuf path; see
[docs/research/cef-input.md](research/cef-input.md)). **Not verified:** real device input (the
tests emit GTK's controller signals), a real IME, drag selection and drag and drop, kinetic
scrolling. M3 added tabs (see [docs/research/browser-tabs.md](research/browser-tabs.md)); 67 scripted
checks now. Not done: tab favicons, and everything after M3.

Decisions made (2026-11-16): GTK4 + libadwaita; AMD GPUs only for now (other GPUs untested);
YAML config in the style of the author's terminal project; MIT licence; each launch is its own
browser instance. The default search engine is DuckDuckGo (configurable).

## Definition of "simplest browser"

Milestone A: **one window, one tab, an address bar, back/forward/reload, and input that works well
enough to use real sites** (click, scroll, type, select text, dropdowns, cursors). Tabs come next
(milestone B). Everything else is later.

## Recommended order

Input is the largest unknown and the thing most likely to change the design (latency, IME,
focus), so build it early on top of the thinnest navigation UI that lets you test it.

### M0: Restructure the spike (small): DONE

- Split into modules: CEF runtime and handlers, a `BrowserView` widget (frame hand-off, input
  forwarding, size/scale), and the window shell. Keep the thread-safe shared-state pattern.
- Move the benchmarks behind a subcommand or `src/bench/` so they keep running as a regression
  check (idle CPU, 60 fps, zero-copy) while the app grows.
- Make the AMD workaround a small per-GPU policy table instead of a hard-coded env var.

### M1: One tab with navigation UI (small): DONE

- libadwaita header bar with back, forward, reload/stop, and a URL entry that treats non-URLs as
  a search.
- CEF side: `load_url`, `go_back`, `go_forward`, `reload`, `stop_load`; `LoadHandler` for
  loading state and can-go-back/forward; `DisplayHandler` for address and title changes.
- Done when: you can open a site, go back, and the URL and title update. (Following links needs
  mouse input, which is M2; M1 was verified by typing addresses, `--selftest`, and a window
  screenshot.)

### M2: Input (large, the main risk): DONE, with caveats

- **Mouse:** move, enter/leave, click (with click count), wheel (pixel vs line scrolling), all
  with correct coordinates (logical position times the backing scale) and modifier state.
- **Keyboard:** key down/up/char events from `GtkEventControllerKey`, mapping GDK keys to the key
  codes CEF expects; text entry through `GtkIMContext` (commit and preedit to CEF's IME calls).
- **Focus and cursors:** `set_focus` on focus change; map `on_cursor_change` to GDK cursors.
- **Popups and tooltips:** `<select>` dropdowns arrive as separate popup paint elements
  (`on_popup_show`, `on_popup_size`); tooltips need our own widget.
- **Clipboard and context menu:** check copy/paste and selection work on Wayland; build a
  minimal context menu (OSR has none by default).
- **Measure input latency** (key or click to the frame that shows it) against a native window.
  If it is poor, this is where the design might need changing (for example `external_begin_frame`
  driven from GTK's frame clock).
- Done when: you can search on a site, log in, scroll smoothly, select and copy text, and use
  dropdowns, with measured latency recorded in `docs/research/`.
- **Status:** built and verified with emitted GTK signals (46 checks) and the latency measured
  (excluding GTK drawing and scan-out). **Still to verify by hand or with a real input device:**
  typing, scrolling and selecting with a real mouse and keyboard, a real IME, drag selection, drag
  and drop. Candidate-window placement for IMEs and non-text clipboard content are not done.

### M3: Tabs (medium) = milestone B: DONE, without favicons

- `AdwTabView` / tab bar, one CEF browser per tab, new/close, per-tab title and favicon.
- Background tabs hidden (`was_hidden`); popups and `window.open` become new tabs
  (`on_before_popup`).
- Basic frame-rate policy: focused 60, visible-unfocused capped, hidden not rendering (see
  [frame rate note](research/cef-variable-frame-rate.md)).
- Standard shortcuts (new, close, next/previous, focus address bar).
- **Status:** built and checked by `--selftest`. **Not done:** favicons (CEF's
  `on_favicon_urlchange` plus `download_image`), reopen closed tab, drag tabs between windows,
  `window.opener` for popups (they are opened as plain tabs). Unfocused-but-visible frame cap is not
  needed until split views exist.

### M4: Robustness (runs alongside M2-M3)

- Watchdog: if no accelerated frame arrives within 3 s, switch to `OnPaint`: **DONE**
  ([note](research/browser-tabs.md#dmabuf-watchdog)); not tried on non-AMD GPUs.
- Renderer crash: **DONE** (crashed-tab page with Reload; [note](research/browser-tabs.md#renderer-crash)). Still to do: GPU-process crash, memory-pressure kills, correct shutdown ordering.
- Per-site zoom (replace the periodic zoom re-apply), fractional scales, monitor scale changes.
- Test on other hardware: Intel, NVIDIA, and a second compositor.

### M5: Daily-use features (later)

Downloads, JavaScript dialogs and permission prompts (deny by default), find-in-page, history
and bookmarks (SQLite), persistent profile and cookies, fullscreen, DevTools window, tab
discarding (the research shows ~42 MB per tab is recoverable) and a settings file.

### M6: Packaging and security (before anyone else runs it)

- Enable the CEF sandbox (setuid helper or user namespaces).
- Ship the ~300 MB CEF runtime: `.deb`, Flatpak, or a tarball with the exact `libcef` the
  binary was built against.
- An optional `browsah codecs` command installs a pinned third-party H.264/AAC runtime for personal
  use, with version and API checks; broader compatibility, updates and redistribution remain open
  ([video codecs](video-codecs.md)).
- **Security updates are your job:** a browser is a high-value target, so plan to track CEF
  releases and ship patched builds quickly.

## What I would do next, concretely

1. **Use it for a day** (or ask someone to): the browser is now complete enough to
   browse, and real use will find the input problems the emitted-signal tests cannot (drag
   selection, IME, touchpad scrolling, sites that behave differently from the test pages).
2. **M4 items that matter early:** the watchdog fallback from dmabuf to readback, renderer-crash
   handling, per-site zoom.

## Next phase: profiles, workspaces, web apps

**Workspaces are done (2026-11-17),** apart from what is listed here as not built. Built: one window per
workspace with its own directory and `workspace.yaml`; `browsah workspace add|list|remove` and
`--workspace=`; windows remembered across runs; ephemeral windows (`Ctrl+Shift+N`); tab dragging
between windows; a workspace config layer with per-window live reload; colour (light/dark); navigation
controls (back, forward, reload, address bar editable/read-only/hidden, new-tab button, tab bar); locked
workspaces (no new tabs; links and popups go to an ephemeral window); pinned tabs that cannot be closed,
with optional titles in the tab bar and scope limits; tab restore with sleeping tabs behind blurred screenshots; per-workspace bookmarks and
bookmark bar with bookmark renaming; the profile database. **Permissions and downloads are built** (allow/deny/ask per kind and site, remembered answers, download folder
and ask-where).
**Tab sweeping is built** (rules per workspace, close or discard, protection for unsaved input and playing
media).
**History is built** (visits per workspace, closed tabs, the history window, reopen, ephemeral windows
recoverable, retention, `browsah history`). **Modals and popup policy are built** (the `modal` routing action, `tabs.popups`, `modals.style`).
**Routing is built too** (rules in `config.yaml`, `profile.yaml` and `workspace.yaml`; a tab's right-click
*Send to workspace*; `browsah route test`). **Cookie rules and isolated workspace storage are built** (`cookies:`, `storage: isolated`). **Web apps are built** (their own `apps/<id>/app.yaml`, `browsah app`, install from a page's manifest, launcher entry and icon, own Wayland app id). **Notifications are shown by the desktop** (Chromium's own, attributed to the profile). **The downloads list is built.** **Save as PDF and print (via the PDF viewer),** **developer tools (`F12`), fullscreen, find in page and per-site page zoom are built.** **JavaScript dialogs are built** (alert, confirm, prompt and the leave-page question). **Favicons are built.** **Tab groups are built.** **Not built:** 
sweeping actions beyond close and discard.
Known gaps: pinned tabs reload on every open (not restored), bookmarks cannot be reordered or put in
folders, no import, restore keeps only address, title and selection (not scroll position or history).

**Done so far (2026-11-17):** profile directories, `browsah profile add|list|remove`, per-profile
`.desktop` entry and icon, per-profile Chromium data directory, one unique process per profile with
link and activation hand-off (a second launch exits in about 35 ms), `selftest` profile isolation.
Layered config (application, then profile) with file watching and live reload is built; see the design note. Not yet: routing, workspaces and web apps (and their config layers), history.

Designed in [research/profiles-workspaces-design.md](research/profiles-workspaces-design.md), after
a set of spikes (per-window Wayland app id, manifest read, popup with opener, cookie filter, link
source, service workers and WebAuthn, codecs). Build order: profile directories and a unique
per-profile instance with D-Bus URL handoff; layered config with a schema per level; the router and
`route test`; workspaces; web apps; sweeping and history; permissions as cascading config.
Password-manager integration (1Password) is a separate investigation.

**Chrome extensions are built** (2026-11-19): installed from the Chrome Web Store with `browsah extension add|list|update|remove` (package signature
checked), loaded through Chromium's `--load-extension`, native messaging hosts copied from Chrome's folders, a toolbar button with the popup in a
popover, per-window button lists (`extensions.hide` for workspaces, `extensions.show` for apps; they decide what is shown, not what runs). The 1Password extension was measured to connect, unlock, suggest and fill. Not built: a bridge that shows browsah's tabs to `chrome.tabs`,
extension keyboard shortcuts and context-menu items, background auto-update; extensions are absent from `storage: isolated` windows (measured). See
[docs/config/extensions.md](config/extensions.md).

## Still open

- **Name, repo layout:** the repo is `browsah`; the app id is `be.geee.browsah`.
- **Other hardware:** Intel and NVIDIA need testing (the Mesa workaround is GPU-specific).
- **Update cadence:** how quickly to follow new CEF/Chromium releases.
- **Dev ergonomics:** `libcef.so` is found via `LD_LIBRARY_PATH`; an rpath (`$ORIGIN`) in the build
  would remove that, at the cost of one full rebuild. Part of M6.
- **Third-party notices:** generate a notices file for the Rust dependencies (as the terminal
  project does) and ship CEF's `CREDITS.html`; needed before any binary distribution (M6).
