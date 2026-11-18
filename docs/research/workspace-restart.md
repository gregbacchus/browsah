# Why did some workspaces disappear on restart?

**Date:** 2026-11-18

The profile already wrote workspace ids to `state.yaml` as windows opened.
An individual close request removed its id while other windows were open.
Closing several windows individually therefore did not preserve the session.
A cold launch with `--workspace=ID` also bypassed the remembered list.

**Decision:** Quit profile saves each window's tabs and closes its browsers
through the same cleanup as a window close. It leaves the remembered workspace
list intact and checks for unsaved changes before closing any window. On a cold
launch, restore remembered workspaces before opening the named workspace so
that the requested window comes forward and receives any URL.

The desktop's own Quit (the dock's "Quit N Windows") closes the windows one
after another, so each close except the last forgot its workspace. Windows closed
within three seconds of the last window's close are now remembered again, so a
quit through the dock restores them like Quit profile. Not reproduced here: a real
dock menu; the close sequence was reasoned from the code, not observed.

The in-window picker dismisses its popover before activating the workspace
action, which presents the existing window. This keeps the request in the
user's click handler while releasing the popover's input grab.

**Verified on GNOME Wayland:** The focused `picker,jsdialog` self-test checked
that selecting an already open workspace activates that window without creating
another. A disposable profile with two open workspaces was quit and launched
again, both with and without a workspace argument. Both windows reopened on
each launch. The URL on the named launch went to the requested workspace.

To repeat the focus checks, run `scripts/test selftest picker,jsdialog` with
temporary XDG data, config and cache paths. For the restart check, use a
disposable profile with two workspaces and launch both. Choose **Quit profile**,
then launch the same profile without arguments. Quit again and repeat with
`--workspace=ID` and a local test URL.

## Closing speed: screenshots

A closing window saves a picture of its selected tab, and the PNG encode ran on the main thread: 150 to 210 ms per
window on the development machine at scale 2, and 1.28 s for one user-reported window (a larger one; not measured here).
Pictures are now shrunk to at most 1024 px on the longest side (a whole-number box average) and written on a background
thread, which the process joins before it exits. On the two-window test profile each write went from about 160 ms to
about 46 ms. Only the selected tab of each window is captured at close; other tabs are captured when deselected, now also
off the main thread. A sleeping tab shows its picture blurred, so the lower resolution is not visible there; a
sweep-discarded tab still writes its picture before the page is replaced.
