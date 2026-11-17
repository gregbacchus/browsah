# Design: profiles, workspaces, web apps, routing

**Date:** 2026-11-17 · **Status:** design decisions and open questions, agreed in conversation with
the author. Almost nothing here is built or measured; each "to verify" item is a spike to run
before committing to it. CEF/GNOME facts marked *(from knowledge)* are not tested on this machine.

## Model

```
Application (browsah)  ->  Profile  ->  Workspace | Web app
```

- **Application:** high-level usability settings, per-process settings, nothing that varies per site.
- **Profile:** the root of data (cookies, storage, cache, history, state). Examples: Home, Work.
  Each profile has its own GNOME app id, its own coloured icon (colours accept a light and a dark
  value everywhere), and its own directory containing config, data and cache.
- **Workspace:** a single window with several tabs and possibly tab groups. Several workspace windows
  share the profile's app id and appear bundled under the profile's icon. Has an optional bookmark bar
  (bookmarks are per workspace). Naming: "workspace" also means a GNOME virtual desktop; accepted
  (Edge uses the same word).
- **Web app (PWA):** belongs to a profile, shows in GNOME as its own app id, takes defaults from its
  web manifest. No bookmark bar.
- **Ephemeral window:** a window with no workspace, discarded when closed but visible and
  recoverable from history.

Workspaces and web apps each have their own config directory and file, manageable by CLI:
`browsah profile add work --name "Work" --color "#4a2f80 #7a56c2"`, `profile list`, `profile remove work`,
and similar for workspaces and web apps.

## Architecture consequences

1. **One process per profile.** Chromium locks its cache directory (observed: a second launch prints
   "Opening in existing browser session" and CEF fails to initialise). All windows of a profile
   therefore live in one process, which is a unique `GApplication` named after the profile's app id.
   Opening a URL or a web app activates that instance over D-Bus, starting it if needed.
2. **Per-window app id (to verify).** Separate GNOME identities for web apps inside one process need
   each window to carry its own Wayland `app_id` (*from knowledge:* `gdk4-wayland` exposes a per-toplevel
   setter), plus a generated `.desktop` file per web app and per profile. Check that GNOME Shell groups
   and shows them as intended.
3. **Default browser needs no router of our own.** Each profile's `.desktop` declares the
   `http`/`https` handlers; GNOME's default-browser setting picks one of them, and external links go
   to that profile. (Only one entry can be the default at a time, so the global config does not need
   a default-profile setting.) A profile can still forward a URL to another profile as a routing
   action over D-Bus.
4. **Where we cannot know the source app (see below)** routing for external links cannot depend on
   which application sent the link.

## Config

- Four schemas, one per level: browsah, profile, workspace, web app. An option declares the layers it
  may appear at. Cascade where it makes sense; much will not (see "process-wide" below).
- **Merge rules:** scalars override; maps merge by key; lists either replace or append (explicit
  syntax); the web manifest is a layer beneath the web app's own config
  (application, profile, manifest, web app overrides).
- **Process-wide options cannot cascade:** `chromium.flags`, `gpu.*`, and probably
  `rendering.mode` and `device_resolution` are per process (so per profile at most).
- Colours accept a single value or a light/dark pair.
- Permissions (allow / deny / ask) and the download directory are config options that cascade.
- Hand-edited **config** is separate from app-written **state** (history, bookmarks, tab state,
  screenshots): state lives in SQLite or similar, not in YAML. CLI edits must not rewrite
  hand-edited files (generated files or drop-ins).
- The docs and example test in `AGENTS.md` must grow to cover each schema (per-layer option lists).

## Routing

- Applies to **main-frame navigations**, with these inputs where CEF provides them: source tab or
  workspace, link click versus script `window.open` versus form versus typed versus redirect, link
  target name, user gesture. Hooks *(from knowledge)*: `on_before_browse`, `on_open_url_from_tab`,
  `on_before_popup`.
- Actions: open in workspace or web app (new tab, reuse a matching tab), open as modal, current tab,
  another profile, system handler, block, ask.
- **Redirect and login flows are excluded from routing by default** (configurable).
- **Popups are not re-routed.** Per workspace or web app a popup policy chooses: allowed as a modal,
  as a tab, or not at all. This keeps `window.opener` flows (OAuth, payment) intact, which needs
  the popup to stay attached to its opener; today popups are cancelled and opened as detached tabs,
  which loses the opener (known limitation).
- **Modals:** default to links that should be viewed and closed (email, chat links), chosen by
  allow or deny matching. Read-only URL bar and a button to move the page to a workspace via the
  router. Preferred presentation: an in-window overlay; a separate transient window as a fallback
  or option.
- **Rule test:** `browsah route test <url> [--profile P] [--from WORKSPACE] [--target _blank]`.
  `--profile` omitted tests external routing. The router is a pure function, so it can be table-tested.
- **Locking:** workspaces can fix the URL bar, forbid new tabs (tabs only arrive by "send to"), and
  define pinned tabs restricted to domains or paths, with out-of-bounds navigation opening a new tab.
  This is a convenience against tab bloat, not a security feature. Needs allowances for identity-
  provider redirect chains.

## Web apps and the manifest

Details from the web manifest, read through the DevTools `Page.getAppManifest` call
*(to verify in our windowless browser)*, with icons fetched through the profile:

| Manifest | Use |
|---|---|
| `id` | stable identity and app id |
| `name`, `short_name`, `description`, `categories` | `.desktop` name, comment, categories; title |
| `icons` | GNOME icon (prefer `purpose: any`; GNOME does not mask) |
| `theme_color`, `background_color` | header tint (single colour: derive the dark variant or override) |
| `start_url`, `scope` | launch URL and the navigation boundary (in-scope stays, out-of-scope is routed) |
| `display`, `display_override` | default navigation controls |
| `shortcuts` | `.desktop` Actions |
| `launch_handler` | focus existing window or navigate it |
| `protocol_handlers` | `x-scheme-handler/*` (opt-in only) |

- Add flow: the CLI asks the running profile process to load the URL in a hidden tab and read the
  manifest; plain HTTP fetch is a fallback. Snapshot the manifest and icons into the web app's
  directory so the `.desktop` entry and icon work offline.
- The manifest is untrusted site input: pin name, `id`, scope and handlers at add time, show a diff
  and ask on change; icons and colours may refresh automatically.
- Open question: should a web app icon carry a small profile badge so Work and Home copies of the same
  app differ?

## History, tabs, sweeping

- History per profile, aware of the workspace and web app (SQLite).
- **Tab state is restored** when a workspace opens. Keep a screenshot per tab (we already hold the
  last frame as a texture) to show, blurred, until the tab is activated or when offline.
- **Sweeping** is configured per workspace by rules: match (domain, URL, title), condition (idle longer
  than X, a decaying access score below Y, count per domain), action (close now; later discard,
  archive, other). The score is `sum(exp(-(now - t_i) / tau))` over access events.
  - **Never sweep** a tab with unsaved form input (and show an indicator on the tab), playing media,
    camera or microphone in use, or a call.
  - **Record why** every tab closed: `user`, `sweep:<rule>`, `session`, `crash`. Swept tabs stay
    recoverable.
  - To verify: detecting dirty form state (likely a script in each page's context).

## Findings so far

### Codecs: measured

Run on 2026-11-17 on this machine (CEF 156.5.0 from the `cef` crate download) with
`bench/pages/codecs.html`, which asks the page what it can play (no media was played):

| Codec | `canPlayType` / support |
|---|---|
| H.264 (`avc1.42E01E`) | **no** (also not via Media Source Extensions or `mediaCapabilities`) |
| AAC (`mp4a.40.2`) | **no** |
| MP3, VP8, VP9, AV1, Opus | probably (supported) |
| WebRTC video senders | VP8, VP9, AV1 (no H.264) |

So the stock CEF build has no proprietary codecs. Many sites and some call or meeting services use
H.264 or AAC. **Can it be installed?** Not as a separate package, as far as I know: the codecs are
compiled into the CEF library's ffmpeg, so it takes a CEF build made with proprietary codecs enabled
(a full Chromium build, hours and a very large checkout), which also raises a patent-licensing
question if binaries are shared. **To verify:** whether a prebuilt CEF with codecs exists and what its
licence terms are; whether the target services (Teams in particular) work with VP8/VP9/AV1 only.
Widevine (DRM) is a separate question, not measured.

### Source application of an external link

There is no portable "which app" field in the request (`platform_data` was empty), but the sender
process can be identified: read its pid from the bus while the call is in flight, then use the
`app-<desktop id>-<n>.scope` cgroup of the first ancestor that has one (spike 5, below). Sandboxed
applications arrive through the portal and cannot be identified this way. Making the source known by
design remains the reliable option: a link clicked inside a browsah web app is known, and an
application with a custom browser command can launch `browsah open --source <name>`.

## Risks to look at early

- **Password manager (1Password):** a separate investigation. CEF's Alloy runtime lacks Chrome
  extension support, and the Chrome-style runtime did not paint windowless in our spike
  ([spike](cef-osr-wayland-spike.md)). Native messaging, system integration, or an alternative flow
  are all unknown.
- **WebAuthn and passkeys** (needed): security keys over USB may work; platform passkeys and a
  password-manager passkey flow are unknown in CEF. Spike.
- **Background workers, service workers, push, notifications** (needed): the process must keep running
  with all windows closed. Chromium's push relies on Google's messaging service which CEF does not
  provide *(from knowledge)*; page-opened notifications are different. Spike.
- **Cookie and storage rules:** cookies can be allowed or denied per request with a CEF cookie access
  filter *(from knowledge)*. Other storage (localStorage, IndexedDB, cache, service workers) cannot be
  filtered that way; per workspace the choice is to share the profile store or use an isolated
  sub-store (its own request context and cache path under the profile).

## Spike results (2026-11-17)

Environment for all: this machine (AMD Radeon 890M, Mesa 26, GNOME/Mutter on native Wayland, GTK 4.22,
CEF 156.5.0), single runs, local test pages and local HTTP servers. "Verified" means observed here.

| # | Spike | Result |
|---|---|---|
| 1 | Per-window Wayland app id | **Works at the protocol level.** `gdk4-wayland`'s `WaylandToplevel::set_application_id` sent a second `xdg_toplevel.set_app_id` with the web app's id (seen with `WAYLAND_DEBUG=client`) when called from the window's `map` handler. Calling it earlier (before present or on realize) is overwritten by GTK. **Not verified:** that GNOME Shell regroups the window under the matching `.desktop` entry and icon (its window introspection is denied to us, so this needs a visual check); the id is briefly the profile's before it changes. The spike was a throwaway crate, not kept in this repo. |
| 2 | Manifest and icons | **Works.** `Page.getAppManifest` through `execute_dev_tools_method` answered in about 20 ms with the raw manifest text and a parsed form with resolved URLs (`id`, `startUrl`, `scope`, colours as `rgba(...)`, `display` as `kStandalone`, `displayOverrides`, `launchHandler`, `protocolHandlers`, shortcut name and URL). **The parsed form omits some fields** (icon `purpose`, shortcut `description`), so also read the raw text. Icons: `download_image` returned decoded images for both PNG and SVG in about 20 ms, and a 404 for a missing one. A 192 px PNG reported 512x512 (rescaled to the requested maximum): ask for the size wanted and check. Code: `BrowserView::cdp` and `download_image`; test: `BROWSAH_MANIFEST_TEST` with `bench/pwa/` served over HTTP. |
| 3 | Popup that keeps its opener | **Works, and is now the behaviour.** `on_before_popup` creates the popup as another windowless browser (client, window info and settings filled in) and the shell adopts it as a tab. A `postMessage` from the child to `window.opener` reached the parent (`--selftest`). Consequence: a popup shares its opener's renderer process, so a crash takes both down (measured, now a self-test check). Without a user gesture popups are still blocked. The popup policy (modal, tab, denied) is not built. |
| 4 | Cookie filter | **Works for save and send.** A `CookieAccessFilter` (via the request handler's resource request handler) with a deny list: without it, a `Set-Cookie` and then a second request sent `spike=1`; with the host denied the cookie was neither saved nor sent; denying only the second URL saved it but did not send it. Gotcha: a `ResourceRequestHandler` that does not override `on_before_resource_load` **cancels every request** (the default return value is zero), so override it. **Not tested:** a second request context with its own cache path (an isolated sub-store), the memory cost of several contexts, matching by cookie domain instead of URL substring. |
| 5 | Source application of a link | **Partly possible; measured with a throwaway handler** (a D-Bus-activatable `.desktop` for a private URL scheme, removed afterwards; no default-browser setting was touched). Calls arrive as `org.freedesktop.Application.Open(uris, platform_data)`. From `gio open` and `xdg-open` the `platform_data` was empty and the sender is the helper process (`gio`), not the application. But a handler can ask the bus for the sender's process id (`GetConnectionUnixProcessID`) while the call is in flight and read `/proc`: the parent chain showed `gio <- xdg-open <- bash (argv0 set to a fake app name) <- ...`, so the originating process is in the ancestry. Better still, `/proc/<pid>/cgroup` of GNOME-launched apps names the desktop file id (for example `app-com.anthropic.Claude-13721.scope` gives `com.anthropic.Claude`), which is a robust identity when the app was started from its `.desktop` entry. **Limits:** apps in a sandbox (Flatpak, Snap) go through the OpenURI portal and the sender seen is the portal (confirmed: the call came from the portal's connection, not the caller); the ancestry can include shells and terminals, so the walk needs a rule (first ancestor whose cgroup is an `app-*.scope`); apps started outside the desktop (a terminal) show a terminal's scope. Not tested with real chat applications. |
| 6 | Service workers, notifications, WebAuthn | `serviceWorker.register` reached `activated`. The `Notification` and `PushManager` APIs exist but `Notification.requestPermission()` and `pushManager.subscribe()` did not resolve in 40 s: **there is no permission prompt** until we implement CEF's permission handler (needed anyway for the allow/deny/ask config). WebAuthn: the API exists, there is no platform authenticator, conditional mediation reports available, and `credentials.create` did not resolve (no prompt UI, no security key attached): **not working yet**; it needs the same prompt work plus a security-key test. Geolocation timed out (no provider); `enumerateDevices` listed 5 devices. Server push through Google's messaging service was not tested end to end and is expected not to work. Test page: `bench/pwa/caps.html`. |
| 7 | Codecs | Measured above: no H.264 or AAC. Prebuilt-with-codecs options and Teams behaviour not checked. |

Reproduce 2 and 6 (serve `bench/pwa/` on port 8765 with any static server first):

```bash
export LD_LIBRARY_PATH=$PWD/target/release
BROWSAH_MANIFEST_TEST=http://localhost:8765/index.html ./target/release/browsah --selftest | grep -A3 spike
BROWSAH_NET_WAIT_MS=40000 BROWSAH_NET_TEST=http://localhost:8765/caps.html ./target/release/browsah --selftest | grep "net test"
```

## Spikes, in order (results above)

1. Per-window Wayland app id and GNOME grouping, with per-profile and per-web-app `.desktop` files.
2. Manifest read (`Page.getAppManifest`) and icon download in our windowless browser.
3. Popup that keeps its opener (needs the popup created attached, not cancelled).
4. Cookie access filter and a second request context.
5. D-Bus activation: what does a link opened from other applications reveal?
6. Service workers, notifications, WebAuthn under CEF.
7. Codec options (prebuilt CEF with codecs, licensing) and Teams with the available codecs.

## Status of the build order

Step 1 below is built (see `src/profile.rs`). Decisions taken while building it, not discussed
before: the profile directory is `$XDG_DATA_HOME/browsah/profiles/<id>/` holding `profile.yaml` and
`cef/` (Chromium's data); profile ids are lower case letters, digits, `-` and `_`, starting with a
letter, so `be.geee.browsah.<id>` is always a valid application id; `default` and `selftest`
exist implicitly; `profile remove` refuses a running profile and needs `--yes` because it deletes
data (`--keep-data` keeps it); the `.desktop` entry launches through `env LD_LIBRARY_PATH=<dir>`
because the binary has no rpath yet, and records the current binary path, so a moved or rebuilt
binary needs `profile add` repeated (or the file edited). Measured here: two profiles ran at the
same time without CEF errors, and a second launch of a running profile handed over its link and
exited in 34 ms. **Not verified:** the new link opening as a visible tab (the running process
logged receiving it), GNOME listing the entries in its default-browser chooser, making an entry the
default browser, the icon's look in the shell.

Step 2 (layers, first part) is built: application file, then the profile's `profile.yaml`, merged
by the rules in `docs/config/README.md` (maps merge, values and lists replace, `key+:` appends,
application-only options ignored in a profile). Choices made while building it: a profile may set any
non-application-only option, including process-wide ones, because a profile is its own process
(only workspaces and web apps, which share a process, will be limited to per-window options); the
schema per level is the same option set filtered by the application-only list, with each level's
own identity keys (`name`, `color` for a profile) kept out of the merge. Files are watched with GIO
file monitors (`watch_config`, `--no-watch`), changes are coalesced for 0.3 s, and options are
classified as live (`search_url`, `home_page`, `rendering.frame_rate`, `input.*`) or restart-only
(`chromium`, `gpu`, `rendering.mode`, `device_resolution`, `window`). Measured here by editing a
profile file while the browser ran: a live edit, a restart-only edit, an invalid edit (kept the old
settings), an editor-style rename over the file and a no-change save were all handled as intended; the
restart message stays correct when a restart-only option is edited back; `--no-watch` gave no
reload. **Not verified:** the in-window message (only the debug log was read), live frame-rate change
on an existing tab, a symlinked config file, a config directory that does not exist yet.
Not built: workspace and web app layers, list items with provenance beyond option paths, a config
reference for what is application-only beyond `watch_config`.

## Status: workspaces (2026-11-17)

Built as described in the model section, in six stages (window registry; workspace model and CLI;
config layer; SQLite and restore; controls, colour, locking and pinned tabs; bookmarks). Decisions made
while building, not discussed before:

- **Windows.** One `Shell` per window in a registry; a window with no workspace is ephemeral. A plain
  start reopens the workspaces that were open at the last exit (closing windows one at a time forgets
  them, except the last), else `default_workspace`, else an ephemeral window. `--workspace=` against a
  running profile goes through an application action (`open-workspace`) or the `open` hint for links.
- **Tab dragging between windows** works (the tab's owner is looked up per event and updated from the
  tab view's attach and detach signals); verified with `transfer_page`, not with a real drag.
- **Config.** `nav` and `tabs` are workspace-only options (not cascading); every other option can be
  overridden per workspace except the process-wide ones, which are ignored with a warning. Scrolling
  options apply while a window is the active one (they are process-wide statics).
- **Storage.** `data.sqlite` in the profile directory (rusqlite with bundled SQLite, MIT; all new
  dependencies are permissive). Tabs are replaced as a whole per workspace; bookmarks are keyed by
  workspace and address.
- **Restore.** A remembered tab is a view without a browser (a *dormant* tab) showing its screenshot
  behind a CSS blur; the selected one wakes at once. Screenshots must be copied out of the GPU buffer
  when the tab is left: a later PNG save failed intermittently, and a page left before its first frame
  has no screenshot (measured: a static page's first frame arrived about 300 ms after its title).
- **Locking.** `tabs.allow_new: false` disables new tabs; a link out of a pinned tab's scope, or a popup,
  goes to an ephemeral window. Scope is checked on CEF's thread in `on_before_browse`; it exempts
  redirects and pages already outside the scope so single sign-on works. Not a security boundary.

Measured here (this machine, scripted, 122 self-test checks, two or three runs each of the new ones):
opening and closing workspace windows, a second window, tab transfer, restore with sleeping tabs and a
wake on first show, pinned tab behaviour including a redirect out of scope, bookmarks across a reopen,
and live reload of workspace files. **Not verified:** a real drag of a tab between windows; the look of
the blurred placeholder (the CSS parses, but the window screenshot of a restored window failed);
colour contrast beyond the light/dark pair in one screenshot (the header tint was seen in dark mode
only); behaviour with many tabs or large pages (screenshot size, restore time); the database with two
profile processes (each profile has its own file); upgrade of the database schema on a real older file;
`Ctrl+D` and the bookmark bar with a real keyboard and mouse (the checks activate actions and emit
signals). Pinned tabs are not restored (they load fresh); the screenshot of a tab that was never left
until the window closed is taken at close.

## Status: routing (2026-11-17)

Built: `routing:` rules in the application, profile and workspace files (workspace rules judge links from
their own tabs, then the profile's, then the application's; first match wins), the actions `allow`,
`block`, `ephemeral`, `system`, `workspace:<id>`, `profile:<id>[/workspace:<id>]`, `routing.external`
as the default for links from other applications, and `browsah route test` (`--profile` omitted: only
the application's rules, which is how an external link is first judged; `--explain` shows every rule).
A tab's right-click menu can send it to another workspace or a new window (the live page moves).

Decisions made while building, not discussed before:
- **What is routed.** Same-tab link clicks (a user gesture on a GET), Ctrl/middle-click links,
  `target=_blank` and plain `window.open(url)` (all asking for a tab), typed navigations only if a rule
  lists `typed`, and links from other applications. Never routed: redirects, form posts, reloads,
  frames, a tab's first load, and popups opened with window features, which keep their opener.
- **Why those popups are exempt.** Chromium reports `window.open(url)` and `target=_blank` the same
  way (a tab), so a rule that names a sign-in domain *will* reroute a plain `window.open`; a sized popup
  is the sign-in/payment kind and is left alone. No popup policy (modal, tab, deny) exists yet.
- **Source of an external link.** Not used: rules can match `from: external` but not which application
  sent it (see spike 5).
- **Cross-profile.** `profile:<id>` starts a new `browsah --profile=<id> [--workspace=<id>] URL`, which
  hands the link to that profile's process (or starts it).
- The router lives in a process-wide `RwLock<Arc<Router>>` so CEF's thread can call it; it is
  reinstalled when a configuration file changes.

Measured: 11 unit tests of matching; the self-test routes a clicked link, a new-tab link, an external
link by an application rule, a blocked link, and a command-line workspace (skipping the router), against a
local server using two host names. **Not verified:** a Ctrl-click or middle-click (the self-test uses a
`target=_blank` link); `system` and `profile:` actions (not exercised); rules from a real application's
links; many rules (no performance test; the router runs on every navigation); the toast messages.
Regular expressions and a "which application" selector are not offered.

## Status: sign-ins started by other applications (2026-11-17)

Requested: command-line tools and desktop applications (Claude, ChatGPT) open the browser to sign in;
this should open an ephemeral window in the receiving profile. Built as `routing.app_sign_in` (default
`ephemeral`) with a heuristic on the address alone (`router::looks_like_app_sign_in`: an OAuth address whose
`redirect_uri` points at loopback or an application scheme and which has a `client_id`, `response_type` or
`code_challenge`; or a device sign-in with a `user_code`), a rule selector `match.app_sign_in`, and
`route test` support. Measured: unit tests over positive and negative addresses; the self-test sends a
loopback-style OAuth address through the `open` path and an ephemeral window opens on it.

Application-scheme callbacks: **measured with a throwaway scheme handler** (registered with a
`.desktop` file, removed afterwards): a 302 to `browsah-cb://done?code=1` showed an error page and nothing
reached the handler by default. Chromium calls the embedder's `on_protocol_execution` only for schemes
that have an OS handler, and letting it launch the handler itself did nothing (it needs a prompt that
a windowless browser does not have). What works: the handler cancels the navigation and the shell hands
the address to the desktop with GIO; this is allowed only for the scheme in the sign-in's own
`redirect_uri`, only in the window the sign-in opened, and then the window closes after 1.5 s. Verified by
hand with the throwaway handler (the log showed the address); **not** in the self-test (it would need a
registered scheme handler), and **not** with a real application. Risks: the heuristic can miss a flow
(a `redirect_uri` that is not in the query, a non-standard parameter name) or catch an ordinary link;
an application that checks the browser's user agent is unaffected; a sign-in that needs the same
browser session as a normal profile window (a password manager, existing cookies) is opened in an
ephemeral window of the same profile, which shares the profile's cookies (the profile is the storage
unit), so existing sign-ins are available there.

## Status: profile selector (2026-11-17)

Requested: a `.desktop` that is a profile chooser usable as the default web browser. Built as
`browsah select [URL]` (a libadwaita list of the profiles; `1`-`9`, click or `Enter` chooses, `Esc`
cancels; running profiles are marked) and `browsah selector install|uninstall|status` for the
`be.geee.browsah.desktop` entry (the application's id without a profile). It starts no CEF, so it opens
at once. It skips the question when there is one profile, when `default_profile` is set in
`config.yaml`, or when an application routing rule names a profile (`profile:<id>[/workspace:<id>]`)
or blocks the link. Setting the system default browser is left to the person (the command only
prints how). Profile entries still declare the web handlers, so either style of default works.

Measured here: the chooser built and reported the activated row (`browsah select --check`, run by hand,
not part of `--selftest`), the install, status and uninstall commands wrote, read and removed the entry
(the entry validated and the default browser was left untouched), and `select` with two profiles stayed
open for the 4 s of a timed run. **Not verified:** how the window looks, a real pick that starts a
profile (not run, to avoid starting a real profile), being chosen as the default browser through the
desktop's settings, or links arriving from real applications through it. Not built: remembering a
choice per site (an application routing rule does that, written by hand), and offering the selector's
list as desktop actions.

## Status: modals and popup policy (2026-11-17)

Built: the routing action `modal`; a modal is a browser view in an `adw::Dialog` over the window (or an
`adw::Window` above it with `modals.style: window`), with a read-only address bar, a *Move to workspace*
button and a menu of workspaces; `tabs.popups: tab | modal | block` per workspace for popups a page's
script opens with window features (the kind that keeps its opener).

Decisions made while building, not discussed before:
- **Move.** The page keeps running: its view leaves the dialog and becomes a tab (the view's event handler is
  replaced). *Move to workspace* asks the router where the link would go with modal rules skipped
  (`route_ignoring_modal`); if no rule applies it becomes a tab of the window the modal was over. Actions that
  hand an address to another profile or the desktop close the modal and send the address (the running page is
  not carried over).
- **Links in a modal** are routed like links from its window (a rule can open them as another modal or
  elsewhere); a popup of a modal becomes a tab of the window under it.
- **Popup policy applies after routing**, so a plain `window.open(url)` or `target=_blank` link (a tab request)
  is routed by rules first, and `tabs.popups: block` blocks only sized popups. A modal popup keeps its opener.
- **A whitelist or blacklist of what opens as a modal** is the order of routing rules (docs/config/routing.md).
- **Options.** `modals.style` and `tabs.popups` are workspace-only; ephemeral windows use overlays and allow popups
  as tabs.

Measured (scripted, this machine, 14 new self-test checks): a clicked link whose rule says modal opens a
modal that is an overlay with a read-only address bar while the tab under it stays; *move* sends the live page to
the workspace the router names and it keeps updating its new tab; closing removes it; a script popup opens as a
modal under `popups: modal` and not at all under `block`; `modals.style: window` makes a window; a window
screenshot of an overlay modal looked right (dimmed window, header with the buttons). **Not verified:** the
workspace menu's items by clicking, `Esc` to close (libadwaita's own), narrow windows (the dialog turns into a
bottom sheet; not looked at), several modals at once, a modal opened while the window is not active, a modal's
own popups, keyboard focus handling in the modal.

## Status: history (2026-11-17)

Built: per-profile history in `data.sqlite` (schema version 3: `visits`, `closed_tabs`), workspace aware
(`None` for ephemeral windows), recorded when a page finishes loading (a reload of the same address in a tab is
not a new visit), plus tabs closed by the user (reason `user`) and the tabs of an ephemeral window closed with
tabs open (reason `window`, one group per window). `Ctrl+H` opens a history window (search; this workspace or all;
open here or in a new tab; *Recently closed* lists tabs and ephemeral windows), `Ctrl+Shift+T` reopens the last
closed tab of the workspace or, in an ephemeral window, the last closed window. Options (cascading, so per
workspace too): `history.enabled`, `history.retention_days` (purged at start-up), `history.exclude`.
`browsah history [QUERY]` and `history clear`.

Decisions made while building, not discussed before:
- **Private sign-ins.** Windows opened for another application's sign-in are never recorded (their addresses
  carry one-time codes). Everything else in ephemeral windows *is* recorded, so they are recoverable as asked.
- **Only web pages** (`http`, `https`) are recorded; `file:`, `about:` and `chrome:` are not.
- **Sweeping will reuse `closed_tabs`**: the `reason` column is where `sweep:<rule>` goes, which is how a swept
  tab is told apart from one the user closed.
- **Ctrl+D and Ctrl+H** were being swallowed by the page (the shell-shortcut list did not include them): added.

Measured (scripted, 17 new self-test checks and 4 unit tests): visits recorded with workspace and title;
reload not duplicated; excluded address and history-off workspace not recorded; sign-in window not recorded;
the history window lists, searches and opens a visit; a closed tab is kept with its reason and reopens once; a
closed ephemeral window returns with both tabs. **Not verified:** the window's appearance and large histories
(search is a `LIKE` scan; no index on text; `visits` has one on time and workspace only), retention purge in a
running browser (unit-tested at the store level), clearing history from the command line against a real
database, history across two workspaces' windows at once, titles that change after load (the title at load end is
stored). Not built: address-bar suggestions from history, per-visit deletion, import.

## Status: tab sweeping (2026-11-17)

Built as designed: rules per workspace (`sweep:` in `workspace.yaml`, off until enabled) with an address
pattern, `idle_for`, a decaying `score_below` (events weighted by recency with a per-rule `half_life`),
and the actions `close` and `discard` (extensible). A swept close is recorded in the history of closed tabs
as `sweep:<rule>`, so it is told apart from a user close and `Ctrl+Shift+T` reopens it. Never swept: the
selected tab, pinned tabs, tabs loading, tabs with unsaved form input (marked with an edit icon), tabs playing
media (marked with a speaker), and tabs whose probe did not answer.

Decisions made while building, not discussed before:
- **Use model.** Events (time, weight): being shown 1.0; the selected tab of the active window 0.2 each pass; typing,
  clicking or scrolling since the last pass 0.3. A list of the last 120 events per tab (not a running total) is
  kept so each rule can use its own half-life; it is stored with the remembered tabs, so idle time continues
  across runs, and it survives a restore.
- **Detecting unsaved input** is a script in the page, run on each pass (not continuously): any text box or
  text area whose value differs from its HTML default, a changed checkbox or choice, or non-empty rich
  text. It errs towards protection (a box filled by script looks unsaved). Measured with a page whose box was
  set by script. **Not detected:** microphone or camera use without a media element, sound from WebAudio,
  work in a canvas, text in a field inside a closed shadow root or a cross-origin frame.
- **Discard** keeps the tab with its screenshot (as a restored tab) and frees the page; closing, discarding and
  reopening are recorded only for closes (discards are not history).
- **Applying a change to the rules** needs the workspace to be reopened (the section is read when the window opens).
- **Not built:** other actions (archive, move to another workspace), per-rule protection lists, a dry run, a way
  to see a tab's score, sweeping for ephemeral windows.

Measured (scripted, 11 new self-test checks and 8 unit tests): a close rule closes an idle tab and records
`sweep:Stale`; a discard rule puts an idle tab to sleep; a tab with script-set input is left alone and marked;
the selected tab is never swept; `Ctrl+Shift+T` reopens the swept tab; use is saved with the tabs. The rule
conditions, decay arithmetic and duration parsing are unit tests. **Not verified:** the timer in real use (the
test calls the pass directly; the interval is validated), a long-running window, a swept tab being the
only tab, many tabs (each pass probes every live tab), the media probe against real video, restore of a
tab that was discarded, the look of the indicators.

Update (requested after the first version): **sweeping also happens when the workspace's window is closed.** A
running profile sweeps the remembered tabs of closed workspaces (about eight seconds after start-up, then each
workspace on its own interval, skipping workspaces whose window is open), and `browsah sweep [--profile]
[--workspace] [--dry-run]` does the same for a profile that is not running (for cron or a systemd timer; it
refuses while the profile runs, `--dry-run` is always allowed). The rules and use records are the same; a
closed workspace has no selected tab, no running page (so no unsaved-input check) and nothing to discard.
Measured (self-test, 7 checks, plus a unit test): a dry run changes nothing; the pass removes the idle tab that
matches a rule and leaves fresh, pinned (these come from the configuration) and other-site ones; it is recorded
as `sweep:<rule>`; a second pass does nothing; the workspace opens with the rest; an open workspace is skipped.
**Not verified:** the eight-second start-up pass and the one-minute timer in a real run, the command against a
real profile with stored tabs (only the empty case was run), a systemd timer. Limitation: a profile that is not
running is only swept if something runs `browsah sweep`; the browser itself exits when its last window closes.

## Status: permissions and downloads (2026-11-17)

Built: `permissions:` (per kind: camera, microphone, location, notifications, clipboard, screen, midi, other; per site
rules; `allow`/`deny`/`ask`; default `ask`) and `downloads:` (`directory`, `ask`), both ordinary cascading options, so
they can differ per profile and per workspace. Chromium's permission handler asks us for camera, microphone and screen
(a media request) and for everything else (a prompt). A request is judged by the view's copy of the configuration on
CEF's thread; only `ask` goes to the shell, which looks for a remembered answer (profile database, per site and kind)
and otherwise shows an `AlertDialog` (Block / Allow, "Remember for this site", Esc = block once), queued one at a time
per window and withdrawn if the page moves on. `browsah permissions list|reset`. Downloads: a handler that saves into the
folder under a sanitised, never-overwriting name, optionally asking with a Save-as dialog, with toasts (and *Show in
folder*) when a download starts and finishes.

**Measured and found while building** (this machine):
- The spike's finding that permission requests hung is fixed: with a handler, `Notification.requestPermission()` and
  geolocation resolve as configured (allow -> granted, deny -> a permission-denied error, a site rule beats a kind).
- **Chromium persists the answer it is given to a prompt and then answers that origin itself**, never calling the
  handler again (a second `requestPermission()` reached the handler 0 times). That would have made config changes,
  *forget* and "allow but do not remember" ineffective. Clearing the stored setting at once after answering made
  Chromium refuse a request it was still working on (geolocation returned denied after a remembered allow). What
  works: clear the stored settings for the page's origin when a main-frame load starts (a permission lasts as long as the
  page it was given to), verified by a repeat request after a reload reaching the handler again.
- Calling `set_content_setting` for the screen-capture type **crashes Chromium** (SIGTRAP on the first https page), found by
  bisecting the kinds; screen is left out (it goes through the media path).
- Chromium's origins have a trailing slash; they are normalised before use.
- Downloads: nothing is saved without a download handler; with one, files arrive (hello.txt-style test file saved, a repeat
  became `note (1).txt`, a chosen path was used, cancelling the Save-as question saved nothing).

Measured (scripted, 17 new self-test checks and 12 unit tests): the cases above. **Not verified:** the real dialogs
(the test holds the question), the Save-as dialog, camera, microphone and screen requests with real devices (a media
request path exists but is only unit tested; the test avoids real devices), what Chromium does for clipboard or MIDI
requests, notifications actually appearing (not built), geolocation actually finding a position, the toasts' look,
`permissions reset` against a real profile, behaviour across restarts (Chromium's cache keeps what is not cleared: an
answer given on a page that is never reloaded stays until the next load of the origin).

## Status: cookie rules and isolated storage (2026-11-17)

**Cookie rules** (`cookies:` in any layer, `src/cookies.rs`): `default` plus ordered `rules` of `match` patterns
(same patterns as a pinned tab's scope) and `allow`/`deny`. Enforced by a CEF `CookieAccessFilter` returned from
the per-request `ResourceRequestHandler`, so it runs on CEF's network thread against the view's own copy of the
configuration; a denied site's cookies are neither sent nor saved. The filter is skipped when the rules are open.
Measured (`--selftest`, 2 checks, local test server): a site a rule denies saves nothing and sends nothing, a
site not denied keeps its cookie. Not measured: third-party cookies set from an allowed page for a denied
host (the filter judges the request's URL, so the denied host's own requests are filtered; not tested), and
JavaScript-set `document.cookie` for a denied site (the filter sees network cookies only; not tested).

**Isolated storage** (`storage: isolated` in `workspace.yaml`): the workspace's views are created with their own
CEF `RequestContext` whose `cache_path` is `<profile>/cef/workspaces/<id>` (must lie under CEF's root cache
folder), made on first use and cached per folder. Measured (`--selftest`, 4 checks): shared workspaces see each
other's cookie, an isolated one does not see the profile's, keeps its own, and two isolated ones do not see
each other's. Applies to tabs created after the setting changes. **Session cookies** (`action: session`): the filter allows them; when a window closes, `end_session_cookies`
visits the window's cookie store (the profile's, if no other window shares it; an isolated workspace's own)
and deletes cookies whose domain a `session` rule matches, then flushes the store. CEF cannot rewrite a cookie's
expiry in the filter, hence delete-on-close. Measured (`--selftest`, 3 checks, isolated workspaces): the cookie
works while open, is gone after close and reopen; an `allow` one stays. Not measured: the profile's shared store
(same code, `None` storage), quitting the whole browser (the delete runs on CEF's thread as the window closes and
may race shutdown), and cookies a crash leaves (removed at the next close, not at start-up).

Not measured: persistence of an isolated
workspace's cookies across a restart, and the disk space of many contexts. Caveat: permission answers Chromium
remembers and the per-site content settings are per context, and browsah clears them per load anyway.

## Build order after the spikes

1. Profile directories plus a unique per-profile instance with D-Bus URL handoff (the cache path and
   app id are expensive to change later).
2. The layered config framework and per-level schemas.
3. The router and `route test`.
4. Workspaces (tab restore, bookmarks, locking), then web apps and manifest, then ephemeral windows.
5. Sweeping, history and recovery.
6. Permissions and download directories as cascading config.

## Reproduce the codec measurement

```bash
cargo build --release
export LD_LIBRARY_PATH=$PWD/target/release
BROWSAH_NET_TEST=file://$PWD/bench/pages/codecs.html ./target/release/browsah --selftest 2>&1 | grep "net test"
```
