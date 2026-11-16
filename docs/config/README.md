# Configuration overview

browsah reads one optional YAML file.

| File | Purpose |
|---|---|
| `~/.config/browsah/config.yaml` (honours `$XDG_CONFIG_HOME`) | The user config: everything on these pages |
| `--config=PATH` on the command line | Use a different file instead |

**Every option is optional.** A missing file, an empty file, or a missing key falls back to the
defaults listed on these pages. A file that cannot be parsed is reported on stderr and the
defaults are used, so the browser always starts. Unknown keys are ignored. Values that cannot
work (for example a window narrower than 400 px) are replaced by the nearest usable value.
A copy-paste starting point with every option set is
[`examples/config.yaml`](../../examples/config.yaml).

Options apply **at startup**: restart the browser after editing the file. (Live reload is not
implemented yet.)

## Sections

| Key | Page |
|---|---|
| `home_page`, `search_url` | [Options](options.md#startup-and-search) |
| `window` | [Options](options.md#window) |
| `rendering` | [Options](options.md#rendering) |
| `chromium` | [Options](options.md#chromium) |
| `input` | [Options](options.md#input) |
| `gpu` | [Options](options.md#gpu) |

Profiles have their own file, [profile.yaml](profile.md) (name and colour; more will arrive with workspaces and routing).

Command-line flags override the matching config options: `--mode=paint|accel`
(`rendering.mode`), `--backing=off` (`rendering.device_resolution`), a bare URL or `--url=URL`
(`home_page`). Any other `--flag` is passed to Chromium.
