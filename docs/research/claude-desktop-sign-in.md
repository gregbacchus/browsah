# Claude Desktop sign-in and the `claude://` hand-back

Date: 2026-11-20. Question: signing in to Claude Desktop through browsah ends on a `claude://` address that does nothing. Why?

## Environment

- browsah 0.3.4, profile `home`, Linux (GNOME, Wayland). Claude Desktop 2.1.325 (`/usr/lib/claude-desktop`, Electron).
- `xdg-mime query default x-scheme-handler/claude` gives `com.anthropic.Claude.desktop` (measured), so the desktop side is registered.

## Measured (read from the installed app's `app.asar`, with `strings`; not run)

- The app signs in with OAuth against `https://claude.ai/oauth/authorize` (a `client_id` of its own, `code_challenge` with S256, `state`).
- Its `redirect_uri` is **`https://claude.ai/desktop/callback`**: a normal web address, not `http://localhost` and not `claude://`. (Staging and local builds use `claude-ai.staging.ant.dev/desktop/callback` and `localhost:4000/desktop/callback`.)
- The app registers `claude:` (also `claude-dev:`, `claude-nest:` ...) as its protocol; it accepts deep links such as `claude://claude.ai/new`, `claude://code/...`, `claude://cowork/...`.

## Why browsah fails (code reading, consistent with the above; the final redirect itself was not observed)

1. `router::looks_like_app_sign_in` counts an OAuth address as another application's sign-in only if its `redirect_uri` is loopback or an application scheme (`src/router.rs`). `https://claude.ai/desktop/callback` is neither, so the link is judged an ordinary page.
2. It therefore opens as a normal tab (in the journal: a `claude.ai/login/popup-google-auth` tab in workspace `mapctrl`), and `BrowserView::allow_scheme` is never called for it.
3. After sign-in, `claude.ai/desktop/callback` is a web page whose job is to send the browser to a `claude://` address. Chromium reports it through `on_protocol_execution`, which drops every scheme the view was not told to expect (`src/app/view.rs`). Nothing is shown.

Not measured: the exact `claude://` address the callback page uses (it needs a real sign-in; the page is server-side, so it is not in the app files), and whether it uses a redirect, `location.href` or a button click (all reach `on_protocol_execution`, but a click needs a user gesture which is present).

## Options

- **`routing.desktop_schemes`** (new, any layer): schemes a page may hand to the desktop. Does not depend on recognising the sign-in. Cost: any page in scope may then launch that application.
- **Recognise the pattern**: a built-in rule that a `claude.ai/oauth/authorize` address with `redirect_uri=https://claude.ai/desktop/callback` is an app sign-in expecting `claude`. Precise, but tied to one application; the generic form (same-site https redirect) would also catch every ordinary "Sign in with X" and is rejected.
- Tell the person: a toast when a scheme is blocked, naming the option.

## Built (2026-11-20)

`routing.desktop_schemes` (application and profile lists joined; live) and a toast naming the scheme when one is blocked. Checked by unit tests only (the list's parsing and lookup, docs, examples); a real Claude Desktop sign-in was **not** run.

## Reproduce

- `BROWSAH_DEBUG=1 browsah`, sign in from Claude Desktop, read the router lines.
- Workaround meanwhile: `BROWSAH_ALLOW_SCHEMES=claude browsah` (all views).

## Update 2026-11-23: the hand-back worked, but the tab showed an error page

Reported (screenshot): with `desktop_schemes: [claude]` the sign-in did open Claude Desktop, yet the tab showed "This site can't be reached"
for `claude://claude.ai/sso-callback?code=...` (`ERR_UNKNOWN_URL_SCHEME`). Cause: the address was handed over from `on_protocol_execution`, but
the navigation itself carried on and failed. Now `on_before_browse` hands over an address whose scheme is allowed and cancels the navigation,
so the tab keeps the page it was on (`view::desktop_scheme`; `on_protocol_execution` stays as the fallback). The `routing` self-test section
checks both (it failed on the old code with the tab's title replaced by the error page, and passes now); a real Claude Desktop sign-in was
not run here.
