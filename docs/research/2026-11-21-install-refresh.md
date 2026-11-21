# After an upgrade: codec runtime without a program copy, and `browsah install refresh` (2026-11-21)

**Question:** after `brew upgrade`, do I have to run `codecs install` every time? Why does CEF depend on the browsah hash?

**Answer (before):** yes, and it should not have. `codecs install` copied the browsah binary into the runtime folder, next to the codec
`libcef.so`, and recorded its SHA-256; launch re-ran that *copy* and refused to start (it **exited**) when the program being started, the program
it was installed from, or the copy no longer had that hash. So every upgrade broke the runtime until a 190 MB download was repeated. A system
upgrade (`system-install` swaps the whole `/opt/browsah` folder) also deleted the system copy's `codec-runtime`.

**Why the copy was not needed:** `browsah` has no RUNPATH, only `NEEDED libcef.so`, so the dynamic loader takes whichever `libcef.so` comes first on
`LD_LIBRARY_PATH`; CEF finds its resources, locales and snapshot next to the library, and its child processes (`/proc/self/exe`) inherit the path.

**Measured** (2026-11-21, this machine; a copy of the real user runtime, CEF 156.0.26, in a temporary `XDG_DATA_HOME`; the first two with an earlier launcher that re-ran the program itself):
- `LD_LIBRARY_PATH=<runtime> ./target/release/browsah` loads the runtime's `libcef.so` (`LD_DEBUG=libs`) and the `basic` self-test passes (23 of 23).
- With the new launcher, an *old-style* runtime (stale hash, copy inside) starts normally: the first process loads the bundled CEF, re-execs the same
  program, and five processes (main + CEF children) then initialise the runtime's `libcef.so`. `codecs status` passes.
- Unit tests: the launcher script with and without a runtime; `status` refuses a runtime with no `libcef.so`; `uninstall` refuses while a process has a
  runtime file mapped (it used to look for the copy as an executable); `system-install` keeps `codec-runtime`.

**Not measured:** H.264/AAC playback itself after this change (the runtime files are unchanged, the loaded library is the same one),
the sudo steps on a real `/opt/browsah` (`system-upgrade` was only run as a dry run), and a different CEF version (the API check still refuses it).

**Now:**
- A runtime is only CEF. The launcher script, which already sets `LD_LIBRARY_PATH`, puts `codec-runtime/current` first when its `libcef.so` exists
  (`system_install::launcher_script` for `/usr/local/bin/browsah`, `packaging/homebrew/browsah.rb.in` for Homebrew). The program does not re-run itself:
  the library path has to be set before a process starts, which is exactly what a launcher is for. No copy, no recorded hash, no status check at
  start. Tested: the launcher script run with and without a runtime (`the_launcher_puts_an_installed_codec_runtime_first_on_the_library_path`).
- Compatibility is checked by CEF's own fingerprint, not by the program's: `cef_rt::check_loaded_cef` asks the loaded `libcef.so` for `cef_api_hash`
  (the call `init` already makes) and compares it with `EXPECTED_API_HASH`. **Measured: 31 microseconds** (unit test, which also fails if the
  bundled CEF's hash changes). On a mismatch browsah does **not** try another CEF: it shows a dialog (and prints the text) saying the runtime does
  not fit and what to run: `browsah codecs install` to upgrade it, and, if that finds no matching runtime, `browsah codecs uninstall`. Then it exits.
- `codecs install` checks the runtime *after the download and unpacking*, against the same fingerprint (`BROWSAH_CODEC_API`, which `main` passes to
  the script): a runtime that does not fit is not installed and the message gives the uninstall instructions. `codecs status` checks an installed one.
- Checked: a real runtime (same API as the bundled CEF) starts normally and passes the `basic` self-test; `status` passes for it and, with another
  expected fingerprint, prints the uninstall instructions; with `BROWSAH_FORCE_API_HASH` (a test aid pretending another API) browsah printed the text
  and a dialog window was mapped (the process stayed up until killed). **Not checked:** how the dialog looks (no picture was taken), and the
  download path of `install` (it needs the real 190 MB archive; its check is the same function `status` uses).
  A program run directly from `target/release` does not use the runtime.
- Old launcher scripts do not know the runtime: `browsah install refresh` (system copy) and the next `brew upgrade` write the new ones.
- `browsah install refresh` is the one command after an upgrade; today its only step is `system-upgrade` (the root-owned copy, which rewrites its launcher).

## Reproduce

```bash
T=$(mktemp -d); mkdir -p $T/browsah; cp -a ~/.local/share/browsah/codec-runtime $T/browsah/
XDG_DATA_HOME=$T BROWSAH_SELFTEST_ONLY=basic LD_DEBUG=libs ./target/release/browsah --selftest 2>&1 | grep "calling init: .*libcef"
cargo test --release codecs; cargo test --release refresh; cargo test --release system_upgrade
```
