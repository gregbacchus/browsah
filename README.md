# browsah

A custom web browser: a Rust **GTK4 + libadwaita** application around **Chromium (CEF)**, with
CEF rendering windowless into a GTK widget so the window, chrome and tabs are entirely ours. Built
for **native Wayland**. Licensed MIT.

**Status: early.** One window with tabs (new, close, switch; `window.open` and `target=_blank`
open tabs; background tabs stop rendering), a navigation bar (back, forward, reload/stop, address bar
with search), and input to the page: mouse, scrolling, keyboard and input-method text, cursors,
tooltips, dropdowns, copy/paste through the system clipboard, and a context menu. **Not yet:**
tab favicons, downloads, history, bookmarks. Input is verified by scripted tests and measurements, but
not yet with a real keyboard, mouse or IME (see [docs/roadmap.md](docs/roadmap.md)).

## Run

```bash
cargo build --release        # the first build downloads a prebuilt CEF (~300 MB)
export LD_LIBRARY_PATH=$PWD/target/release
./target/release/browsah                     # opens the home page
./target/release/browsah example.com         # or a URL, host name, or search words
./target/release/browsah --help              # the options and the commands (profile, workspace, app, config, ...)
```

Needs GTK >= 4.14 and libadwaita >= 1.5 (Ubuntu 24.04 or newer) and a Wayland session. Type a URL,
a host name, or search words in the address bar. Shortcuts: `Ctrl+L` address bar (clicking into it also selects the address), `Ctrl+Alt+C` copy the page address (also in the menus, for windows with no address bar), `Ctrl+R`/`F5`
reload, `Alt+Left`/`Alt+Right` back/forward, `Ctrl+T` new tab, `Ctrl+W` close tab,
`Ctrl+Tab`/`Ctrl+Shift+Tab` (or `Ctrl+PageDown`/`PageUp`) next/previous tab, `Alt+1`..`Alt+9` tab by
number. `Ctrl+Plus`/`Ctrl+Minus`/`Ctrl+0` (or `Ctrl` with the wheel) zoom the page, remembered per site. `Ctrl+F` finds in the page (`Enter`/`Shift+Enter` or
`Ctrl+G`/`Ctrl+Shift+G` move between matches, `Esc` closes). `F11` fills the screen; so does a video's own
fullscreen button (the window's bars hide; `Esc` or `F11` leaves). `F12` (or `Ctrl+Shift+I`) opens Chromium's
developer tools for the page in a window of their own. `Ctrl+Shift+P` saves the page as a PDF (a file dialog
asks where); `Ctrl+P` writes a PDF and opens it in your PDF viewer to print from there. Paper size and
backgrounds: the `print` options.

Page dialogs: `alert`, `confirm` and `prompt` from a page, and the "Leave this page?" question for a page with
unsaved changes, are shown as a dialog over the window (headed with the page's address); the page waits for the answer.

Profiles: each profile is its own data directory, its own GNOME application (own `.desktop`
entry and icon) and its own process. Several can run at once, and launching a running profile
again, or opening a link with it, hands the work to the running process.

```bash
browsah profile add work --name "Work" --color "#4a2f80 #7a56c2"   # light then dark
browsah profile list
browsah profile remove work --yes
browsah --profile=work https://example.com
```

With no `--profile`, the `default` profile is used. Choose a profile's entry as the default browser
in GNOME's settings and links from other applications open in that profile. Details:
[docs/config/profile.md](docs/config/profile.md). A profile's `profile.yaml` can also override any
configuration option, and config files are reloaded when they change (`watch_config`, `--no-watch`);
see [docs/config/README.md](docs/config/README.md#layers-and-reloading).

Workspaces: a workspace is a named window of tabs inside a profile, with its own colour, optional
locked-down controls, pinned tabs with optional titles in the tab bar and site restrictions,
bookmarks, and its tabs remembered between runs (sleeping behind a blurred screenshot until shown, with their back and forward pages).
Use **Quit profile** (`Ctrl+Q`) to restore all open workspaces when you next start the profile.

```bash
browsah workspace add mail --profile=work --name "Mail" --color "#2a6f97 #61a5c2"
browsah --profile=work --workspace=mail
```

Options: [docs/config/workspace.md](docs/config/workspace.md). Where links go (from other applications and from
pages) is set by routing rules, tried without a browser by `browsah route test`: [docs/config/routing.md](docs/config/routing.md). Right-click a tab to send it to another workspace. `Ctrl+Shift+N` (or the main menu, or `--new-window`) opens an ephemeral window
(not remembered), `Ctrl+D` bookmarks the page in a workspace window.

Default browser: `browsah selector install` adds a "Browsah" entry that asks which profile should open
a link from another application (it does not load Chromium, so it appears at once, and it can skip the
question with `default_profile` or a routing rule); then choose it as the default web browser in your
desktop's settings. See [docs/config/selector.md](docs/config/selector.md).

Modals and popups: a routing rule can open a link as a **modal** (a page to look at and close, over the
window, with a read-only address bar and a *Move to workspace* button), and a workspace decides whether a
page's script popup becomes a tab, a modal or nothing (`tabs.popups`). See
[docs/config/routing.md](docs/config/routing.md#modals) and [docs/config/workspace.md](docs/config/workspace.md).

History: each profile remembers the pages visited (with the workspace) and the tabs closed. `Ctrl+H` opens
the history window, `Ctrl+Shift+T` reopens the last closed tab (in an ephemeral window: the last closed
window, so ephemeral windows are recoverable). `browsah history` lists and clears it from the command line.
See [docs/config/history.md](docs/config/history.md).

Tab sweeping: rules (in a workspace, or in `profile.yaml` or `config.yaml` for all of them to inherit) can tidy up tabs you no longer use (idle for how long, a decaying use
score, which sites), closing them (recoverable with `Ctrl+Shift+T`, recorded as swept) or putting them to
sleep. `sleep_tabs_after: 30m` (at any level, also for web apps) simply puts background tabs to sleep after that idle time, never the one
you are looking at. Sleeping tabs are dimmed with a grey icon (`sleeping_tabs`), and a broom button next to reload lists tabs a sweep closed lately so you can restore them (`recently_swept`). Tabs with unsaved work (text you typed, or a page whose leave-page prompt asks) or playing media are never swept. See
[docs/config/sweep.md](docs/config/sweep.md).

Permissions and downloads: pages that ask to use the camera, microphone, location, notifications and so on are
allowed, denied or asked about by rules (per kind and per site, in any layer), and the answers you choose to
remember are kept per profile (`browsah permissions list`). Downloads go to a configurable folder per profile or
workspace, optionally asking where to save each file. See [docs/config/permissions.md](docs/config/permissions.md).

Web apps: `browsah app add --url https://mail.example.com/` reads the site's web manifest and makes a window for one site with its own
launcher entry and icon (its own `app.yaml`, not a workspace). See [docs/config/webapp.md](docs/config/webapp.md).

Chrome extensions: `browsah extension add <id or Chrome Web Store address>` installs an extension (checked against its signing key), and a toolbar button
opens its popup (`extensions.hide` / `extensions.show` choose which buttons a workspace or app window shows); the 1Password extension works with the desktop app. Chromium loads the extensions (CEF has no extension API of its own). See
[docs/config/extensions.md](docs/config/extensions.md).

The workspace picker: a button at the start of the header lists the profile's workspaces with a badge in each one's colour, ordered by last use or by name; `workspace_picker: false` hides it (and a web app's window never has it).

Favicons: a page's icon is shown on its tab (a grouped tab has its group's dot on it) and, remembered per site in the
profile, on restored tabs, in address-bar suggestions, bookmarks and the history window.

Tab groups: right-click a tab, *Add to new group…*; groups have a name and a colour, can be collapsed to one tab
and are remembered with a workspace's tabs. See [docs/config/workspace.md](docs/config/workspace.md).

Cookies and storage: `cookies:` rules (any layer) allow or deny cookies per site, and a workspace with
`storage: isolated` keeps all its cookies and site data apart from the rest of the profile. See
[docs/config/options.md](docs/config/options.md).

Right-click menus offer the commands that fit what you click: links (open in a tab, window, workspace, app, other profile or popup; copy a clean link), images (save, copy, search), media, selected text, fields and the page (with developer items), each with an icon. See [docs/config/context-menu.md](docs/config/context-menu.md).

Log: browsah logs to the system journal (`journalctl -t browsah`, or `browsah log`; why a tab was swept, closed or put to sleep is in it; `logging.level` sets how much). See [docs/config/log.md](docs/config/log.md).

Diagnostics: `BROWSAH_DEBUG=1` logs the render scale, zoom level, address-bar input and every key
press (with the focused widget); `BROWSAH_DEBUG_SCROLL=1` logs each scroll event.

## Configuration

One optional YAML file, `~/.config/browsah/config.yaml` (or `--config=PATH`); every option is
optional. Documented in [docs/](docs/README.md) (every option: [docs/config/](docs/config/README.md)); worked examples of
every option are in [examples/](examples/): `config.yaml`, `profile.yaml`, `workspace.yaml`, `webapp.yaml`.

The main menu's **Documentation** shows the docs rendered as a site, and **Application / Profile / Workspace config (all
options)** shows your file padded out with every option, its default and its explanation (a copy you can save over the
real file); `browsah config full [--profile=ID [--workspace=ID]]` prints the same.

Dragging: a drag that starts in a page (a sortable list, a reorderable panel: any `draggable` element) and ends in the same page works,
`Esc` cancels it; files, text and links dragged in from other applications work too. Dragging *out of* the page (to another
application) is not supported.

The main menu's **Browser test page** opens `bench/pages/edge.html`: buttons for popups, JS dialogs, permissions, downloads, file pickers, drag and drop, context menus, TLS and HTTP errors, storage and more, with a log of what each did.

## Tests

```bash
export LD_LIBRARY_PATH=$PWD/target/release
cargo test --release                 # unit tests, incl. config docs coverage and URL handling
BROWSAH_SELFTEST_ONLY=cookies ./target/release/browsah --selftest   # scripted check of the real window, one section (needs a display)
scripts/test selftest list                                          # the sections; running all of them needs BROWSAH_SELFTEST_ALL=<reason>
./target/release/browsah --latency   # input latency: event to the next frame at GTK
```

`--selftest` loads local pages and checks navigation, title and address updates, back/forward,
reload, frame delivery, and input (mouse, wheel, keyboard, text input, focus, cursors, tooltips,
dropdowns, clipboard, context menu) by emitting GTK's input signals, and tabs (popups from a click
open a tab and ones without a gesture are blocked, hidden tabs get no frames, switching wakes
them, new and close). It prints PASS/FAIL and exits
non-zero on failure. Set `BROWSAH_SCREENSHOT=out.png` to also save images of the window.

## Research and benchmarks

Why it is built this way, with measurements: [docs/research/](docs/research/README.md) (start with
[summary.md](docs/research/summary.md)). The benchmarks behind it still run:

| Command | Measures |
|---|---|
| `--bench=single [--mode=paint\|accel] [--seconds=20]` | one browser: paint rate, copy cost, frame pacing |
| `--bench=tabs --tabs=10 [--tab-mode=live\|static]` | multi-tab memory, CPU, power, and wake times |
| `--bench=fps --tabs=10` | power versus per-tab frame-rate cap |

Useful flags: `--mode=paint\|accel` (readback or dmabuf), `--backing=off` (render at 1x), any other
`--flag` goes to Chromium. `BROWSAH_NO_GPU_WORKAROUNDS=1` skips the per-GPU environment fixes
(see [docs/research/summary.md](docs/research/summary.md#workarounds-that-are-required)).

## Install with Homebrew, and releasing

```bash
brew tap gregbacchus/tap
brew install browsah
browsah example.com
```

For personal use, an optional separately installed CEF runtime enables H.264 and AAC:

```bash
browsah codecs install
browsah
```

Close any existing window of the same profile first. Run `browsah codecs status` to check the
installation, or `browsah codecs uninstall` to return to the bundled runtime. See
[video codecs](docs/video-codecs.md) for a separate test profile, updates and
the limits of this third-party runtime.

The brew package is a prebuilt directory (the binary with CEF's `libcef.so` and resources, about 500 MB
installed, debug info stripped) that uses the **system** GTK 4 and libadwaita (Ubuntu 24.04 or newer, x86_64 or aarch64;
`sudo apt install libgtk-4-1 libadwaita-1-0` if they are missing). `browsah` on the PATH is a small
launcher that sets `LD_LIBRARY_PATH` and tells browsah its stable `opt` paths (`BROWSAH_LAUNCHER`, `BROWSAH_LIBS`), so the
desktop entries it writes keep working after `brew upgrade browsah`. Entries written by an earlier version (which named the
versioned install directory) are repaired the next time any profile is started from the command line.

**1Password (and anything else that insists on a root-owned browser):** a Homebrew install is owned by you, so 1Password refuses it.
`sudo "$(brew --prefix)/bin/browsah" system-install --1password` puts a root-owned copy in `/opt/browsah` and a start-up script in
`/usr/local/bin`, after printing what it will change and asking; after a `brew upgrade`, `browsah system-upgrade` updates it; see [docs/config/extensions.md](docs/config/extensions.md#1password).
For H.264/AAC on that copy, run `sudo /usr/local/bin/browsah codecs install`, then launch
`/usr/local/bin/browsah` as your normal user. See [system codecs](docs/video-codecs.md#system-copy-and-1password).

**Releasing** (`.github/workflows/release.yml`): bump `version` in `Cargo.toml`, commit, then

```bash
git tag v0.1.0 && git push origin v0.1.0
```

The workflow builds both architectures, publishes a GitHub release with the tarballs and checksums, and
writes `Formula/browsah.rb` (from `packaging/homebrew/browsah.rb.in`) into the tap. It uses the same
`homebrew-tap` repository and `HOMEBREW_TAP_TOKEN` secret as paua-terminal; for this repository, add the
secret (a fine-grained token with **Contents: read and write** on `homebrew-tap`). Without it the release
still works and only the tap update is skipped.

## Licence

MIT, see [LICENSE](LICENSE). Dependencies are permissive (MIT, Apache-2.0, BSD, ISC, Unicode,
Zlib). CEF and Chromium are BSD-3-Clause plus many third-party licences; a shipped build must
include CEF's own `CREDITS.html` notices (see the roadmap, M6).
