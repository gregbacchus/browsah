# Camera and microphone allowed in config, still unusable in Microsoft Teams (2026-11-20)

**Answer:** a probable cause, not a reproduced one. Browsah mirrored `allow`/`deny` into Chromium's content settings for notifications,
location, clipboard and MIDI, but deliberately not for camera and microphone. `getUserMedia` itself reached the handler and was answered
`allow` (reading the code), but a page that looks first saw Chromium's default ("prompt"; `enumerateDevices` with empty ids/labels). Teams
reads this before asking and reports no microphone/camera. Camera and microphone are now mirrored like the other kinds
(`permissions::mirrored_choice`, applied at every main-frame load start by `sync_to_chromium`).

## Environment and what was (not) measured

- Sandbox with no display, no devices, no CEF binary and no Teams account. **Nothing was run against real Teams, real devices or a
  running CEF.** `cargo`/self-test were not run for CEF behaviour.
- Read (measured by inspection): `on_request_media_access_permission` (CEF Alloy style) handles camera/microphone, `cont(mask)` with
  the allowed bits; permission-prompt bit values in `permissions.rs` match `CEF_PERMISSION_TYPE_*` in cef-dll-sys 156.5.0; site patterns
  are exact hosts (`*.x` excludes `x`).
- Unit test added (`scripts/test unit what_chromium_is_told`); it passes (run after the cold release build finished). The docs_ and example tests and all self-test sections were not run.

## Suspected causes, in order

1. Page-visible state not mirrored (fixed above). Unverified that this is what Teams checks; documented behaviour of Chromium is that
   `enumerateDevices` hides ids/labels and `permissions.query` says "prompt" until granted.
2. Site rule does not match the address Teams really runs on: Teams uses `teams.microsoft.com`, `teams.live.com` and now
   `teams.cloud.microsoft`; `*.microsoft.com` does not match `microsoft.com`. Docs and example now say so. `BROWSAH_DEBUG=1` now prints
   the origin, kinds and path (media access or prompt) of each request, to confirm.
3. Requests from cross-origin iframes: `sync_to_chromium` only syncs the main frame's origin. Not changed.
4. No H.264 in the stock CEF build (see codec-options.md): may limit some Teams call paths; unrelated to permissions, unverified.
5. A prompt carrying an unrecognised bit becomes `Other`, which follows `permissions.default` (ask), so a camera/microphone `allow`
   can still produce a question. Not seen for media; unverified.

Flags such as `--use-fake-ui-for-media-stream` were rejected: they bypass the configuration (and `deny`) for every site.

## Reproduce

`BROWSAH_DEBUG=1 browsah ...` then join a Teams call; look for `permission request from ...`. If none appears for the Teams origin the
request is not reaching the handler. Narrow test: `scripts/test unit what_chromium_is_told`.
