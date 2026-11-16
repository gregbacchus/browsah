//! browsah: a GTK4 shell around Chromium (CEF) windowless rendering, native Wayland.
//! See docs/research/summary.md for the design and docs/roadmap.md for the plan.

mod app;
mod bench;
mod cef_rt;
mod cli;
mod config;
mod frames;
mod gpu;
mod profile;

use adw::prelude::*;
use gtk::gio;
use std::rc::Rc;

fn main() {
    // CEF re-executes this binary for its renderer / GPU / utility processes. Handle those
    // straight away: no config, no GTK.
    if std::env::args().any(|a| a.starts_with("--type=")) {
        match cef_rt::init(Vec::new(), None) {
            cef_rt::Init::Subprocess(code) => std::process::exit(code),
            cef_rt::Init::Browser(_) => unreachable!("a --type process is never the browser process"),
        }
    }

    // `browsah profile ...` manages profiles on disk and needs neither CEF nor a window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("profile") {
        std::process::exit(profile::cli(&args[1..]));
    }

    let opts = Rc::new(cli::Options::parse());
    let is_bench = opts.bench != cli::Bench::None;
    // Benchmarks use fixed defaults so results do not depend on a user's config file.
    let cfg = if is_bench { config::Config::default() } else { config::load(opts.config_path.as_deref()) };

    // Which profile this process is. Tests and benchmarks use a throwaway profile so they never
    // collide with a running browser.
    let test_run = is_bench || opts.selftest || opts.latency;
    let profile_id = match (&opts.profile, test_run) {
        (Some(id), _) => id.clone(),
        (None, true) => profile::TEST_ID.to_owned(),
        (None, false) => profile::DEFAULT_ID.to_owned(),
    };
    let profile = match profile::Profile::resolve(&profile_id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("browsah: {e}");
            std::process::exit(2);
        }
    };

    // One process per profile (Chromium locks the profile's cache directory): if this profile is
    // already running, hand it the link (or ask it to show its window) and leave, before CEF
    // starts.
    let app = adw::Application::builder().application_id(profile.app_id()).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    if let Err(e) = app.register(gio::Cancellable::NONE) {
        eprintln!("browsah: cannot register application {}: {e}", profile.app_id());
        std::process::exit(1);
    }
    if app.is_remote() && !test_run {
        match &opts.url {
            Some(url) => app.open(&[gio::File::for_uri(&app::resolve_address(url, &cfg.search_url))], ""),
            None => app.activate(),
        }
        std::process::exit(0);
    }
    if let Err(e) = profile.ensure_dirs() {
        eprintln!("browsah: cannot create {}: {e}", profile.dir.display());
        std::process::exit(1);
    }

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

    let runtime = match cef_rt::init(flags, Some(profile.cef_dir())) {
        cef_rt::Init::Subprocess(code) => std::process::exit(code),
        cef_rt::Init::Browser(rt) => rt,
    };

    let code = if is_bench {
        bench::run(opts);
        0
    } else {
        app::run(app, opts, cfg, profile)
    };

    runtime.shutdown();
    std::process::exit(code);
}
