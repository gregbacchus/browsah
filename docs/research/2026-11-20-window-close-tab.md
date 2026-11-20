# A tab whose page calls window.close() stayed open (2026-11-20)

**Answer:** `ViewEvent::Closed` (sent from `on_before_close`) was acted on in `Shell::handle` only for tabs the *person*
was closing (`Shell::closing`, filled by `begin_close`). When a page closed itself (`window.close()`, as sign-in popups do) the
browser was destroyed, nothing removed the tab: a frozen last frame, a spinner that never stops, no events. Modal popups already
handled `Closed` (`Modal::handle` dismisses), so only tabs were affected. `do_close` is not overridden; CEF's default is right
for windowless browsers, and `on_before_close` does fire.

**Fix:** in `Shell::handle`, a `Closed` for a tab that is not in `closing` (and not caused by `close_all`, flag `all_closing`)
removes the tab with `close_tab` and then the page with `tab_view.close_page` (which finds no tab, so asks nothing).

## Evidence (measured)
- Environment: headless weston, release build, selftest profile, CEF 156.
- `BROWSAH_SELFTEST_ONLY=opener`: before the fix, "the tab closes when its page calls window.close()" fails (`tabs=2`); after the fix 18 of 18 pass.
- The `modals` section (self-closing modal popup) passes its self-close checks. "closing a modal removes it" fails with and
  without this change (measured both ways): a separate, earlier problem, not investigated.

## Not verified
- Real sign-in flows (OAuth sites), an unselected popup tab, a popup tab in an app window or a window with several tabs of groups.
- Pages closed by Chromium for other reasons (renderer crash does not call `on_before_close`).

Reproduce: `scripts/test selftest opener` (needs a Wayland display; `weston --backend=headless`). Page: `bench/pages/popup-close.html`.
