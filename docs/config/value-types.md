# Value types

The *Type* column of every option table uses these names.

## Basics

| Type | Accepts |
|---|---|
| bool | `true` or `false` |
| integer, number | `1280`, `0.025`. Out-of-range values are replaced by the nearest usable one (the option says the range) |
| text | any string; quote it when it contains `:`, `#`, `{` or starts with a special character |
| list of text | `["a", "b"]` or one entry per line with `- `. |
| `allow` / `deny` / `ask` | one of those words |
| `icon` or `full` | pinned tab size: icon only or icon and title at regular tab width |

## Colours

`#rgb` or `#rrggbb` (quote it in YAML, otherwise `#` starts a comment). Where a colour has a light and a
dark form (a profile's or workspace's `color`) it is written as one of:

```yaml
color: "#4a2f80"                 # one colour, used for both
color: "#4a2f80 #7a56c2"         # light then dark, in one string
color:
  light: "#4a2f80"
  dark: "#7a56c2"
```

On the command line (`--color`) the same one-string form is used.

## Address patterns

Used by a pinned tab's `scope`, `history.exclude`, `cookies.rules[].match`, `permissions.sites[].match`,
`routing.rules[].match.url`, `routing.unwrap[].url` and `sweep.rules[].match.url`.

| Pattern | Matches |
|---|---|
| `example.com` | exactly that host (any path, scheme and port ignored) |
| `*.example.com` | any sub-domain, but not `example.com` itself |
| `**.example.com` | the domain and every sub-domain |
| `example.com/app/*` | that host with a path starting `/app/` (`*` is any characters; `example.com/` is the site root only) |

Only web addresses are matched; the full rules and edge cases are under *Scope patterns* in
[workspace.md](workspace.md#options). `browsah route test <url>` shows what a routing rule does with
an address.

## Durations

Whole numbers with a unit: `30s`, `15m`, `2h`, `7d`. Zero is rejected where the option says it must be
longer than zero. Used by `sweep.interval`, `sweep.rules[].idle_for` and `sweep.rules[].half_life`.

## Actions

Where a link goes (`routing.rules[].open`, `routing.external`, `routing.app_sign_in`): `allow`,
`workspace:<id>`, `ephemeral`, `modal`, `system`, `profile:<id>[/workspace:<id>]`, `block`. The table is in
[routing.md](routing.md#options). What a tab does when swept (`sweep.rules[].action`): `close` or `discard`.

## Merging layers

A profile's or workspace's file uses the same option names as `config.yaml` and is merged over it:
maps merge key by key; a value or a list replaces; a key ending in `+` appends to a list
(`chromium: { flags+: ["--foo"] }`, `history: { exclude+: ["*.bank.example"] }`). See
[Layers and reloading](README.md#layers-and-reloading).

## Paths and the environment

`~` at the start of a path is your home folder. Config files honour `$XDG_CONFIG_HOME`; profile data
honours `$XDG_DATA_HOME`.
