# The page's right-click menu

What the menu offers depends on what you click. Every item has a Lucide icon (see [the icons note](../research/icons-lucide.md));
workspaces in the sub-menus have their colour badge instead. Items that do not apply are left out, not greyed out, except where
the state matters (Undo, Paste).

| Click on | Items |
|---|---|
| **A link** | *Open in <workspace, app or profile>* (only when a [routing](routing.md) rule claims the link: what a plain click would do), *Open in new tab*, *Open in new window*, *Open in workspace ▸* (workspaces and web apps), *Open in other profile ▸*, *Open as popup*; *Copy link address*, *Copy clean link* (redirectors unwrapped, `utm_*`, `fbclid`, `gclid` and other tracking parameters removed), *Copy link text*, *Save link as…*, *Bookmark link* |
| **An image** | *Open image in new tab*, *Save image as…*, *Copy image*, *Copy image address*, *Search for this image* (only when `context_menu.image_search_url` is set). An image that is also a link gets the link items first |
| **Video or audio** | *Play* or *Pause*, *Mute* or *Unmute*, *Loop*, *Show controls*, *Open video in new tab*, *Save video as…*, *Copy video address* |
| **Selected text** | *Copy*, *Search for "…"* (with `search_url`), *Open …* (when the selection reads like an address), *Copy link to selected text* (a `#:~:text=` link), *Select all* |
| **An editable field** | spelling suggestions and *Add to dictionary* (when the spell checker has any), *Undo*, *Redo*, *Cut*, *Copy*, *Paste*, *Delete*, *Select all* |
| **The page** | Back, Forward, Reload (Stop while loading) as a row, *Reload ignoring the cache*, *Bookmark this page* (a workspace with bookmarks on), *Find in page*, *Save as PDF…*, *Print…*, *Copy page address*, *Install as web app…* (in regular browser windows), *Send to workspace ▸*, *Open frame in new tab* and *Copy frame address* (inside a frame), *View page source* |
| **Anything** | *Inspect* (opens the developer tools on the element under the pointer) |

Back and Forward in the menu use the tab's saved list as well as the browser's own (see [tab history](../research/tab-history.md)).
A modal's page gets the same menu without the items that act on a tab (bookmark, find, print, install, send to workspace).
*Open in new tab* leaves you on the page you are on by default; *Search for this image* and *Search for "…"* switch to the new tab.

## Options

`context_menu` is an ordinary layered option (`config.yaml`, `profile.yaml`, `workspace.yaml`, `app.yaml`, the lowest layer winning);
a change applies to the next menu you open.

| Key | Type | Default | Description |
|---|---|---|---|
| `context_menu.developer` | bool | `true` | Show *View page source* and *Inspect*. |
| `context_menu.links_in_background` | bool | `true` | *Open in new tab* (and *Open image in new tab* and the like) leaves the current page selected. `false` switches to the new tab. |
| `context_menu.image_search_url` | text or `null` | `null` | Where *Search for this image* goes; `{url}` is the image's address, percent-encoded. `null` leaves the item out (an image address is sent to that service). A value without `{url}` is reported and ignored. |
| `context_menu.hide` | list of item ids | `[]` | Items to leave out, by id (below). An id that is not an item is reported and ignored. |

```yaml
context_menu:
  developer: true
  links_in_background: true
  image_search_url: "https://lens.google.com/uploadbyurl?url={url}"   # or https://tineye.com/search?url={url}
  hide: ["open-popup", "print"]
```

Item ids for `hide`:

- Page: `back`, `forward`, `reload`, `stop`, `hard-reload`, `bookmark`, `find`, `save-pdf`, `print`, `copy-address`, `install-app`, `send-tab`, `open-frame`, `copy-frame-address`, `view-source`, `inspect`
- Links: `open-routed`, `open-tab`, `open-window`, `open-workspace`, `open-profile`, `open-popup`, `copy-link`, `copy-clean-link`, `copy-link-text`, `save-link`, `bookmark-link`
- Images and media: `open-image` (also the media "open in new tab"), `save-image` (also media "save as"), `copy-image`, `copy-image-address` (also media "copy address"), `search-image`, `media-play`, `media-mute`, `media-loop`, `media-controls`
- Text and fields: `copy`, `search-web`, `open-selection`, `copy-text-link`, `select-all`, `undo`, `redo`, `cut`, `paste`, `delete`, `spelling`, `add-word`

## Notes

- *Copy image* fetches the picture again through the page's network stack at full size and puts it on the clipboard as an image
  (the page's own copy cannot reach the Wayland clipboard).
- *Save … as…* hands the address to the download handler, so the folder question and the downloads list work as for any download.
- *Paste* inserts the clipboard's text; there is no separate "paste as plain text" because it is the only kind (see
  [the clipboard note](../research/cef-input.md)).
- *Copy link text* and the media items find the element under the pointer with a short script, so an overlay that covers the link
  (a transparent layer on a video player) can make them do nothing, or act on the first video on the page.
