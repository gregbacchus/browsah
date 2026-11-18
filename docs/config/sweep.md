# Tab sweeping

Sweeping tidies up tabs you are no longer using, by rules you write (`sweep:`). It is off until `sweep.enabled` is `true`. The
rules are checked every `sweep.interval` (5 minutes by default). A simpler setting, `sleep_tabs_after`, puts idle background
tabs to sleep without any rules (see [Sleeping tabs automatically](#sleeping-tabs-automatically)).

## Where the rules are set

`sweep:` can be written in `config.yaml` (every workspace), in a profile's `profile.yaml` (that profile's workspaces) and in a
workspace's `workspace.yaml` or a [web app](webapp.md)'s `app.yaml`, and the layers combine like any other option (see
[Layers and reloading](README.md#layers-and-reloading)): a workspace **inherits** the profile's `sweep` section, and what it
writes itself wins key by key. So a profile can say how often to check and give the rules every workspace shares:

```yaml
# profile.yaml
sweep:
  enabled: true
  interval: 10m
  rules:
    - name: Stale searches
      match: { url: ["*.google.com/search*"] }
      idle_for: 1h
```

```yaml
# workspace.yaml of one workspace: the profile's rules apply, plus its own (`rules+` adds to the list)
sweep:
  rules+:
    - name: Old docs
      match: { url: ["docs.example.com/*"] }
      idle_for: 2d
      action: discard
```

Writing `rules:` (without the `+`) in the workspace replaces the inherited list, and `sweep: { enabled: false }` switches the
inherited sweeping off for that workspace. Windows with no workspace are not swept by rules (they have none to apply), a
closed workspace is swept by its inherited rules too, and a change to the rules (or to `sweep.interval`) applies to windows that
are already open when the configuration is reloaded.

## Sleeping tabs automatically

`sleep_tabs_after` is the simple form of a `discard` rule: any **background tab** that has not been used for this long is put to
sleep (its page is freed, the tab stays and shows its last screenshot, and it loads again when you select it).

```yaml
sleep_tabs_after: 30m       # null (or leaving it out) is off; a number and a unit: 30s, 15m, 2h, 7d
```

It is an ordinary option, so it can be set in `config.yaml`, a profile's `profile.yaml`, a workspace's `workspace.yaml` or an app's
`app.yaml`, the lowest layer winning (`sleep_tabs_after: null` in a workspace undoes a profile's). It needs no `sweep:`
section and works in windows with no workspace too; if `sweep:` rules exist, they are checked first and the sleep rule is
added after them. Details:

- **never the tab you are looking at**: the selected tab of every window is left awake, so the foreground window's tab and
  every other visible window's stay running. (The selected tab of a minimised window stays awake too: a sleeping tab wakes when it is
  selected, so there is no moment at which it could come back.)
- the same protections as sweeping: a tab with unsaved form input, one playing sound or video, and one still loading are not put to
  sleep;
- **pinned tabs may sleep** (sweep rules never touch them, because a rule can close; sleeping only frees the page);
- it is quiet: no message, unlike a `discard` rule you wrote yourself;
- the check runs every half of the time given, at least every 30 seconds and at most every `sweep.interval`, so `sleep_tabs_after: 1m` is
  honoured within about a minute and a half;
- a time that is not a number and a unit is reported on the terminal and sleeping stays off.

## When the workspace is not open

Sweeping does not need the workspace's window. While the profile is running, a workspace whose window is
closed is still swept: its remembered tabs are checked by the same rules, using how they were used before
the window closed (idle time keeps counting while it is closed). This happens about eight seconds after
the profile starts, so tabs that went idle while the browser was closed are dealt with at once, and then
on each workspace's interval. A swept tab is removed from the workspace and kept in the history of closed
tabs as `sweep:<rule name>`, just as for an open window; when you open the workspace you find what is left.

For a profile that is **not running** at all there is a command, to run from a cron job or a systemd timer:

```bash
browsah sweep --profile=work              # sweep every workspace of the profile that has rules
browsah sweep --profile=work --dry-run    # say what would be swept, change nothing
browsah sweep --workspace=mail            # only one workspace
```

(It refuses to run while the profile is running, because the running profile does it; `--dry-run` is always
allowed.)

Differences from sweeping an open window: nothing is being looked at and no page is running, so there is no
"tab being looked at" and no unsaved form input to protect; `discard` rules do nothing (the tabs are already
asleep, without a page); a tab with no recorded use is treated as used now, so it is not swept at once; and
pinned tabs are left alone.

## How use is measured

Each tab has a record of when it was used. Being shown counts, and so does working in it (typing,
clicking, scrolling); a tab you only have open in a background window counts for nothing.

- **Idle for** is the time since the tab was last used.
- **Score** adds up the tab's uses with older ones counting for less: a use `half_life` ago counts for
  half, two half-lives ago for a quarter. A tab used often keeps a high score for a long time after the
  last use, one looked at once fades quickly. Compare it with `score_below`.

A rule fires when **all** of its conditions hold: the address matches `match.url` (if given), the tab has
been idle for `idle_for` (if given) and its score is under `score_below` (if given). Give it at least
`idle_for` or `score_below`. The first rule that fires decides what happens. Examples, from the shortest
to the most elaborate:

```yaml
sweep:
  enabled: true
  rules:
    - match: { url: ["www.google.com/search*"] }     # a search page not touched for an hour
      idle_for: 1h
    - name: Rarely used
      idle_for: 7d                                    # a week unused, and not used much before it
      score_below: 0.5
      half_life: 1d
      action: discard
```

## What happens

- **`close`** closes the tab. It is kept in the history of closed tabs as *closed by sweep:<rule name>*,
  which tells it apart from a tab you closed yourself; `Ctrl+Shift+T` and the history window
  (`Ctrl+H`) bring it back. A short message says what was swept.
- **`discard`** keeps the tab where it is but frees its page: it shows its last screenshot, blurred, and loads again
  when you show it (the same as a tab restored from last time).

## What is never swept

- the tab you are looking at (in any window), a workspace's pinned tabs (from the rules; `sleep_tabs_after` may put them to sleep), and **the last tab of a workspace**: a sweep never
  leaves a workspace without a tab. (For a workspace whose window is closed, when every remembered tab would
  be swept and there is no pinned tab, the one used most recently is kept.);
- a tab with **unsaved form input**: a typed value in a text box, a changed checkbox or choice, or text in
  an editor (the tab is marked with an edit icon while that is so). Pages that fill a box in by script
  can look unsaved when they are not; they are protected, never swept by mistake;
- a tab that is **playing sound or video** (marked with a speaker icon) or is still loading.

Pages that use the microphone or camera without playing media, and sound produced by a page without a
media element, are not detected: write a rule that does not match such sites.

Use is remembered when a workspace is closed, so a tab that sat unused for a week while the browser was
closed is idle for a week. Changes to the rules apply to open windows when the configuration is reloaded, and to a workspace
when it is opened.
