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
```

Needs GTK >= 4.14 and libadwaita >= 1.5 (Ubuntu 24.04 or newer) and a Wayland session. Type a URL,
a host name, or search words in the address bar. Shortcuts: `Ctrl+L` address bar, `Ctrl+R`/`F5`
reload, `Alt+Left`/`Alt+Right` back/forward, `Ctrl+T` new tab, `Ctrl+W` close tab,
`Ctrl+Tab`/`Ctrl+Shift+Tab` (or `Ctrl+PageDown`/`PageUp`) next/previous tab, `Alt+1`..`Alt+9` tab by
number.

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
[docs/config/profile.md](docs/config/profile.md).

Diagnostics: `BROWSAH_DEBUG=1` logs the render scale, zoom level, address-bar input and every key
press (with the focused widget); `BROWSAH_DEBUG_SCROLL=1` logs each scroll event.

## Configuration

One optional YAML file, `~/.config/browsah/config.yaml` (or `--config=PATH`); every option is
optional. Documented in [docs/config/](docs/config/README.md); a full example is
[examples/config.yaml](examples/config.yaml).

## Tests

```bash
export LD_LIBRARY_PATH=$PWD/target/release
cargo test --release                 # unit tests, incl. config docs coverage and URL handling
./target/release/browsah --selftest  # scripted check of the real window (needs a display)
./target/release/browsah --selftest --mode=paint   # the same on the readback rendering path
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

## Licence

MIT, see [LICENSE](LICENSE). Dependencies are permissive (MIT, Apache-2.0, BSD, ISC, Unicode,
Zlib). CEF and Chromium are BSD-3-Clause plus many third-party licences; a shipped build must
include CEF's own `CREDITS.html` notices (see the roadmap, M6).
