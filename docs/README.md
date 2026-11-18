# browsah documentation

New here? Start with the [README](../README.md), then [`examples/`](../examples/): every option appears in
a worked example with a comment.

| Page | What it covers |
|---|---|
| [Configuration overview](config/README.md) | Which files exist, how the layers merge, live reload versus restart, errors, command-line overrides |
| [Seeing every option](config/README.md#seeing-every-option) | The *Documentation* window, the *all options* copies of your files, `browsah config full` |
| [Value types](config/value-types.md) | Colours, address patterns, durations, actions, list merging, YAML notes |
| [Options](config/options.md) | The application file (`config.yaml`): search, theme, window, rendering, chromium, input, history, permissions, downloads, cookies, print, gpu |
| [Profiles: `profile.yaml`](config/profile.md) | Name, colour, default workspace; the `browsah profile` command |
| [Workspaces: `workspace.yaml`](config/workspace.md) | Navigation controls, pinned tabs, popups, modals, groups; the `browsah workspace` command |
| [Web apps](config/webapp.md) | `app.yaml`: one site in a window of its own, its launcher entry and icon; installing from a page |
| [Routing](config/routing.md) | Where links go: rules, redirectors, sign-ins, modals, `browsah route test` |
| [Permissions and downloads](config/permissions.md) | What pages may do, remembered answers, the downloads folder |
| [History](config/history.md) | What is remembered, retention, address-bar suggestions, the history window |
| [Chrome extensions](config/extensions.md) | Loading extensions, installing from the Chrome Web Store, native messaging, 1Password |
| [The right-click menu](config/context-menu.md) | What the menu offers for links, images, media, text, fields and the page; `context_menu` options |
| [The log](config/log.md) | The log file, `browsah log`, what is logged at each level |
| [Sweeping tabs](config/sweep.md) | Closing or discarding tabs that are not used |
| [The profile selector](config/selector.md) | Which profile gets a link from another application |

This documentation is also in the program: main menu > *Documentation* shows it rendered as a site (the pages are compiled in, so it matches the version you run).

Examples (each is checked by the test suite, so every key in it is real):

| File | Shows |
|---|---|
| [`examples/config.yaml`](../examples/config.yaml) | the application file, every option |
| [`examples/profile.yaml`](../examples/profile.yaml) | a profile's file, with overrides and routing |
| [`examples/workspace.yaml`](../examples/workspace.yaml) | a workspace's file: navigation, pinned tabs, sweeping, routing |
| [`examples/webapp.yaml`](../examples/webapp.yaml) | a web app (`app.yaml`) |

These pages are **exhaustive and must stay in sync with the code.** Tests fail when an option is missing from
the docs, documented but gone, or missing from the examples. See [`AGENTS.md`](../AGENTS.md).

Design notes and measurements are in [research/](research/README.md); what comes next is in the
[roadmap](roadmap.md).
