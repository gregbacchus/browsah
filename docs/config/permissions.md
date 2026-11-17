# Permissions and downloads

## Permissions

A page can ask to use your camera or microphone, know your location, show notifications, read the
clipboard, share your screen or use MIDI devices. Browsah answers by the `permissions:` section of the
configuration ([options.md](options.md#permissions)): per kind, and per site, **allow**, **deny** or **ask**.
Like other options it can be set in `config.yaml`, a profile's `profile.yaml` or a workspace's
`workspace.yaml`, the lowest layer first and the higher ones overriding it. (To add site rules to a lower
layer's instead of replacing them, write `sites+:`.)

For a request the decision is, in order:

1. a **site rule** (`permissions.sites`) whose `match` fits the page's address and that says something about
   the kind (or has a `default`): the first such rule decides;
2. the setting for the kind (`permissions.camera`...), else `permissions.default` (which is `ask` unless you
   change it).

`allow` and `deny` are final. `ask` first looks at what you answered before for that site and kind (below); if
there is nothing, a question appears over the window: *Block* or *Allow*, and a box *Remember for this site*
(ticked). Closing the question with `Esc` counts as *Block* this time and is not remembered. A question goes
away by itself if the page moves on. A request for several things at once (camera and microphone) is one
question.

A permission lasts as long as the page it was given to: when a page of the same site loads again (a
reload, a new address on the site, another tab), the request is judged afresh by the configuration and
remembered answers. (Chromium would otherwise keep the first answer for the site for good and stop asking
Browsah, so a later `deny`, a forgotten answer or a changed rule would have no effect; Browsah clears what
Chromium kept whenever a page starts loading.)

Remembered answers are kept per profile (in its database), for the site and the kind. They never override an
explicit `allow` or `deny` in the configuration, only fill in for `ask`. Manage them from the command line:

```bash
browsah permissions list [--profile ID]                          # what has been remembered
browsah permissions reset [--origin https://x.example] [--profile ID] --yes   # forget one site's, or all
```

(`reset` is refused while the profile is running.)

**Notifications.** A page that is allowed to show notifications has them shown by the desktop (Chromium sends them
to `org.freedesktop.Notifications` itself), marked as coming from the profile's launcher entry
(`be.geee.browsah.<profile>`), so GNOME groups them under the profile's name and icon. They need the profile's
`.desktop` entry (`browsah profile add` writes it) to be named and iconed properly.

**What this does not do yet:** a notification's own icon is Chromium's logo, not the site's; clicking one is
handled by Chromium (the page's `onclick`) and does not select the tab that sent it; location needs a location service the system provides and may be unavailable;
sharing the screen needs a portal flow that is not built. The permission itself is decided as above.

## Downloads

Files go to `downloads.directory` (your Downloads folder by default; `~` is your home), under the name the
site suggests with anything unsafe removed (folders in the name are dropped); a name already taken becomes
`name (1).ext`, so nothing is overwritten. With `downloads.ask: true` a *Save as* question appears for each
file, starting in that folder. A message shows when a download starts and, when it is done, with a *Show in
folder* button. The folder and the question can be set per workspace.

### The downloads list

A downloads button appears in the header (a folder-download icon) once there is a download to show, and stays while the
list is not empty. It opens a list of the profile's recent downloads, newest first, the same in every window: a running
download shows a progress bar and the bytes received so far with a *Cancel* button; a finished one shows its size with
*Open the file* and *Show in folder* (the desktop's default handlers); a cancelled or failed one says so; a file that has
since been moved or deleted says that. *Remove* forgets a row and *Clear list* forgets all the finished ones; the files
themselves are never touched. Finished and stopped downloads are remembered between runs (the last 50 are shown); one
that was running when the browser closed is not.

