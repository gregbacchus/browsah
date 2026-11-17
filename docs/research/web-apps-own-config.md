# Web apps as their own kind, not a kind of workspace

Date: 2026-11-18. A design decision (no measurements): what an app is, and how a link reaches it.

## Decision

A web app is `<profile>/apps/<id>/app.yaml`, parallel to (and exclusive with) a workspace. It was first built as a
workspace with an `app:` section; that left an app carrying options that mean nothing for one site (pinned tabs, tab
groups, sweeping, a home page) and made `apply_app` decide whether the app's implied settings or the file's won. A
separate file with its own schema removes both problems and lets an app have options a workspace cannot (`multiple_windows`,
`multiple_tabs`, remembered window state, a launcher "New Window").

## Choices and why

- **Shared window code.** `webapp::window_config` turns an `AppConfig` into the `WorkspaceConfig` the shell already
  consumes, so windows, config layers, cookies, permissions and routing are not duplicated. The cost: `Workspace` stays
  the window context for both, with `config.app` set for an app (never read from a `workspace.yaml`).
- **One id namespace.** Remembered tabs, session cookies and the window registry are keyed by the id alone. Keeping the
  keys bare and forbidding an app and a workspace with the same id (`Workspace::id_taken`) was far less invasive than
  prefixing every key; the isolated storage folders are separate (`cef/apps/<id>`).
- **Storage defaults to `shared`** (as an installed web app in Chromium does), because out-of-scope and sign-in links go
  through the profile's routing and open in the profile's storage; an isolated app would be signed out of what those
  pages set. `storage: isolated` is the opt-in for a second account or for keeping an app's data apart.
- **Delivery of a routed link** (`open: app:<id>`): no window → open one; `multiple_tabs` → tab; `multiple_windows` →
  window; otherwise the one window navigates (so Back works). A rule for an address outside the app's scope, or for a
  link already in the app, is skipped, which avoids a loop (the app's tabs route their own links first).
- **Window state** is saved from GTK's `default-width/height` (kept at the unmaximized size) and `maximized`, a moment after
  a change and on close. Not done for workspaces.
- **`browsah app route`** prints a rule by default and only writes `profile.yaml` with `--write` (a rewrite drops comments).

## `app add` reads the manifest (added the same day)

Installing from a tab read the manifest through DevTools while `app add` only took flags, so the same site gave different
apps. `browsah app add` now fetches the page and its `<link rel="manifest">` itself and builds the same `AppInfo`
(`manifest.rs`, two readers: Chromium's answer and the manifest file, with the spec's defaults for `start_url`, `scope` and `id`);
flags override it, and one `webapp::create` writes the app for both paths. GIO could not fetch https here (`gio cat
https://example.com`: "Operation not supported"), so the CLI uses `ureq` with rustls and the system's certificates
(`rustls-no-provider` + `platform-verifier`, ring provider installed by us). That avoids the CDLA-licensed `webpki-roots`
bundle in the shipped binary (it remains a build dependency of CEF's downloader, as before). The page's own icons (apple-touch-icon, favicon)
are deliberately not used when there is no manifest, to match installing from a tab.

## Verified

On this machine (Wayland, GNOME session): `scripts/test selftest webapp,webapp_open,install,routing,modals,menu` passes,
including a remembered size and maximized state across a closed and reopened app window, and the four delivery cases.
Measured against a local test site and `https://example.com/`: a manifest gave name, start page, scope, display, colour and the
largest non-maskable icon; flags overrode each; a page with no manifest gave its title and a generated icon; https worked. Not tried against a
real-world manifest site.
The launcher's right-click "New Window" action is written and parsed by the unit tests, and `--app=ID --new-window`
forwards to a running profile (`new-app-window`), but that path was not driven from a real GNOME dash.
