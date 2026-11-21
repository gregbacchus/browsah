# Web apps (`app.yaml`)

A web app is one site in a window of its own. The window shows the page and little else, it has its own icon and
its own entry in the desktop's launcher, and the desktop (GNOME) shows it as an application of its own, not as part
of the browser. An app is **not a workspace**: it lives in its own directory, `<profile>/apps/<id>/`
(`~/.local/share/browsah/profiles/<id>/apps/<id>/`), with an `app.yaml` and optionally its icon. App ids and
workspace ids share one namespace within a profile (remembered state is keyed by the id), so an id is either an app or a
workspace.

An app can set the same options as a workspace that make sense for one site (`storage`, `cookies`, `permissions`,
`downloads`, `theme`, `routing`, `window.frame`, ... any option of [`config.yaml`](options.md) that a workspace may
set); they are layered as for a workspace: application, then profile, then the app's own file
([README](README.md#layers-and-reloading)). What an app does not have is what only makes sense for a set of pages: pinned
tabs, tab groups, sweeping, a home page.

```bash
browsah app add --profile=work --url https://mail.example.com/          # everything from the site's web manifest
browsah app add mail --profile=work --url https://mail.example.com/ --name "Mail" \
    --scope "mail.example.com,accounts.example.com/signin/*" --display standalone \
    --color "#2a6f97 #61a5c2" --icon ./mail.png --start-url https://mail.example.com/inbox   # each flag overrides the manifest
browsah app list --profile=work
browsah app install mail --profile=work     # write the launcher entry and icon again
browsah app update mail --profile=work      # preview manifest changes
browsah app update mail --profile=work --apply
browsah app route mail --profile=work       # show a routing rule that opens the app's site in the app
browsah app route mail --profile=work --write    # ...and put it in the profile's profile.yaml (--remove takes it out)
browsah app remove mail --profile=work --yes
```

`app add` reads the site the way installing from a page does (below), so the same site gives the same app either way.
`--url` is the page to read; the app starts on the manifest's start page (`--start-url` to choose another). Every flag
(`--name`, `--scope`, `--display`, `--color`, `--icon`) overrides what the manifest says, and the id is optional (made from the
name; `-2`, `-3`... if taken). It fetches the page and its manifest itself, with no browser (a short timeout, the
system's certificates), and says what it found. `--no-manifest` skips reading the site (offline, or to use only the
flags); if the site cannot be read it warns and carries on with the flags. Without a manifest the app takes the page's title
and address and a generated icon.

It creates `apps/<id>/app.yaml`, writes `~/.local/share/applications/be.geee.browsah.<profile>.<id>.desktop` and the icon
(the file given with `--icon`, else the best icon the manifest lists that downloads, else a generated one: a circle in the
app's colour with four swishes, see [the generated icon](profile.md#the-generated-icon)). A manifest's PNG and SVG icons are used as they are; other formats GdkPixbuf can read
(ICO, JPEG, WebP if its loader is installed) are converted to PNG, keeping any transparency; of an ICO's sizes the largest is used.
Launching the profile itself (its launcher icon, or `browsah --profile=<profile>`) while an app is open opens the profile's own windows as usual; it does not just bring the app forward. Start an app from the launcher, or with `browsah --profile=<profile> --app=<id>` (add a link to open the app on it; add
`--new-window` for another window of an app that allows more than one). The launcher entry is written again when the
profile starts if it is out of date (for example after `multiple_windows` or `name` changed), so edits show up in the
dash without `app install`.

**Sending links to the app.** `browsah app route <id>` prints the rule that makes links to the app's site (its `scope`,
or the start page's site) open in the app; `--write` adds it to the profile's `routing.rules` (first, so it beats
broader rules; running it again replaces it, `--remove` deletes it). The rule is named `Open in <the app's name>`
(`Open in Mail`); renaming the app and running `--write` again replaces that rule, and a rule of your own for the app
under another name is left alone. Writing rewrites `profile.yaml` and drops its comments. The rule is an ordinary [routing](routing.md) rule (`open: app:<id>`) that you can edit or write yourself
for other sites, other sources or `routing.external`. Nothing is routed to an app until you add a rule.

**Installing from a page.** In a regular browser window, right-click a tab and choose *Install as web app…*. The page's web manifest is read
(through DevTools, as Chromium sees it): its name, start page, scope, display mode, theme colour and the best icon.
You are shown the name and start page and can change the name before anything is saved; a site without a
manifest can be installed too, as its own page. The result is what `browsah app add --url` makes for the same site (both
read the manifest by the same rules: `name` or `short_name`, `start_url`, a `scope` that contains the start page, `display`,
`theme_color`, `id`, and the icons marked for general use, largest first). The manifest
is the site's own claim. Browsah reads it at install time and when you explicitly run an update.
Icons marked only `maskable` or `monochrome` are not used (GNOME does not mask).

**Updating from a manifest.** `browsah app update <id>` fetches the manifest and previews changes
to the name, start URL, scope, display mode, theme colour and icon. Add `--apply` to save the
changes and refresh the desktop launcher. Existing app windows keep their current page. Close
and reopen the app to use its updated window settings and start page.

Browsah verifies the saved manifest ID and start origin. It applies only values that still
match the last import. Local values and custom icons take precedence. If the proposed start URL
and retained scope conflict, Browsah keeps both existing values. A manifest icon that fails to
download is kept. For apps installed before manifest tracking, the start page must still link
to a manifest with the same ID. The first `--apply` records a baseline while keeping the current
settings and icon. Apps installed without a manifest have no site metadata to update.

## What the window does

- The start page is the app's first tab, pinned, and **the scope applies to the app**: by default the start page's site.
  A link that leaves the scope is handled by the routing rules (the profile's, the app's own and the application's), like
  any link from a workspace: it opens in a window of its own unless a rule sends it elsewhere.
- No new tabs unless `multiple_tabs`; popups open as modals over the app (a payment or sign-in window); controls follow
  `display`.
- It is a window of its profile's process and shares the profile's cookies and sign-ins unless it sets
  `storage: isolated` (below).
- Its window announces its own application id, `be.geee.browsah.<profile>.<id>`, which is also the name of its
  `.desktop` file; that is how GNOME groups the window under the app's own icon.

### Storage: shared or isolated

By default an app **shares** the profile's storage (`storage: shared`, as an installed web app does in Chromium): the
sites' cookies, local storage, IndexedDB, caches and service workers are the same as in the profile's other windows, so
you are signed in once and a sign-in that happens outside the app (a redirect to your identity provider, a link opened
from the app into the profile) just works.

With `storage: isolated` the app gets its own (`<profile>/cef/apps/<id>/`): it starts signed out, nothing it stores is
visible to the profile's other windows and the reverse, and the same site can be signed in as a different account in the
app and in a workspace (work Slack in an app, personal Slack in a tab). The cost is a second sign-in, a second cache on
disk, and sign-in flows that bounce through a page the routing rules open in the profile's storage rather than the app's.
All windows of one app share the app's storage. It applies to tabs and windows opened after the change.

### Extensions

An app's window shows no Chrome extension buttons by default; [`extensions.show`](options.md#extensions) in `app.yaml` lists the ids to
show. With `storage: isolated` there are none at all. The list decides what is shown, not what runs: see
[extensions.md](extensions.md#per-window-buttons).

## Windows and tabs

- `multiple_windows` (default `true`): the app may have several windows. The launcher gets a *New Window* action
  (right-click the app's icon in GNOME's dash) and a link routed to the app opens in a new window if the app has no
  tabs to put it in. With `false` there is one window: launching the app again, or *New Window*, shows it, and a link
  routed to the app loads in it (a normal navigation, so Back returns to the page it was on).
- `multiple_tabs` (default `false`): a window may have more than one tab (the tab bar appears; there is no new-tab button or Ctrl+T, the tabs come from links: Ctrl+click, middle click, *Open link in new tab*); a link
  routed to the app opens as a tab of its most recent window.

A link routed to the app (a rule `open: app:<id>`, see [routing.md](routing.md)) is therefore delivered like this, first
match wins: no window open → a new window on the link; `multiple_tabs` → a new tab in the most recent window;
`multiple_windows` → a new window on the link; otherwise → the existing window loads it. A link outside the app's scope
(its scope, plus the start page's site, which always counts) is not delivered to it: the rule is skipped, and
`browsah route test --explain` says so. `multiple_windows` is read each time a link is delivered; `multiple_tabs` and
`display` when a window opens.

## Remembered window state

The size of an app's window and whether it was maximized are remembered per app (in the profile's database, saved a
moment after you change them and when the window closes) and used for the next window of the app, the way `window.width`
and `window.height` are used the first time. Maximizing keeps the size the window returns to. Removing the app forgets them.

## Options

Every key below is in `app.yaml` at the top level (there is no `app:` section). Paths are relative to the file.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | text | the id | The window title and the launcher entry's name. |
| `color` | colour or two colours | none | The app's accent colour (light and dark, see [value types](value-types.md#colours)): the window frame (`window.frame`) and the generated icon's primary colour (the light one; an app with no colour uses its profile's, then the default blue). A site's `theme_color` fills it in when the app is created. |
| `url` | text | none (required) | The page the app opens on. |
| `scope` | list of text | the start URL's site | Where the app may navigate, as for a pinned tab's scope in a workspace (`host`, `host/path`, `*.example.com`); the scope of every tab and window of the app. Links outside it go through the routing rules. |
| `display` | `standalone`, `minimal`, `browser` or `custom` | `standalone` | `standalone`: only the page (back, forward and reload remain, the address is hidden). `minimal`: also the address, read-only. `browser`: the usual controls, with the tab bar. `custom`: back, forward, reload and the address bar as the `nav` section below says, for what the three presets do not give (for example no back button, or a read-only address without forward); the tab bar still follows `multiple_tabs`. |
| `nav` | section | see below | Which controls the window shows, **only with `display: custom`**: back, forward, reload and the address bar, as a workspace's [`nav`](workspace.md#options) has them. Keys left out are shown (the address editable), not the `standalone` look. The **tab bar is not an option**: it shows when `multiple_tabs` is true. An app has no new-tab button, no bookmarks and no split button. With another `display` the section is ignored and a warning is printed when a window opens. Applies to windows opened after the change. `browsah app add --display custom` writes the section with every key at its default, ready to edit. |
| `nav.back` | bool | `true` | Show the back button (and allow `Alt+Left`). |
| `nav.forward` | bool | `true` | Show the forward button (and allow `Alt+Right`). |
| `nav.reload` | bool | `true` | Show the reload and stop buttons (`Ctrl+R` still works). |
| `nav.address_bar` | `editable`, `readonly` or `hidden` | `editable` | `readonly` shows the address but cannot be typed in; `hidden` shows the page title instead. |
| `icon` | text | a generated icon | A file in the app's directory used as its icon (`icon.png`, `icon.svg`). Applied to the launcher entry by `browsah app install`. |
| `manifest_id` | text | none | The installed web manifest’s identity. `browsah app update` uses it to verify the fetched manifest. Keep this value when editing other settings. |
| `multiple_windows` | bool | `true` | May the app have more than one window. The launcher entry (written by `browsah app add` / `app install`) has a *New Window* action only when this is true. See above. |
| `multiple_tabs` | bool | `false` | May a window have more than one tab. The tab bar shows (there is no new-tab button, and no Ctrl+T: an app's tabs come from its links). A link opened with Ctrl+click, middle click or *Open link in new tab* then opens as a tab; without `multiple_tabs` it opens as an overlay (a modal) over the app. The app's start tab is an ordinary tab: it shows its title and can be closed, and closing the last tab closes the window. With the address hidden (`standalone`, or `custom` with `address_bar: hidden`) the tabs are not a row of their own but fill the header bar's first row, and only while there is more than one tab (with a single tab the header shows the title as usual). Applies to windows opened after the change. |
| `modals` | section | see below | How modals look; the same as a workspace's. |
| `modals.style` | `overlay` or `window` | `overlay` | `overlay`: a modal covers the app's page; `window`: a separate window that stays above the app's. |

```yaml
name: Mail
url: https://mail.example.com/inbox
scope: ["mail.example.com", "accounts.example.com/signin/*"]
display: standalone      # or custom, with a nav: section (see below)
multiple_windows: true
multiple_tabs: false
storage: isolated        # any option a workspace may set, here the one described above
```

```yaml
display: custom
nav:                     # only what differs from the defaults needs to be written
  back: false
  address_bar: readonly
```

An example with every option is [`examples/webapp.yaml`](../../examples/webapp.yaml). Web apps do not appear in a
tab's *Send to workspace* list.
