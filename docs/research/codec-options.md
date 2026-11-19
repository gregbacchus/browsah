# H.264 and AAC options

**Date:** 2026-11-18. The existing capability probe on the AMD Radeon 890M / Mesa 26,
GNOME Wayland machine with CEF 156.5.0 reported no H.264 or AAC; it did not play media.
See [the original measurement](profiles-workspaces-design.md#codecs-measured).

CEF's maintainer states that proprietary codecs are disabled by default and identifies
`proprietary_codecs=true ffmpeg_branding=Chrome` as build arguments for enabling them
([CEF issue 3559](https://github.com/chromiumembedded/cef/issues/3559)). A matching custom CEF
build is one route for browsah; installing system FFmpeg or GStreamer packages alone does not
change those CEF build flags. A prebuilt codec-enabled CEF was tested below. A standalone
replacement codec library has not been verified.
Chromium does support building FFmpeg as a separate shared library using `is_component_ffmpeg`
([upstream build options](https://chromium.googlesource.com/chromium/third_party/ffmpeg/+/refs/heads/sushi-2026-10-02-19-35-50/ffmpeg_options.gni)),
but CEF also needs its proprietary-codec support built in. Inspection of this machine's
`target/release/libcef.so` with `readelf -d target/release/libcef.so | rg 'NEEDED.*(avcodec|avformat|avutil|ffmpeg)'`
found no FFmpeg/libav shared-library dependency. That does not rule out dynamic loading, but no
separate codec loading mechanism has been established for this CEF distribution. A custom shared
FFmpeg build and later replacement have not been tested.

## Prebuilt candidate and licensing (2026-11-18)

### Tested: obs-rust CEF 8078

The [obs-rust CEF 8078 release](https://github.com/obs-rust/cef/releases/tag/8078) supplies a
Linux x86-64 build with H.264/AAC enabled and records its source and build flags. The downloaded
`cef_binary_8078_linux_x86_64.tar.xz` matched the published SHA-256
`fb36df11bc8280c9cb666732becc1fe267f833bb964dbb13d69cad33488171dc`.
It is CEF **156.0.8**, Chromium **156.0.8078.48**; browsah's locked bindings are CEF 156.0.28,
so this experiment does not establish API compatibility or make it a safe drop-in release.
It is older and lacks subsequent browser security fixes. This third-party build is not OBS Studio's
official release; its publisher says it was built from upstream CEF without the fork's patches.

On the AMD Radeon 890M, Mesa 26, GNOME Wayland machine, 2026-11-18, browsah's existing release
binary ran with the candidate `Release/` and `Resources/` files in a temporary directory and
temporary XDG directories. `BROWSAH_SELFTEST_ONLY=basic` passed 23/23 checks. The codec capability
probe reported H.264 and AAC `probably`, H.264 MSE `true`, and H.264 among WebRTC sender codecs.
A generated 320x180 H.264/AAC MP4 produced 15 video frames with 47 sampled colours and a decoded
3.04-second mono audio buffer with peak 0.837. This checks decoded content, not audible output.
The same playback probe against stock CEF failed both audio decode and video loading; its `basic`
section also passed 23/23. Streaming playback, WebRTC calls, DRM, hardware decoding and other
machines were not tested. The support report `powerEfficient=true` is not a measurement of GPU use.

**Release cadence checked 2026-11-18:** The publisher's [release list](https://github.com/obs-rust/cef/releases)
contains three prereleases: `7871` on October 13 (a mirror of OBS's Chromium 150 binaries),
`8078` on October 23 (the Chromium 156.0.8078.48 candidate tested above), and `8078-r2` on
October 26 (the same upstream CEF revision rebuilt with debug line tables, not a security update).
The Linux x86-64 asset for the candidate was uploaded October 23; macOS assets were added later.
No newer CEF source revision was published there by November 18. The checked-in stock CEF header
reports Chromium 156.0.8078.88, newer than the candidate's .48. Three releases, only one new
codec-enabled CEF revision, do not establish a recurring cadence or security-update commitment.
The publisher's [provenance record](https://github.com/obs-rust/cef/blob/6533-fix-stutter-and-osr-extra-info/PROVENANCE.md)
documents the distinction and notes that its Linux recipe followed a branch tip.

Reproduce the binary check with `sha256sum` on the release archive. Extract it, copy `Release/`
and `Resources/` next to a copy of the browsah binary, set `LD_LIBRARY_PATH` to that directory,
set all three XDG data/config/cache directories to temporary directories, and run
`BROWSAH_SELFTEST_ONLY=basic BROWSAH_NET_TEST=file://$PWD/bench/pages/codecs.html <copy>/browsah --selftest`.
The playback sample used `gst-launch-1.0` with `videotestsrc ! x264enc ! h264parse`,
`audiotestsrc ! avenc_aac ! aacparse`, and `mp4mux`; an HTML page decoded audio with Web Audio,
played the muted video and inspected the frame with canvas. The temporary sample and harness were
not added to the repository.

### Other candidate

### Karere and Arch packages (researched 2026-11-18)

[Karere's 156.0.26 build 2](https://github.com/tobagin/karere/releases/tag/cef-156.0.26-proprietary-codecs-2)
has linux64 and linuxarm64, codecs enabled, and Chromium 156.0.8078.88, matching the Chromium
version in browsah's CEF 156.0.28 headers. Its CEF commit differs, and it includes Karere's
message-pump and accelerated OSR patches. The linux64 zip's SHA-256 matched the publisher's
`027473508ec25227972a8bc9182b15723f98cfa2e301746f8e2224c6a58517d5`.
On the AMD Radeon 890M / Mesa 26 / GNOME Wayland machine, the existing browsah release binary
with that runtime in a temporary directory passed `BROWSAH_SELFTEST_ONLY=basic` (23/23), including
dmabuf frames without fallback. The codec probe reported H.264 and AAC `probably`, MSE H.264
`true`, and H.264 among WebRTC senders. The generated MP4 playback probe decoded 15 video frames
and a non-silent 3.04-second AAC buffer (peak 0.837); audible output, WebRTC calls, ARM, broader
behavior and ABI compatibility were not tested. Karere's repository is GPL-3.0 and its CEF binary
contains project patches, so their applicable copyright terms and compatibility with browsah's
MIT/permissive dependency policy need clarification before adoption, in addition to patent rights.
Its published codec builds span CEF 148, 149, 150, 152 and 156 from June to November 2026; this
shows repeated updates but not a promised security-update cadence.

**Further compatibility check, 2026-11-19:** Against the current browsah release binary and stock
CEF 156.0.28, the downloaded Karere 156.0.26 build returns the same Linux versioned CEF API hash
for API 15600 (`6dc09eb2834fe5b61d7d8c42bbb8d8c27a2bb1b6`). The two distributions'
`include/cef_api_versions.h` files are byte-identical; this is evidence for C API compatibility,
not proof of identical behavior or all ABI details. Re-tested with a fresh temporary profile and
the current binary: `BROWSAH_SELFTEST_ONLY=basic` passed 23/23 and the sample decoded H.264 video
(16 frames, 47 sampled colours) and AAC audio (peak 0.837). `BROWSAH_SELFTEST_ONLY=hidpi` passed
18/18 at native scale 2, using dmabuf with no fallback. The input assertion emits synthetic GTK
signals, not real device events. No real audio, sites, calls or ARM were tested. The library and
its matching resource files were used together from `/tmp/browsah-karere-test/bin`, without
recompiling or replacing the installed browser. A compatible Rust binding release also exists:
[`cef` 156.4.0+156.0.26](https://docs.rs/crate/cef/156.4.0%2B156.0.26); compiling browsah against
that version has not been tried or shown necessary.

`download-cef` 3.0.1's `check_archive_json` accepts an archive whose version is *at most* the
crate's CEF version, not necessarily identical, and a failed `CEF_PATH` check falls back to a stock
download. An explicit version/API hash check and a pinned archive digest would be needed for an
automatic build; a synthetic `archive.json` must not be mistaken for verification of the bytes.

**Deployment implications, 2026-11-19:** The existing Homebrew formula bundles browsah and the
CEF runtime under `opt_libexec`; its launcher prepends that path to `LD_LIBRARY_PATH`, and desktop
entries use `BROWSAH_LAUNCHER` and `BROWSAH_LIBS`. Simply exporting another `LD_LIBRARY_PATH` before
invoking that wrapper does not select the other CEF, because the wrapper puts its own runtime first.
CEF settings do not set a separate resources directory. The tested experiment copies the browsah
binary next to matching CEF `Release` and `Resources` files, then uses that directory in
`LD_LIBRARY_PATH`. `browsah codecs install` now implements this selection with a pinned digest and
version/API checks; ordinary launches re-exec using the installed runtime, and `uninstall` restores
bundled CEF. The `basic` self-test and local MP4 probe passed through ordinary startup; after
uninstall, the same probe reported unsupported video and audio. A running profile retains its
existing CEF until closed. There is no automatic update for the third-party runtime; reassess the
CEF version and API hash when changing browsah or its runtime.

Arch's [156.0.28 package](https://eu.edge.kernel.org/archlinux/pool/packages/) matches the CEF
commit but uses Chromium 156.0.8078.93 rather than browsah's .88. Its [package recipe](https://gitlab.archlinux.org/archlinux/packaging/packages/cef/-/blob/e3dbdb4ec34329fd6e614c32a7f06ad07a2002a3/PKGBUILD)
sets `proprietary_codecs=true` and `ffmpeg_branding="Chrome"`. Extracting it and inspecting `ldd`
on this Ubuntu machine showed it requires GLIBC_2.44 (host 2.43) and `libsimdutf.so.36`, which is
missing, so it was not run natively. The separately downloaded package signature was not verified.
It is x86-64; no matching ARM package was investigated. Redistribution terms are unverified.

**Follow-up isolated test, 2026-11-18:** An Arch Linux Podman container with its own glibc 2.44,
`simdutf` and Arch's CEF 156.0.28-1 package resolved the runtime dependencies. Pacman installed
the package and dependencies from configured Arch repositories with its normal signature checks;
this was not the separately downloaded archive above. The host was Ubuntu 26.04.1, glibc 2.43,
AMD Radeon 890M / Mesa 26, GNOME Wayland. The existing browsah binary and local self-test pages
were bind-mounted read-only, with a temporary XDG profile and private D-Bus session; network was
disabled and the host Wayland socket and GPU were passed through. CEF reported Chromium
156.0.8078.93, vs the Rust binding's .88. `BROWSAH_SELFTEST_ONLY=basic` passed 23/23 including
dmabuf frames without fallback. The codec probe reported H.264/AAC `probably`, MSE H.264 `true`,
and H.264 among WebRTC sender capabilities. The H.264/AAC sample decoded 15 frames (47 sampled
colours) and a 3.04-second non-silent audio buffer (peak 0.837). No audible output, networking,
streaming, real WebRTC calls, hardware video decode, ARM build, broad compatibility, distribution
licensing or packaged container integration were verified. Container portal warnings appeared
because its D-Bus session and devices were isolated. This verifies a container experiment, not
native Ubuntu loading. The initially interrupted run lacked the self-test page mount; its failed
checks were a harness error, and the clean rerun used a new temporary profile.

Reproduce by installing `gtk4 libadwaita cef mesa` in a temporary Arch container; mount the browsah
binary alongside `/usr/lib/cef` resources and bind `bench/`, the sample probe, the Wayland socket
and render device. Use separate XDG directories and `dbus-run-session`, set
`LD_LIBRARY_PATH=/usr/lib/cef`, `BROWSAH_SELFTEST_ONLY=basic`, and point `BROWSAH_NET_TEST` at
the probes. The local sample/probe was generated as described above and remains in `/tmp` only.

[GodwinMix CEF 154.0.34](https://github.com/psmux/godwinmix-cef/releases/tag/cef-154.0.34)
also publishes codec-enabled builds for both Linux architectures in the official archive layout,
with a compatible `cef` downloader index and checksums. Its CEF and Chromium versions are older;
neither its Linux builds nor compatibility were tested here.

### Weekly build feasibility (2026-11-18)

Estimated, not implemented or benchmarked: a prototype scheduled build and archive is roughly
1–2 engineering days; dependable multi-architecture builds, playback validation, version pinning
and release integration could take several more days. Upstream's [automated build guidance](https://github.com/chromiumembedded/cef/blob/master/docs/automated_build_setup.md)
recommends at least 16 GB RAM and 200 GB free disk; GitHub's [standard public Linux runners](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job)
list 16 GB RAM and 14 GB storage, so they do not meet that guidance. GitHub-hosted jobs also have
a [six-hour limit](https://docs.github.com/en/actions/reference/limits). A larger runner or a
dedicated runner with sufficient space is needed for a practical trial; runtime and costs are unknown.

Resolve the desired stable CEF version to immutable CEF and Chromium revisions, build with codec
flags, record the source and recipe, validate real H.264/AAC playback, and publish versioned,
checksummed archives. GitHub Actions artifacts in public repositories expire within a maximum
90-day retention ([GitHub documentation](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository));
release assets or other durable storage are better for pinned dependencies. `cef-dll-sys` checks
`archive.json` against its crate version and downloads the default if `CEF_PATH` fails validation,
so integration must package the expected metadata, reject fallback, and match Rust bindings.
Current release builds both x86-64 and aarch64. A weekly schedule may delay security fixes by a week
and should have a manual trigger. Publishing codec-enabled binaries requires the licensing assessment
above. No build, runner capacity, workflow or legal clearance was verified for this proposal.

An [Arch Linux x86-64 CEF 156.0.28 package](https://eu.edge.kernel.org/archlinux/pool/packages/) exists,
matching the CEF version in the lockfile. It was subsequently inspected above and cannot load on
the current Ubuntu machine without newer dependencies. The official [CEF automated downloads](https://cef-builds.spotifycdn.com/index.html)
include the stock kind already measured here. A third-party issue claims a codec-enabled variant
exists there but itself calls for confirming the artifact; it is not evidence that one exists.

Copyright and patent rights are distinct. [FFmpeg's licensing guidance](https://ffmpeg.org/legal.html)
describes LGPL obligations and optional GPL components, which depend on the actual build. Via LA's
[AVC terms](https://www.via-la.com/licensing-programs/avc-h-264/) list $0 for the first 100,000
annual units under its PC codec manufacture/sales category, subject to its license terms; whether
that category covers browsah's free distribution is unverified. [Via LA's AAC program](https://www.via-la.com/licensing-programs/aac/)
separately describes licenses for end-user decoder products. A zero royalty is not proof that no
license or reporting applies, and patent coverage varies by jurisdiction and use. No legal conclusion
or rights for redistributing a specific prebuilt binary has been established.

`Cargo.lock` currently selects `cef` and `cef-dll-sys` 156.5.0+156.0.28. The latter's build
script accepts `CEF_PATH` for an existing CEF distribution, but falls back to downloading its
usual archive if the path is invalid. Confirm the build output and runtime libraries actually
come from the custom distribution; build-time flags on `cargo build` do not rebuild CEF.
Use a matching CEF version and architecture. Patent licensing for distribution needs separate
assessment; enabling codecs is distinct from Widevine DRM and hardware acceleration.

To reproduce the prior capability probe, use the commands in the original measurement. After
building against custom CEF, repeat it and test actual H.264/AAC files, an MSE stream, and a
relevant meeting service: capability reporting alone does not establish playback or WebRTC.
