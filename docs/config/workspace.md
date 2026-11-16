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
`Ctrl+Shift+N`) is not remembered. All options apply at startup unless stated otherwise.

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
