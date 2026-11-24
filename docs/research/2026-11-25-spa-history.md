# History flooded by single-page apps (2026-11-25)

**Problem:** Google Maps rewrites its address on every pan and zoom (`history.replaceState`), and browsah recorded a history visit
for each one.

**Cause (measured, this machine):** CEF calls `on_loading_state_change` (loading, then not loading) for every same-document
navigation, `pushState` and `replaceState` included. A test page making 40 `replaceState` and 5 `pushState` calls gave 47
loading-state pairs but only 2 `on_load_start` calls (documents). `Shell` recorded a visit on every "finished loading".

**Fix:** a visit is recorded at the end of a load only if `on_load_start` (a document, `ViewEvent::NavStart`) happened since the last
one (`Tab::new_document`). Same-page address changes are not visits. Chrome does record some `pushState` pages; we do not (a map's
"place" pushes would otherwise return); say so if you want those back.

**Not changed (suspected, not measured):** other handlers of `ViewEvent::Loading` (zoom re-apply, refocus, `load_cycles`) also run once per
same-document change; they are cheap but could be given the same filter.

Test: `scripts/test selftest spahistory` (also ran `history,navhistory`).
