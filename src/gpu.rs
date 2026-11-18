//! Per-GPU environment workarounds, applied before CEF starts (its child processes inherit them).
//!
//! Evidence and status for each entry: docs/research/summary.md.

use std::path::Path;

/// PCI vendor ids from `/sys/class/drm/card*/device/vendor`.
fn vendors() -> Vec<u32> {
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // `card0`, `card1`...; skip connectors such as `card1-DP-1`.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        if let Ok(v) = std::fs::read_to_string(Path::new("/sys/class/drm").join(&name).join("device/vendor")) {
            if let Ok(id) = u32::from_str_radix(v.trim().trim_start_matches("0x"), 16) {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
    out
}

fn append_env_token(var: &str, token: &str) {
    let mut v = std::env::var(var).unwrap_or_default();
    if v.split(',').any(|t| t == token) {
        return;
    }
    if !v.is_empty() {
        v.push(',');
    }
    v.push_str(token);
    // Single-threaded at this point: called from `main` before any thread is started.
    unsafe { std::env::set_var(var, v) };
}

const AMD: u32 = 0x1002;
const INTEL: u32 = 0x8086;

/// The environment variable tokens to set for the GPU vendors present: (variable, token, description for the log).
fn plan(vendors: &[u32]) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut out = Vec::new();
    if vendors.contains(&AMD) {
        // radeonsi gives Chromium's capture pool a DCC-compressed buffer layout it cannot use, so
        // shared-texture rendering delivers no frames on native Wayland. Cause suspected, not
        // confirmed; only the workaround is verified (AMD Radeon 890M, Mesa 26).
        out.push(("AMD_DEBUG", "nodcc", "AMD_DEBUG=nodcc (AMD GPU)"));
    }
    if vendors.contains(&INTEL) {
        // The same failure with Intel's compressed (CCS) layout: "Unable to allocate frame for first frame capture" and
        // no accelerated frame. `INTEL_DEBUG=noccs` made the accelerated frames arrive on an Intel + NVIDIA laptop
        // (docs/research/intel-noccs.md); it was also reported by others for an Intel UHD 630.
        out.push(("INTEL_DEBUG", "noccs", "INTEL_DEBUG=noccs (Intel GPU)"));
    }
    out
}

/// Apply the workarounds that the research found necessary. Returns a short description of what
/// was applied, for the log.
pub fn apply_workarounds() -> Vec<&'static str> {
    plan(&vendors())
        .into_iter()
        .map(|(var, token, what)| {
            append_env_token(var, token);
            what
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_vendor_gets_its_own_workaround() {
        assert!(plan(&[]).is_empty());
        assert!(plan(&[0x10de]).is_empty(), "NVIDIA alone needs none (the proprietary driver delivered frames in the reports)");
        assert_eq!(plan(&[AMD]), [("AMD_DEBUG", "nodcc", "AMD_DEBUG=nodcc (AMD GPU)")]);
        assert_eq!(plan(&[INTEL]), [("INTEL_DEBUG", "noccs", "INTEL_DEBUG=noccs (Intel GPU)")]);
        // A hybrid laptop: Intel and NVIDIA gets Intel's; Intel and AMD gets both (each variable is read only by its own driver).
        assert_eq!(plan(&[INTEL, 0x10de]).len(), 1);
        assert_eq!(plan(&[AMD, INTEL]).len(), 2);
    }

    #[test]
    fn tokens_are_added_once_and_keep_what_is_there() {
        let var = "BROWSAH_TEST_GPU_TOKENS";
        // SAFETY: a variable only this test uses.
        unsafe { std::env::remove_var(var) };
        append_env_token(var, "noccs");
        append_env_token(var, "noccs");
        assert_eq!(std::env::var(var).unwrap(), "noccs");
        unsafe { std::env::set_var(var, "perf") };
        append_env_token(var, "noccs");
        assert_eq!(std::env::var(var).unwrap(), "perf,noccs");
        unsafe { std::env::remove_var(var) };
    }
}
