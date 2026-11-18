# Chrome extensions

browsah can run Chrome extensions, such as the 1Password extension. CEF (the Chromium embedding browsah uses) has no extension
API of its own, so browsah relies on Chromium's own extension support: when it starts it hands Chromium the folders of the
extensions to load (`--load-extension`). Measured (see [the research note](../research/chrome-extensions.md)): content scripts,
background service workers (manifest v3), native messaging to a desktop program, and an extension's own pages work. Extensions
load for the whole profile, and a change applies at the next start.

**What is not there:** the extensions' view of the browser's tabs (`chrome.tabs.query` returns nothing; 1Password's inline menu and
filling worked without it), keyboard shortcuts an extension declares, extension context-menu items, and extensions in windows
whose workspace or app uses `storage: isolated` (measured: those windows have a Chromium storage context of their own, and extensions load
only into the profile's shared one, so an isolated window has no extensions at all). A web app's window shows no extension buttons unless you list them (below).

## Installing from the Chrome Web Store

```sh
browsah extension add aeblfdkhhhdcdjpifhhbdiojplfjncoa            # an id, or a Chrome Web Store address
browsah extension add https://chromewebstore.google.com/detail/1password/aeblfdkhhhdcdjpifhhbdiojplfjncoa
browsah extension list [--profile=ID]
browsah extension update [ID] [--profile=ID]                      # every installed extension, or one
browsah extension remove ID --yes [--profile=ID]
```

`add` downloads the package from `clients2.google.com` (the Web Store's download address), checks that it is signed by the key
whose hash is the extension's id (a package that has been changed, or is for another id, is refused), and unpacks it into
`<profile>/extensions/<id>/<version>/`. It writes the signing key into the unpacked `manifest.json` as `"key"`, because a Web
Store package carries the key in its header, and an unpacked extension without one would get an id made from its folder (and
anything that names the real id, such as 1Password's native host, would refuse it). `update` asks the Web Store for the newest
version and installs it next to the old one; the two newest versions are kept (a running browser may still be using the older).
Nothing updates by itself: run `browsah extension update` when you want to. Restart browsah afterwards.

The download address is the one Chrome's own updater uses, but it is not a documented interface for other browsers: it may
change or refuse a request. The package is checked as above, over HTTPS with the system's certificates.

Everything `browsah extension` does is written to the [log](log.md) (the journal), under the profile it ran for: `browsah log --grep=extension`.

## Which extensions load

- every extension installed in the profile (the newest version of each);
- the folders in [`extensions.paths`](options.md#extensions), for an unpacked extension you are developing;
- except the ids in `extensions.disable`.

A problem with one of them (a folder with no `manifest.json`) is printed at start-up and the others still load. Each profile has
its own extensions (`<profile>/extensions/`); the Chromium profile data is shared by all of a profile's windows.

## Per-window buttons

Each loaded extension with a toolbar popup (`action.default_popup`) can have a button in the window's header, left of the bookmark
star, showing the extension's icon. It opens the popup page in a popover of 400 x 600 pixels; the page is made when the popover
opens and closed with it. A link the popup opens (`window.open`, Ctrl+click) becomes a tab of the window.

Which buttons a window shows depends on the window, and follows the configuration live:

| Window | Shows |
|---|---|
| a workspace window, or the main window | every extension's button **except** the ids in [`extensions.hide`](options.md#extensions) |
| a web app's window | **only** the ids in [`extensions.show`](options.md#extensions) |
| any window with `storage: isolated` | none (no extension is loaded there) |

Set the lists in `config.yaml`, a profile, a `workspace.yaml` or an app's `app.yaml`; a list in a more specific file replaces the
one below it, and `hide+:` / `show+:` appends. For example, to keep 1Password out of one workspace's toolbar:

```yaml
# workspace.yaml
extensions:
  hide: ["aeblfdkhhhdcdjpifhhbdiojplfjncoa"]
```

and to give one app the 1Password button:

```yaml
# app.yaml
extensions:
  show: ["aeblfdkhhhdcdjpifhhbdiojplfjncoa"]
```

**These lists decide what browsah shows, not what runs.** Chromium loads an extension into the profile's shared storage, so its
content scripts (1Password's inline menu on login forms, say) run in *every* window that shares that storage, shown or not. A
window with `storage: isolated` has its own storage and really has no extensions (measured), but then it starts signed out. Use
`extensions.disable` to stop an extension loading anywhere in the profile.

## Native messaging hosts

Some extensions talk to a program on the computer through a *native messaging host* described by a small JSON file. Chromium looks
for those files in `<profile>/cef/NativeMessagingHosts/`. With [`extensions.native_hosts`](options.md#extensions) on (the
default), browsah looks at start-up in the folders Chrome and Chromium-based browsers use (`~/.config/google-chrome`, `chromium`,
Brave, Edge, Vivaldi, and `/etc/opt/chrome/native-messaging-hosts`) and copies the manifests whose `allowed_origins` name a loaded
extension and whose program exists. Each copy is the original text, unchanged. Install the program that provides the host
(1Password's desktop app) the usual way so that its manifest exists in one of those folders.

## 1Password

1Password's browser integration only talks to a browser it trusts, and it trusts a program that is **owned by root and that you cannot
modify** and whose file name is on its list. A build you made, or a Homebrew install (both owned by you), is refused (`binary
permission verification failed` in 1Password's log). browsah has one command for this, and it needs `sudo`:

1. Install the 1Password desktop app and turn on *Settings, Browser, Integrate with 1Password*.
2. Run, once (and again after you upgrade browsah):

   ```bash
   sudo "$(command -v browsah)" system-install --1password
   ```

   It first prints what it will do, and asks before doing it (`--dry-run` only prints, `--yes` skips the question):

   1. copy the program to `/opt/browsah`, owned by root and not writable by anyone else (an older copy there is replaced);
   2. write the start-up script `/usr/local/bin/browsah` (sets the library path, runs `/opt/browsah/browsah`; it will not overwrite
      a file it did not write);
   3. with `--1password`: add the line `browsah` to `/etc/1password/custom_allowed_browsers`, 1Password's list of trusted browsers
      (skipped if it is there; leave the flag off and add the line yourself if you prefer).

   It changes nothing else: your profiles, extensions and settings stay where they are. `sudo "$(command -v browsah)" system-uninstall
   --1password` removes it again (the copy, the script and the line).
3. Restart the 1Password desktop app so that it reads its list.
4. Start browsah as `/usr/local/bin/browsah` (a `browsah` earlier in your `PATH`, such as Homebrew's, is a different copy that 1Password
   will refuse). As your normal user, not with sudo: `browsah extension add aeblfdkhhhdcdjpifhhbdiojplfjncoa`, then start browsah. The
   1Password button appears in the header; its popup unlocks through the desktop app (no password is typed into browsah), and the
   inline menu on login forms offers saved logins.

The system copy is a copy: after `brew upgrade browsah` (or a new build) run step 2 again to update it. The log of 1Password's browser
connection is `~/.config/1Password/logs/BrowserSupport/` and says why a browser was refused (if browsah runs with `XDG_CONFIG_HOME`
changed, the log is under that folder instead). When an extension that uses native messaging starts without a root-owned program,
the [log](log.md) says so (`browsah log --grep=root`).
