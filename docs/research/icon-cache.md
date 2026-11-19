# New profile icons not shown: a stale icon cache

2026-11-19. Environment: Ubuntu, GNOME on Wayland, one machine (the developer's). Reported: a newly created profile's
launcher had no icon. Not reproduced by creating a profile; found by inspecting the files.

## Measured

- `browsah profile add` did write `~/.local/share/icons/hicolor/scalable/apps/be.geee.browsah.<id>.svg`.
- `~/.local/share/icons/hicolor/icon-theme.cache` was dated 2026-11-12, older than every browsah icon (first written
  2026-11-17). GTK trusts such a cache: an icon newer than it is not found, so the entry shows the generic icon.
- `profile::install` and the two calls in `webapp.rs` ran `gtk-update-icon-cache -q -t <icons_dir>/../../..`. From
  `.../icons/hicolor/scalable/apps` that is `~/.local/share/icons`, not the theme folder `icons/hicolor`, so the real cache was never
  rebuilt (and a meaningless `~/.local/share/icons/icon-theme.cache` was created).

## Fix

`profile::refresh_icon_cache()` runs `gtk-update-icon-cache -q -t -f ~/.local/share/icons/hicolor` (`-t`: the folder has no
`index.theme`; `-f`: rebuild even if it looks current), falling back to `gtk4-update-icon-cache`; all three call sites use it.
Rebuilding by hand with that command made the cache current on the developer's machine (the new icon was not looked at).

## Not verified

That GNOME Shell shows the icon right after a new profile is created on a fresh machine; the cause fits the files but the
reported launcher was not seen.
