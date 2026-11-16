# Profile configuration (`profile.yaml`)

Each profile has its own directory, `$XDG_DATA_HOME/browsah/profiles/<id>/`
(`~/.local/share/browsah/profiles/<id>/` by default), containing:

| Path | What it is |
|---|---|
| `profile.yaml` | this file; hand edited |
| `cef/` | Chromium's data for the profile: cookies, local storage, caches |

Profiles are managed with the command line:

```bash
browsah profile add work --name "Work" --color "#4a2f80 #7a56c2"   # light then dark
browsah profile list
browsah profile remove work --yes            # deletes the directory; --keep-data keeps it
browsah --profile=work [URL]                 # run a profile, or hand it a link if it is running
```

`add` also writes a `.desktop` entry and an icon, so the profile appears in GNOME as its own
application (application id `nz.co.bacchus.browsah.<id>`) and can be chosen as the default
browser. A profile is one process: launching it again, or opening a link with it, hands the work
to the running process. The ids `default` and `selftest` exist without being created (`selftest`
is used by `--selftest` and the benchmarks so they never touch a real profile).

All options apply at startup. Unknown keys are ignored (in release builds).

## Options

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | text | the profile id | Display name, used in the `.desktop` entry (`Browsah (Work)`) and the icon's initial. |
| `color` | colour pair | derived from the id | The profile's icon colour. Either one colour, two colours in one string (`"#4a2f80 #7a56c2"`, light then dark), or a map with `light` and `dark`. Colours are `#rgb` or `#rrggbb`. |

```yaml
name: Work
color:
  light: "#4a2f80"
  dark: "#7a56c2"
```
