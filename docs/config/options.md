# Options

All options apply at startup. Defaults are shown; omit a key to get its default.

## Startup and search

| Key | Type | Default | Description |
|---|---|---|---|
| `home_page` | string (URL) | `https://duckduckgo.com/` | The page opened in a new window. An empty value uses the default. A URL given on the command line takes priority. |
| `search_url` | string (URL) | `https://duckduckgo.com/?q={query}` | Search used when the address bar text is not a URL or a host name. `{query}` is replaced by the percent-encoded text. A value without `{query}` is rejected and the default is used. |

```yaml
home_page: https://duckduckgo.com/
search_url: https://duckduckgo.com/?q={query}
```

## `window`

| Key | Type | Default | Description |
|---|---|---|---|
| `window.width` | integer (px) | `1280` | Initial window width, in logical pixels. Clamped to 400 to 16384. |
| `window.height` | integer (px) | `800` | Initial window height, in logical pixels. Clamped to 300 to 16384. |

```yaml
window:
  width: 1280
  height: 800
```

## `rendering`

How pages are drawn. See [the research notes](../research/summary.md) for the reasoning and the
measured costs.

| Key | Type | Default | Description |
|---|---|---|---|
| `rendering.mode` | `accelerated` or `readback` | `accelerated` | `accelerated` imports Chromium's GPU frames directly (no copy). `readback` copies each frame through CPU memory: slower (about 7x the main-process CPU at 2880x1568 in our test) but a fallback if accelerated frames never arrive on your GPU. |
| `rendering.frame_rate` | integer (fps) | `60` | Maximum frames per second for the page. Clamped to 1 to 120. Lower values save power on animated pages. |
| `rendering.device_resolution` | bool | `true` | Render at the screen's physical resolution on HiDPI displays. With `false`, pages render at 1x and are stretched, which looks blurry. |

```yaml
rendering:
  mode: accelerated
  frame_rate: 60
  device_resolution: true
```

## `chromium`

| Key | Type | Default | Description |
|---|---|---|---|
| `chromium.flags` | list of strings | empty | Extra Chromium command-line switches, for example `--disable-features=Foo`. `--ozone-platform=wayland` is always applied unless a flag here or on the command line sets `--ozone-platform`. Use with care: switches can break rendering. |

```yaml
chromium:
  flags: []
```

## `input`

| Key | Type | Default | Description |
|---|---|---|---|
| `input.wheel_notch` | number | `100` | CSS pixels scrolled by one mouse-wheel notch, 10 to 1000. Applies at startup. |
| `input.touchpad_speed` | number | `3.0` | Multiplier on touchpad scrolling distance (and on the coasting after a flick), 0.1 to 20. Raise it if scrolling feels slow compared with other browsers. Applies at startup. |
| `input.touchpad_acceleration` | number | `0.05` | Extra multiplier per pixel of one touchpad scroll event, 0 to 1: an event of `d` px is scaled by `1 + acceleration * d`, so fast swipes (large events, 30 to 40 px) scroll disproportionately further than slow ones (5 to 10 px). `0` is linear. Applies at startup. |

```yaml
input:
  wheel_notch: 100
  touchpad_speed: 3.0
  touchpad_acceleration: 0.05
```

## `gpu`

| Key | Type | Default | Description |
|---|---|---|---|
| `gpu.workarounds` | bool | `true` | Apply per-GPU environment workarounds before Chromium starts. Currently: `AMD_DEBUG=nodcc` on AMD GPUs, without which Chromium delivers no accelerated frames on native Wayland. Set `false` to run without them (to test, or if your driver no longer needs them). |

```yaml
gpu:
  workarounds: true
```
