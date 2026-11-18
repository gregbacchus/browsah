# Intel: accelerated frames need `INTEL_DEBUG=noccs`

**Date:** 2026-11-18. **Environment:** a laptop with an Intel and an NVIDIA GPU (the person's second machine), reported with the
Homebrew build of browsah 0.1.8 and CEF 156. **Not recorded:** the Intel GPU model, the Mesa version, the compositor and whether the
NVIDIA GPU was the one rendering (the session type was asked for and not given). **Measured by the person, not by us:** the two
runs below; nothing else was measured on that machine.

## Question

On that laptop browsah printed `ERROR … frame_sink_video_capturer_impl.cc:939 Unable to allocate frame for first frame capture:
OOM?` followed by `browsah: no accelerated frame within 3s; falling back to readback rendering`. Is it the AMD failure
([cef-osr-wayland-spike.md](cef-osr-wayland-spike.md)) on Intel, and does the Intel analogue of the AMD fix help?

## Result

| Run | Result |
|---|---|
| `browsah` | the error above, then the readback fallback after 3 s |
| `INTEL_DEBUG=noccs browsah` | **no fallback message**: accelerated frames arrived |

So it is the same failure, and `INTEL_DEBUG=noccs` (which stops Mesa's Intel driver using the compressed CCS buffer layout) fixes it
there. Others reported the same error on an Intel UHD 630 and the same variable as the fix, so it is not one machine's quirk
(moq-dev/studio#107 and #111, quoted in the spike note).

## What changed

`gpu::apply_workarounds` now sets `INTEL_DEBUG=noccs` whenever an Intel GPU is present (`/sys/class/drm/card*/device/vendor` is
`0x8086`), as it sets `AMD_DEBUG=nodcc` for AMD. The variable is read only by Intel's driver, so it does nothing on a machine where
Intel is present but not rendering. It is added to an existing `INTEL_DEBUG` (comma separated), and `gpu.workarounds: false` or
`BROWSAH_NO_GPU_WORKAROUNDS=1` turns it off. NVIDIA alone gets nothing (the proprietary driver painted in the reports).

## Caveats

- **Only "no fallback message" was observed**, not frame rate, CPU or power. Not measured: what `noccs` costs (it turns off the
  driver's buffer compression, which can cost some bandwidth).
- **Which GPU rendered is not known.** On a hybrid laptop Chromium may have used either; the fix working says the Intel one was in
  the path. Whether the NVIDIA GPU would also fail, or work, if it rendered, is untested.
- A laptop with Intel and an AMD GPU now gets both variables; each is read only by its own driver.
- The cause (the compressed layout) is the reported one and is **suspected, not confirmed** here, as for AMD.

## Reproduce

```bash
browsah                          # the error and the fallback after 3 s, without the workaround
BROWSAH_NO_GPU_WORKAROUNDS=1 browsah   # the same, on a build that applies it
INTEL_DEBUG=noccs browsah        # accelerated frames arrive
```
