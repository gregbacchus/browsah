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
cargo test --release cookies                  # unit tests whose name contains "cookies" (see below: be targeted)
BROWSAH_SELFTEST_ONLY=install ./target/release/browsah --selftest   # one self-test section, seconds
./target/release/browsah --selftest           # the whole scripted check of the real window (~4 minutes)
./target/release/browsah --bench=single       # benchmarks: single | tabs | fps
```

### Be targeted when testing

`scripts/test` is the way in: `scripts/test unit cookies` (unit tests by name), `scripts/test selftest install,cookies`
(self-test sections, seconds), `scripts/test selftest list` (the section names; a misspelt name is an error, not a
silent no-op), `scripts/test selftest all` (the whole thing). A test keeps the section list in step with `run`.

Do **not** run everything after every change. Run what the change can affect:

- **Unit tests:** `cargo test --release <name>` for the module you touched (`cookies`, `workspace`, `config::`,
  `webapp`...). The whole unit suite is fast (about a second once built), so run it all before committing, but
  not after each edit.
- **Self-test:** `BROWSAH_SELFTEST_ONLY=a,b` runs only the sections named (a section is a `run_<name>` function
  listed in `selftest.rs`'s main sequence: `basic` (typing an address, back, forward, reload), `input`, `scroll`, `tabs`, `locked`, `routing`, `permissions`,
  `downloads`, `cookies`, `storage`, `session_cookies`, `session_scope`, `webapp`, `install`, `suggest`, ...). It takes
  seconds. Add a section's check next to the feature, and run that section while developing. Run the **whole**
  self-test (about four minutes; run it in the background and wait for it, `timeout 298`) once, before a commit
  that touches shared code (view.rs handlers, the shell, config layering), not for an isolated feature.
- **Look at new UI.** `BROWSAH_SHOTS=/some/dir` makes the self-test save pictures of the widgets it calls `shot(...)` on
  (popovers, dialogs, windows; `downloads-finished`, `downloads-running`, `suggestions`, `install-dialog`,
  `web-app-window`...) as PNGs you can open with the Read tool. Add a `shot` next to a new piece of UI and look at it:
  passing checks say nothing about layout (a first look found a downloads list too narrow to show file names, labels
  centred by `xalign`, and a highlighted suggestion with no visible highlight).
- Never start the user's real profile in a test: the self-test uses the throwaway profile `selftest`, and a CLI
  experiment should set `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` to a temporary directory.
- Do not run two self-tests at once (they share the profile and collide on CEF's lock).

System packages: `build-essential pkg-config libgtk-4-dev libadwaita-1-dev` (Ubuntu 24.04+).
Changing dependencies or `RUSTFLAGS` makes `cef-dll-sys` re-verify its CEF archive, which can
take minutes.

## Layout

| Path | What lives there |
|---|---|
| `src/main.rs` | Entry: CEF subprocess handling, config, GPU workarounds, then the app or a benchmark |
| `src/app/` | The browser: `mod.rs` (window, tab bar and tab model, navigation bar, actions), `view.rs` (`BrowserView`: CEF handlers, frames, input controllers, popups, clipboard bridge, context menu), `input.rs` (key/modifier/wheel/cursor mapping, unit tested), `nav.rs` (address bar to URL), `selftest.rs` (`--selftest`), `latency.rs` (`--latency`), `install.rs` (install a page as a web app from its manifest), `downloads.rs` (the downloads button and list; rows are the profile's, thread-local, remembered in the `downloads` table) |
| `src/cef_rt.rs` | CEF start-up and settings, `HostHandle`/`BrowserHandle` |
| `src/frames.rs` | `View` geometry and dmabuf frame import shared by app and benchmarks |
| `src/gpu.rs` | Per-GPU environment workarounds (AMD: `AMD_DEBUG=nodcc`) |
| `src/profile.rs` | Profiles: ids, directories, `profile.yaml`, `.desktop` entry and icon, the `browsah profile` command line (no CEF or window needed) |
| `src/workspace.rs` | Workspaces: `workspace.yaml` schema (name, colour, `nav`, `tabs`, pinned tabs, `app`), scope patterns, the `browsah workspace` command line |
| `src/webapp.rs` | Web apps (a workspace with an `app:` section): application id, `.desktop` entry and icon, the `browsah app` command line |
| `src/groups.rs` | Tab group model: palette, ids, the dot icon, where a joining tab goes, collapsed title (pure, tested); the window side is `app/groups.rs` |
| `src/suggest.rs` | Address-bar suggestion ranking (pure, tested); the popover is `app/suggest.rs` |
| `src/cookies.rs` | Cookie rules (`cookies:`): `allow`/`session`/`deny` per site, session-cookie keys, allow-rule YAML for blocked sites |
| `src/history.rs` | History helpers and the `browsah history` command line |
| `src/sweep.rs` | Tab sweeping model: durations, `Usage` (decaying score), rules and the `Sweeper` (pure, tested); applying it is `app/sweep.rs` |
| `src/permissions.rs` | Permission model (`Choice`, `Kind`, `PermissionsConfig::decide`) and `browsah permissions` |
| `src/downloads.rs` | Download folder resolution, file names and `DownloadsConfig` |
| `src/store.rs` | The profile's SQLite database: remembered tabs and bookmarks (GTK thread only) |
| `src/router.rs` | URL routing: rules (`routing:` in config, profile and workspace files), the pure `Router`, the process-wide installed router (read on CEF's thread), `browsah route test` |
| `src/selector.rs` | The profile selector: `browsah select [URL]` (a small chooser window, no CEF), `browsah selector install|uninstall|status` (the `be.geee.browsah.desktop` entry) |
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
- **Workspaces** are windows of the profile's process (`Shell` per window, registry in `app/mod.rs`:
  `windows()`; a tab's owner window changes when it is dragged, see `Tab::owner`). A workspace window
  saves its tabs (`Shell::save_now`, debounced by `schedule_save`, and in the close handler before
  browsers close) and restores them as *dormant* views: `BrowserView::new_dormant` builds the widgets
  and shows the screenshot, `wake()` creates the browser on first show. Screenshots are copied out of
  the GPU buffer at once on deselect (`capture`, a later download failed intermittently) and encoded
  when idle. A pinned tab's scope is enforced on CEF's thread in `on_before_browse` (`Shared::scope`);
  redirects and pages already outside the scope are exempt. Schema changes to the database need a
  migration in `Store::migrate` and a bump of `SCHEMA_VERSION`.
- **Permissions and downloads**: CEF's `PermissionHandler` and `DownloadHandler` are in `view.rs`
  (`handle_permission`, `ViewDownloads`) and judge by the view's copy of the configuration
  (`BrowserView::set_permissions`/`set_downloads`, pushed by `Shell::apply_view_settings` whenever a view is
  created or the configuration reloads). Only `ask` reaches the shell (`app/prompts.rs`: a queue of
  `AlertDialog`s, remembered answers in `permission_decisions`, the Save-as dialog, download toasts). Chromium
  keeps the answer it is given to a prompt and then stops asking us, so `forget_in_chromium` clears it on every
  main-frame load start; setting the screen-capture content setting crashes Chromium, so that kind is skipped.
  The self-test holds questions (`prompts::set_ui_test`) instead of showing them.
- **Sweeping closed workspaces**: `sweep::sweep_stored` applies the rules to a workspace's remembered tabs in
  the database (the profile process calls it from `app::sweep::background_sweep`, eight seconds after start and
  every minute per workspace interval, skipping workspaces with an open window; `browsah sweep` calls it for a
  profile that is not running).
- **Sweeping** (`app/sweep.rs`, model in `sweep.rs`): a workspace's `sweep:` rules run on a timer. Each pass
  probes every live tab with a script (`PROBE`: unsaved form input, playing media) through DevTools
  `Runtime.evaluate`, waits 1.5 s, then closes (reason `sweep:<rule>` in `closed_tabs`) or discards
  (`Shell::discard_tab`: screenshot, then a dormant view replaces the page) the tabs a rule fires for. A tab
  whose probe gives no answer is left alone. Use is recorded as events (`Usage`) on selection and activity
  (`BrowserView::input_events`) and stored with the tabs (`tabs.usage`).
- **History** lives in the profile database (`Store::record_visit`, `record_closed`, `Scope`), written by
  `Shell::record_visit` (on load end, deduped per tab), `record_closed` (user close) and
  `record_window_closed` (an ephemeral window's tabs, as a group). `Shell::private` (sign-in windows) and
  `history.exclude` / `history.enabled` decide what is remembered (`Shell::remembers`). The history window is
  `app/history.rs`. Closed tabs carry a `reason` (`user`, `window`; sweeping will add `sweep:<rule>`).
- **Modals** (`app/modal.rs`) are a browser view in an `adw::Dialog` (or an `adw::Window`) over a window;
  *Move* takes the live view out of the dialog and into a tab of the destination (`Shell::adopt_live_view`,
  which swaps the view's event handler with `BrowserView::set_event_handler`). The router's
  `route_ignoring_modal` says where the page belongs. A workspace's `tabs.popups` is the popup policy:
  `block` cancels the popup in `on_before_popup` (CEF's thread, `Shared::popup_policy`), `modal` adopts it
  into a modal.
- **Routing:** `router::route` is called from CEF's thread (`routed_elsewhere` in `view.rs`:
  `on_before_browse` for same-tab links and typed navigations, `on_open_urlfrom_tab` for Ctrl/middle
  clicks, `on_before_popup` for `target=_blank` and plain `window.open`) and from the `open` handler for
  links from other applications. A non-Allow decision cancels the navigation and sends a `Route` event;
  the shell carries it out (`perform_route`). Redirects, form posts and sized popups are never routed.
  `routing:` is stripped from the option layers (`strip_routing`) and read only by the router; the
  router and its docs test live in `router.rs` / `docs/config/routing.md`.
- **Config layers:** the effective configuration is `config.yaml` with the profile's `profile.yaml`
  and then, per window, the workspace's `workspace.yaml` merged over it (`config::load_layers`,
  `load_window`; process-only options are ignored in a workspace, see `WORKSPACE_IGNORED`; rules in `docs/config/README.md`). Option names are the
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
- **Cookies** (`src/cookies.rs`, filter in `view.rs`): a `CookieAccessFilter` returned from the per-request
  `ResourceRequestHandler` (only when the rules are not open) judges each cookie sent or saved by the request's
  URL against the view's copy of `cookies:` (`BrowserView::set_cookies`, pushed by `apply_view_settings`).
  `session` cookies are recorded per owner (the workspace id, or `window-N`) in `SESSION_COOKIES` and sent only to
  the owner that saved them; `end_session_cookies` deletes them through a `CookieVisitor` when the window closes
  (`ending`), and at window open deletes session cookies nobody owns (left by an earlier run). `deny` records the
  blocked host per view (cleared in `on_before_browse` of a main-frame navigation, not at load start: Chromium saves
  response cookies before that); the tab menu's *Copy allow rules* turns them into YAML.
- **Storage:** `storage: isolated` gives a workspace its own CEF `RequestContext` (`cache_path`
  `<profile>/cef/workspaces/<id>`, cached per folder in `view.rs`); `Shell::prepare_storage` sets the folder for the
  next view just before it is created (thread-local, read in `Shared::new`).
- **Web apps:** a workspace whose `workspace.yaml` has `app:` (`WorkspaceConfig::apply_app` fills in the pinned
  start tab, no new tabs, modal popups and the controls for `app.display`; whatever the file sets itself wins).
  Its window sets its own Wayland application id (`be.geee.browsah.<profile>.<id>`, the `.desktop` file's name) when
  mapped, through `gdk4-wayland` (`set_application_id` must run at `map`; GTK overwrites it earlier). Apps are left
  out of the *Send to workspace* lists. `install.rs` reads the manifest with `Page.getAppManifest` (`parse_manifest`
  is pure and tested; the answer's `manifest` object has resolved URLs, `data` the raw text with icon purposes),
  asks the user to confirm the name, downloads the icon with `download_image` and writes the same files as
  `browsah app add`. The self-test profile never writes launcher entries (`webapp::install` returns early for it).
- **CEF API gotchas:** `Image::as_png` returns nothing unless the pixel width/height out parameters are passed;
  `BinaryValue::data` needs a pre-sized buffer; setting the screen-capture content setting crashes Chromium;
  a cookie's `domain` field is read with `.to_string()`.
