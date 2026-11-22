# Tab groups as one tab with a drop-down (2026-11-22)

Question: the first groups folded into a `▸ Name (3)` stand-in tab that expanded on click. The purpose of a group is to save
tab-bar space, so a group should always take the room of one tab and list its other tabs in a drop-down.

Design (built and checked on the developer's machine, GNOME/Wayland, `scripts/test selftest groups`, picture looked at):
- A group's *front* tab (the one in the bar) is the real tab of the group's choice, so selection, titles, favicons, closing and
  saving keep working. The other tabs are parked in a hidden `AdwTabView` (kept alive).
- Colour: the tab bar's `AdwTab` widgets are libadwaita's private type, found by type name (already done for the tab menu);
  a css class per colour (`browsah-group-<hex>`) and a display-wide style sheet paint them (translucent fill, 3 px underline).
- Arrow: the tab page's *indicator* slot (`chevron-down`); libadwaita puts the indicator at the start of the tab, not where the
  close button is, and has no hook to replace the close button. Sweeping's pen/speaker indicators win over the arrow while shown.
- Click: a capture-phase click on the bar opens the list on release (so dragging a tab still works); buttons inside the tab
  (close) are left alone.
- Not verified: a real pointer click on the tab (the self-test calls the same code, not the gesture), and drag of a group tab
  between windows (it leaves its group; the next tab comes forward).
