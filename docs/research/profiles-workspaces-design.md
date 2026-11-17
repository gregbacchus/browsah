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
works while open, is gone after close and reopen; an `allow` one stays. **Window binding:** cookies saved under a `session` rule are recorded per owner (the workspace id, or a number for an ephemeral window) with store, domain, name and path; the filter sends a session cookie only to the owner that saved it, so another workspace sharing the store, or a leftover from an earlier run, never gets it. Closing a window deletes exactly its cookies; opening one deletes session cookies no window owns. Measured (`--selftest`, 4 checks): a cookie saved under `allow` earlier is not sent once the site is `session` (this cannot tell the filter from the purge at open apart); a second workspace in the same shared store is not sent the first one's cookie; closing the first deletes it. **Blocked-cookie discovery:** the filter records the host of each request whose cookies a deny blocked, per view, cleared when the next main-frame navigation starts (clearing at load start was too late: Chromium saves response cookies before it). The tab menu copies `allow` rules for them. Measured: the list and its reset (2 checks). Not measured: the clipboard copy itself and the menu in real use. Not measured: the profile's shared store
(same code, `None` storage), quitting the whole browser (the delete runs on CEF's thread as the window closes and
may race shutdown), and cookies a crash leaves (never sent; deleted at the next window open).

Not measured: persistence of an isolated
workspace's cookies across a restart, and the disk space of many contexts. Caveat: permission answers Chromium
remembers and the per-site content settings are per context, and browsah clears them per load anyway.

## Status: web apps (2026-11-17)

**Built as a kind of workspace**, not a fourth schema: a `workspace.yaml` with an `app:` section (`url`, `scope`,
`display`, `icon`, `id`) is a web app. `apply_app` fills in the pinned start tab limited to the scope, no new
tabs, modal popups and the controls for `display`; the app lives in `workspaces/<id>/`, so storage, cookies,
permissions, routing and tab restore all work unchanged. `browsah app add|list|remove|install` creates one and writes
`be.geee.browsah.<profile>.<id>.desktop` and an icon (given, downloaded from the manifest, or generated). The
window calls `WaylandToplevel::set_application_id` when mapped. *Install as web app…* in the tab menu reads
`Page.getAppManifest`, asks for the name, downloads the icon and does the same as `app add`.

Measured (`--selftest`, 30 checks in sections `webapp` and `install`): the app opens on its start page with one
pinned tab, no address bar, no new tabs, modal popups; a link off the app's site opens in its own window; the
manifest is read, the icon arrives as PNG (needs `as_png`'s size outputs), the saved configuration matches and the
installed app opens. With `WAYLAND_DEBUG=client` the window sent `set_app_id("be.geee.browsah.work")` and then
`set_app_id("be.geee.browsah.work.mail")` (a temporary data directory, the real binary).
**Not verified:** that GNOME Shell groups the window under the app's own launcher entry and icon (needs a look at
the dash), the confirmation dialog in real use (the self-test calls the functions behind it), icon quality from a
real site (only an SVG on a local server was tried), manifest shortcuts, `launch_handler`, protocol handlers
(not used), a later import that compares the saved `id`, a profile badge on the icon.

## Status: notifications (2026-11-17)

No display code was needed: with `permissions.notifications: allow`, `new Notification(...)` made Chromium call
`org.freedesktop.Notifications.Notify` itself (seen with `dbus-monitor`, the real binary, a local test page).
What was wrong was attribution: the call named the app "Chromium" with a `desktop-entry` hint of whatever launched
the process (`CHROME_DESKTOP` in the environment; here `com.anthropic.Claude`). `main` now sets `CHROME_DESKTOP` to
`<profile application id>.desktop` before CEF starts (not for tests), and the hint became `be.geee.browsah.default`.
**Not verified:** how GNOME Shell shows it (header name and icon, grouping), what a click does (Chromium handles
`ActionInvoked` and fires the page's `onclick`; nothing selects the sending tab), the notification's own icon (a
Chromium logo file in a temporary directory), a web app's notifications (they carry the profile's id, not the app's).
Reproduce: serve a page that calls `Notification.requestPermission()` then `new Notification("x")`, run with a
temporary `XDG_*` directory and `permissions: { notifications: allow }`, and `dbus-monitor --session
"interface='org.freedesktop.Notifications'"`.

## Status: address-bar suggestions (2026-11-17)

`suggest::rank` (pure, 5 unit tests) scores bookmarks (the window's workspace), open tabs and history (all
workspaces, 300 newest matches for the first typed word) by: bookmark 50, tab 35, history 10 plus frequency (to 10) and
recency (to 10); plus 25 when what was typed starts the address (scheme and `www.` ignored), 20 when it starts the host,
8 for a title word. A popover under the entry (`app/suggest.rs`; non-focusable so the entry keeps the keyboard; arrow
keys forwarded from the entry's key controller) shows up to six; Enter opens the highlighted one, else what was typed.
Measured (`--selftest` section `suggest`, 7 checks): matching history is offered and unrelated history not, no
highlight until an arrow key, down then up returns to none, Enter opens the highlighted address, the list clears
afterwards and for an empty bar. A first version of the test failed because the selftest profile's database held
history from an earlier run (the section now clears it). **Not measured:** mouse clicks on rows, how the popover looks
and is placed (width, scrolling, theme), IME, typing speed with a large history (a `LIKE` scan on each keystroke; no
index; the history window does the same).

## Status: downloads list (2026-11-17)

`app/downloads.rs`: one thread-local list of rows for the profile, shown by a header `MenuButton` and popover in each
window (rebuilt on every change). `DownloadHandler::on_download_updated` now sends the URL, bytes received and
percent, and a `DownloadCancel` that posts `DownloadItemCallback::cancel` to CEF's UI thread. Finished and stopped
downloads are written to a new `downloads` table (schema 6) and the last 50 are loaded when the first window opens.
Measured (`--selftest` section `downloads`, 9 new checks, a local server that streams 1 MB over about four
seconds): finished downloads are listed by file name, each window's list matches, they are remembered in the
database, *Clear list* empties list, table and button while the files stay, a running download is listed with bytes
received, and cancelling it through the row's handle ends it as cancelled. A first version named a running download
"download" after the cancel because the path is empty on the last event; the name is now kept from earlier events.
**Not measured:** the button, popover and row layout by eye, the Open and Show-in-folder buttons (they hand to the
desktop; the test sets `BROWSAH_NO_OS_LAUNCH`), a failed (interrupted) download, a download that outlives its window,
resuming, drag-out, and the file-moved note.

## Visual pass (2026-11-17)

Pictures taken by the self-test (`BROWSAH_SHOTS`, GTK's own renderer on the widget, Ubuntu dark theme, not a compositor
screenshot) of the downloads popover (running and finished), the address-bar suggestions, the install dialog and a
standalone web app window. Found and fixed: the downloads popover was narrow enough that file names showed as "…"
(now 400 px wide); labels were centred inside their allocation until `xalign(0)`; the highlighted suggestion had no
visible highlight (now an accent tint under `.suggest-list row:selected`). The install dialog and the web app window
(only back, forward, reload and the page title) looked right. **Not looked at:** the tab context menu (with *Copy allow
rules*, *Install as web app…*), the header with the downloads button in a workspace window (its picture failed, the
window was not in front), light theme, HiDPI, long names and many rows, the dash and launcher entries, notifications.

## Status: JavaScript dialogs (2026-11-17)

Found while reviewing the daily-use basics: the browser had no `JsdialogHandler`, so CEF's default cancelled every
`alert`, `confirm` and `prompt` (its log says "Default dialog implementation requires a parent window handle;
canceling the JS dialog"). `ViewJsDialog` (view.rs) now hands each to the owning window (`app/jsdialog.rs`), which shows
an `AdwAlertDialog` (heading "<host> says", a text entry for a prompt, OK/Cancel; for the leave-page question
Stay/Leave, destructive) and answers the callback on CEF's UI thread. One dialog at a time per window, queued;
the self-test holds them as it does permission questions. Measured (`--selftest` section `jsdialog`, 8 checks):
prompt message, default text and origin; typed text reaches the page; confirm cancel gives `false`, accept `true`;
an alert blocks the page until answered; closing the real dialog with Esc gives a prompt `null`. A picture of the dialog
was looked at. **Not measured:** the leave-page question (needs a user gesture on the page; not driven), dialogs from
background tabs and from modals, a dialog still open when its tab is closed (`on_reset_dialog_state` dismisses
queued ones on navigation, but a page blocked in a dialog cannot navigate, so that path is untested), long messages,
"stop showing dialogs from this page".

## Status: page zoom (2026-11-17)

The person's zoom (`BrowserView::user_zoom`, a factor) is added to the existing device-resolution correction in
`apply_zoom` (CEF's zoom level is `ln(factor) / ln(1.2)`), so HiDPI keeps working. Steps are the usual 25 % to 500 %
list (`app/zoom.rs`); `Ctrl+Plus/Minus/0` are window actions, `Ctrl` with a wheel notch sends a `Zoom` event. The factor is
stored per host in `site_zoom` (schema 7; 100 % deletes the row) and `restore_site_zoom` puts it back on every address
change, since CEF resets zoom when the host changes. Measured (`--selftest` section `zoom`, 6 checks): two steps give
125 % and the page's `innerWidth` shrinks to 80 %; remembered for the site; another site is 100 %; coming back
restores it; zoom out; reset forgets it. **Not measured:** Ctrl with the wheel (the self-test's emitted scroll signals
carry no modifier), the keyboard accelerators on real keys and layouts (`Ctrl+=` and `Ctrl++`), the "Zoom 125%" toast by
eye, a fractional display scale, zoom of a sleeping or modal view, and no zoom indicator or reset button in the
header yet.

## Status: find in page (2026-11-17)

A `FindHandler` (view.rs) stores and forwards `on_find_result`; `app/find.rs` is the bar (a `Revealer` at the bottom of the
`ToolbarView`: `SearchEntry`, count, previous/next/close) and `win.find`, `find-next`, `find-prev`, `find-close`
(`Ctrl+F`, `Ctrl+G`/`F3`, `Ctrl+Shift+G`/`Shift+F3`). Typing searches case-insensitively; Enter and Shift+Enter move on;
Escape or a tab switch closes it and clears the highlights in every tab. In windowless mode Chromium reports the first
result with match 0 until told to move on, so the first result of a new search triggers one `find_next` to settle on
match 1. Measured (`--selftest` section `find`, 6 checks, and a picture looked at): the bar opens, three matches (one
capitalised) counted and the first active, next and previous, no-match text, closing clears. **Not measured:**
match-case and whole-word options (none offered), searching a page that is still loading, a sleeping tab, PDFs, find
across a tab switch back, the text selection afterwards, and the key bindings on real keys (the test uses the window
actions).

## Status: fullscreen (2026-11-17)

`DisplayHandler::on_fullscreen_mode_change` becomes a `Fullscreen(bool)` event; `app/fullscreen.rs` sets the window
fullscreen and hides the top and bottom bars (`ToolbarView::set_reveal_*_bars`). `F11` toggles the window (and, if a page
had asked for fullscreen, makes the page leave it with `document.exitFullscreen()`). The intent is kept in a `Cell`
because `gtk::Window::is_fullscreen` only follows the compositor's answer later (a first test read it too early).
Measured (`--selftest` section `fullscreen`, 6 checks): F11 on and off; a page's `requestFullscreen()` (run with a user
gesture through DevTools, `cdp_eval_as`) fills the window and hides the bars; the page leaving restores them; F11
while the page is fullscreen makes the page leave. **Not measured:** a real video's fullscreen button, `Esc` from the
keyboard (Chromium's own handling), the compositor's behaviour on a second monitor, hover-to-reveal bars, a page
requesting fullscreen from a background tab or a modal, keeping the screen awake during video.

## Status: developer tools (2026-11-17)

`win.devtools` (`F12`, `Ctrl+Shift+I`) calls `BrowserHost::show_dev_tools` with a plain `WindowInfo` (Chromium's own
window) and no client of ours, from CEF's UI thread; calling it again closes them. Two first attempts failed and are worth
knowing: forcing the Alloy runtime style on the tools' window gives "Alloy style is not supported for this browser" (no
window), and passing our own client made the tools' title ("DevTools - <address>") overwrite the page window's title because
the client routes into this view. Measured (`--selftest` section `devtools`, 3 checks, and `WAYLAND_DEBUG=client`): the
action opens the tools (a second `xdg_toplevel` titled "DevTools - <address>" appears, the page window's title is
untouched) and toggles them closed. **Not measured:** how the window looks and is placed, that it closes when its page's
window does, closing it with its own X and then pressing F12 (the toggle reads `has_dev_tools` on CEF's UI thread, so it
should reopen), inspect-element from the context menu, and DevTools in a web app or modal.

## Status: print and save as PDF (2026-11-17)

`BrowserHost::print_to_pdf` works in windowless mode (a `PdfPrintCallback` reports the result as a `PdfSaved` event);
CEF's own print dialog does not, because that needs a `PrintHandler` and a print UI. So: `Ctrl+Shift+P` (and the tab
menu) asks for a file name with a `FileDialog` and writes the PDF; `Ctrl+P` (*Print...*) writes a temporary PDF under
`~/.cache/browsah/print/` (files older than a day are removed on the next print) and opens it in the desktop's PDF
viewer, which has the printer dialog. Options `print.paper` (`a4` or `letter`, in inches for CEF) and `print.backgrounds`;
the page's own `@page` size wins (`prefer_css_page_size`). Measured (`--selftest` section `print`, 3 checks; unit tests for
names and paper sizes): a page is written as a real PDF (`%PDF`), the MediaBox is A4 (595 pt wide), and the job is
cleared when the file is reported. **Not measured:** the file dialog and the Print hand-off to a viewer (the test sets
`BROWSAH_NO_OS_LAUNCH`), Letter paper, headers and footers (none), page ranges, landscape, a very long or still-loading
page, selection-only printing, direct printing to a printer without a viewer.

## Status: tab groups (2026-11-17)

`AdwTabView` has no groups, so a group is a model on top of it (`groups.rs`, `app/groups.rs`): each `Tab` has a group id
(0: none); joining moves the page next to the group's last tab (`join_position`, pure and tested) and sets a coloured dot as
the page's icon (a `BytesIcon` of a small SVG) and the group name as its tooltip. **Collapsing** inserts a placeholder page
(`▸ Name (n)`) and moves the group's pages into a second, never shown `AdwTabView` (`transfer_page`; there is no public
detach), so the tabs stay alive and nothing is lost; selecting or closing the placeholder expands the group (after the
signal, via an idle callback; a flag stops the collapse's own selection changes from re-expanding it). While pages are
transferred the `page-detached` handler is told to ignore them (it would treat them as dragged away). Parked tabs are saved
where their placeholder is, so the order survives. Schema 8 adds `tabs.grp` and a `tab_groups` table. Sweeping skips parked
tabs. Measured (`--selftest` section `groups`, 14 checks, pictures looked at, plus the neighbouring tab, window, restore,
locked, bookmark, send-to-workspace, sweep and history sections: 124 checks): grouping moves tabs together; icon, tooltip,
rename and recolour; collapse leaves one stand-in and a real tab selected; the saved records still hold all tabs; expand
restores order; a collapsed group comes back collapsed after a restart; removing the last tab forgets the group; closing a
group closes its tabs. **Not measured:** the dialogs (name, rename), the menus themselves, dragging a grouped tab within the bar
(it can leave its group's run; the dot stays), a page popup (`target=_blank`, `window.open`) from a grouped tab joins its group (built, not driven by a test; other new tabs, such as Ctrl+T, do not), middle-click close of a
stand-in, many groups, light theme, and no group name chip in the bar (only the dot and, collapsed, the title).

## Status: windows without a workspace, main menu (2026-11-17)

Asked how to open a window with no workspace once a workspace is open: `Ctrl+Shift+N` existed but nothing showed it. Added a
main menu (header button, built each time it opens: *New window*, *Open workspace* with the profile's workspaces, find,
PDF, print, developer tools, history, reopen closed tab), `browsah --new-window [URL]` (forwarded to a running profile as
the `new-window-at` action; when starting, it skips the remembered workspaces), and a *New Window* action in the profile's
`.desktop` entry (`browsah profile install <id>` rewrites an existing entry). Windows with no colour of their own now take
the profile's colour. Measured: `--selftest` section `menu` (menu model and workspace list, picture looked at), and by hand
in a temporary data directory with the real binary: a running profile with a workspace open gets a second, plain window
(two toplevels, the second without the workspace's name in its title); starting with `--new-window` opens one plain window
although a workspace was remembered. **Not measured:** the launcher action in GNOME's dash (not installed for existing
entries until `profile install`), the profile colour on screen, the menu on a small window.

## Fix: dark lines around a coloured header (2026-11-17)

Reported with a picture (yellow workspace, white page): thin dark lines above the header and below the tab bar. The
colour was applied to the `headerbar` and the tab bar's inner box only; libadwaita's `ToolbarView` wraps the top bars in
`revealer.top-bar > windowhandle`, which is 3 px taller at the top and bottom than the bars it holds, and those rows showed
the window background. Found by a pixel check (`pixels()` in the self-test renders the window with GTK's renderer and reads
points): rows 0 to 2 and 83 to 85 were the window's grey (44, 44, 44) while the bars were the tint. The style sheet now also
colours `toolbarview > revealer.top-bar` and its `windowhandle`; the self-test section `locked` checks ten rows of the header
and tab bar against the tint with a white page loaded (and the `workspace-colour-white-page` picture was looked at).
**Not measured:** the light theme (the test machine was in the dark theme, so the dark variant of the tint was used), HiDPI,
a bookmark bar showing under the tab bar, the same area with a web app's header.

## Window colour frame (2026-11-17)

A window with a colour (its workspace's, else its profile's) shows it as a border around the page, like a terminal with a
coloured window chrome: the page sits in a `.browsah-frame` box (`ToolbarView` content) whose style gives it the colour as
background and padding on the sides and bottom, and the tab view inside it rounded corners (GTK CSS has no `overflow`, so
the clip is `set_overflow(Hidden)` in code). `window.frame.enabled`, `thickness` (default 4 px, 0 to 40) and `radius` (default 8)
are live options, per workspace; the window's style sheet is rewritten when the configuration is reloaded
(`refresh_color_css`). The bottom border is dropped while the find bar (the only footer) is open, by a `has-footer` class on
the window. The frame stays when maximized (asked for) and goes in fullscreen, where the bars are hidden. The bottom bar is
tinted too, so the find bar sits in the colour. Measured (`--selftest` section `frame`, 12 checks, pictures looked at, whole
self-test 322 of 322): pixels at the sides, bottom and the page's corner (tinted in the corner, white just inside it) for
unmaximized and maximized windows; the default 4 px; a changed thickness and turning the frame off applied at once with the
header still tinted; fullscreen shows only the page. The first thickness (8 px) was judged too thick, then 6 px; the default is 4 px. **Not measured:** the
light theme, a window with no colour (unchanged, no frame), tiled windows, HiDPI beyond the test machine's scale 2, the find bar
footer by eye, the web app window with its tab bar hidden.

## Routing links that go through a redirector (2026-11-17)

Reported: Google makes links point at a tracking address (`https://www.google.com/url?q=<destination>&...`) that redirects to
the destination, so a rule for the destination never fired (the router only saw the first address, and redirects are
deliberately not routed so sign-in flows work). Two mechanisms:

1. **Unwrap** (always on): `router::unwrap_url` replaces a known redirector's address by the address in its query before the
   rules are applied and the destination is what gets opened. Built-in: Google (`google.<tld>/url`, `q` or `url`; the host is
   checked strictly, so `google.evil.example` is not treated as Google), Facebook/Messenger `l.php`, Outlook Safe Links, Slack,
   LinkedIn, DuckDuckGo, YouTube; plus `routing.unwrap` entries; nested up to three levels; only web-address values count
   (`javascript:` is ignored). No request is made to the tracker for a link that a rule sends elsewhere.
2. **Follow redirects** (`routing.follow_redirects`, off by default): for opaque shorteners (`t.co`, `bit.ly`) nothing in the
   address names the destination, so each redirect hop of a **clicked same-tab link** is judged like a click on that address
   (`on_before_browse` is called with `is_redirect` for server redirects: measured). It is off by default because sign-in flows
   are chains of redirects and one that ends on a rule's address would be moved to that workspace.

Measured: unit tests (known redirectors, the strictness cases, configured redirectors, switching the list off), `browsah route test`
against a real profile (a Google-wrapped Miro link goes to the Miro workspace from another application and from another workspace;
a wrapped link to elsewhere is left alone), and `--selftest` section `redirects` (clicks on a wrapped link, on a link that
302-redirects with the option on and off; 17 checks; with routing, modals, locked and send-to-workspace 62 checks). **Not measured:**
real Google pages (the shape of their links was taken from reports of Docs, Gmail and Chat links, not captured; current result pages
mostly link straight to the destination and report clicks with a `ping` attribute, which needs no handling), new-tab links through
a shortener (not followed: the new tab's first load is never routed), client-side (script) redirects, `t.co` or `bit.ly`
themselves (no network in the tests).

## Theme for page content (2026-11-17)

Requested: light, dark or system (default) for the browser/CEF content, settable per profile, workspace and web app. Measured
first (`--selftest` section `theme` printed what a page sees): with no flag a page saw `prefers-color-scheme: light` although
the desktop portal's `org.freedesktop.appearance color-scheme` was 1 (dark) and libadwaita followed it, so **Chromium's own default
does not follow this desktop**; `--force-dark-mode` and `--force-light-mode` work but are process-wide (no per-workspace theme).
So browsah decides: the `theme` option (`system` default, `light`, `dark`; a normal layered option, so profile, workspace and web
app all work) is applied to each browser with the DevTools call `Emulation.setEmulatedMedia` (`prefers-color-scheme`), when the
browser is created and whenever the configuration or the desktop's scheme changes; `system` means libadwaita's `is_dark()`, so the
page content matches the window and follows it live. Measured (section `theme`, 20 checks): default follows the desktop; dark and light
force the page's media query and its dark style; it survives navigation to another page and to another site (without being
re-sent); a new tab has it from the start; `system` follows the desktop switching (the application-wide light/dark override stands in
for the desktop); an explicit theme ignores the desktop; a workspace's and a web app's own `theme` apply in their windows and others
keep the default; unit tests for the layering. A picture was looked at. **Not measured:** the real desktop switching live (not
changed on the test machine: it is the user's setting), the portal's third value (no preference, treated as light by libadwaita),
the flash before the first frame of a page (the emulation is set when the browser is created, before it loads, not measured with a
slow page), DevTools' own panel with its own media emulation open. **Limits:** it changes what the page is told, as a browser's
setting does: pages without a dark style stay light, and form controls and scrollbars only darken for pages that declare `color-scheme`
(the picture shows a light text field on a dark page). It does not change the window's own appearance (that follows the desktop).

## Favicons (2026-11-18)

CEF reports a page's icon addresses (`on_favicon_urlchange`); `download_image(is_favicon, 64 px)` fetches the first that works (the rest
are tried if it fails; no addresses or none that work: the icon is cleared) and `as_png` gives the bytes (the pixel-size outputs are
needed, as for app icons). The tab shows it as the page's icon (`gdk::Texture` is a `GIcon`). Because the icon slot is also the tab
group's dot and the indicator slot is the sweeper's, a grouped tab's icon is the favicon with the dot drawn on its corner (a snapshot
rendered to a texture, with a white ring so it shows on any icon). Icons are remembered per origin in `favicons` (schema 9, newest 2000)
so restored sleeping tabs, bookmarks, address-bar suggestions and the history window show them before a page has loaded; private
windows store none; the tab spinner (`page.loading`) shows while a page loads. Measured (`--selftest` section `favicon`, 19 checks, plus
groups, suggestions, bookmarks and history: 57, pictures looked at): a PNG icon and an SVG icon (rendered in its colour) reach the tab, the icon is
stored, a page without an icon clears it, the group dot appears on the icon and goes, a restored sleeping tab has the site's icon at once,
and the helpers give icons to bookmark and suggestion rows. **Not measured:** real sites (only a local server's PNG and SVG; `.ico`
files with several sizes, `apple-touch-icon`, data URIs, redirected or very large icons), icon sharpness on a high-DPI screen (64 px is
asked for 16 logical pixels), icons that change without a page navigation, the cache being per origin (so two pages of a site with
different icons show the last one when restored), the history window by eye, and the tab spinner by eye.

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
