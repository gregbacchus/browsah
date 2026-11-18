# Configuration overview

All pages are indexed in [docs/README.md](../README.md); the types used in the *Type* columns are in [value types](value-types.md). Every option is a table row whose first column is its full path in backticks (`rendering.frame_rate`; `[]` means each entry of a list: `cookies.rules[].action`).

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

Most options apply as soon as the file changes (see [Layers and reloading](#layers-and-reloading) for
which do and which need a restart); each option's description says when it applies.

## Seeing every option

The main menu has **Documentation** (these pages rendered as a small site, in a window of their own, with a side bar
and links between pages) and **Application / Profile / Workspace config (all options)…**. The last three open a
*generated copy* of the file: the documented example for that level with every option and its explanation, the options
your file sets switched on with your values and everything else commented out (remove `# ` to set one). It is a copy: edit it,
copy from it, or choose *Save over the file…* to replace the real file (the text is checked first, and the old file is kept
as `<name>.bak`; comments you wrote in the old file are not carried over). The same text is printed by
`browsah config full` (application file), `browsah config full --profile=ID` and
`browsah config full --profile=ID --workspace=ID`; a web app is a workspace.

## Sections

| Key | Page |
|---|---|
| `home_page`, `search_url` | [Options](options.md#startup-and-search) |
| `window` | [Options](options.md#window) |
| `theme`, `storage`, `new_tab_page`, `workspace_picker` | [Options](options.md#startup-and-search) |
| `watch_config`, `default_profile` | [Options](options.md#behaviour) |
| `rendering` | [Options](options.md#rendering) |
| `chromium` | [Options](options.md#chromium) |
| `input` | [Options](options.md#input) |
| `gpu` | [Options](options.md#gpu) |
| `history` | [Options](options.md#history), [History](history.md) |
| `permissions`, `downloads` | [Options](options.md#permissions), [Permissions](permissions.md) |
| `cookies`, `print` | [Options](options.md#cookies) |
| `routing` | [Routing](routing.md) |

## Layers and reloading

Settings come in layers, each a YAML file with the same option names as [options.md](options.md):

1. the application file, `~/.config/browsah/config.yaml`;
2. the profile's file, `~/.local/share/browsah/profiles/<id>/profile.yaml` (see [profile.md](profile.md));
3. a workspace's file, `.../profiles/<id>/workspaces/<id>/workspace.yaml` (see [workspace.md](workspace.md)),
   for that workspace's window only.

A web app's file, `.../profiles/<id>/apps/<id>/app.yaml` ([webapp.md](webapp.md)), is the third layer in the same way, for that app's windows only. The upper layer is merged over the lower:

- maps merge key by key, so a profile can change one option inside `input` and keep the rest;
- a value or a list in the upper layer replaces the lower one;
- a key ending in `+` appends a list instead: `chromium: { flags+: ["--foo"] }` adds to the
  application's flags;
- application-only options (`watch_config`) are ignored in a profile or workspace, and options fixed
  per process (`chromium`, `gpu`, `rendering.mode`, `rendering.device_resolution`) are ignored in a
  workspace, with a warning.

A profile is its own process, so it may also set options that are fixed per process (`chromium`,
`gpu`, `rendering.mode`). See the result with `browsah config show --profile=<id>`, which also lists
the options the profile set.

**Reloading.** Unless `watch_config` is `false` (or `--no-watch` is given), the files are watched
and reloaded about 0.3 s after a change, even when an editor replaces the file. A message in the window
says what happened. An invalid file is reported and the running settings are kept.

| Takes effect | Options |
|---|---|
| immediately | `search_url`, `home_page` (new windows), `rendering.frame_rate`, `input.*` |
| next start (the message says so) | `chromium.*`, `gpu.*`, `rendering.mode`, `rendering.device_resolution`, `window.*` |

Permissions and downloads are described in [permissions.md](permissions.md). Cookie rules and `storage` are in [options.md](options.md). Web apps (one site in a window of its own, with its own launcher entry; their own `app.yaml`) are in [webapp.md](webapp.md).

Where links go is configured under `routing:` in any of these files: see [routing.md](routing.md).

Workspaces have theirs, [workspace.yaml](workspace.md).

The profile selector (which profile gets a link from another application) is described in [selector.md](selector.md).

Tab sweeping rules (which can be set at every layer and are inherited) and automatic tab sleeping (`sleep_tabs_after`) are described in [sweep.md](sweep.md).

Profiles have their own file, [profile.yaml](profile.md) (name, colour, default workspace, and any option overrides).

Command-line flags override the matching config options: `--mode=paint|accel`
(`rendering.mode`), `--backing=off` (`rendering.device_resolution`), a bare URL or `--url=URL`
(`home_page`). Any other `--flag` is passed to Chromium.
