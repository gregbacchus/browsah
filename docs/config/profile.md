# Profile configuration (`profile.yaml`)

Each profile has its own directory, `$XDG_DATA_HOME/browsah/profiles/<id>/`
(`~/.local/share/browsah/profiles/<id>/` by default), containing:

| Path | What it is |
|---|---|
| `profile.yaml` | this file; hand edited |
| `cef/` | Chromium's data for the profile: cookies, local storage, caches |

Profiles are managed with the command line:

```bash
browsah profile install work                                    # write the launcher entry and icon again (e.g. after an update)
browsah profile add work --name "Work" --color "#4a2f80 #7a56c2"   # light then dark
browsah profile list
browsah profile remove work --yes            # deletes the directory; --keep-data keeps it
browsah --profile=work [URL]                 # run a profile, or hand it a link if it is running
```

`add` also writes a `.desktop` entry and an icon, so the profile appears in GNOME as its own
application (application id `be.geee.browsah.<id>`) and can be chosen as the default
browser. A profile is one process: launching it again, or opening a link with it, hands the work
to the running process. The ids `default` and `selftest` exist without being created (`selftest`
is used by `--selftest` and the benchmarks so they never touch a real profile). `default` is what a plain `browsah` starts. It is listed
(`profile list`, the selector) once it exists, that is once it has been used, or when it is the only profile; with other profiles and no
`default` directory it is not listed. `browsah profile remove default --yes` removes it like any other profile once there is another one
(starting a plain `browsah` creates it again).

`name`, `color` and `default_workspace` are read at startup; overrides of other options apply as described in [Layers and reloading](README.md#layers-and-reloading). Unknown keys are ignored (in release builds).

## The generated icon

`browsah profile add` (and `profile install`), `browsah app add` for an app with no icon of its own, and `browsah selector install`
write a scalable icon: a circle in the **primary colour** with four swishes. The primary colour is the profile's `color` (its light
colour); an app uses its own `color` (from the site's `theme_color` when it was created), else its profile's; with none it is a
default blue (`#2b5db2`, the middle of the profile selector's blue), which is also the selector's icon. The circle shades a little
lighter toward the top left and a little darker toward the bottom right. The swishes are a tint of the primary colour toward white
on a dark circle or toward black on a light one, never pure white or black, so they always contrast without looking stark.

Icons are written when the entry is installed, and a running profile rewrites its stale generated icons (its own, the selector's and
its apps') at start, so changing `color` shows after the next start. An app's own icon file is never replaced.

## Overriding configuration

Besides the options below, `profile.yaml` can contain any option from [options.md](options.md)
(except application-only ones such as `watch_config`); its values are merged over the application
config for this profile. Maps merge, values and lists replace, and a key ending in `+` appends a
list. Details and what reloads live: [Layers and reloading](README.md#layers-and-reloading).

```yaml
search_url: "https://www.google.com/search?q={query}"   # this profile uses another search engine
input:
  touchpad_speed: 8.0                                    # only this option changes in `input`
chromium:
  flags+: ["--force-dark-mode"]                          # appended to the application's flags
```

## Options

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | text | the profile id | Display name, used in the `.desktop` entry (`Browsah (Work)`) and the icon's initial. |
| `default_workspace` | text | none | The [workspace](workspace.md) opened when the profile starts with no workspace named and none remembered from the last run. Must be the id of an existing workspace. |
| `color` | colour pair | none (the icon is the default blue) | The profile's icon colour (the light one, see [the generated icon](#the-generated-icon)), which also tints the windows that have no colour of their own (windows with no workspace, and workspaces that set none); a workspace's own colour wins in its window. Either one colour, two colours in one string (`"#4a2f80 #7a56c2"`, light then dark), or a map with `light` and `dark`. Colours are `#rgb` or `#rrggbb`. |

```yaml
name: Work
color:
  light: "#4a2f80"
  dark: "#7a56c2"
```
