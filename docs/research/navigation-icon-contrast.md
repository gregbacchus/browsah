# Icon contrast on tinted windows

**Date:** 2026-11-20

The Lucide SVGs have fixed strokes. Choosing a stroke by desktop appearance could put a light
icon on a light workspace or profile tint. The header controls and tab status indicators now pick
the stroke with higher contrast against the active tint. GTK symbolic controls in the header, tab
bar and find bar inherit a foreground selected by the same calculation. Tab group markers have
light and dark edges so the group colour stays identifiable even when it matches the tint.
Untinted chrome follows the desktop appearance. Favicons and extension artwork keep their original
colours, and GTK still dims disabled buttons.

Calculated from the configured RGB values, the cream test tint (`#f7f1e8`) has 1.06:1 contrast
against the light stroke and 10.76:1 against the selected dark stroke. The charcoal test tint
(`#191919`) has 1.45:1 against the dark stroke and 14.75:1 against the selected light stroke.
These ratios describe solid colours before rasterization or disabled opacity.

On this development machine (GNOME on native Wayland, AMD GPU), the frame self-test checked both
tints, a live appearance change, and the bookmark star switching between its two icons. Other
compositors were not tested.

```bash
test_dir="$(mktemp -d)"
XDG_CONFIG_HOME="$test_dir/config" XDG_DATA_HOME="$test_dir/data" \
XDG_CACHE_HOME="$test_dir/cache" BROWSAH_SHOTS="$test_dir/shots" scripts/test selftest frame
```
