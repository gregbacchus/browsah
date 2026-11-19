# Codec-enabled CEF licensing investigation

**Date:** 2026-11-19. Source and package notice review, not legal advice. No permission or patent
clearance was obtained, and no exact-build complete source or license audit was established.

## CEF's own license and H.264 alone

[CEF's BSD-3-Clause license](https://github.com/chromiumembedded/cef/blob/master/LICENSE.txt)
permits redistribution with modifications, retaining the required notices and honoring the
non-endorsement clause. It does not require publication of modified source. It does not by itself
grant a permissive license to independently authored modifications, nor does it settle the licenses
of Chromium and other bundled code. A patch offered upstream under appropriate contribution terms
may become available through upstream, but none of the Karere patches' upstream acceptance or
separate permissive grants was established here.

H.264 and AAC are separately licensed patent technologies. Removing AAC from a build could avoid
AAC-specific decoder concerns, but the tested Karere and Arch binaries contain both; turning off
AAC at runtime is not proof that its implementation has been removed from the distributed binary.
The standard CEF `proprietary_codecs=true ffmpeg_branding=Chrome` build enables both; an H.264-only
prebuilt matching browsah and a practical way to obtain one have not been established. Many MP4
videos pair H.264 video with AAC audio, so H.264 alone may leave them silent or unsupported.

[Cisco OpenH264](https://www.openh264.org/faq.html) is another H.264 implementation: Cisco describes
conditional patent coverage for its distributed binary module. That coverage must not be assumed to
extend to FFmpeg's H.264 decoder, a self-built module or repackaging. Chromium integration into CEF,
HTML video coverage, technical limitations and the actual applicable Cisco terms are unverified for
browsah. This is an investigation route, not an established drop-in codec for the stock CEF build.

## Karere's CEF 156.0.26 build 2

The [release](https://github.com/tobagin/karere/releases/tag/cef-156.0.26-proprietary-codecs-2)
says it contains two message-pump fixes and an additional accelerated OSR patch. Its tagged
[README](https://github.com/tobagin/karere/blob/cef-156.0.26-proprietary-codecs-2/README.md)
says Karere is GPL-3.0-or-later. The tagged
[#151 patch](https://github.com/tobagin/karere/blob/cef-156.0.26-proprietary-codecs-2/tools/cef-patches/karere-151-detach-worksource.patch)
and [#173 patch](https://github.com/tobagin/karere/blob/cef-156.0.26-proprietary-codecs-2/tools/cef-patches/karere-173-pump-continuation.patch)
do not state a separate license. The tagged patch directory contains these two files; we did not
locate the source of the release's additional OSR patch or establish the exact build recipe for it.
Its bundled `LICENSE.txt` is the CEF BSD-3-Clause notice, and `CREDITS.html` lists third-party
notices; neither establishes that the project's changes are separately BSD licensed.

The repository GPL notice is evidence of the project's licensing, not conclusive proof of each
patch's copyright status or of all terms for the distributed binary. Under the conservative reading
that the incorporated changes are GPL-covered, distributing the dynamically linked browsah+CEF
combination would raise GPL compliance obligations. The [FSF FAQ](https://www.gnu.org/licenses/gpl-faq.html#IfLibraryIsGPL)
states its view that the whole linked combination must be available under GPL terms; MIT source can
remain MIT licensed, while a distributed combination can have GPL obligations. That is different
from saying MIT and GPL cannot be combined. The result would not fit browsah's stated
permissive-dependency policy without a decision or additional permission. Obtain a clear written
permissive grant from all relevant rights holders for all three modifications and the actual binary,
or a documented unmodified permissively licensed alternative. No one was contacted.

## Chromium and FFmpeg

The candidate's `CREDITS.html` includes FFmpeg's license text, which describes LGPL-2.1-or-later
and optional GPL code. A Chromium [FFmpeg metadata example](https://chromium.googlesource.com/chromium/third_party/ffmpeg/+/544f198aa40a93b14e5252999f8022211920ce3b/README.chromium)
labels FFmpeg LGPL-2.1 and shipped; this is not a file-by-file audit of the exact 156 build. The
text in CREDITS includes source-tree notices and optional code, so presence alone does not prove a
particular optional GPL module is compiled. [FFmpeg's own guidance](https://ffmpeg.org/legal.html)
distinguishes LGPL from opt-in GPL components and discusses source, notices, and relinking. Exact
configured sources, linking, applicable license and compliance must be verified for the selected
binary—including stock CEF and Arch—before asserting that all shipped components are permissive.

## Patents

These concerns apply separately from copyright and apply to both Karere and Arch codec builds.
[Via LA's H.264 schedule](https://www.via-la.com/licensing-programs/avc-h-264/) lists $0 for the
first 100,000 annual units under its products-sold-to-PC-end-users/OEMs category, subject to the
agreement and the affiliated-group condition; it does not establish that free browsah downloads are
covered automatically or that no agreement/reporting is required. Via LA says its pool may not
cover every essential patent.

[Via LA's AAC FAQ](https://www.via-la.com/licensing-programs/aac/) states developers of end-user
decoder products need a patent license and distinguishes component licenses from end products. It
lists an initial $15,000 fee on executing its license, reduced to $1,000 for small entities meeting
its employee and revenue criteria; PC software pricing requires approval. Its terms speak of sales,
and whether they apply to browsah's free distribution and specific territories is unresolved. No
conclusion can be drawn just from another project's distribution or a package's BSD label. An
appropriate professional or Via LA can assess scope, territory, existing coverage and terms before
public distribution. Do not assume that a user-initiated download automatically removes obligations.
