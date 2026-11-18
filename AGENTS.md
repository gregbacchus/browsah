# AGENTS.md: browsah

browsah is a **Rust GTK4 + libadwaita web browser** around CEF (Chromium) rendered windowless.
Native Wayland is a hard requirement. MIT licensed. Read `docs/research/summary.md` for why it is
built this way and `docs/roadmap.md` for what comes next.

## Rule: test narrowly. Never run a wide test set unless told to (read before running ANY test)

**Wide test runs are forbidden by default.** That means: the whole self-test (`--selftest` with no sections named,
`scripts/test selftest all`), and the whole unit suite (`cargo test` with no filter, `scripts/test unit` with no name). Do not run
them after a change, before a commit, at the end of a task, "to be safe", "to check nothing else broke", after fixing a flaky
failure, or because a change touched shared code. There is no "run everything once before committing" rule. The user has asked for
this repeatedly; running wide sets anyway wastes minutes per run and is the single thing that most frustrates them.

What to run instead, always the smallest thing that exercises the code you changed:

- unit tests: `scripts/test unit <name>` (a module or test name, e.g. `cookies`, `workspace`, `config::`);
- self-test: `scripts/test selftest <section>[,<section>]` (see `scripts/test selftest list`); add or extend a section next to
  the feature and run only that. If your change affects other features, name those few sections too, not all of them.

A wide run is allowed in exactly two cases:

1. **The user asks for it** in their own words in this conversation ("run all the tests", "run the full suite"). A past request does
   not carry over to later changes.
2. **There is a specific reason a narrow test cannot suffice**, which you write down in your message *before* running it: which
   shared mechanism changed and why no named section or test covers it. "It touches shared code" or "to be thorough" is not a
   reason. Even then, run the few sections that cover it, not everything.

Also: if a narrow test fails, investigate and re-run *that* test; do not widen the run to look for other damage. If a failure looks
flaky, re-run that one section, never the whole set. When you report, say exactly which narrow tests you ran and do not imply wider
coverage you did not run. The tools enforce this: the self-test binary refuses to run all sections unless `BROWSAH_SELFTEST_ALL`
holds a reason (`scripts/test selftest all "<reason>"` sets it), and `scripts/test unit` refuses to run without a name. Do not set
these to get around the refusal; setting them is only for the two cases above.

## Rule: keep the configuration documentation and examples up to date

Whenever you add, rename, remove or change the meaning, type, range, default or reload behaviour of ANY option
(in `src/config.rs` or in any other config struct: profile, workspace, web app `app.yaml`, `routing`, `sweep`,
`permissions`, `cookies`, `downloads`, ...), update ALL of these in the same change. Passing tests are not
proof the docs are right: the tests only check that paths exist.

1. **The docs page that owns the option** (`docs/config/*.md`; index in `docs/README.md`): every option has a table
   row whose first column is the full dotted path in backticks (`` `rendering.frame_rate` ``; `[]` = each list entry),
   with type, the actual default, and a description. Say **when it applies** (live, on reload, new windows or tabs
   only, or restart), how it layers (profile/workspace overrides, `key+` append), and how it interacts with other
   options. Keep the section's example YAML block correct.
2. **`docs/config/value-types.md`** if you add a new kind of value (colour form, pattern, duration, action).
3. **`docs/config/README.md`**: the sections table, and the "Layers and reloading" table if it is restart-only or
   application-only (`config::RESTART_REQUIRED`, `APP_ONLY`, `WORKSPACE_IGNORED`).
4. **The examples** (`examples/config.yaml`, `profile.yaml`, `workspace.yaml`, `webapp.yaml`): show the option with a
   sensible value and a comment, and delete stale keys. Tests load them with the real types rejecting unknown keys, and
   `example_config_shows_every_option` / `examples_show_every_profile_workspace_app_routing_and_sweep_option` fail when
   an option appears in no example.
5. **`README.md`** and `docs/roadmap.md` if it is user-visible or changes what is planned.
6. **Tests' option lists**: `all_option_paths()` (and the `roots` list in
   `docs_do_not_describe_options_that_do_not_exist`) in `src/config.rs` when you add a **new config struct**; the
   field lists in the profile, workspace, routing and example-coverage tests when you add a struct there.
7. Every config struct keeps `#[cfg_attr(test, serde(deny_unknown_fields))]` (release builds ignore unknown keys on
   purpose; test builds reject them so docs and examples cannot hide typos).
8. The "all options" copies and the Documentation window are built from the examples and `docs/` automatically: keep the examples in the 2-space block style they use (the generator reads lines, not full YAML) and keep links between docs pages valid (`scripts/test unit docsite` and `scripts/test unit fullconfig` check both).
9. Run the narrow docs/example tests (`scripts/test unit docs_`, `scripts/test unit example`). A failure means the
   docs are out of date: fix the docs, never the test.

**Before reporting a config change as done**, re-read each item above against the implementation (the row, the real
default, the reload behaviour, the example, the README) and say in your message what you checked and anything you
could not verify. If an item is unaffected, verify that and say so; no empty edit is needed.

## Rule: record findings in docs/research/

Research, measurements and design decisions go in `docs/research/` (one dated note per question,
with the environment, measured versus suspected results, caveats and reproduce commands), indexed
in `docs/research/README.md` and rolled up in `summary.md`. Do not put findings in the top-level
README. State which results were measured on which machine and which were not.

## Build, run, test

```bash
cargo build --release                         # first build downloads CEF (~300 MB) via cef-dll-sys
export LD_LIBRARY_PATH=$PWD/target/release    # libcef.so is copied next to the binary
cargo test --release cookies                  # unit tests whose name contains "cookies" (always filter; see the testing rule above)
BROWSAH_SELFTEST_ONLY=install ./target/release/browsah --selftest   # one self-test section, seconds (never run it without naming sections)
./target/release/browsah --bench=single       # benchmarks: single | tabs | fps
```

### Running tests (narrowly: see the rule at the top)

`scripts/test` is the way in: `scripts/test unit cookies` (unit tests by name), `scripts/test selftest install,cookies`
(self-test sections, seconds), `scripts/test selftest list` (the section names; a misspelt name is an error, not a
silent no-op). A test keeps the section list in step with `run`.

- **Unit tests:** a name filter every time (`scripts/test unit workspace`).
- **Self-test:** `BROWSAH_SELFTEST_ONLY=a,b` runs only the sections named (a section is a `run_<name>` function
  listed in `selftest.rs`'s main sequence: `basic` (typing an address, back, forward, reload), `input`, `scroll`, `tabs`, `locked`, `routing`, `permissions`,
  `downloads`, `cookies`, `storage`, `session_cookies`, `session_scope`, `webapp`, `install`, `suggest`, ...). It takes
  seconds. Add a section's check next to the feature and run that section while developing.
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
| `src/extensions.rs` | Chrome extensions: which folders to load (`extensions.*`, the profile's `extensions/<id>/<version>/`), the `--load-extension` switch, native messaging host discovery and copy, Web Store install and update (`browsah extension`), id derivation; `src/crx.rs` is the CRX3 package format (parse, signature check, unzip, add `key`); `src/app/extensions.rs` is the header button and popup popover (pure, tested; Chromium itself loads the extensions: CEF 156 has no extension API) |
| `src/system_install.rs` | `sudo browsah system-install [--1password] [--dry-run] [--yes]` / `system-uninstall`: a root-owned copy of the program in `/opt/browsah` (the release tarball's file list), a start-up script in `/usr/local/bin`, optionally `browsah` in 1Password's `custom_allowed_browsers`; it leaves `<prefix>/INSTALL` (source folder, version, whether `--1password`) so `browsah system-upgrade` can re-run the source copy's `system-install` through sudo, and `stale_check` (called at start) warns when the source copy has changed; plans and copies are tested in temporary folders, what root adds (ownership) is not testable without sudo |
| `src/gpu.rs` | Per-GPU environment workarounds (AMD: `AMD_DEBUG=nodcc`) |
| `src/profile.rs` | Profiles: ids, directories, `profile.yaml`, `.desktop` entry and icon, the `browsah profile` command line (no CEF or window needed) |
| `src/workspace.rs` | Workspaces: `workspace.yaml` schema (name, colour, `nav`, `tabs`, pinned tabs, `app`), scope patterns, the `browsah workspace` command line |
| `src/manifest.rs` | What a page says about itself as an app (`AppInfo`): read from Chromium's `Page.getAppManifest` answer (install from a tab) or from the manifest file itself (`parse_raw_manifest`, `page_links`, `resolve`; `browsah app add`) (pure, tested) |
| `src/webapp.rs` | Web apps (`apps/<id>/app.yaml`, not a workspace): the `AppConfig` schema and `window_config` (what it implies for a window), application id, `.desktop` entry (with the "New Window" action) and icon, the `browsah app` command line (`add`, `list`, `install`, `route`, `remove`); `create` is the one way an app is made (CLI and install from a page); `info_from_site` fetches with `ureq` (rustls, system certificates, no bundled root list) |
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
| `src/icons.rs` | Lucide icons (`assets/icons/lucide/`, ISC): drawn from a 24 px grid with a 1.5 stroke (shown at the usual 16 px), installed as per-style ordinary icons and kept right when the style changes (`icons::bind`); use these for any new icon (docs/research/icons-lucide.md) |
| `src/fullconfig.rs` | The "all options" copy of a config file (`browsah config full`, the menu): `examples/*.yaml` are the templates, so their comments are the explanations; set values are kept, the rest commented out (pure, tested; window: `app/fullconfig.rs`) |
| `src/docsite.rs` | The docs rendered as a static site (`pulldown-cmark`); `build.rs` compiles `docs/**/*.md`, `README.md` and `examples/*.yaml` into the binary; the test fails on a broken link or anchor between pages (window: `app/docs.rs`, written to the cache folder) |
| `src/config.rs` | Config types, loading, and the docs/example tests |
| `src/cli.rs` | Command-line options |
| `src/bench/` | `--bench=single|tabs|fps`, kept as a regression check for the rendering path |
| `bench/`, `docs/`, `examples/` | Test pages (`bench/pages/` for `--selftest` and `--latency`), documentation (`docs/README.md` indexes `docs/config/` for every option, `docs/research/`, `docs/roadmap.md`), the four example files (`config`, `profile`, `workspace`, `webapp`; see the config rule above) |

## Conventions and gotchas

- **Icons are Lucide** (24 px grid, 1.5 stroke, shown at the usual 16 px; `src/icons.rs`, `assets/icons/lucide/`, docs/research/icons-lucide.md). Add the SVG unchanged to `assets/icons/lucide/`, list it in `src/icons.rs` and `icons::bind` it to the widget. Do not use a `-symbolic` name for an outline icon: GTK fills symbolic icons. Stock symbolic icons remain only where nothing is converted yet.

- **CEF runs on its own UI thread** (`multi_threaded_message_loop`). Never pump it from the GTK
  loop: that pins the main thread at 100 % CPU (docs/research/cef-multi-tab-cost.md).
- CEF handlers therefore run off the GTK thread. They may touch only `Arc<Shared>` state (mutexes
  and atomics) and hand work to the GTK thread with `glib::MainContext::invoke`. GTK objects live
  in thread-locals keyed by view id and are used on the GTK thread only.
- Never hold a mutex while calling into CEF, and use method (not field) access on `Send` wrapper
  types inside closures (edition 2021 captures disjoint fields otherwise).
- `Browser`/`BrowserHost` methods used from the GTK thread must be thread-safe; the frame-rate
  setter and DevTools calls were posted to CEF's UI thread (`post_task`).
- **HiDPI:** CEF is given a *logical* view size and the *real* device scale factor (`View::dsf`), paints at device resolution and
  works in logical (DIP) coordinates (`View::k()` is 1). This needs Chromium's `WaylandFractionalScaleV1` feature off
  (`cef_rt::with_default_flags`); with it on, Chromium ignores the scale CEF reports (docs/research/hidpi-scale.md). Do not zoom the page
  to fake a scale: scrollbars would stay 1x. Only the person's own zoom is applied (`apply_zoom`, re-applied after navigation, since CEF
  resets zoom when the host changes). Chromium saves per-host zoom in the profile's `Preferences`; start-up clears it
  (`cef_rt::forget_saved_zoom`) because hosts browsah never sets a zoom for (DevTools, error pages) would otherwise keep a stale one.
  **Text scaling** (GNOME "Large Text", `text-scaling-factor`) is an extra scale factor in `View::text` (`frames::text_scale`, read each
  tick): CEF gets `scale * text` as its device scale and a view of the window divided by `text`; input goes through `View::k() = 1/text`
  (docs/research/text-scale.md). `BROWSAH_FORCE_TEXT_SCALE=1.25` pretends it; only the `hidpi` section is text-scale aware.
  Test aids: `BROWSAH_FORCE_SCALE=2` pretends a display scale; `BROWSAH_NO_SCALE_FIX=1` leaves the Wayland feature on (the `hidpi` section
  then fails, which is the point); `scripts/devtools-probe.py` asks the DevTools frontend its own scale (see the note).
- **Window colour:** `workspace_css` writes the style sheet; the page is inside a `.browsah-frame` box so the colour can frame it
  (`window.frame.*`, live). Tests that need an un-maximized window must wait for the real resize, not only the `maximized` class
  change (the compositor resizes later), and start by maximizing (it may remember either state).
- **Check pictures, not just numbers.** A DevTools emulation attempt passed every numeric check (`devicePixelRatio`, `innerWidth`) while
  the page was painted at half size; the `hidpi` section now checks the frame size and layout width too, and a picture was looked at.
- **Licence:** browsah is MIT. `ureq` (MIT/Apache-2.0) fetches a site for `browsah app add`: keep its features to `rustls-no-provider` + `platform-verifier` (the default `rustls` feature pulls the CDLA-licensed `webpki-roots`). Do not add a dependency whose licence is not permissive
  (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib) without asking.
- **One process per profile.** Chromium locks its cache directory, so a profile (`src/profile.rs`)
  is one process with its own `GApplication` id (`be.geee.browsah.<id>`). `main` registers
  the application *before* starting CEF; if the profile is already running it forwards the URL
  (`open`) or `activate` and exits. `--selftest`, `--latency` and benchmarks use the `selftest`
  profile so they never touch a real profile or collide with a running browser. Profile config
  (`profile.yaml`) has its own docs and example, with tests that check both (see
  `docs/config/profile.md`, `examples/profile.yaml`).
- **Tab history across restarts:** CEF cannot be given navigation entries, so a tab keeps its earlier and later pages in `Tab::hist_back`/`hist_fwd` (`src/navhistory.rs`, saved in `tabs.nav_history`); `Shell::history_back/forward` use them when the browser has nothing to go to, `Tab::keep_browser_history` folds the browser's entries in before a view is replaced (sleep, fall-back). A step through the lists must run `location.replace/assign` with a user gesture (`BrowserView::step_to`) and is not told apart from a new navigation for 2 s (`skip_nav_until`). docs/research/tab-history.md.
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
- **Web app tabs:** an explicit new-tab request (`ViewEvent::OpenTab`: Ctrl/middle click, "open in new tab") opens a tab whenever the window allows
  tabs, even though an app's popup policy is `modal` (the overlay is for windows with no tabs). An app with a hidden address bar and tabs puts the
  `adw::TabBar` (`inline` style) in the header's title position while `n_pages > 1` (`build`: `tabs_in_header`); with one tab the title shows. An app's
  start tab is an ordinary (unpinned) tab so the strip shows its title (`add_pinned` skips pinning for apps; the scope applies to every tab of an app
  anyway), apps have no new-tab button or Ctrl+T, and their tab menu has no "send to workspace".
- **Sweeping config and sleeping:** `sweep` and `sleep_tabs_after` are options of the layered `Config` (not workspace-only keys), so a workspace
  inherits the profile's and the application's `sweep` (its own keys win, `rules+` appends; `sweep::effective_sweep` reads the layers for
  closed workspaces). `sleep_tabs_after` adds a last implicit `discard` rule (`SLEEP_RULE`, matches every tab, `pinned_ok`) in
  `Sweeper::from_config`; the selected tab of every window is always protected (a sleeping tab wakes on selection, so a hidden window's selected
  tab cannot sleep). `Shell::sweeper` is a `RefCell` rebuilt by `apply_config` and the 30 s timer runs the rules when their interval has passed.
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
- **Redirectors:** a link that wraps its destination (`google.com/url?q=`) is *unwrapped* before routing (`router::unwrap_url`,
  called in `routed_elsewhere` and the external-link handler; built-in list in `builtin_unwrap`, plus `routing.unwrap`), so rules judge
  and open the destination. `routing.follow_redirects` (off by default) also routes the redirect hops of a clicked same-tab link:
  `on_before_browse` with `is_redirect` is judged while `Shared::link_nav` is set (a link click began the navigation; cleared when it
  commits). Redirects are otherwise never routed (sign-in flows).
- **Theme:** the `theme` option (`system`|`light`|`dark`, any layer) is what pages see as `prefers-color-scheme`, set per browser with
  `Emulation.setEmulatedMedia` (`BrowserView::set_color_scheme`, from `apply_view_settings`; applied again when the browser is created).
  Chromium's own default does not follow the desktop here (reports light while the portal says dark), so `system` is resolved by us from
  `adw::StyleManager::is_dark()` and re-applied on its `dark` notify (`Shell::refresh_scheme`). It does not change the window's chrome.
- **Favicons:** `DisplayHandler::on_favicon_urlchange` lists icon addresses (read with the raw `cef_string_list_*` calls: the crate's
  `CefStringList` has no reader); `fetch_favicon` downloads the first that works (`download_image`, 64 px, falling through the rest) and
  sends `ViewEvent::Favicon(png)`. `app/favicon.rs` shows it (`Tab::favicon`, `refresh_tab_icon`), remembers it per origin in the
  `favicons` table (newest 2000) and reads it back for restored tabs and lists (`cached_for`, `small_image`, with a decoded-texture
  cache). The tab's icon slot is shared: favicon, the group's dot drawn on it (`with_dot`), a generic icon for pinned tabs; the indicator
  slot belongs to sweeping. Not stored for private windows.
- **Launcher entries and the program path:** `.desktop` entries name the program through `profile::launcher()`: `BROWSAH_LAUNCHER` and
  `BROWSAH_LIBS` when a package sets them (Homebrew's launcher script does, with the stable `opt` paths, because the versioned Cellar
  directory is deleted on upgrade), else this binary's canonical path. `webapp::refresh_launchers` (start-up thread) rewrites the
  running profile's entries, icons and apps in full, and any other profile's or the selector's entry only when its program no longer
  exists (`profile::entry_program_missing`), so a source build never repoints another install's entries.
- **Signals:** `SIGTERM`/`SIGINT`/`SIGHUP` quit the profile like the menu's *Quit profile* (`on_signal` in `run`: `finish_window_close` for every window, then `Application::quit`); a second signal exits at once. Check with `scripts/test signals` (docs/research/signals.md).
- **Shutdown:** `Runtime::shutdown` waits for the counted browsers (`cef_rt::browser_created/closed`) to close, not a fixed sleep
  (docs/research/shutdown.md); the benchmarks, which do not count, keep 500 ms. The `sqlite_persistent_store… Failed to post task`
  warning at exit is Chromium's own teardown and harmless (data lands); do not try to "fix" it with flushes or delays.
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
  clipboard in `view.rs`. A page's own copy button (`writeText`, `execCommand('copy')`, a `copy` handler) is bridged by `COPY_HOOK`: run in every frame from `on_load_start`, it
  reports the text as a console message that `on_console_message` takes (`ViewEvent::ClipboardWrite`); the shell uses it only within 5 s of the person using the page
  (`BrowserView::used_within`). DevTools message ids must increase in the order calls run: use `Shared::post_cdp`, never take an id and post separately (docs/research/page-copy-buttons.md).
- Verify behaviour changes with the relevant `--selftest` section(s) (never the whole self-test: see the testing rule at the top) and, for rendering or power claims, the benchmarks;
  tests passing is not proof that the docs are accurate.
- **Drags that start in the page** (`draggable` elements: sortable lists, reorderable panels): `RenderHandler::start_dragging` (view.rs) takes the
  drag and stores it in `Shared::page_drag`; while it runs the pointer is the drag (`BrowserView::pointer_moved` → `page_drag_move`:
  `drag_target_drag_enter/over`), the left button's release is the drop (`page_drag_end`) and `Esc` cancels; then
  `drag_source_ended_at` / `drag_source_system_drag_ended`. The mouse-up is not sent to the page. Without `start_dragging`, CEF aborts every
  in-page HTML5 drag after `dragstart`. Dragging out of the page to another application is still not supported. Tested by `drop`.
- **Activation of a running profile:** `connect_activate` treats only the first activation as this process's own command line; a later one
  (the profile's launcher icon, `browsah --profile=ID`) is a plain launch (`activation_workspaces`). Otherwise a process started with `--app=ID`
  would turn "show the profile" into "focus the app".
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
- **Web apps** are not workspaces: `<profile>/apps/<id>/app.yaml` (`Workspace::resolve_app`, `list_apps`; `resolve_any` for
  either kind). `webapp::window_config` turns an `AppConfig` into the `WorkspaceConfig` the shell already understands
  (pinned start tab whose scope is every tab's scope, `allow_new` = `multiple_tabs`, modal popups, controls by `display`), and
  `Workspace::config_file()` names the file (`app.yaml`); `config::workspace_layer` picks the app identity keys by that
  file name. App and workspace ids share one namespace (`Workspace::id_taken`) because tabs, session cookies and the window
  registry are keyed by the id alone. Isolated storage is `cef/apps/<id>`. A link routed to an app (`Action::App`) is
  delivered by `open_app` in `app/mod.rs` (`multiple_tabs`: tab; `multiple_windows`: window; else navigate the window) and the
  router skips an `app:` rule for an address outside the app's `webapp::route_patterns` or from the app itself (`app_refusal`).
  App windows are left out of `browser_windows()` (plain launches and other applications' links never land in one) and
  of what a plain start remembers. An app window's size and maximized state are in the `app_windows` table
  (`remember_app_window`; GTK's `default-width/height` stay at the unmaximized size). The `.desktop` entry has a
  "New Window" action when `multiple_windows` (`--app=ID --new-window`, the `new-app-window` action), and
  `webapp::refresh_launchers` rewrites out-of-date entries at profile start.
  Its window sets its own Wayland application id (`be.geee.browsah.<profile>.<id>`, the `.desktop` file's name) when
  mapped, through `gdk4-wayland` (`set_application_id` must run at `map`; GTK overwrites it earlier). Apps are left
  out of the *Send to workspace* lists. `install.rs` reads the manifest with `Page.getAppManifest` (`parse_manifest`
  is pure and tested; the answer's `manifest` object has resolved URLs, `data` the raw text with icon purposes),
  asks the user to confirm the name, downloads the icon with `download_image` and writes the same files as
  `browsah app add`. The self-test profile never writes launcher entries (`webapp::install` returns early for it).
- **Chrome extensions** (`src/extensions.rs`, `src/crx.rs`, `src/app/extensions.rs`; docs/config/extensions.md, docs/research/chrome-extensions.md): CEF 156
  has no extension API, so `main` builds one `--load-extension=a,b` switch (merged with a user's own: Chromium keeps only the last) from the profile's
  `extensions/<id>/<version>/` folders and `extensions.paths`; this runs in the browser process only (child processes exit earlier in `main`). An unpacked
  extension's id is its manifest `key` (else a hash of its folder path, which matches Chromium's); Web Store packages carry the key in the CRX header, so
  `install_package` writes it into the manifest, otherwise 1Password's native host rejects the id. Native host manifests are copied (never edited) into
  `<profile>/cef/NativeMessagingHosts/`. `chrome.tabs` is empty (no tab bridge). `extensions.paths`, `.disable` and `.native_hosts` are process-only (dotted entries in `PROCESS_ONLY`/`WORKSPACE_IGNORED`/`RESTART_REQUIRED`); `extensions.hide` (workspace windows: all but these) and `.show` (app windows: only these) are per-window and live (`Shell::refresh_extension_buttons`, `extensions::shown_in_window`), and only change what is *shown*: a `storage: isolated` window has no extensions at all, every other window gets every loaded extension's content scripts. The popup is a `BrowserView` in a popover: a page's `Picture` is as big as its
  frame, so it needs a fixed-size `ScrolledWindow` holder with `Automatic` policies (`Never` ignores the minimum size and the popover collapses). The router is
  skipped for `chrome-extension://` in `on_before_browse`. The `extensions` self-test section loads `bench/extensions/test-ext` only when it is named.
- **CEF API gotchas:** `Image::as_png` returns nothing unless the pixel width/height out parameters are passed;
  `BinaryValue::data` needs a pre-sized buffer; setting the screen-capture content setting crashes Chromium;
  a cookie's `domain` field is read with `.to_string()`.
