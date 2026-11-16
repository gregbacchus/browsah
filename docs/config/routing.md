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
`ephemeral`, or `workspace:<id>`; `--explain` shows how every rule fared.

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
| `routing.rules[].match.from` | list of text | any source | Where the request came from: `external`, `ephemeral` (a window with no workspace) or `workspace:<id>`. Any one may match. |
| `routing.rules[].match.kind` | list | `link` and `external` | `external`, `link` or `typed`. |
| `routing.rules[].match.target` | list | any | `same_tab` or `new_tab`. |
| `routing.rules[].match.app_sign_in` | bool | either | `true`: only addresses that look like another application's sign-in; `false`: only ones that do not (below). |
| `routing.rules[].open` | action | required | What to do (below). |

**Actions** (`open:` and `routing.external`):

| Action | Meaning |
|---|---|
| `allow` (or `current`) | carry on as usual: a clicked link goes where it was going, an external link opens as a tab in the active window |
| `workspace:<id>` | open in that workspace's window, as a new tab (the window is opened if needed). From a tab already in that workspace this is the same as `allow` |
| `ephemeral` | open in a window with no workspace |
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
