//! browsah: a GTK4 shell around Chromium (CEF) windowless rendering, native Wayland.
//! See docs/research/summary.md for the design and docs/roadmap.md for the plan.

mod app;
mod bench;
mod cef_rt;
mod cli;
mod config;
mod frames;
mod gpu;
mod history;
mod profile;
mod router;
mod selector;
mod store;
mod sweep;
mod workspace;

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

    // `browsah config show [--profile=ID] [--config=PATH]`: the effective configuration.
    if args.first().map(String::as_str) == Some("config") && args.get(1).map(String::as_str) == Some("show") {
        std::process::exit(config_show(&args[2..]));
    }

    // The profile selector (the system web browser entry) needs no CEF and no profile.
    if args.first().map(String::as_str) == Some("select") {
        std::process::exit(selector::select_cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("selector") {
        std::process::exit(selector::selector_cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("history") {
        std::process::exit(history::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("route") {
        std::process::exit(router::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("workspace") {
        std::process::exit(workspace::cli(&args[1..]));
    }

    let opts = Rc::new(cli::Options::parse());
    let is_bench = opts.bench != cli::Bench::None;
    // Tests and benchmarks use a throwaway profile so they never collide with a running browser.
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

    // The application config with the profile's file merged over it. Benchmarks use fixed
    // defaults so results do not depend on a user's files.
    let app_file = opts.config_path.as_deref().map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
    let loaded = if is_bench {
        config::Loaded::defaults()
    } else {
        config::load_for_startup(&app_file, Some(&profile.dir.join("profile.yaml")))
    };
    let cfg = &loaded.config;

    if let Some(w) = &opts.workspace {
        if let Err(e) = workspace::Workspace::resolve(&profile, w) {
            eprintln!("browsah: {e}");
            std::process::exit(2);
        }
    }

    // One process per profile (Chromium locks the profile's cache directory): if this profile is
    // already running, hand it the link (or ask it to show its window) and leave, before CEF
    // starts.
    let app = adw::Application::builder().application_id(profile.app_id()).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    if let Err(e) = app.register(gio::Cancellable::NONE) {
        eprintln!("browsah: cannot register application {}: {e}", profile.app_id());
        std::process::exit(1);
    }
    if app.is_remote() && !test_run {
        // Hand the request to the running process: a link (with the workspace as the hint), a
        // workspace to show, or just "show a window".
        let hint = opts.workspace.clone().unwrap_or_default();
        match (&opts.url, &opts.workspace) {
            (Some(url), _) => app.open(&[gio::File::for_uri(&app::resolve_address(url, &cfg.search_url))], &hint),
            (None, Some(w)) => gio::prelude::ActionGroupExt::activate_action(&app, "open-workspace", Some(&w.to_variant())),
            (None, None) => app.activate(),
        }
        // Calls to a remote instance are asynchronous: let the message leave before exiting.
        gtk::glib::MainContext::default().iteration(false);
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
        app::run(app, opts, loaded, profile)
    };

    runtime.shutdown();
    std::process::exit(code);
}

/// Print the effective configuration for a profile (and optionally one of its workspaces) and
/// which options each layer set.
fn config_show(args: &[String]) -> i32 {
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned));
    let id = value("--profile").unwrap_or_else(|| profile::DEFAULT_ID.to_owned());
    let profile = match profile::Profile::resolve(&id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("browsah: {e}");
            return 2;
        }
    };
    let ws = match value("--workspace").map(|w| workspace::Workspace::resolve(&profile, &w)) {
        Some(Err(e)) => {
            eprintln!("browsah: {e}");
            return 2;
        }
        Some(Ok(w)) => Some(w),
        None => None,
    };
    let app_file = value("--config").map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
    let profile_file = profile.dir.join("profile.yaml");
    let ws_file = ws.as_ref().map(|w| w.dir.join("workspace.yaml"));
    match config::load_window(&app_file, &profile_file, ws_file.as_deref()) {
        Ok(l) => {
            println!("# profile: {id}{}", ws.as_ref().map(|w| format!(", workspace: {}", w.id)).unwrap_or_default());
            println!("# application file: {}\n# profile file: {}", app_file.display(), profile_file.display());
            if let Some(f) = &ws_file {
                println!("# workspace file: {}", f.display());
            }
            for layer in ["profile", "workspace"] {
                let o = l.overrides_of(layer);
                if layer == "workspace" && ws.is_none() {
                    continue;
                }
                println!("# options set by the {layer}: {}", if o.is_empty() { "none".to_owned() } else { o.join(", ") });
            }
            for w in &l.warnings {
                println!("# warning: {w}");
            }
            print!("{}", serde_yaml_ng::to_string(&l.config).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("browsah: {e}");
            1
        }
    }
}
