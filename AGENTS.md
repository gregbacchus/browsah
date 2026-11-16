# AGENTS.md: browsah

browsah is a **Rust GTK4 + libadwaita web browser** around CEF (Chromium) rendered windowless.
Native Wayland is a hard requirement. MIT licensed. Read `docs/research/summary.md` for why it is
built this way and `docs/roadmap.md` for what comes next.

## Rule: keep the configuration documentation up to date

Whenever you add, rename, remove or change the meaning, type or default of ANY option in
`src/config.rs`, update in the same change:

1. `docs/config/options.md`: every option has a table row whose first column is the full dotted
   path in backticks (`` `rendering.frame_rate` ``), with type, default and description. Say when
   it applies (all options currently apply at startup).
2. `examples/config.yaml`: the worked example with every option and a comment. A test loads it
   with the real types, rejecting unknown keys.
3. `README.md` / `docs/config/README.md` if it is user-visible.
4. `all_option_paths()` in the `config.rs` tests if you add a **new config struct**.
5. Every config struct keeps `#[cfg_attr(test, serde(deny_unknown_fields))]` (release builds
   ignore unknown keys on purpose; test builds reject them so docs and examples cannot hide typos).

`cargo test --release` fails if the docs and the types disagree. Fix the docs, never the test.

## Rule: record findings in docs/research/

Research, measurements and design decisions go in `docs/research/` (one dated note per question,
with the environment, measured versus suspected results, caveats and reproduce commands), indexed
in `docs/research/README.md` and rolled up in `summary.md`. Do not put findings in the top-level
README. State which results were measured on which machine and which were not.

## Build, run, test

```bash
cargo build --release                         # first build downloads CEF (~300 MB) via cef-dll-sys
export LD_LIBRARY_PATH=$PWD/target/release    # libcef.so is copied next to the binary
cargo test --release                          # unit tests
./target/release/browsah --selftest           # scripted check of the real window (needs a display)
./target/release/browsah --bench=single       # benchmarks: single | tabs | fps
```

System packages: `build-essential pkg-config libgtk-4-dev libadwaita-1-dev` (Ubuntu 24.04+).
Changing dependencies or `RUSTFLAGS` makes `cef-dll-sys` re-verify its CEF archive, which can
take minutes.

## Layout

| Path | What lives there |
|---|---|
| `src/main.rs` | Entry: CEF subprocess handling, config, GPU workarounds, then the app or a benchmark |
| `src/app/` | The browser: `mod.rs` (window, tab bar and tab model, navigation bar, actions), `view.rs` (`BrowserView`: CEF handlers, frames, input controllers, popups, clipboard bridge, context menu), `input.rs` (key/modifier/wheel/cursor mapping, unit tested), `nav.rs` (address bar to URL), `selftest.rs` (`--selftest`), `latency.rs` (`--latency`) |
| `src/cef_rt.rs` | CEF start-up and settings, `HostHandle`/`BrowserHandle` |
| `src/frames.rs` | `View` geometry and dmabuf frame import shared by app and benchmarks |
| `src/gpu.rs` | Per-GPU environment workarounds (AMD: `AMD_DEBUG=nodcc`) |
| `src/profile.rs` | Profiles: ids, directories, `profile.yaml`, `.desktop` entry and icon, the `browsah profile` command line (no CEF or window needed) |
| `src/config.rs` | Config types, loading, and the docs/example tests |
| `src/cli.rs` | Command-line options |
| `src/bench/` | `--bench=single|tabs|fps`, kept as a regression check for the rendering path |
| `bench/`, `docs/`, `examples/` | Test pages (`bench/pages/` for `--selftest` and `--latency`), documentation (config, research, roadmap), example config |

## Conventions and gotchas

- **CEF runs on its own UI thread** (`multi_threaded_message_loop`). Never pump it from the GTK
  loop: that pins the main thread at 100 % CPU (docs/research/cef-multi-tab-cost.md).
- CEF handlers therefore run off the GTK thread. They may touch only `Arc<Shared>` state (mutexes
  and atomics) and hand work to the GTK thread with `glib::MainContext::invoke`. GTK objects live
  in thread-locals keyed by view id and are used on the GTK thread only.
- Never hold a mutex while calling into CEF, and use method (not field) access on `Send` wrapper
  types inside closures (edition 2021 captures disjoint fields otherwise).
- `Browser`/`BrowserHost` methods used from the GTK thread must be thread-safe; the frame-rate
  setter and DevTools calls were posted to CEF's UI thread (`post_task`).
- Render at device resolution on HiDPI (`View::k`) and re-apply zoom after navigation.
- **Licence:** browsah is MIT. Do not add a dependency whose licence is not permissive
  (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib) without asking.
- **One process per profile.** Chromium locks its cache directory, so a profile (`src/profile.rs`)
  is one process with its own `GApplication` id (`be.geee.browsah.<id>`). `main` registers
  the application *before* starting CEF; if the profile is already running it forwards the URL
  (`open`) or `activate` and exits. `--selftest`, `--latency` and benchmarks use the `selftest`
  profile so they never touch a real profile or collide with a running browser. Profile config
  (`profile.yaml`) has its own docs and example, with tests that check both (see
  `docs/config/profile.md`, `examples/profile.yaml`).
- **Config layers:** the effective configuration is `config.yaml` with the profile's `profile.yaml`
  merged over it (`config::load_effective`; rules in `docs/config/README.md`). Option names are the
  same at every level, so adding an option to `Config` makes it overridable in a profile; mark it
  application-only in `config::APP_ONLY` if a profile must not set it, and classify it live or
  restart-only in `config::RESTART_REQUIRED` (otherwise it is treated as live, so it must be read
  from the live configuration, not copied at start-up). Reloading is `app::reload` plus
  `app/watch.rs`.
- **Tabs:** one `BrowserView` per tab inside an `adw::TabView`; the address bar and buttons are
  shared and show the selected tab (`Shell::refresh`). A view tells CEF it is hidden when its widget
  is unmapped (`BrowserView::set_hidden`, posted to CEF's UI thread). Events carry no tab: each tab
  has its own closure. `window.open` is cancelled in `on_before_popup` and becomes a tab, only with
  a user gesture. A navigation requested before the browser exists is queued (`pending_url`).
- A real mouse pointer resting over the window gets re-delivered whenever layout shifts, which
  races the signals `--selftest` emits; the self-test therefore ignores real pointer events
  (`set_synthetic_pointer_only`).
- Input is tested by emitting the GTK controllers' signals (`Driver` in `selftest.rs`), which covers
  the translation to CEF but not real devices or a real IME; say so when reporting input results.
- Answer CEF's context-menu callback *before* closing the popover (closing cancels it), and run
  its methods on CEF's UI thread (`cef_rt::on_ui`).
- Windowless Chromium cannot reach the Wayland clipboard: copy, cut and paste go through GTK's
  clipboard in `view.rs`.
- Verify behaviour changes with `--selftest` and, for rendering or power claims, the benchmarks;
  tests passing is not proof that the docs are accurate.
