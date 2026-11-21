# One command after an upgrade: `browsah install refresh` (2026-11-21)

**Question:** after `brew upgrade`, do I have to run `codecs install` every time?

**Answer (before):** yes, and worse. The codec runtime holds a copy of the browsah program and a SHA-256 of it; any new program fails the check
and start-up **exited** with "this browsah installation differs from the codec runtime". `codecs install` then downloaded 190 MB again, and a
system upgrade (`system-install` swaps the whole `/opt/browsah` folder) deleted the system copy's `codec-runtime` too.

**Now:**
- `browsah codecs refresh` keeps the runtime's Chromium files, makes a new generation of hard links with the new program, its hash and source
  recorded (files are replaced, never written in place, so a running process keeps its generation), and switches `current`. No download.
- `system-install`/`system-upgrade` move the old `codec-runtime` into the new copy.
- `browsah install refresh [--dry-run] [--yes] [--force] [--from=DIR] [--prefix=DIR]` runs, for what is installed: `system-upgrade`, then
  `sudo <launcher> codecs refresh` (system runtime), then `codecs refresh` (your runtime).
- A runtime that does not match no longer stops start-up: `main` runs `codecs.sh status` first and, if it fails, starts on the bundled CEF
  (no H.264/AAC) with a one-line message naming `browsah install refresh`.

**Measured** (a copy of the real user runtime in a temporary `XDG_DATA_HOME`, 2026-11-21): `status` failed on the stale runtime, `install refresh`
made a new generation (libcef.so shared by hard link with the old one), `status` then passed, a second refresh said "already current", and the
`basic` self-test passed both with the stale runtime (bundled CEF, message shown) and after the refresh (re-exec into the runtime).
**Not measured:** the sudo steps on a real `/opt/browsah` (the plan is unit tested; `system-upgrade` itself was only run as a dry run here),
and a runtime refresh across a CEF version change (the API check still refuses that: reinstall is needed).

## Reproduce

```bash
T=$(mktemp -d); mkdir -p $T/browsah; cp -a ~/.local/share/browsah/codec-runtime $T/browsah/
XDG_DATA_HOME=$T ./target/release/browsah codecs status            # stale: differs
XDG_DATA_HOME=$T ./target/release/browsah install refresh          # no download
XDG_DATA_HOME=$T ./target/release/browsah codecs status            # installed
cargo test --release refresh; cargo test --release system_upgrade_keeps
```
