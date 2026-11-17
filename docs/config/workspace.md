# Workspace configuration (`workspace.yaml`)

A workspace is a window of tabs that belongs to a profile. It has its own directory,
`<profile directory>/workspaces/<id>/` (`~/.local/share/browsah/profiles/<profile>/workspaces/<id>/`),
containing `workspace.yaml`, this file. All workspaces of a profile are windows of the profile's one
process and share its application id, so they appear under the profile's icon in GNOME.

```bash
browsah workspace add mail --profile=work --name "Mail" --color "#2a6f97 #61a5c2"
browsah workspace list --profile=work
browsah workspace remove mail --profile=work --yes      # deletes the workspace directory
browsah --profile=work --workspace=mail [URL]           # open it (or raise it if it is open)
```

`--profile` defaults to `default`. A plain start of a profile opens the workspaces that were open
when it last ran (closing windows one at a time forgets them, except the last), else the profile's
`default_workspace`, else a window with no workspace. Such an **ephemeral window** (also made with
`Ctrl+Shift+N`) is not remembered.

**Sending tabs.** Right-click a tab and choose *Send to workspace* to move it, still running, to another
workspace's window (opened if needed) or to a new window. Pinned tabs cannot be sent. A window whose
last tab is sent away closes. All options apply at startup unless stated otherwise.

**What a workspace remembers.** When its window closes, the workspace's tabs (address, title,
selection) are saved in the profile's database (`data.sqlite`), with a screenshot of each page. The
next time it opens, the tabs come back: the selected one loads at once, and the others stay asleep
(no browser process) behind a blurred screenshot until you first show them. Blank tabs are not
remembered, and pinned tabs come from this file each time. Bookmarks are stored there too, per
workspace. Deleting a workspace (`browsah workspace remove`) deletes its tabs and bookmarks.

## Home page

A workspace can have its own home page: put `home_page: "https://..."` in its `workspace.yaml` (it overrides the
profile's and the application's, like any option below). The home page is what the workspace's window opens on
when it has nothing else to show: no remembered tabs and no pinned tabs. A workspace that is reopened shows its
remembered tabs instead, a workspace with pinned tabs opens on those, and a link sent to the workspace opens as
a tab (without the home page). New tabs (`Ctrl+T`) open blank with the address bar focused unless `new_tab_page` says otherwise: `new_tab_page: home` opens the home page in new tabs, an address opens that page, and `null` (the default) is blank.

## Overriding configuration

`workspace.yaml` can also contain options from [options.md](options.md); they are merged over the
application and profile configuration **for this workspace's window only**, with the same rules as
for a profile (maps merge, values and lists replace, `key+:` appends). Options that are fixed per
process cannot be set here and are ignored with a warning: `chromium`, `gpu`, `rendering.mode`,
`rendering.device_resolution` (set them in the profile), and the application-only `watch_config`.
The window size (`window.*`) and the other options can differ per workspace. Scrolling settings
(`input.*`) apply while the workspace's window is the active one.

```yaml
search_url: "https://www.bing.com/search?q={query}"   # this workspace searches elsewhere
home_page: "https://mail.example.com/"                # what its window opens on
rendering:
  frame_rate: 30                                      # a lighter workspace
```

The files are reloaded when they change, like the others ([Layers and reloading](README.md#layers-and-reloading)).
`browsah config show --profile=<id> --workspace=<id>` prints the result and which options each layer set.

## Options

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | text | the workspace id | Display name, shown in the window title. |
| `color` | colour pair | none | The workspace's accent colour: one colour, two in one string (`"#4a2f80 #7a56c2"`, light then dark), or a map with `light` and `dark`. It tints the window's header and tab bar. |
| `nav` | map | all shown | Which navigation controls the window shows (below). |
| `nav.back` | bool | `true` | Show the back button (and allow `Alt+Left`). |
| `nav.forward` | bool | `true` | Show the forward button (and allow `Alt+Right`). |
| `nav.reload` | bool | `true` | Show the reload and stop buttons (`Ctrl+R` still works). |
| `nav.address_bar` | `editable`, `readonly` or `hidden` | `editable` | `readonly` shows the address but cannot be typed in; `hidden` shows the page title instead. |
| `nav.new_tab_button` | bool | `true` | Show the "+" button next to the tabs. |
| `nav.tab_bar` | bool | `true` | Show the tab bar. Hide it for a window with a single pinned tab. |
| `nav.bookmarks_bar` | bool | `true` | Bookmarks for this workspace: the star button and `Ctrl+D` bookmark (or un-bookmark) the current page, and a bar under the tabs lists the bookmarks (it is shown only when there are some). Bookmarks belong to the workspace; ephemeral windows have none. |
| `tabs` | map | see below | Rules for the window's tabs. |
| `tabs.allow_new` | bool | `true` | When `false`, tabs cannot be opened from the window (no `Ctrl+T`, no "+" button, links that would open a tab open in an ephemeral window instead); tabs can only be sent to the workspace. |
| `tabs.popups` | `tab`, `modal` or `block` | `tab` | What a popup opened by a page's script (`window.open` with window features) becomes: a tab of the window, a [modal](routing.md#modals) over it, or nothing. Plain `window.open(url)` and `target="_blank"` links ask for a tab and are [routed](routing.md) instead. |
| `modals` | map | overlay | How modals are shown. |
| `modals.style` | `overlay` or `window` | `overlay` | `overlay`: over the window's content. `window`: a separate window kept above the workspace's window. |
| `sweep` | map | off | Tidying up tabs that are not used (below, and [sweep.md](sweep.md)). Applies when the workspace opens. |
| `sweep.enabled` | bool | `false` | Sweeping is off unless this is `true`. |
| `sweep.interval` | duration | `5m` | How often the rules are checked: a number and a unit (`30s`, `5m`, `2h`, `1d`), from 30 seconds to a day. |
| `sweep.rules` | list | none | The rules; a tab is swept by the first rule whose conditions all hold. A rule with a mistake is skipped with a warning. |
| `sweep.rules[].name` | text | `rule N` | Shown in messages and in the history of closed tabs (`closed by sweep:<name>`). |
| `sweep.rules[].match` | map | every tab | Which tabs the rule is for. |
| `sweep.rules[].match.url` | list of text | every address | Address patterns, as for a pinned tab's scope (`*.google.com`, `example.com/app/*`). Any one may match. |
| `sweep.rules[].idle_for` | duration | none | The tab has not been used for at least this long. |
| `sweep.rules[].score_below` | number | none | The tab's use score (see [sweep.md](sweep.md)) is below this. A rule needs `idle_for`, `score_below` or both; both must hold. |
| `sweep.rules[].half_life` | duration | `12h` | How fast use is forgotten when working out the score: use this long ago counts for half. |
| `sweep.rules[].action` | `close` or `discard` | `close` | `close` closes the tab (recorded as closed by the rule; `Ctrl+Shift+T` reopens it). `discard` keeps the tab but frees its page (it shows its last screenshot and reloads when shown). |
| `tabs.pinned` | list | none | Tabs that are always present, first in the window, and cannot be closed. Each is a map with the keys below. They load fresh each time the workspace opens. |
| `tabs.pinned[].url` | text | required | The address the tab opens on. |
| `tabs.pinned[].title` | text | the page's title | Label shown until the page gives its own title. |
| `tabs.pinned[].scope` | list of text | anywhere | Where the tab may navigate (patterns below). A navigation to an address outside it opens as a new tab (an ephemeral window if `allow_new` is `false`) and the pinned tab stays where it is. |

**Scope patterns** are `host` or `host/path`. The host is exact (`mail.example.com`), or `*.example.com`
for any subdomain (not `example.com` itself), or `**.example.com` for the domain and every subdomain.
The path may contain `*` (any characters); with no path any path matches, `host/` matches only the
site root. Only `http` and `https` addresses match; scheme and port are ignored. Redirects are not
restricted, and a page that is already outside the scope (reached by a redirect, such as a
single sign-on login) can navigate freely, so sign-in flows work. This keeps tabs from piling up; it
is not a security feature.

```yaml
name: Mail
color:
  light: "#2a6f97"
  dark: "#61a5c2"
nav:
  address_bar: readonly
  forward: false
tabs:
  allow_new: false
  pinned:
    - url: https://mail.example.com/
      title: Mail
      scope: ["mail.example.com", "accounts.example.com/signin/*"]
```

## Web apps (`app:`)

A web app is a workspace for one site: a window with the page and little else, its own icon and its own
entry in GNOME's launcher and dash. Make one with `browsah app add` ([webapp.md](webapp.md)), or add an
`app:` section to any `workspace.yaml`. The window then has the app's start page as its only (pinned) tab, no
new tabs, popups shown as modals, and controls chosen by `app.display`; whatever the file itself sets under
`nav`, `tabs` and the like is kept.

| Key | Type | Default | Description |
|---|---|---|---|
| `app` | section | none | Present when the workspace is a web app. |
| `app.url` | text | none (required) | The page the app opens on. |
| `app.scope` | list of text | the start URL's site | Where the app may navigate, as for a pinned tab's scope. A link outside it opens in a window of its own (or where a routing rule sends it). |
| `app.display` | `standalone`, `minimal` or `browser` | `standalone` | `standalone`: only the page (back, forward and reload remain, the address is hidden). `minimal`: also the address, read-only. `browser`: the usual controls, with the tab bar. |
| `app.icon` | text | a generated icon | A file in the app's directory used as its icon (`icon.png`, `icon.svg`). |
| `app.id` | text | none | The web manifest's `id`, kept so a later import can recognise the same app. |
