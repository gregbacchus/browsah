# Which icons does the interface use?

**Date:** 2026-11-19

**Decision:** browsah's own icons come from [Lucide](https://lucide.dev), drawn from a **24 px grid with a 1.5 px stroke** (the proportions),
in the colour of the current style, and **shown at the size the interface already uses** (16 px buttons; the grid scales down). Use Lucide for any new icon. Stock GNOME symbolic icons are left only where nothing has been
converted yet.

Why: the stock set has no icon for some of what browsah does (a broom for swept tabs, a swatch book for workspaces), and mixing
filled and outlined icons looked uneven. Lucide is one consistent outline family with a permissive licence (ISC; a few icons
derived from Feather are MIT; both texts are kept in `assets/icons/lucide/LICENSE`, which satisfies AGENTS.md's licence rule).

## What is converted

| Place | Icon |
|---|---|
| Reload | `rotate-cw` |
| Back, forward | `arrow-left`, `arrow-right` |
| Bookmark (not bookmarked, bookmarked) | `star-plus`, `star` filled |
| Downloads button | `arrow-down-to-line` |
| Main menu | `menu` |
| Workspace picker | `swatch-book` (`layout-grid` was the alternative: a grid says "apps"; a swatch book says colour-coded sets) |
| History: Visited, Recently closed | `rotate-ccw-clock`, `trash` |
| Recently swept button | the broom with sparkles supplied for it (`swept-broom.svg`; Lucide's own `brush-cleaning` and `broom-sparkles` are different drawings) |

Not converted yet, so still the stock symbolic icons: stop, new tab, the downloads list's row buttons, find bar, tab group and
pinned-tab icons, the window buttons. When converting, add the file and a line in `src/icons.rs`; the toolbar will look uneven until
stop and new tab follow.

## How it works

- `assets/icons/lucide/<name>.svg` are Lucide's files exactly as published (`lucide-static` 1.75.0). `src/icons.rs` writes them to
  `~/.cache/browsah/icons/hicolor/scalable/actions/browsah-<name>-dark.svg` and `-light.svg` with `stroke-width` 1.5 and the colour
  set (`#ebebeb` dark, `#363636` light), and adds that folder to the icon theme's search path. `icons::bind(widget, icon)` sets a
  widget's `icon-name` and re-sets it when the desktop style changes (`adw::StyleManager` `dark`).
- **Why not symbolic icons:** GTK recolours a symbolic icon (a name ending in `-symbolic`) by forcing `fill` on its shapes, so an
  outline icon turns into solid blobs (measured: the broom became a filled lump). Ordinary icons keep their strokes, at the price of a
  fixed colour per style instead of the theme's own foreground, and no insensitive (greyed) look.
- Icons are not enlarged: buttons ask GTK for their usual 16 px and the 24-grid drawing scales to it, so a 1.5 stroke is about 1 px on screen. (A first version showed them at 24 px; that was too large and was reverted.)
- The filled bookmark is Lucide's `star` with `fill` set on its shape (Lucide has no filled variants).

## Measured and not measured

On the development machine (GNOME Wayland, dark style, libadwaita 1.9.1) the `swept` and `history` self-test sections draw the
header and the history window, and the pictures were looked at (`recently-swept-window`, `history-closed-reason`). Not checked: the
light style, a custom icon theme, fractional scales, a high-contrast theme, the greyed look of a disabled button (the back and forward
buttons keep their colour when disabled except for GTK's own dimming of the whole button).

Reproduce:

```bash
run_dir=$(mktemp -d)
XDG_DATA_HOME="$run_dir/data" XDG_CONFIG_HOME="$run_dir/config" \
  XDG_CACHE_HOME="$run_dir/cache" BROWSAH_SHOTS="$run_dir/shots" \
  scripts/test selftest swept
```
