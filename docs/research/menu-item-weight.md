# Font weight in browser menus

**Date:** 2026-11-20

Adwaita can apply semibold weight to button labels. Menu choices in custom popovers and the
workspace picker use button widgets, and GTK's native menu models generate button CSS nodes. One
application CSS rule gives these choices regular weight, including submenu navigation and picker
sort buttons. List rows in popovers and bookmark titles in the bookmark bar use the same weight. Popup headings are regular weight too (the “Recently swept tabs” heading was bold until 2026-11-21; it is now dimmed instead): the
self-test's `check_no_bold` walks every label of the main, tab, bookmarks, page, history, downloads, swept and workspace-picker popups.

The targeted self-tests check the computed label weight in the main menu, workspace picker, page
context menu, recently swept popover, and bookmark bar. The tests run with GTK on GNOME Wayland. Web content
inside extension popups controls its own typography.

```bash
test_dir="$(mktemp -d)"
XDG_CONFIG_HOME="$test_dir/config" XDG_DATA_HOME="$test_dir/data" \
XDG_CACHE_HOME="$test_dir/cache" BROWSAH_SHOTS="$test_dir/shots" \
scripts/test selftest menu,picker,ctxmenu,swept,bookmarks
```
