# Workspace configuration (`workspace.yaml`)

A workspace is a window of tabs that belongs to a profile. It has its own directory,
`<profile directory>/workspaces/<id>/` (`~/.local/share/browsah/profiles/<profile>/workspaces/<id>/`),
containing `workspace.yaml`, this file. All workspaces of a profile are windows of the profile's one
process and share its application id, so they appear under the profile's icon in GNOME.

```bash
browsah workspace add mail --profile=work --name "Mail" --color "#2a6f97 #61a5c2"
browsah workspace list --profile=work
browsah workspace remove mail --profile=work --yes      # deletes the workspace directory
browsah --profile=work --workspace=mail [URL]           # open it (or raise it if it is open)
```

`--profile` defaults to `default`. A plain start of a profile opens the workspaces that were open
when it last ran (closing windows one at a time forgets them, except the last), else the profile's
`default_workspace`, else a window with no workspace. Such an **ephemeral window** (also made with
`Ctrl+Shift+N`) is not remembered. All options apply at startup unless stated otherwise.

## Options

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | text | the workspace id | Display name, shown in the window title. |
| `color` | colour pair | none | The workspace's accent colour: one colour, two in one string (`"#4a2f80 #7a56c2"`, light then dark), or a map with `light` and `dark`. |

```yaml
name: Mail
color:
  light: "#2a6f97"
  dark: "#61a5c2"
```
