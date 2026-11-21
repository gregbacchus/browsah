# Why the tab menu always said "No cookies blocked" (2026-11-21)

## Question

The tab menu's cookie item always reported no cookies, even on sites that surely set some. Is the detector broken?

## Finding (code reading; not run against a page)

- The item counts hosts whose cookies the **rules blocked** (`Shared::blocked_cookies`, filled by `CookieRules::can_send_cookie` and
  `can_save_cookie` in `app/view.rs` when a `deny` decision is made). It never listed cookies a page uses.
- With the default configuration (`cookies.default: allow`, no rules) `CookiesConfig::is_open()` is true and
  `ViewResources::cookie_access_filter` returns no filter at all, so nothing is observed. The item said "none" for every page by design.
- The count covers every request of the page (third-party hosts included): a blocked host is the host of the request URL.

## Change

The "No cookies blocked on this page" row is gone: the item appears only when the rules blocked at least one site
(`Copy allow rules for N blocked cookie sites`). docs/config/options.md says so.

## Decided not to do yet (the person will come back to it)

A cookie submenu listing every host that sent or set a cookie on the page (own site, then third parties) with allow / session / deny per
host or per site, saved in the profile database in front of the YAML rules, and a workspace "minimal cookies" default. Needs the
filter to run always (to see anything under the allow-all default), a scope model (exact host, whole site, only while on this site) and
somewhere to store the choices. Not designed further.
