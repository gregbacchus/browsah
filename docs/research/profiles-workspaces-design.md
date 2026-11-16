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

### Source application of an external link: not reliably available

*(from knowledge, untested)* When an application hands a URL to the default browser (`xdg-open`,
GIO, the OpenURI portal), the receiving application gets the URL and, with D-Bus activation, the
sender's process id at best; there is no portable "which app" field. A Wayland activation token
does not name its source to us. Possible best effort: read the D-Bus sender's pid and walk its
parents, which is fragile (short-lived helpers such as `xdg-open`). Better: make the source known by
design. A link clicked inside a browsah web app is known; an application that supports a custom
browser command can launch `browsah open --source <name>`. **To verify:** log what a D-Bus
activation reveals for links opened from a few real applications.

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

## Spikes, in order

1. Per-window Wayland app id and GNOME grouping, with per-profile and per-web-app `.desktop` files.
2. Manifest read (`Page.getAppManifest`) and icon download in our windowless browser.
3. Popup that keeps its opener (needs the popup created attached, not cancelled).
4. Cookie access filter and a second request context.
5. D-Bus activation: what does a link opened from other applications reveal?
6. Service workers, notifications, WebAuthn under CEF.
7. Codec options (prebuilt CEF with codecs, licensing) and Teams with the available codecs.

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
