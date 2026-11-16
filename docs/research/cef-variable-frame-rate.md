# Variable frame rate for windowless tabs: how much does it save, and what does it cost?

**Date:** 2026-11-16 · **Code:** `--bench=fps` in [`src/bench/tabs.rs`](../../src/bench/tabs.rs) ·
Same machine and versions as [cef-osr-wayland-spike.md](cef-osr-wayland-spike.md); builds on
[cef-multi-tab-cost.md](cef-multi-tab-cost.md), which found that power scales with visible,
animating tabs (~2.7 W per extra visible busy tab at 60 fps).

## Question

Could tabs the user isn't interacting with (several visible at once, e.g. split view) run at a
lower frame rate to save power, how much would it save, and how quickly can a tab go back to
full rate when the user touches it?

## Answer

- **Yes, and the saving is large.** Ten visible busy tabs at 60 fps cost **28.1 W**; with the
  focused tab at 60 fps and the other nine at **15 fps: 21.9 W (-22 %)**, at **5 fps: 14.7 W
  (-48 %)**. For reference, one tab at 60 fps with the other nine hidden was 11.3 W.
- **CEF honours the cap exactly.** `set_windowless_frame_rate(n)` takes effect on a running
  browser, and the achieved rate matched the cap (15.3 fps at a 15 fps cap, 5.2 at 5), as
  long as the machine can sustain it.
- **Ramp-up is fast enough, with a floor.** Raising a tab back to 60 fps delivered its first
  frame after about **110 ms from 5 fps, 45 ms from 15 fps and 3 ms from 30 fps**, then ran at
  ~60 fps. So a floor of **~15 fps for visible panes** is imperceptible on wake; 5 fps is
  noticeable (about a tenth of a second) unless the tab is raised early (pointer-enter).
- **At 60 fps the machine could not keep up** with ten busy tabs: they achieved only 39 fps
  each with the GPU at 100 %. A cap is useful even before power is considered.

## Method

`--bench=fps` (10 tabs, all visible, `live` page: 360 animated cells plus a 100 ms timer),
native Wayland, accelerated mode, 2880x1568 per tab, CEF on its own UI thread. For each row the
cap is set with `BrowserHost::set_windowless_frame_rate` (posted to CEF's UI thread), the system
settles for 3 s, then CPU, package power, GPU busy and PSS are sampled for 6 s and the frames
CEF actually delivered are counted (paint callbacks per tab per second). Same measurement
caveats as the multi-tab note: whole-APU package power from one sensor, about +/-1 W noise,
laptop on the charger, single run per row.

Ramp-up: tab 1 is capped at 5, 15 or 30 fps for 2.5 s, then set to 60 fps; the time to its next
delivered frame is taken, then frames are counted over the following 500 ms. Four trials per
starting rate.

## Results

| Scenario | Achieved fps per tab | Tab 0 fps | Main CPU % | Children CPU % | Power W | GPU busy % |
|---|---|---|---|---|---|---|
| All 10 visible, 60 fps cap | 39.4 | 39.5 | 16.2 | 463 | 28.1 | 100 |
| All 10 visible, 30 fps cap | 30.7 | 30.7 | 13.5 | 337 | 28.0 | 80 |
| All 10 visible, 15 fps cap | 15.3 | 15.3 | 8.7 | 250 | 17.7 | 42 |
| All 10 visible, 5 fps cap | 5.2 | 5.2 | 4.2 | 133 | 9.3 | 15 |
| Tab 0 at 60, 9 others at 15 | 19.8 | 61.0 | 11.3 | 258 | 21.9 | 55 |
| Tab 0 at 60, 9 others at 5 | 10.7 | 60.8 | 8.8 | 144 | 14.7 | 34 |
| Tab 0 at 60, 9 hidden | 6.1 | 60.7 | 6.5 | 47 | 11.3 | 23 |

(PSS was 1380-1400 MB in every visible row and 1057 MB with nine hidden; frame rate does not
change memory.)

Ramp-up from a low cap to 60 fps:

| From | First frame after the change (median, 4 trials) | Frames in the next 500 ms |
|---|---|---|
| 5 fps | 110 ms (103-113) | ~62 fps |
| 15 fps | 45 ms (37-54) | ~62 fps |
| 30 fps | 3 ms (3-5) | ~59 fps |

## Interpretation

1. **Power follows frame rate once the machine is not saturated.** From 30 to 15 to 5 fps the
   package power fell 28.0 to 17.7 to 9.3 W, and children CPU and GPU busy fell in proportion.
   The 60 and 30 fps rows have the same power only because 60 fps is unattainable here (GPU
   100 % busy, 39 fps achieved).
2. **Ten tabs at 5 fps cost about the same as one tab at 60** (9.3 W vs 9.7 W for one busy tab
   in the multi-tab note). A background pane at 5 fps is nearly free, though not zero; a hidden pane costs nothing measurable.
3. **The focused tab is unaffected:** tab 0 held ~61 fps in all mixed rows.
4. **The wake latency is the old frame period.** Chromium is waiting for its next scheduled
   frame at the low rate, so raising the cap costs up to one old interval (200 ms at 5 fps,
   67 ms at 15 fps; the measured medians are about half of that). The fix is policy: raise on
   pointer-enter, scroll, key press or touch, ideally before the click, and keep a floor.

## Suggested policy for a tab strip or split view

| Tab state | Cap |
|---|---|
| Focused, or input in the last ~2 s | 60 (or the display rate) |
| Visible, unfocused | 15 (30 for content that is clearly video or audio-playing) |
| Visible but tiny, partly covered or offscreen in a scroller | 5 |
| Hidden | not rendering (`was_hidden`), already free |

Ease back down after a second or two of inactivity rather than snapping, and exempt tabs that
are playing video or audio (their content rate must not be capped below 24-30).

## Caveats

- **CEF side only.** Non-focused tabs' frames are counted but not imported into GTK or
  composited in this benchmark, so the real saving would be somewhat larger once the GTK and
  compositor work for each extra visible pane is included. That cost was not measured.
- **One synthetic page** (compositor animations plus a timer). Real pages differ: video has a
  required content rate, and pages that only repaint on interaction already cost ~0.
- **Single run per row, one machine, +/-1 W power noise,** laptop on the charger. The large
  steps here (28 to 9 W) are well outside that noise, the small ones are not.
- The 60 fps row is limited by the GPU, not the cap, so it measures saturation.
- Detection of "video" or "the user is interacting" is not built or tested.

## Reproduce

```bash
cargo build --release
export LD_LIBRARY_PATH=$PWD/target/release
./target/release/browsah --mode=accel --bench=fps --tabs=10 --tab-mode=live
```

Takes about three minutes and prints the tables above.

## Open questions

- Real GTK/compositor cost of several visible panes, with their frames imported and drawn.
- Does an `external_begin_frame_enabled` pull model (driving frames from GTK's frame clock and
  skipping frames per pane) save more than a cap, and does it improve pacing?
- Does the cap reduce JavaScript/animation work in the page, or only rendering? (Animation
  callbacks follow the frame rate, so it likely does; not measured separately.)
- Video detection and heuristics for "interaction" (pointer hover, scroll, focus, audio).
- Rates above 60 on a 120 Hz display: CEF's documented maximum is unclear for this version.
