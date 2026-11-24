# Our own tab strip instead of `adw::TabBar` (2026-11-24)

**Question.** `tab_min_width: 40` in `config.yaml` did nothing. Why, and can tabs be narrower?

**Measured (this machine, libadwaita 1.x via `adw` 0.9, GNOME, Wayland).** With 25 tabs open the narrowest tab was 118 px
(131 px with its margins) for the default and for `tab_min_width: 40`; empty titles, no padding and `expand-tabs` off made no
difference. The floor is in the private `AdwTabBox`, not in CSS, so an option could only raise it.

**Decision.** Replace the drawing, keep the model: `src/app/tabstrip.rs` draws the pages of the `adw::TabView` itself (see the
AGENTS.md layout table for the structure). Everything that read `AdwTab` widgets by type name now asks the strip
(`tab_widget`, `page_at`). Group, pin and window colours are CSS on the `tabitem` node (`tabitem.browsah-group-*`, `tabitem.browsah-pin-*`,
`window.<ws> tabstrip tabitem:selected`).

**Behaviour.** Tabs share the strip equally down to `tab_min_width` (default 130, range 40 to 1000) and the strip scrolls below that,
bringing the selected tab into view. Below 72 px a tab is icon only (a generic icon when the page has none), without a close button;
the title is the tooltip. Pinned icon tabs are 40 px. A drop marks the tab it will go before or after.

**Checked.** `scripts/test selftest tabstrip,tabwidth,groups,favicon,locked,tabs,windows,frame,contrast,sleep,webapp,menu,swept`
(pictures `tabwidth-40.png`, `tabwidth-80.png`, `groups-bar.png`, `favicon-tabs.png`, compared with the pictures of the old strip).
Scripted only: the drop is the strip's `drop_page`, selection and closing are signals on the widgets.

**Not checked (needs real GNOME).** A real pointer drag (reorder, and between two windows), drag icon, wheel scrolling, long press and
the touch menu, keyboard focus on the strip. Not done: dragging a tab out of the window to make a new one (libadwaita's bar did not
do it either, no `create-window` handler), live reorder animation (a marker shows the drop place instead).

**Caveat.** The `groups` check "the pin is drawn in its colour (pixel in the tab)" is flaky when the test window is not the active
window (state flags then include `BACKDROP`, and the whole window, header included, is drawn washed out in that state); it passed and failed on consecutive runs of the same build. Not checked: whether the old strip had the same failure in a backdrop window.
