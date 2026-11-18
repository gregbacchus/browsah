# Options

Defaults are shown; omit a key to get its default. Each option says when it applies: most apply as soon as the file is reloaded, and `chromium.*`, `gpu.*`, `extensions.paths`, `extensions.disable`, `extensions.native_hosts`, `rendering.mode`, `rendering.device_resolution`, `window.width` and `window.height` need a restart (the list is in [Layers and reloading](README.md#layers-and-reloading)). Every option can also be set in a [profile](profile.md) or [workspace](workspace.md) file, except the application-only ones (`watch_config`, `default_profile`), and the process-wide ones in a workspace. Types are explained in [value types](value-types.md).

## Startup and search

| Key | Type | Default | Description |
|---|---|---|---|
| `theme` | `system`, `light` or `dark` | `system` | The colour scheme web pages are shown in, that is what the page sees as `prefers-color-scheme` (pages that have a dark style use it; most also set their own colours). `system` follows the desktop's light/dark setting as the rest of the window does and changes with it while running; `light` and `dark` fix it. It does not change the window's own appearance. A profile, a workspace or a web app can set its own, and a change applies to open pages when the file is reloaded. |
| `storage` | `shared` or `isolated` | `shared` | Where a workspace's cookies, local storage and cache live. `shared`: with everything else in the profile (sign in once, signed in everywhere). `isolated`: the workspace has its own, so sites cannot see or share what other workspaces store, and the same site can be signed in as different accounts in two workspaces. Only meaningful in a workspace's `workspace.yaml` (ephemeral windows are always shared). Applies to tabs opened after the change; tabs already open keep what they had. |
| `new_tab_page` | `home`, an address, or `null` | `null` | What a new tab (`Ctrl+T`, the "+" button) opens. `null` (or leaving it out) is a blank tab with the address bar ready for typing; `home` opens the `home_page` (a workspace's own, if it sets one); an address opens that page. A profile or workspace can set its own, and `null` there undoes a lower layer's value. Applies immediately when the file is reloaded. |
| `workspace_picker` | bool | `true` | The workspace picker: a button at the start of a window's header bar (before the back button) that opens a list of the profile's workspaces, each with a badge in its colour (a ring for a workspace with none), to open one or bring its window forward; the current workspace has a check and an already open one says "open". The list is ordered by **Recent** (last used first, then never-used ones by name) or **A–Z** (by name, ignoring case): the switch at the foot of the list chooses, and the choice is remembered in the profile for every window. A workspace counts as used when its window opens or is focused. Web apps are not listed, and **a web app's own window never shows the button**, whatever this says. `false` hides it: set it in `config.yaml` for every window, in `profile.yaml` for a profile, or in a `workspace.yaml` for that workspace's window (a window with no workspace follows the profile and application). Applies immediately when the file is reloaded. |
| `home_page` | string (URL) | `https://duckduckgo.com/` | The page opened in a new window. An empty value uses the default. A URL given on the command line takes priority. A profile can set its own, and so can a workspace (`home_page` in its `workspace.yaml`): see [Home page](workspace.md#home-page). |
| `search_url` | string (URL) | `https://duckduckgo.com/?q={query}` | Search used when the address bar text is not a URL or a host name. `{query}` is replaced by the percent-encoded text. A value without `{query}` is rejected and the default is used. |

```yaml
home_page: https://duckduckgo.com/
search_url: https://duckduckgo.com/?q={query}
```

## Behaviour

| Key | Type | Default | Description |
|---|---|---|---|
| `watch_config` | bool | `true` | Reload this file and the profile's `profile.yaml` when they change, and apply what can change live (see [Layers and reloading](README.md#layers-and-reloading)). Set `false` to read the files only at start-up. Application-only: a profile cannot set it. `--no-watch` turns it off for one run. Read at start-up; turning it off in a running browser stops watching, turning it on needs a restart. |
| `sleep_tabs_after` | duration or `null` | `null` | Put a background tab to sleep (free its page, keep the tab; it loads again when selected) after this much idle time: a number and a unit, `30s`, `15m`, `2h`, `7d`. `null` is off. Never the tab being looked at; not a tab with unsaved form input, playing media or still loading; pinned tabs may sleep. It is the simple form of a `discard` sweep rule and works at every level (config, profile, workspace, app; the lowest wins) and in windows with no workspace; any sweep rules are checked first. Applies to open windows when the configuration reloads. See [sweep.md](sweep.md#sleeping-tabs-automatically). |
| `sweep` | section | off | Rules that close or put to sleep tabs that are not used: `sweep.enabled`, `sweep.interval`, `sweep.rules[]`. Set here, in a profile or in a workspace; a workspace inherits the profile's and the application's, its own keys win and `rules+` appends. The keys are described in [workspace.md](workspace.md#options) and [sweep.md](sweep.md). |
| `default_profile` | text | none | The profile the [selector](selector.md) opens web links in without asking, by id. Without it the selector asks (unless there is only one profile). Application-only. |

```yaml
watch_config: true
default_profile: work
```

## `window`

| Key | Type | Default | Description |
|---|---|---|---|
| `window.width` | integer (px) | `1280` | Initial window width, in logical pixels. Clamped to 400 to 16384. |
| `window.height` | integer (px) | `800` | Initial window height, in logical pixels. Clamped to 300 to 16384. |
| `window.frame` | section | see below | The colour border around the page: `enabled`, `thickness` and `radius`. |
| `window.frame.enabled` | bool | `true` | A window that has a colour (its workspace's `color`, else its profile's) shows it as a border around the page: down both sides and along the bottom, with rounded page corners, maximized or not (not in fullscreen). `false` tints only the header and tab bar. A workspace can set its own. Applies immediately when the file is reloaded. |
| `window.frame.thickness` | integer (px) | `4` | How thick the border is, in logical pixels, clamped to 0 to 40 (0 is the same as off). The bottom is as thick as the sides, except while the find bar is shown there. |
| `window.frame.radius` | integer (px) | `4` | The radius of the page's corners inside the border, clamped to 0 to 40. The default matches the default `thickness` (a thicker border looks better with a larger radius, for example 8 with 8). |

```yaml
window:
  width: 1280
  height: 800
```

## `rendering`

How pages are drawn. See [the research notes](../research/summary.md) for the reasoning and the
measured costs.

| Key | Type | Default | Description |
|---|---|---|---|
| `rendering.mode` | `accelerated` or `readback` | `accelerated` | `accelerated` imports Chromium's GPU frames directly (no copy). `readback` copies each frame through CPU memory: slower (about 7x the main-process CPU at 2880x1568 in our test) but a fallback if accelerated frames never arrive on your GPU. |
| `rendering.frame_rate` | integer (fps) | `60` | Maximum frames per second for the page. Clamped to 1 to 120. Lower values save power on animated pages. |
| `rendering.device_resolution` | bool | `true` | Render at the screen's physical resolution on HiDPI displays (CEF is told the screen's scale factor, so text, images and scrollbars are sharp and the right size). With `false`, CEF is told the scale is 1: pages render at 1x and are stretched, which looks blurry but costs less. |

```yaml
rendering:
  mode: accelerated
  frame_rate: 60
  device_resolution: true
```

## `chromium`

| Key | Type | Default | Description |
|---|---|---|---|
| `chromium.flags` | list of strings | empty | Extra Chromium command-line switches, for example `--disable-features=Foo`. `--ozone-platform=wayland` is always applied unless a flag here or on the command line sets `--ozone-platform`. Use with care: switches can break rendering. |

```yaml
chromium:
  flags: []
```

## `input`

| Key | Type | Default | Description |
|---|---|---|---|
| `input.wheel_notch` | number | `100` | CSS pixels scrolled by one mouse-wheel notch, 10 to 1000. Applies at startup. |
| `input.touchpad_speed` | number | `6.0` | Multiplier on touchpad scrolling distance (and on the coasting after a flick), 0.1 to 20. Raise it if scrolling feels slow compared with other browsers. Applies at startup. |
| `input.touchpad_acceleration` | number | `0.025` | Extra multiplier per pixel of one touchpad scroll event, 0 to 1: an event of `d` px is scaled by `1 + acceleration * d`, so fast swipes (large events, 30 to 40 px) scroll disproportionately further than slow ones (5 to 10 px). `0` is linear. Applies at startup. |

```yaml
input:
  wheel_notch: 100
  touchpad_speed: 6.0
  touchpad_acceleration: 0.025
```

## `history`

| Key | Type | Default | Description |
|---|---|---|---|
| `history.enabled` | bool | `true` | Remember the pages visited (per profile, with the workspace they were visited in) and the tabs that were closed, so they can be found in the history window (`Ctrl+H`) and reopened (`Ctrl+Shift+T`). `false` turns recording off (existing history stays until cleared). Can differ per workspace. |
| `history.retention_days` | whole number | `365` | History older than this many days is deleted when the browser starts. `0` keeps everything. |
| `history.suggest` | bool | `true` | While you type in the address bar, offer matching bookmarks, open tabs and history (best first: what you typed is the start of the address, bookmarks, how often and recently you visited). Arrow keys choose, Enter opens, Escape closes. Sign-in windows never show any. `false` turns it off. Applies immediately when the file is reloaded. |
| `history.exclude` | list of text | none | Addresses that are never remembered: patterns as for a pinned tab's scope (`host` or `host/path`, `*.example.com`). Use `key+:` in a profile or workspace to add to the application's list. |

Sign-in windows opened for another application are never recorded. See [History](history.md).

```yaml
history:
  enabled: true
  retention_days: 365
  exclude: ["*.bank.example", "intranet.example.com/hr/*"]
```

## `permissions`

What pages may do. Each kind of permission is `allow`, `deny` or `ask`; `ask` shows a question the first time
and remembers the answer for the site (see [Permissions](permissions.md)). Can differ per workspace.

| Key | Type | Default | Description |
|---|---|---|---|
| `permissions.default` | `allow`, `deny` or `ask` | `ask` | For any kind with no setting of its own, including kinds that are not listed (`other`). |
| `permissions.camera` | `allow`, `deny` or `ask` | the `default` | Whether pages may use the camera. |
| `permissions.microphone` | `allow`, `deny` or `ask` | the `default` | Whether pages may use the microphone. |
| `permissions.location` | `allow`, `deny` or `ask` | the `default` | Whether pages may know the device's location. |
| `permissions.notifications` | `allow`, `deny` or `ask` | the `default` | Whether pages may show notifications. |
| `permissions.clipboard` | `allow`, `deny` or `ask` | the `default` | Whether pages may read the clipboard. |
| `permissions.screen` | `allow`, `deny` or `ask` | the `default` | Whether pages may share the screen or a window. |
| `permissions.midi` | `allow`, `deny` or `ask` | the `default` | Whether pages may use MIDI devices. |
| `permissions.sites` | list | none | Rules for particular sites, tried in order; the first matching rule that says something about a kind decides it. Each is a map with the keys below. Use `sites+:` in a profile or workspace to add to the list from a lower layer instead of replacing it. |
| `permissions.sites[].match` | list of text | none | The sites: address patterns as for a pinned tab's scope (`meet.example.com`, `*.zoom.us`, `**.example.com`). Any one may match. |
| `permissions.sites[].default` | `allow`, `deny` or `ask` | no opinion | For these sites, any kind this rule does not name. |
| `permissions.sites[].camera` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may use the camera. |
| `permissions.sites[].microphone` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may use the microphone. |
| `permissions.sites[].location` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may know the device's location. |
| `permissions.sites[].notifications` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may show notifications. |
| `permissions.sites[].clipboard` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may read the clipboard. |
| `permissions.sites[].screen` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may share the screen or a window. |
| `permissions.sites[].midi` | `allow`, `deny` or `ask` | no opinion | For these sites: whether pages may use MIDI devices. |

```yaml
permissions:
  default: ask
  notifications: deny
  sites:
    - match: ["meet.example.com", "*.zoom.us"]
      camera: allow
      microphone: allow
```

## `downloads`

| Key | Type | Default | Description |
|---|---|---|---|
| `downloads.directory` | text | your Downloads folder | The folder files are saved in; `~` is your home folder. A name already there is never overwritten (`file (1).txt`). Can differ per workspace. |
| `downloads.ask` | bool | `false` | Ask where to save each file; the question starts in `directory`. |

```yaml
downloads:
  directory: ~/Documents/Work
  ask: false
```

## `cookies`

Which sites' cookies are used. A denied site's cookies are neither sent with its requests nor saved from its responses (so it cannot sign you in or track you through cookies). Applies to newly loaded pages after a reload of the configuration.

**Finding what to allow:** with `default: deny` (or deny rules), right-click a tab and choose *Copy allow rules for N blocked cookie sites*: it copies a `cookies:` block allowing the sites whose cookies were blocked since that page started loading. Paste it into `config.yaml`, `profile.yaml` or a workspace's `workspace.yaml` (merge it into an existing `cookies:` section). Use it when a site stops working, then keep only the sites you want. Other site data (local storage, IndexedDB) is not filtered; keep it apart with a workspace's `storage` option instead. Can differ per workspace.

| Key | Type | Default | Description |
|---|---|---|---|
| `cookies.default` | `allow`, `session` or `deny` | `allow` | For sites no rule covers. `deny` makes the rules an allow list; `session` makes every site's cookies forgotten at the end of the session. |
| `cookies.rules` | list | none | Rules, tried in order; the first whose `match` fits decides. |
| `cookies.rules[].match` | list of text | none | Address patterns as for a pinned tab's scope (`tracker.example.com`, `*.ads.example`, `shop.example.org/cart/*`); any may match. |
| `cookies.rules[].action` | `allow`, `session` or `deny` | none (required) | What to do with cookies of matching sites. `session`: they work normally while you browse, and are deleted when the window closes (or, for the profile's shared storage, when the last window using it closes), so nothing is kept for next time. A session cookie belongs to the workspace (or ephemeral window) that saved it: only that window sends it, even where other windows share the same cookie store, and it is deleted when that window closes. One left on disk by an earlier run (a crash, a failed delete) is never sent, and is deleted when a window opens. |

```yaml
cookies:
  default: allow
  rules:
    - match: ["tracker.example.com", "*.ads.example"]
      action: deny
    - match: ["news.example.com"]
      action: session       # works while open, forgotten on close
```

## `print`

Printing (`Ctrl+P`) and saving a page as a PDF (`Ctrl+Shift+P`). Can differ per workspace; applies to the next print.

| Key | Type | Default | Description |
|---|---|---|---|
| `print.paper` | `a4` or `letter` | `a4` | The paper size of the PDF (a page's own CSS `@page` size wins when it sets one). |
| `print.backgrounds` | bool | `true` | Include background colours and images. |

```yaml
print:
  paper: letter
  backgrounds: false
```

## `logging`

The log, written to the system journal ([log.md](log.md)). Fixed for the running process: applies at the next start, and is ignored in a workspace's or app's
file.

| Key | Type | Default | Description |
|---|---|---|---|
| `logging.level` | `error`, `warn`, `info` or `debug` | `info` | What is written. `debug` adds why sweeping kept each tab. `BROWSAH_LOG=debug` overrides it for one run. |

```yaml
logging:
  level: info
```

## `gpu`

| Key | Type | Default | Description |
|---|---|---|---|
| `gpu.workarounds` | bool | `true` | Apply per-GPU environment workarounds before Chromium starts. Currently: `AMD_DEBUG=nodcc` on AMD GPUs and `INTEL_DEBUG=noccs` on Intel GPUs (set when one is present, added to any value you already have), without which Chromium delivers no accelerated frames on native Wayland and browsah falls back to slower readback rendering after 3 seconds. Set `false` to run without them (to test, or if your driver no longer needs them). |

```yaml
gpu:
  workarounds: true
```

## `extensions`

Chrome extensions, loaded into Chromium at start-up ([extensions.md](extensions.md): installing from the Chrome Web Store, setting up 1Password, what works). Extensions installed in the profile load without being listed. `extensions.paths`, `extensions.disable` and `extensions.native_hosts` are fixed for the running process: they apply at the next start and are ignored in a workspace's or app's file. `extensions.hide` and `extensions.show` apply to one window and take effect when the file is reloaded.

| Key | Type | Default | Description |
|---|---|---|---|
| `extensions.paths` | list of paths | `[]` | Folders of unpacked extensions to load, in addition to the ones installed in the profile (`~` is your home folder; `paths+:` appends to a lower layer's list). Each folder needs a `manifest.json`; one without is reported at start-up and skipped. An extension whose manifest has no `key` gets an id derived from its folder's path, so moving the folder changes the id. |
| `extensions.disable` | list of extension ids | `[]` | Installed extensions not to load (the 32-letter id, as `browsah extension list` shows it). |
| `extensions.native_hosts` | bool | `true` | At start-up, copy the native messaging host manifests that a loaded extension names in `allowed_origins` from Chrome's and Chromium's folders (`~/.config/google-chrome`, `chromium`, Brave, Edge, Vivaldi and `/etc/opt/chrome`) into the profile, so an extension that talks to a desktop program (1Password) finds it. The copies are overwritten when the original changes; set `false` to keep your own manifests in `<profile>/cef/NativeMessagingHosts/`. |

| `extensions.hide` | list of extension ids | `[]` | Extensions whose toolbar button (and popup) a **workspace** window does not show. A workspace window shows all the profile's extensions by default, so this is the deny list; ignored in an app. Layers: a list in the workspace replaces the profile's, `hide+:` appends. This decides what browsah *shows*, not what runs: see [extensions.md](extensions.md#per-window-buttons). |
| `extensions.show` | list of extension ids | `[]` | Extensions whose toolbar button (and popup) a **web app's** window shows. An app shows none by default, so this is the allow list; ignored in a workspace. Layers as for `hide`. A window with `storage: isolated` shows no extension buttons at all, whatever these lists say (extensions are not loaded there). |

```yaml
extensions:
  paths: ["~/src/my-extension"]
  disable: []
  native_hosts: true
  hide: []
  show: []
