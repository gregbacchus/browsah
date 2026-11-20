# Address bar and selected tab on a strongly coloured window

Date: 2026-11-20. Answer: the entry and the selected tab are now solid surfaces computed from the window colour, at least 1.6:1 (WCAG) away from it, with text of at least 4.5:1 on them.

## Cause

`workspace_css` set the header and tab bar to the window colour and the text to a light or dark colour, but left the entry and the selected tab to libadwaita. Libadwaita draws both as a translucent tint of the text colour (about 10 to 15 %). On a saturated colour such as `#b000ff` that tint is nearly the same colour as the header (reported with a screenshot of a purple workspace).

## Change

- `icons::surface_on(bg, ratio)` (pure, `src/icons.rs`): mixes `bg` toward white and toward black in 1 % steps, keeps the smaller mix that reaches the contrast ratio (`MIN_SURFACE_CONTRAST` = 1.6), prefers the direction that leaves better text, and picks the text colour (the usual light or dark text, or pure white/black when that is needed to reach 4.5:1). Yellow can only go darker, navy only lighter; the function finds that itself.
- `workspace_css` uses it for `headerbar entry` and `tabbar tab:selected`. No new option; it applies in light and dark style because it works from whichever colour of the pair is in use.
- Tests: `scripts/test unit surface` (named colours and a 216-colour sweep of the RGB cube: surface at least 1.55:1 from the colour, text at least 4.5:1). Self-test section `contrast` opens purple `#b000ff`, yellow `#ffe600` and navy `#0a1a40` workspaces, saves `contrast-<name>.png` with `BROWSAH_SHOTS` and measures the entry's pixels against the header (at least 1.5:1).

## Measured and not measured

Measured: unit tests and the `contrast` section (15 of 15 checks) on a headless weston compositor, software rendering; the pictures were looked at. The page area was blank there (no GPU), which does not affect the chrome. Not verified: real GNOME (Mutter, system dark/light switching, other accent colours), the entry's focus ring on top of the new background.

Reproduce: `BROWSAH_SHOTS=/tmp/shots scripts/test selftest contrast` and `scripts/test unit surface`.
