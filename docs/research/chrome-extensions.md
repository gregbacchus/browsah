# Chrome extensions (1Password) in browsah: spike

Date: 2026-11-18. Machine: Linux 7.0, AMD GPU, CEF 156.5.0 (Chromium 156), Alloy runtime (windowless),
1Password desktop for Linux installed, extension 8.12.46.29. Throwaway profile `spike`, no browsah code changed.

## Measured

- CEF 156 has **no extension API**: no `cef_extension.h`, no `LoadExtension`, no `ExtensionHandler`, nothing in the Rust crate.
- Chromium's own `--load-extension=DIR` still works (browsah passes unknown `--` flags through). A trivial MV3 extension:
  the content script ran in the page, the service worker started, `chrome.storage/scripting/alarms/contextMenus/
  webNavigation/webRequest/action/offscreen/runtime.connectNative` all exist (an API appears only when the manifest
  declares its permission).
- **Native messaging works unmodified.** Host manifests are read from `<profile>/cef/NativeMessagingHosts/`. An echo host
  round-tripped a message; its parent process was the `browsah` binary.
- `chrome.tabs.query({})` returns `[]`: browsah's tabs are not known to the extension system (there is no tab strip model).
- The real 1Password extension (unpacked copy, its `key` gives the official id) loads and its service worker runs. Its native
  host starts and answers, then the desktop app sends `BrowserVerificationFailed` / `BrowserSignatureInvalid` and the host
  exits: `browsah` is not in `/etc/1password/custom_allowed_browsers`.

- **Desktop connection works** once two things hold (measured, 1Password 8.12.44 on Linux, from BrowserSupport's own log):
  1. the binary's file name is in `/etc/1password/custom_allowed_browsers` (`browsah`);
  2. the binary is root-owned and not user-writable (`binary permission verification failed` for a build in `target/release`;
     `verified successfully` for a root-owned copy in `/opt/browsah-test`). The extension then stays connected.
  Note: BrowserSupport logs under `$XDG_CONFIG_HOME/1Password/logs/BrowserSupport/`, which is the place to look.

- **Popup** (`popup/index.html`, loaded in a normal tab through DevTools `Page.navigate`): renders correctly, shows "Syncing...",
  then the unlocked vault list, so the extension works through the desktop-app link with no password typed into browsah.
  Browsah has no toolbar button or popup host yet; this only shows the page itself works.
- **Autofill UI** (local test page with a username and password field on `http://localhost`): the content script injects
  `com-1password-uso` and `com-1password-button`; focusing the password field shows the 1Password icon and its inline
  menu with a matching saved login. All of this happened while `chrome.tabs.query({})` and `chrome.windows.getAll` returned
  nothing (0 tabs, 0 windows), so detection and the inline menu do not need the tab bridge.
- **Filling works**: the person chose the suggested login in the inline menu in the spike browser and the fields filled (reported by the person, not scripted), still with no tabs registered.

## Implementation (2026-11-19)

Built as designed from the spike: `extensions.paths`/`disable`/`native_hosts`, `browsah extension add|list|update|remove`, native host copy, a
toolbar button with the popup in a popover ([docs/config/extensions.md](../config/extensions.md)).

Measured while building it:
- **Chrome Web Store download and update check work from this machine.** `browsah extension add aeblfdkhhhdcdjpifhhbdiojplfjncoa` (in a
  throwaway profile) downloaded 1Password 8.12.46.29 from `clients2.google.com`, the RSA signature check passed, the package's key was
  written into the manifest, the extension got the id `aeblfdkhhhdcdjpifhhbdiojplfjncoa`, and `extension update` read the store's real
  XML answer ("up to date"). Not measured: installing an actual *newer* version over an old one against the real store (the logic is unit tested).
- **A path-derived id matches Chromium's.** For the spike's folder, the SHA-256 of its path gave `ddigafiagnffdlbeadfleljjfpokoipo`, the id Chromium had assigned.
- **Self-test section `extensions`, 22 checks, passing on four runs:** fixture extension loaded; content script ran; the header button opened the popup; its page
  had the extension APIs; the popup reached the service worker and a native messaging host; a `window.open` from the popup became a tab; the popup closed with its
  popover; the popup is 400 x 600. A picture of the popup was looked at.
- **A bug the picture found:** the popover was about 740 x 415, not 400 x 600, because a page's picture is as big as its frame and the popover grew to fit it. A
  `ScrolledWindow` with `Never` scroll policies made it worse (36 x 12: GTK ignores the minimum content size then); `Automatic` policies with a minimum content size fixed it.
- One run of the section exited with a segmentation fault (exit 139) right after a failed check (the popup script ran before its page had loaded: a test bug, since fixed).
  It was not reproduced in the eight later runs that completed (three of them left the popup open at exit). Cause unknown.

## Extensions belong to a storage context, not to a window (2026-11-19, measured)

A throwaway profile loaded the fixture extension (`bench/extensions/test-ext`, a content script that marks the page) and four windows opened a
local page: a plain workspace (content script ran), a workspace with `storage: isolated` (**did not run**), an app with the default shared storage
(ran) and an app with `storage: isolated` (**did not run**). `--load-extension` loads into the profile's own Chromium context; a window with
`storage: isolated` has its own request context (`cef/workspaces/<id>`, `cef/apps/<id>`) with no extensions, and nothing in CEF 156 loads one there.
So within the shared context every window gets every loaded extension (content scripts and all), and per-window allow or deny lists cannot be
enforced by Chromium: they can only decide what browsah *shows* (the toolbar button and popup).

**Loading into an isolated context was tried and does not work.** With `--enable-unsafe-extension-debugging`, DevTools' `Extensions.loadUnpacked` is
available on the browser target (it returned an id) and refused on a page target ("Method not available"). Loaded that way, a second fixture's content
script ran in a plain workspace and **not** in a `storage: isolated` workspace after a reload: it loads into the profile's shared context only.
Per-window sets would need a Chromium profile per set, which means separate storage, or a patch to Chromium's extension system (not attempted).

## Sandboxing a profile with bubblewrap (2026-11-19, measured on this machine)

`bwrap` 0.11.1 runs here (Ubuntu with `apparmor_restrict_unprivileged_userns=1`). A bubblewrap sandbox cannot give *per-window* extension sets (it
wraps a whole process), but a profile is one process, so a profile could be a sandbox boundary.

- **browsah runs inside a sandbox that hides the home folder.** Read-only `/`, `--tmpfs $HOME`, a private `--tmpfs /tmp`, read-only binds for the program and
  `bench/`, read-write binds for the XDG data/config/cache folders and `/run/user/<uid>` (Wayland, D-Bus): the `extensions` self-test section passed 22 of 22
  (window, GPU frames, content script, popup, native messaging to a script host). The home folder listed 0 entries. A first attempt failed because `/tmp`
  was read-only (Chromium makes its process-singleton socket there: "Failed to create socket directory").
- **1Password's native host does not run in the sandbox.** `1Password-BrowserSupport` exits with "process detected it was running without libc's
  security, aborting". The file is setgid (`-rwxr-sr-x root onepassword`) and bubblewrap sets `NoNewPrivs: 1` (0 outside), which makes the kernel ignore the
  setgid bit. So bubblewrap and 1Password are mutually exclusive for one process as it stands. Not tried: running the host outside the sandbox
  (needs a way to spawn on the host, which weakens the sandbox), or a sandbox that does not set no-new-privs.
- Chromium's own sandbox is off in browsah (`no_sandbox`, M6 in the roadmap); a bubblewrap wrapper would be a filesystem boundary for the browser, its
  renderers, extensions and native hosts, not a replacement for it. Not measured: file pickers, drag and drop, portals, printing and downloads inside it.

## Can two processes share a profile's storage by seeing the same folder? (2026-11-19, measured)

The idea: bubblewrap decides which files a process can see, so give a second browsah process (with its own extension set) the first one's storage folder.
Tried with two throwaway profiles: A ran normally; B ran under `bwrap --dev-bind / / --bind <A>/cef/Default <B>/cef/Default` (its own `cef/` root, so the
process singleton did not clash).
- B started, but its log had errors opening shared databases: "Failed to initialize database" (top sites), "Unable to open the password store database"
  (Login Data), and "Unable to open" for a LevelDB store.
- B read what A had already flushed to disk (a cookie and a `localStorage` value written earlier) but **not** what A wrote about 45 s before B started
  (`document.cookie` entry and a second `localStorage` key), and **A never saw B's writes** (a cookie and a `localStorage` key). So there is no live sharing: B gets
  a snapshot at best, and two processes writing the same files (SQLite, LevelDB, caches held in memory) risk losing writes.
- Not tested: whether the files are damaged after both exit.
Conclusion: visibility is not sharing. A Chromium profile folder is meant for one process at a time; "shared storage" inside browsah is one process with one
profile object, and that object is what extensions belong to.

## A process per window or per app (2026-11-19, reasoning plus one measurement)

browsah runs one process per profile (Chromium locks its data folder), and every window of a profile is a GTK window of that process. A separate process
has its own Chromium profile, so it could load its own `--load-extension` set (content scripts included) and could be wrapped in bubblewrap, but it cannot
share live storage with another process (see above): a standalone process is signed out from the profile's sites, like `storage: isolated` but with
extensions available. Measured: an idle browsah with one window on a local page is **8 processes, about 265 MB total PSS** (shared libraries counted once),
so each extra process costs about that much before any tabs. Needed, not built: a per-app application id and start-up path (the id already names the Wayland
window), forwarding links to that process, no background sweeping there, its own configuration reload, and the profile's SQLite database used from two processes.

## Per-window button lists (built 2026-11-19)

Decision: since Chromium cannot give one window fewer extensions than its storage context (above), the per-window rules only decide what browsah *shows*:
`extensions.hide` (workspace windows show all but these) and `extensions.show` (app windows show only these), live, and none in `storage: isolated` windows.
The self-test `extensions` section checks six real windows (workspace default / hidden id / isolated, app default / allowed id / allowed but isolated), the main
window, and a live change through `apply_config`: 30 checks passed on two runs, and a picture of the header was looked at (the fallback puzzle-piece button
sits left of the star; the shot is faint because the test window is not focused). One earlier run of the section hung after the popup opened (cause not found;
a different profile's browser and another build of the tree were running at the time) and did not repeat; the popup wait is now bounded at 20 s.

## A root-owned install (`system-install`, 2026-11-19)

`sudo browsah system-install [--1password]` copies the program to `/opt/browsah` (root-owned), writes `/usr/local/bin/browsah` and optionally adds
`browsah` to 1Password's allow-list, after printing each change and asking. Tested as a normal user against temporary folders (plan, copy, replace,
launcher, allow-list lines, refusals, uninstall) and as a dry run against the real paths. **Not tested:** a run under `sudo` (so that the copy really is
root-owned), 1Password accepting that copy (it did accept the earlier hand-made `/opt/browsah-test` copy), and the Homebrew path. A source build copies about 1.6 GB
(its `libcef.so` is not stripped; the release tarball's is).

## Not yet known

- Whether the popup's "fill current page" and the keyboard shortcuts work with no tabs registered.
- The built feature has not been run against the real 1Password extension end to end by a person (the spike was; the tests use a fixture extension).
- Several windows at once, and the extension's keyboard shortcuts.
- The Web Store update path with a real newer version.

## Reproduce

    S=<scratch>; export XDG_DATA_HOME=$S/data XDG_CONFIG_HOME=$S/config XDG_CACHE_HOME=$S/cache
    browsah profile add spike
    cp ~/.config/chromium/NativeMessagingHosts/com.1password.1password.json <profile>/cef/NativeMessagingHosts/
    browsah --profile=spike --remote-debugging-port=9333 --remote-allow-origins=* --load-extension=<unpacked dir> https://example.com
