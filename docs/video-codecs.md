# Video codecs

The bundled CEF does not support H.264 video or AAC audio. On Linux x86-64, you can install an
optional third-party runtime for personal use; the normal `browsah` command keeps using the
codec-enabled runtime after installation. The installer downloads Karere CEF 156.0.26 (Chromium 156.0.8078.88), checks
the archive's SHA-256 and the versioned CEF API hash, and keeps its files together in your data
directory. This build includes both H.264 and AAC. Its patches and distribution terms are discussed
in [the licensing research](research/cef-codec-licenses.md).

```bash
browsah codecs install
browsah codecs status
browsah
```

To compare it with the bundled runtime using a separate profile:

```bash
browsah profile add codecs --name 'Browsah Codecs'
browsah --profile=codecs
```

Open [the H.264/AAC sample](https://lorem.space/sample-video/sample-10s-720p.mp4): you should
see moving colour bars and hear beeps. Each profile has one running process. Close the profile's
existing windows before switching runtimes; otherwise the new launch is forwarded to the existing
process and continues to use its CEF. Normal commands and profile launchers use the codec runtime
after installation. Close all codec-enabled windows and run `browsah codecs uninstall` to remove
the downloaded runtime and use bundled CEF again.

The installer copies the current browsah binary beside the selected CEF resources. After a browsah
upgrade, browsah refuses to start until you run `browsah codecs install` again or uninstall the
codec runtime. The
installer currently accepts only the tested stock CEF 156.0.28 / Chromium 156.0.8078.88 and the
pinned Karere CEF 156.0.26 with its expected API hash. A browsah upgrade to a different CEF version
therefore needs a compatible installer update; reinstalling alone will not override that check.
Those checks are not proof that a new browsah build works with this runtime; verify playback and
other features after updating. The third-party runtime
does not update automatically, and the publisher's update schedule is not guaranteed.

The install command needs `bash`, `curl`, `unzip`, `sha256sum` and `python3`. If you have already
downloaded the exact archive, `browsah codecs install --archive /path/to/archive.zip` verifies and
uses that file instead. The runtime and old installed generations are under
`${XDG_DATA_HOME:-~/.local/share}/browsah/codec-runtime`; old generations are retained so an open
browser is not disturbed during a reinstall, and `uninstall` removes them once codec-enabled windows
are closed. Real-world WebRTC calls, ARM and all browser features have not been verified with
this runtime. If you switch from a source build to Homebrew, run `browsah codecs install` from the
Homebrew launcher to bind the runtime to that copy.

## System copy and 1Password

The root-owned copy uses its own codec runtime under `/opt/browsah/codec-runtime`. Both the browser
executable and CEF in that runtime are owned by root. After installing or upgrading the system
copy, close its browser windows and run:

```bash
sudo /usr/local/bin/browsah codecs install
/usr/local/bin/browsah codecs status
/usr/local/bin/browsah
```

Launch the browser as your normal user. Close other Browsah windows using the same profile first
so the launch starts a new process. Run `sudo /usr/local/bin/browsah codecs uninstall` to go back to
bundled CEF on the system copy. A system upgrade replaces the installation directory, so rerun
the system codec install after upgrading. The Homebrew copy keeps using its own per-user codec
runtime. Actual 1Password integration with the codec runtime still needs testing.
