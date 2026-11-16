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

/// Apply the workarounds that the research found necessary. Returns a short description of what
/// was applied, for the log.
pub fn apply_workarounds() -> Vec<&'static str> {
    let mut applied = Vec::new();
    const AMD: u32 = 0x1002;
    if vendors().contains(&AMD) {
        // radeonsi gives Chromium's capture pool a DCC-compressed buffer layout it cannot use, so
        // shared-texture rendering delivers no frames on native Wayland. Cause suspected, not
        // confirmed; only the workaround is verified (AMD Radeon 890M, Mesa 26).
        append_env_token("AMD_DEBUG", "nodcc");
        applied.push("AMD_DEBUG=nodcc (AMD GPU)");
    }
    // Intel: `INTEL_DEBUG=noccs` was reported by others for the same failure but is untested
    // here, so it is not applied.
    applied
}
