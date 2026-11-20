# Menus: icons everywhere, and the tab menu as our own popover (2026-11-21)

## Questions

1. Why did the tab menu, the main menu and the title menu have no icons, when the page and bookmark menus do?
2. Can the tab menu get icons without repeating the failure of a34133d (custom popover that did not dismiss)?

## Findings

- **Measured (GTK 4.22.4, libadwaita 1.9.1, headless weston):** a `gio::MenuItem` with an `icon` attribute in a native
  `GtkPopoverMenu` does not show the icon beside its label (tried on the main menu's "New window", picture looked at). Native menus
  therefore cannot have icons; the page and bookmark menus had them only because they were hand-built popovers.
- The tab menu was native because `AdwTabView` owns it: the tab widgets are internal, and `menu-model` plus `setup-menu` is the only
  documented hook. a34133d kept the model (an empty one, so the signal fired) and added its own popover, so two autohiding popovers
  opened per right click (suspected cause of the dead header, docs/research/tab-menu-grab.md; not reproducible headless).

## Change

- `src/app/menu.rs` draws any `gio::MenuModel` as icon rows in one `gtk::Popover` (`menu::popover`); the model carries the Lucide id
  (`browsah-icon`) and a group colour (`browsah-dot`) as attributes. The page menu's row is the same `menu::row`.
- **The tab view has no menu model now**, so libadwaita opens nothing. `Shell::show_tab_menu` opens our popover (replacing any
  earlier one, unparented when closed, `menu_page` cleared only then) from: right click on a tab (capture phase on the `AdwTabBar`,
  tab found by walking the pick up to the widget of type `AdwTab`, whose `page` property is the page), a touch long press, and the
  menu key / Shift+F10 on the selected tab. The web app's title menu uses the same function.
- Converted to the shared rows: tab menu (and groups, send to workspace), main menu, fallback page menu (extension popups, modals
  with no owner; icons by CEF command id), the modal's workspace chooser, history menu ("More history...", favicons on the rows).
- AGENTS.md has the rule ("every menu is built with `src/app/menu.rs`") so later work does the same.

## Measured (headless weston; `scripts/test selftest menu,ctxmenu,webapp,groups,navhistory,send_to_workspace,bookmarks,picker,swept,modals`)

Rows of the tab and main menu have a Lucide icon, left-aligned label, regular weight (pictures `tab-menu`, `main-menu-popover` looked
at); the tab view has no model; a point on a tab finds that tab; opening the menu twice leaves one popover; after closing, no popover,
no model, and the header takes clicks; send-to-workspace and the groups sections still read and run the same actions.

## Not verified (needs real GNOME)

A real pointer grab: that a click away or Escape closes the tab menu and the header stays responsive afterwards, a real right click
and a real touch long press on a tab, and the menu key. Headless weston gives no input serial. If the dismiss problem comes back,
revert the tab bar part (`tab_view.set_menu_model(Some(&tab_menu))` and the `setup-menu` handler) and keep the title menu.

Not done: icons for Documentation, Browser test page, Add to (new) group, Collapse/Expand/Ungroup group (no suitable Lucide icon is in
`assets/icons/lucide/`; those rows keep a spacer). The picker, swept-tabs, downloads and extension lists keep their own rows.
