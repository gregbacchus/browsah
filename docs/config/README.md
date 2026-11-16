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

## Layers and reloading

Settings come in layers, each a YAML file with the same option names as [options.md](options.md):

1. the application file, `~/.config/browsah/config.yaml`;
2. the profile's file, `~/.local/share/browsah/profiles/<id>/profile.yaml` (see [profile.md](profile.md)).

(Workspaces and web apps will add layers of their own.) The upper layer is merged over the lower:

- maps merge key by key, so a profile can change one option inside `input` and keep the rest;
- a value or a list in the upper layer replaces the lower one;
- a key ending in `+` appends a list instead: `chromium: { flags+: ["--foo"] }` adds to the
  application's flags;
- application-only options (`watch_config`) are ignored in a profile, with a warning.

A profile is its own process, so it may also set options that are fixed per process (`chromium`,
`gpu`, `rendering.mode`). See the result with `browsah config show --profile=<id>`, which also lists
the options the profile set.

**Reloading.** Unless `watch_config` is `false` (or `--no-watch` is given), the files are watched
and reloaded about 0.3 s after a change, even when an editor replaces the file. A message in the window
says what happened. An invalid file is reported and the running settings are kept.

| Takes effect | Options |
|---|---|
| immediately | `search_url`, `home_page` (new tabs), `rendering.frame_rate`, `input.*` |
| next start (the message says so) | `chromium.*`, `gpu.*`, `rendering.mode`, `rendering.device_resolution`, `window.*` |

Workspaces have theirs, [workspace.yaml](workspace.md).

Profiles have their own file, [profile.yaml](profile.md) (name and colour; more will arrive with workspaces and routing).

Command-line flags override the matching config options: `--mode=paint|accel`
(`rendering.mode`), `--backing=off` (`rendering.device_resolution`), a bare URL or `--url=URL`
(`home_page`). Any other `--flag` is passed to Chromium.
