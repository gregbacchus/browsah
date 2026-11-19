# Can a tab keep its back and forward list across restarts?

**Date:** 2026-11-19

**Short answer:** not through CEF: a restored tab used to start with one entry, so Back and Forward were gone after the window was
closed and opened again, and after a tab was put to sleep. CEF can list a browser's navigation entries but has no call to put entries
into one. browsah now saves each tab's list with the remembered tabs and uses it through two stacks kept beside the browser's own
list (`src/navhistory.rs`).

## How it works

- **Saving:** when a page finishes loading, `NavigationEntryVisitor` reads the browser's entries (`BrowserView::nav_entries`). The
  tab's saved text is the tab's earlier pages, the browser's entries and the tab's later pages in one list with the current index
  (`tabs.nav_history`, schema version 12; at most 30 earlier and 10 later pages; only `http`, `https` and `file` addresses).
- **Restoring:** the page itself loads in a new browser with one entry. The earlier pages become the *back* stack and the later ones
  the *forward* stack (nearest last in both). The list is dropped if its current page is not the tab's remembered address.
- **Back and Forward:** the browser's own list first; when it has nothing to go to, a page comes off a stack. Going back with the
  stack replaces the page (`location.replace`; if the browser had pages ahead, they are moved to the forward stack and the browser's
  list is reset with `Page.resetNavigationHistory`, because replacing a page would leave them pointing past it). Going forward adds a
  new entry (`location.assign`), so the browser's Back returns to the page. Both run with a user gesture
  (`Runtime.evaluate` with `userGesture`): without one Chromium's history-manipulation protection skips the added entries on Back.
- **A new navigation** (not back, forward or reload: `TT_FORWARD_BACK_FLAG` and `TT_RELOAD` in `on_load_start`) clears the forward
  stack. The shell's own loads (the restored page, a step through the stacks) are not told apart from new ones for 2 seconds,
  because one navigation reported its start twice with another event between.
- **Sleeping tabs** (sweeping, `sleep_tabs_after`) and the rendering fall-back fold the browser's entries into the stacks before the
  browser is replaced (`Tab::keep_browser_history`), so waking a tab keeps its Back list too.

## Measured (development machine: GNOME Wayland, CEF 156.5.0)

The `navhistory` self-test section (28 checks): three pages are visited and one step back made; the tab is saved with one earlier and one
later page; the window is closed and the workspace opened again; the page, both stacks and both buttons are back; Back, Forward and
Forward again walk the three pages, then Back twice through the browser's own list reaches the first; a reload keeps the pages ahead and
a new page drops them; a tab put to sleep keeps its earlier page, wakes on the right page and goes back to it.

Not measured or known limits: the browser's list is read when a load finishes, so a page still loading is not in a saved list yet; pages
whose address needs a POST or script state come back as a plain load; a link followed within 2 seconds of a step through the stacks keeps
the stale forward pages; only workspace windows remember tabs (ephemeral windows do not); the list is not kept for tabs reopened with
`Ctrl+Shift+T`. The [Back and Forward menus](navigation-history-menu.md) now show the saved page titles.
