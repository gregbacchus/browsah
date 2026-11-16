//! browsah: a GTK4 shell around Chromium (CEF) windowless rendering, native Wayland.
//! See docs/research/summary.md for the design and docs/roadmap.md for the plan.

mod app;
mod bench;
mod cef_rt;
mod cli;
mod config;
mod frames;
mod gpu;

use std::rc::Rc;

fn main() {
    // CEF re-executes this binary for its renderer / GPU / utility processes. Handle those
    // straight away: no config, no GTK.
    if std::env::args().any(|a| a.starts_with("--type=")) {
        match cef_rt::init(Vec::new()) {
            cef_rt::Init::Subprocess(code) => std::process::exit(code),
            cef_rt::Init::Browser(_) => unreachable!("a --type process is never the browser process"),
        }
    }

    let opts = Rc::new(cli::Options::parse());
    let is_bench = opts.bench != cli::Bench::None;
    // Benchmarks use fixed defaults so results do not depend on a user's config file.
    let cfg = if is_bench { config::Config::default() } else { config::load(opts.config_path.as_deref()) };

    // Must happen before CEF starts: its child processes inherit the environment.
    if cfg.gpu.workarounds && std::env::var_os("BROWSAH_NO_GPU_WORKAROUNDS").is_none() {
        for w in gpu::apply_workarounds() {
            eprintln!("gpu workaround applied: {w}");
        }
    }

    // `--ozone-platform=wayland` unless the user chose a platform themselves.
    let mut flags: Vec<String> = cfg.chromium.flags.clone();
    flags.extend(opts.flags.iter().cloned());
    if !flags.iter().any(|f| f.starts_with("--ozone-platform")) {
        flags.insert(0, "--ozone-platform=wayland".into());
    }

    let runtime = match cef_rt::init(flags) {
        cef_rt::Init::Subprocess(code) => std::process::exit(code),
        cef_rt::Init::Browser(rt) => rt,
    };

    let code = if is_bench {
        bench::run(opts);
        0
    } else {
        app::run(opts, cfg)
    };

    runtime.shutdown();
    std::process::exit(code);
}
