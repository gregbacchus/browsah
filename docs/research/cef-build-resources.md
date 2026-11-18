# CEF build disk and time

**Date:** 2026-11-18. Research only; no CEF build was run here. The 200 GB in
[CEF's automated build instructions](https://github.com/chromiumembedded/cef/blob/master/docs/automated_build_setup.md)
is a recommended provisioning figure, not a demonstrated minimum. Its default automation builds
debug and release; a release-only build with no Chromium history and reduced symbols can use less.

Evidence from other machines, not direct measurements here:

- [Chromium's Linux instructions](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/linux/build_instructions.md)
  recommend at least 100 GB free. This is not an observed peak for a CEF build.
- The publisher of the codec-enabled CEF tested in [codec-options.md](codec-options.md) budgets
  about 150 GB for its Linux x86-64 recipe ([issue](https://github.com/obs-rust/obs-studio/issues/2417));
  that is a budget, not a measured peak. Its [recipe](https://github.com/obs-rust/obs-studio-libs/blob/main/recipes/cef/linux/build.sh)
  uses no debug build or Chromium history, `symbol_level=1`, and splits sync, build and pack so
  the build can resume. Its comments say initial sync takes hours.
- [GodwinMix's build report](https://github.com/psmux/godwinmix-cef) measured 46 GB total on
  an Intel macOS x64 CEF 154 release build with symbols disabled: 32 GB checkout plus 14 GB objects.
  Sync took 77 minutes and compilation about 5 h 20 min on an i9-10910 (18 build threads,
  128 GB RAM). This is **not a Linux disk measurement**. The project advises 100 GB free.
- A [separate CEF experiment](https://qiita.com/rairaii/items/bad396ec591698160afb) reports
  a 2 h 26 min x64 full codec-enabled release build in an Ubuntu container on an M3 Ultra,
  using 24 CPUs and 64 GB via Rosetta; sync time is not specified. Its altered ARM cross-build
  took 4 h 58 min. Neither result predicts this machine's duration.

For planning, provision **150–200 GB free per clean Linux build** until an actual high-water mark is
measured. A single release-only build may fit in 100 GB, but that is unverified here. Separate
architecture outputs, temporary sync data and packaging need additional headroom. Weekly incremental
builds can be quicker if the checkout and objects persist, but version changes can require substantial
rebuilds or re-syncs.

This machine, checked with `df -h /tmp .`, `free -h`, `nproc` and `lscpu`, has a Ryzen AI 9 HX 370
(12 cores / 24 threads), 29 GiB RAM, and 353 GiB free on the workspace filesystem; `/tmp` is a 15 GiB
tmpfs with 13 GiB free, unsuitable for the build tree. At the instant checked, only 7.3 GiB RAM was
available and 6.7 GiB swap was used. Rough planning estimate: **4–10+ hours for a first x86-64 build
including sync**, heavily dependent on network, build parallelism, linking memory, thermal limits and
other activity. This is an inference, not a benchmark. ARM cross-build cost is separately unknown.
Standard hosted GitHub runners list 4 CPUs, 16 GB RAM and 14 GB storage, and have a six-hour job cap;
neither success nor a runtime for a larger runner has been measured.

## Staging (2026-11-18)

A resumable pipeline can separate sync, compilation, packaging and validation; the
[obs-rust recipe](https://github.com/obs-rust/obs-studio-libs/blob/main/recipes/cef/linux/build.sh)
does this and resumes compilation on a persistent build tree. Limiting parallel jobs can lower
concurrent compiler memory, but the final link still has its own memory requirement. Splitting jobs
does not remove the need for the source, generated files and compiled objects to be accessible when
linking. Moving them to shared storage or remote execution moves that storage requirement elsewhere.
Release-only builds, fewer symbols and sequential architecture builds can lower disk use, but their
peak here is unmeasured. Reusing the build tree can speed subsequent builds, with variable benefit.

GitHub-hosted jobs run on fresh machines and are limited to six hours; staged jobs need a persistent
external volume or costly artifact transfers. A [self-hosted runner](https://docs.github.com/en/actions/concepts/runners/self-hosted-runners)
can keep its checkout and build output; its [job limit](https://docs.github.com/en/actions/reference/limits)
is five days. These statements do not imply either runner fits within 14 GB. No staged build has been
run or measured for browsah.

## Release builds without debug symbols (2026-11-18)

Use release-only (`--no-debug-build`, `is_official_build=true`, `is_debug=false`) and explicitly
set `symbol_level=0` for the proposed trial. [Chromium's GN guidance](https://www.chromium.org/developers/gn-build-configuration/)
says fewer symbols speed the build; its [Linux instructions](https://chromium.googlesource.com/chromium/src/+/a6241cac3bfcba9cd6b78b8d5f9232de5fd692b/docs/linux/build_instructions.md)
note debugging symbols need more memory. The benefit depends on the baseline: the 4–10+ hour
estimate and 150–200 GB planning budget above already assume release-only and reduced symbols,
so no additional percentage reduction has been established. `symbol_level=0` generates little
debug information, not zero linker/dynamic symbols; production linking still needs them.

The existing release workflow strips debug information from an already-built `libcef.so`, which
reduces the shipped binary but does not recover build-time compiler, linker or peak disk costs.
Building without that information trades away detailed crash diagnostics. The published 46 GB
example above already used `symbol_level=0` and was on macOS, not Linux. No Linux comparison or
resource savings from these flags has been measured here.

## Reusing intermediates (2026-11-18)

Keep the matching source checkout, generated build files and compiled outputs on persistent storage:
the build tool can resume a stopped build and reuse outputs on subsequent builds when their inputs
remain valid. The amount rebuilt after a CEF/Chromium, toolchain, flag or architecture change is not
measured here. A compiler cache or remote build service may reuse compatible compilations across
workspaces or machines, at the cost of storage, setup and network; cache hit rates and compatibility
with the chosen CEF build have not been tested. Chromium [documents remote execution](https://chromium.googlesource.com/chromium/src/+/dd3c5ab322e25e7f34b5fc5f67c8a0a8ba9310cb/docs/linux/build_instructions.md),
but no accessible shared cache of matching codec-enabled CEF objects was identified.

Official [CEF binary distributions](https://github.com/chromiumembedded/cef/blob/master/docs/tutorial.md)
provide an already-linked libcef for applications, not the full object tree to rebuild it with new
codec flags. A complete prebuilt codec-enabled CEF is the strongest shortcut if an exact matching,
maintained build is available; the obs-rust candidate is tested in [codec-options.md](codec-options.md)
but does not match the pinned version. No compatible public intermediate was tested or found.
