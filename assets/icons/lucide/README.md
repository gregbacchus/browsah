# Lucide icons

Icons from [Lucide](https://lucide.dev) (`lucide-static` 1.75.0, ISC licence; a few are derived from Feather, MIT; both licence
texts are in [LICENSE](LICENSE)). The files are kept exactly as published. `src/icons.rs` draws them from a 24 px grid with a 1.5
stroke, shown at the interface's usual 16 px, in the colour of the style (see `docs/research/icons-lucide.md`).

`swept-broom.svg` is the broom-with-sparkles supplied by the project owner (a Lucide drawing, already at stroke 1.5); it is not
byte-identical to any icon in `lucide-static` 1.75.0, which has no icon with these paths.

To add an icon: download it from `https://cdn.jsdelivr.net/npm/lucide-static@1.75.0/icons/<name>.svg` into this folder and list it in
`src/icons.rs`.
