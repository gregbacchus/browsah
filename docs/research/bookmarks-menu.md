# Bookmarks: a menu instead of a bar (2026-11-21)

## Question

Can the bookmarks bar under the tabs give way to a menu, without a new popup to get wrong?

## Change

- A `MenuButton` with the `list-star` icon (a list with a small filled star; `star` itself stays the bookmark toggle's) (`Shell::bookmarks_button`) replaces the bar. It sits at the left of the tab bar (`AdwTabBar` start action
  widget) or, when `nav.tab_bar` is `false`, at the left of the header. A web app has no bookmarks, so tabs shown in an app's header
  are not affected.
- Its popover is built each time it opens (`Shell::bookmarks_popover`, through `set_create_popup_func`, like the main menu) from
  `menu::row` rows: the page's icon (or a Lucide `link-2`) and the title, in a scroller. Click: open here. Middle click: new tab.
  Right click: close the list, then open the bookmark's own menu (rename, remove) on the button, so only one popover is open at a time.
  An empty list shows a dimmed "No bookmarks yet" row.
- `nav.bookmarks_bar` is now `nav.bookmarks` (`#[serde(alias = "bookmarks_bar")]`, so existing files keep working).
- `icons::STAR` was missing from `icons::ALL`, so its file was never written and the first picture showed the theme's missing-image
  icon; fixed and the self-test now checks the button's icon is one the theme knows.
- `check_no_bold` (selftest) walks the labels of each popup; it found one bold label, the swept list's heading, now regular weight.

## Measured (GNOME session on the author's machine, self-test `bookmarks,favicon`: 40 of 40 and 48 of 48 checks before the last
two edits; the window and menu pictures were looked at)

Rows have icons and left-aligned regular-weight titles; the menu lists, renames and removes a bookmark and survives reopening.
Documentation edits afterwards were checked with unit tests only (`workspace`, `webapp`, `docs_`, `example`).

## Not verified

A real right click on a bookmark row opening the second popover after the first closes (the idle hand-off); the self-test calls
`bookmark_menu` directly. The star button's position in a window whose tab bar is hidden. Not re-run after a GNOME crash during
the session (cause unknown), so the `ctxmenu` section's two clipboard checks, which failed in the last runs, are unexplained.

Reproduce: `scripts/test selftest bookmarks,favicon` (one at a time; uses the throwaway `selftest` profile).
