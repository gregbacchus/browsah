# Tab menu: custom popover reverted to the native menu (2026-11-20)

## Problem

Commit a34133d drew the tab menu as a hand-built `gtk::Popover` (the page menu's style) shown from AdwTabView's `setup-menu`
signal. On real GNOME/Wayland (user report, not reproduced here): a click away from the menu did not close it, Escape closed it, and
afterwards the header bar did not react to clicks.

## Suspected cause (not measured)

`setup-menu` is emitted by AdwTabView just before it pops up its own context popover. a34133d gave the view an empty model so the
signal fired, so two autohiding popovers were opened for one right click (ours on the tab bar, AdwTabView's empty one), and they
competed for the popup grab. That matches the symptoms (the outside click went to the wrong popover; the grab was left behind).
This is a reading of the code and libadwaita's documented behaviour, not something a headless run can show.

## Decision

The user's original request was about the bookmark item menu, so the restyle was unnecessary. The tab menu was reverted to the
native GMenu popover AdwTabView owns (a34133d reverted; `tab_view.set_menu_model(tab_menu)`, `setup-menu` only fills the model).
The web app's title right-click (header with no address bar) is kept: it opens the same `tab_menu` as a native
`gtk::PopoverMenu` (`Shell::show_title_menu`): autohide on, parented to the header, unparented from an idle callback after `closed`,
replacing any earlier one.

## Measured (headless weston, `BROWSAH_SELFTEST_ONLY=webapp`, 46 of 46 checks)

After the title menu is popped down, and after a second one replaced the first: the popover is unparented and unmapped, autohide was
on, the header/window/reload are sensitive, and a header button takes a click; the tab menu's model is cleared. While writing the
check two RefCell double-borrows were found (`if let ... = cell.borrow()...` keeps the borrow through the body while the popover's
`closed` handler borrows again); both fixed.

## Not verified

A headless compositor gives no input serial, so a real popup grab, a real click outside the menu and a real Escape key were not
exercised, nor the tab bar's own right-click menu (AdwTabView's, unchanged from before a34133d). Needs a look on real GNOME.

Reproduce: `scripts/test selftest webapp` (weston headless; do not run two self-tests at once).
