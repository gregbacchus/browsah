# Tab sweeping

Sweeping tidies up tabs you are no longer using, by rules you write for a workspace (`sweep:` in
[workspace.yaml](workspace.md)). It is off until `sweep.enabled` is `true`. The rules are checked every
`sweep.interval` (5 minutes by default) while the workspace's window is open.

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

- the tab you are looking at, and a workspace's pinned tabs;
- a tab with **unsaved form input**: a typed value in a text box, a changed checkbox or choice, or text in
  an editor (the tab is marked with an edit icon while that is so). Pages that fill a box in by script
  can look unsaved when they are not; they are protected, never swept by mistake;
- a tab that is **playing sound or video** (marked with a speaker icon) or is still loading.

Pages that use the microphone or camera without playing media, and sound produced by a page without a
media element, are not detected: write a rule that does not match such sites.

Use is remembered when a workspace is closed, so a tab that sat unused for a week while the browser was
closed is idle for a week. Changes to the rules apply when the workspace is opened again.
