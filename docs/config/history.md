# History

Each profile keeps a history in its database (`data.sqlite` in the profile directory), aware of where
things happened:

- **Visits:** every page loaded in a tab, with its title, the time, and the workspace it was in (or
  none, for an ephemeral window). Reloads are not counted twice.
- **Closed tabs:** a tab closed by you is kept so it can be reopened, with the reason it closed
  (`user` today; swept tabs will say which rule swept them). When an ephemeral window is closed with
  tabs open, those tabs are kept together as a group, so the whole window can come back.

The history window opens with `Ctrl+H`: search, show the history of this workspace or of all of them,
open a page (`Enter` here, or in a new tab with `Ctrl`+`Enter`), and a *Recently closed* list for reopening tabs
and ephemeral windows. `Ctrl+Shift+T` reopens the last closed tab of the workspace (in an ephemeral
window: the last closed window).

Never recorded: sign-in windows opened for another application (they carry one-time codes), addresses
matching `history.exclude`, anything while `history.enabled` is `false`, and addresses that are not
web pages (`about:`, `chrome:`, `file:`). Options are in [options.md](options.md#history) and can differ
per workspace.

From the command line:

```bash
browsah history [QUERY] [--profile ID] [--workspace ID | --ephemeral] [--limit N]   # list, newest first
browsah history clear [--profile ID] [--older-than-days N] --yes                    # delete (all, or older than N days)
```

Clearing history also clears the lists of closed tabs. It does not touch bookmarks, remembered tabs or
cookies. Stop the profile first (like removing a workspace, it is refused while the profile runs).

## Address-bar suggestions

While you type in the address bar, a list under it offers up to six matching **bookmarks** (of the window's
workspace), **open tabs** and **history** (all workspaces of the profile), best first: what you typed being the start of
the address (without `https://` or `www.`) ranks highest, then bookmarks over tabs over history, then how often and how
recently a page was visited. Every word typed must appear in the address or the title. The Down and Up arrows highlight a
suggestion (one step past either end returns to what you typed), Enter opens the highlighted one (or what you typed),
Escape closes the list, and a click opens a row. Nothing is offered in sign-in windows or when
`history.suggest` is `false`; with `history.enabled: false` there is no new history to suggest from, though bookmarks and
open tabs still match.
