# URL routing (`routing:`)

Every link the browser is asked to open goes through the router, which decides where it goes: the
way a browser normally behaves, a particular workspace, an ephemeral window, the desktop's handler
for the address, another profile, or nowhere. Rules are written under `routing:` in any of three
files, and the first rule that matches wins:

1. for a link clicked in a workspace's tab: that workspace's `workspace.yaml`;
2. the profile's `profile.yaml`;
3. the application's `config.yaml`.

A workspace's rules judge only links from its own tabs. Links from other applications are judged by
the profile's rules, then the application's. (`routing:` is not merged like other options: the lists
are tried in that order.) The files are reloaded when they change, like the rest of the configuration
([Layers and reloading](README.md#layers-and-reloading)).

**Sign-ins started by other applications.** Command-line tools and desktop applications (for example
Claude or ChatGPT) open the browser to sign in and then wait for the browser to hand the result back. A
link from another application that looks like that opens by default in an **ephemeral window** of the
profile that received it, so it does not land among a workspace's tabs, and nothing about it is
remembered. Close the window when it says you are done. Recognised (by the address alone, so it is a
guess): an OAuth address whose `redirect_uri` (or `redirect_url`) points back to this machine, either
`http://localhost:PORT/...`, `127.0.0.1` or `[::1]`, or the application's own scheme (`myapp://...`),
and that also has a `client_id`, `response_type` or `code_challenge`; and a device sign-in address (a
path containing `device` with a `user_code`). A rule written by you is tried first, so you can send
particular sign-ins elsewhere (`match: { app_sign_in: true }`), and `routing.app_sign_in` changes the
default for the whole profile or application. `browsah route test` shows when it applied.

Many applications finish by redirecting to their own scheme (`claude://...`). A sign-in window opened
this way may hand an address with *that* scheme (and only that one) to the desktop, which starts the
application; the window then closes by itself. Any other scheme a page tries to open is blocked.

## Links that go through a redirector

Many sites do not link straight to the destination: Google Docs, Gmail, Chat and Calendar wrap links as
`https://www.google.com/url?q=<destination>&...`, Facebook as `l.facebook.com/l.php?u=<destination>`, Outlook as a
`safelinks.protection.outlook.com` address, and so on. The click goes to the redirector, which then redirects, and redirects are
not routed, so a rule for the destination never saw it. Two things handle this:

1. **Unwrapping** (always on). Before the rules are applied, a redirector's address is replaced by the address in its query, so
   the rules judge, and open, where the link really goes (`browsah route test` prints an `unwrapped:` line). Known redirectors:
   Google (`google.<tld>/url`, parameters `q` or `url`), Facebook and Messenger (`l.php`, `u`), Microsoft Safe Links in Outlook mail
   (`*.safelinks.protection.outlook.com`, also the Government and China clouds, parameter `url`), Teams' Safe Links page
   (`/evergreen-assets/safelinks/<n>/atp-safelinks.html` on `*.office.net` or `*.microsoft`, e.g. `teams.public.onecdn.static.microsoft`; the address is
   read from `url` in the query or the fragment, else from the first argument that is a web address and not Microsoft's own click-through endpoint) and Outlook on the web's `owa/redir.aspx` (`URL`, any case),
   Slack (`slack-redir.net/link`), LinkedIn (`/redir/redirect`, `/safety/go`), DuckDuckGo (`/l/`, `uddg`) and YouTube
   (`/redirect`, `q`). Add your own with `routing.unwrap`, turn the list off with `routing.unwrap_known: false`. Teams' page then goes on to the address by script, not by an HTTP redirect, so the move away from it is judged as a clicked link
   (a typed navigation would not match rules that are for `link`), and when a rule sends it elsewhere the tab that was opened for the
   Safe Links page is closed. If Teams sends the address some other way than in the page's own address, the journal says so
   (`browsah log --grep="Safe Link"`) and the link is judged as it is. A link that no rule claims
   is left alone: it goes to the redirector as before. Nothing is fetched: the tracking address is never requested for a link
   that a rule sends elsewhere.
2. **Following redirects** (`routing.follow_redirects: true`, off by default). For addresses that hide the destination (a link
   shortener such as `t.co` or `bit.ly`), the only way to learn it is to follow the redirects. With this on, each redirect of a
   **clicked link in the same tab** is judged like a click on the address it leads to; the first one a rule claims cancels the
   navigation and opens there instead. Not followed: typed addresses, scripts, form posts, links that open a new tab, and anything
   after the page has loaded. The redirector itself is requested (it has to be, to say where it leads). It is off by default because
   sign-in flows are made of redirects: a sign-in started in one workspace that ends on an address a rule claims for another
   would be moved there.

What counts as a request:

| Kind | When |
|---|---|
| `external` | another application gave the browser a link |
| `link` | a link clicked in a page (a user action, not a form post, not a redirect) |
| `typed` | an address typed in the address bar or opened from a bookmark, or any other navigation the browser or a page's script starts on its own; not matched unless a rule lists it |

A link can ask to open `same_tab` (an ordinary link) or `new_tab` (`target="_blank"`, a middle click,
`Ctrl`+click). A script's `window.open(url)` with no window features asks for a tab too, so it is
treated like `target="_blank"`. **Not routed:** redirects (so sign-in flows work), form submissions,
reloads and back/forward, pages inside frames, a tab's own first load, and **popups opened with window
features** (a size, as sign-in and payment pages do), which stay with their opener so the page keeps
working. A link a rule sends elsewhere is cancelled in the tab it came from.

Try rules without a browser: `browsah route test <url> [--profile ID] [--from SOURCE] [--kind KIND]
[--target TARGET] [--explain]`. Without `--profile` only the application's rules apply, which is how a
link from another application is first judged. `--from` is `external` (the default),
`ephemeral`, `workspace:<id>` or `app:<id>`; `--explain` shows how every rule fared.

## Options

| Key | Type | Default | Description |
|---|---|---|---|
| `routing.external` | action | open as a tab in the active window | What to do with a link from another application when no rule matches. |
| `routing.app_sign_in` | action | `ephemeral` | What to do with a link from another application that looks like a sign-in it started (see below). `allow` turns the detection off. |
| `routing.rules` | list | none | The rules, tried in order. Each is a map with the keys below. A rule with a mistake is skipped with a warning. |
| `routing.rules[].name` | text | none | A label shown by `route test`. |
| `routing.rules[].match` | map | everything | When the rule applies: all the keys given must match (`match.*` below). With no `match` the rule applies to every link of the default kinds. |
| `routing.rules[].match.url` | list of text | any address | Address patterns: `host` or `host/path`, as for a pinned tab's scope (`*.example.com`, `**.example.com`, `example.com/app/*`; web addresses only, scheme and port ignored). Any one may match. |
| `routing.rules[].match.scheme` | list of text | any scheme | URL schemes, such as `mailto` or `zoommtg`. Any one may match. |
| `routing.rules[].match.from` | list of text | any source | Where the request came from: `external`, `ephemeral` (a window with no workspace), `workspace:<id>` or `app:<id>` (a tab of a [web app](webapp.md)). Any one may match. |
| `routing.rules[].match.kind` | list | `link` and `external` | `external`, `link` or `typed`. |
| `routing.rules[].match.target` | list | any | `same_tab` or `new_tab`. |
| `routing.rules[].match.app_sign_in` | bool | either | `true`: only addresses that look like another application's sign-in; `false`: only ones that do not (below). |
| `routing.rules[].open` | action | required | What to do (below). |
| `routing.unwrap` | list | none | Redirector addresses besides the known ones (see [Links that go through a redirector](#links-that-go-through-a-redirector)): each is a map with `url` and `param`. A profile's list is added to the application's. |
| `routing.unwrap[].url` | list of text | required | Address patterns of the redirector: `host` or `host/path`, as for a pinned tab's scope (`example.com/out`). Any one may match. |
| `routing.unwrap[].param` | text or list of text | required | The query parameter that holds the address it sends you to (the first one present whose value is a web address is used). |
| `routing.unwrap_known` | bool | `true` | `false` turns off the built-in list of redirectors. A profile's value overrides the application's. |
| `routing.follow_redirects` | bool | `false` | Also judge where a clicked link's **redirects** lead (see below). A profile's value overrides the application's. |

**Actions** (`open:` and `routing.external`):

| Action | Meaning |
|---|---|
| `allow` (or `current`) | carry on as usual: a clicked link goes where it was going, an external link opens as a tab in the active window |
| `workspace:<id>` | open in that workspace's window, as a new tab (the window is opened if needed). From a tab already in that workspace this is the same as `allow` |
| `app:<id>` | open in that [web app](webapp.md): a new tab of its most recent window if it has `multiple_tabs`, else a new window if it has `multiple_windows`, else its one window loads the address (Back returns to the page before). The app is opened if it is not. Only addresses inside the app's `scope` go to it: a rule for an address outside is skipped (`route test --explain` says so), and a link clicked in the app itself is not sent back to it |
| `ephemeral` | open in a window with no workspace |
| `modal` | open as a [modal](#modals) over the window the link came from (the active window for a link from another application) |
| `system` | hand to the desktop's handler. Only for schemes other than `http` and `https`; for a web address it acts like `allow` (the desktop would send it straight back here) |
| `profile:<id>` or `profile:<id>/workspace:<id>` | hand to another profile (which is started if it is not running), optionally into one of its workspaces |
| `block` | do nothing; a short message says the link was blocked |

```yaml
routing:
  external: workspace:inbox            # links from other applications with no rule: the inbox workspace
  rules:
    - name: Work mail
      match: { url: ["mail.example.com", "calendar.example.com"] }
      open: workspace:mail
    - name: Meetings go to the desktop client
      match: { scheme: [zoommtg, msteams] }
      open: system
    - name: Chat links open in their own workspace, whatever clicks them
      match: { url: ["chat.example.com"], from: [external, "workspace:mail"] }
      open: workspace:chat
    - name: Private browsing
      match: { url: ["*.bank.example"], target: [new_tab] }
      open: profile:personal
```

## Modals

A modal is a page you want to look at and close, not keep: a link in an email or a chat message, say.
It opens over the window (or in a window of its own: `modals.style: window` in the
[workspace file](workspace.md)), with a read-only address bar, a **Move to workspace** button (**Open in profile** over a web app), and a
menu to pick a workspace yourself, or *No workspace* for a plain window. *Move to workspace* asks the router where the link would go if it
were not a modal (rules that open a modal are skipped), so the same rules that name the right
workspace do the moving; if none applies, the page becomes a tab of the window it was opened over, except over a
[web app](webapp.md)'s window (no address bar, no way back), where it becomes a tab of a window with no workspace. The
page keeps running when it moves. Closing the modal (the close button or `Esc`) closes the page.

Choose what opens as a modal with rules, so a list of sites (a whitelist) or everything except some
sites (a blacklist) is a matter of rule order:

```yaml
routing:
  rules:
    - name: Corporate sites open in their workspace
      match: { url: ["*.example.com"] }
      open: workspace:work
    - name: Everything else I am sent just gets looked at
      match: { from: [external, "workspace:mail", "workspace:chat"] }
      open: modal
```

Popups opened by a page's script with window features follow the workspace's `tabs.popups` instead
(`tab`, `modal` or `block`), not these rules.
