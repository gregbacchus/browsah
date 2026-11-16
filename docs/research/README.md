# Research notes

Findings that informed design decisions, kept here (not in the top-level README) so they survive
README tidy-ups. Each note is dated, records the environment it was measured on, and separates
**measured** results from **suspected** causes.

**Start with [summary.md](summary.md)** (decisions, required workarounds, key numbers, open risks).
Next steps are in [../roadmap.md](../roadmap.md).

| Note | Date | Question | Short answer |
|---|---|---|---|
| [cef-osr-wayland-spike.md](cef-osr-wayland-spike.md) | 2026-11-16 | Can a custom GTK4 shell on Wayland display CEF's windowless output fast enough that CEF never owns a window? | Yes, with zero-copy dmabuf at 60 fps. Needs a Mesa DCC workaround on AMD and a HiDPI backing-scale fix. |
| [cef-multi-tab-cost.md](cef-multi-tab-cost.md) | 2026-11-16 | What do many windowless tabs cost, and how fast can sleeping tabs wake? | Hidden tabs cost ~0 W and ~45 MB each; each extra visible busy tab ~+2.7 W; wake 8-14 ms (hidden/frozen), 70-80 ms (discarded). Found and fixed a 100 % CPU main-thread spin (~10 W at idle): run CEF on its own thread. |
| [cef-input.md](cef-input.md) | 2026-11-16 | Can a windowless CEF page get full input (mouse, keyboard, IME, clipboard, popups, menus) and how fast? | Yes (46 scripted checks pass). Input to next frame at GTK ~7-8 ms (dmabuf), ~11 ms (readback). Wayland clipboard needs a GTK bridge. Real-device input unverified. |
| [cef-variable-frame-rate.md](cef-variable-frame-rate.md) | 2026-11-16 | Can unfocused visible tabs run at a lower frame rate to save power, and how fast can they speed back up? | Yes: 10 busy tabs 28 W at 60 fps; focused at 60 + 9 at 15 fps 22 W (-22 %), at 5 fps 15 W (-48 %). CEF honours the cap; back to 60 fps in 3-110 ms (30/15/5 fps floor). |
| [browser-tabs.md](browser-tabs.md) | 2026-11-17 | How are tabs built, and do hidden tabs really stop rendering in the real app? | One `BrowserView` per tab in an `AdwTabView`; map/unmap drives `was_hidden`; hidden tab got 0 frames in 1 s, wake to a fresh frame within the 3 s check. 67 scripted checks. Real-use cost of many tabs not re-measured. |
| [profiles-workspaces-design.md](profiles-workspaces-design.md) | 2026-11-17 | How should profiles, workspaces, web apps, routing and cascading config work, and what are the hard limits? | Design agreed, spikes listed. One process per profile (Chromium locks the cache dir); the default browser is just a profile's `.desktop`; the stock CEF build has **no H.264/AAC** (measured); source app of an external link is not reliably known. |

## Conventions

- One file per question. Put the answer at the top, the evidence below.
- Record machine, GPU/driver, compositor, toolkit and CEF versions with every measurement.
- Link sources; mark anything unverified as such.
- Reproduction commands must be copy-pasteable from the repo root.
