//! browsah: a GTK4 shell around Chromium (CEF) windowless rendering, native Wayland.
//! See docs/research/summary.md for the design and docs/roadmap.md for the plan.

mod app;
mod appicon;
mod bench;
mod cef_rt;
mod cli;
mod config;
mod cookies;
mod crx;
mod frames;
mod fullconfig;
mod docsite;
mod downloads;
mod extensions;
mod gpu;
mod groups;
mod history;
mod icons;
mod log;
mod manifest;
mod navhistory;
mod permissions;
mod picker;
mod profile;
mod router;
mod selector;
mod store;
mod suggest;
mod sweep;
mod system_install;
mod webapp;
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

    // `browsah --help` / `--version`: answered here, before config, GTK or CEF.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some((text, code)) = cli::root_info(&args) {
        println!("{text}");
        std::process::exit(code);
    }

    // `browsah profile ...` manages profiles on disk and needs neither CEF nor a window.
    if args.first().map(String::as_str) == Some("profile") {
        std::process::exit(profile::cli(&args[1..]));
    }

    // `browsah config show [--profile=ID] [--config=PATH]`: the effective configuration.
    if args.first().map(String::as_str) == Some("config") && args.get(1).map(String::as_str) == Some("show") {
        std::process::exit(config_show(&args[2..]));
    }

    // `browsah config full [--profile=ID [--workspace=ID]]`: a file with every option and its explanation.
    if args.first().map(String::as_str) == Some("config") && args.get(1).map(String::as_str) == Some("full") {
        std::process::exit(config_full(&args[2..]));
    }

    // The profile selector (the system web browser entry) needs no CEF and no profile.
    if args.first().map(String::as_str) == Some("select") {
        std::process::exit(selector::select_cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("selector") {
        std::process::exit(selector::selector_cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("permissions") {
        std::process::exit(permissions::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("sweep") {
        std::process::exit(sweep::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("history") {
        std::process::exit(history::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("log") {
        std::process::exit(log::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("route") {
        std::process::exit(router::cli(&args[1..]));
    }
    if matches!(args.first().map(String::as_str), Some("system-install" | "system-uninstall" | "system-upgrade")) {
        std::process::exit(system_install::cli(&args[0], &args[1..]));
    }
    if args.first().map(String::as_str) == Some("extension") {
        std::process::exit(extensions::cli(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("app") {
        std::process::exit(webapp::cli(&args[1..]));
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
        let found = if opts.app { workspace::Workspace::resolve_app(&profile, w) } else { workspace::Workspace::resolve(&profile, w) };
        if let Err(e) = found {
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
            // The launcher's "New Window" of a web app.
            (None, Some(id)) if opts.new_window && opts.app => gio::prelude::ActionGroupExt::activate_action(&app, "new-app-window", Some(&id.to_variant())),
            // A window with no workspace (with the link, if one was given).
            _ if opts.new_window => {
                let url = opts.url.as_deref().map(|u| app::resolve_address(u, &cfg.search_url)).unwrap_or_default();
                gio::prelude::ActionGroupExt::activate_action(&app, "new-window-at", Some(&url.to_variant()));
            }
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

    log::init(&profile.id, cfg.logging.level);
    log_info!("profile {:?}, {}", profile.id, if test_run { "test run" } else { "normal start" });
    // A root-owned copy made by `system-install` that the copy it came from has since moved past (`brew upgrade`).
    if let Some(hint) = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).and_then(|p| system_install::stale_check(&p)) {
        log_warn!("{hint}");
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
    // Chrome extensions: Chromium loads the folders named by `--load-extension` (CEF 156 has no extension API of its own). The
    // native messaging hosts an extension needs are copied from Chrome/Chromium's folders into this profile's.
    if !is_bench {
        let mut ext_cfg = cfg.extensions.clone();
        // The `extensions` self-test section needs its fixture extension loaded when Chromium starts.
        if opts.selftest && std::env::var("BROWSAH_SELFTEST_ONLY").is_ok_and(|l| l.split(',').any(|s| s.trim() == "extensions")) {
            if let Ok(dir) = std::env::current_dir() {
                ext_cfg.paths.push(dir.join("bench/extensions/test-ext").to_string_lossy().into_owned());
            }
        }
        let (list, warnings) = extensions::collect(&profile.dir, &ext_cfg, &gtk::glib::home_dir());
        for w in warnings {
            log_warn!("extensions: {w}");
        }
        extensions::merge_load_flag(&mut flags, &list.iter().map(|e| e.dir.clone()).collect::<Vec<_>>());
        for e in &list {
            log_info!("extension loaded: {} {} ({}) from {}{}", e.name, e.version, e.id, e.dir.display(), if e.installed { "" } else { " (extensions.paths)" });
        }
        if ext_cfg.native_hosts && !list.is_empty() {
            let ids: Vec<String> = list.iter().map(|e| e.id.clone()).collect();
            for h in extensions::copy_hosts(&profile.cef_dir(), &extensions::matching_hosts(&extensions::host_search_dirs(&gtk::glib::user_config_dir()), &ids)) {
                log_info!("extensions: copied the native messaging host {h}");
            }
        }
        // An extension that talks to a program needs the program's host manifest: say so when none names it.
        for e in list.iter().filter(|e| e.native_messaging) {
            let found = extensions::matching_hosts(&[extensions::host_dir(&profile.cef_dir())], std::slice::from_ref(&e.id));
            if found.is_empty() {
                log_warn!("extension {} ({}) uses native messaging but no host manifest naming it was found in {}{}", e.name, e.id, extensions::host_dir(&profile.cef_dir()).display(), if ext_cfg.native_hosts { " or in Chrome's and Chromium's folders: install the program the extension talks to (1Password's desktop app)" } else { " (extensions.native_hosts is off)" });
            } else {
                log_debug!("extension {} ({}): native messaging host(s) {:?}", e.name, e.id, found.iter().map(|h| h.name.as_str()).collect::<Vec<_>>());
                // Some programs (1Password's desktop app) only talk to a browser that is owned by root: say how to get one.
                if !std::env::current_exe().is_ok_and(|p| system_install::root_owned_and_locked(&p)) {
                    log_info!("extension {} uses native messaging and this browsah program is not owned by root: if the program it talks to refuses to connect (1Password does), install a root-owned copy with `sudo \"$(brew --prefix)/bin/browsah\" system-install --1password` (Homebrew; docs/config/extensions.md)", e.name);
                }
            }
        }
        extensions::set_loaded(list);
    }
    // BROWSAH_NO_SCALE_FIX=1 leaves Chromium's Wayland fractional-scale feature on (to reproduce the HiDPI problem).
    let flags = cef_rt::with_default_flags(flags, std::env::var_os("BROWSAH_NO_SCALE_FIX").is_none());

    cef_rt::forget_saved_zoom(&profile.cef_dir());

    // Chromium names the launcher entry in the desktop notifications it sends (the `desktop-entry` hint) from
    // this variable, so a page's notifications are shown under the profile's own name and icon, and a click
    // raises the profile. (Left alone for the test profile: tests must not look like the user's launcher.)
    if !test_run {
        // SAFETY: nothing else is running yet; this is the start of main, before CEF's threads exist.
        unsafe { std::env::set_var("CHROME_DESKTOP", format!("{}.desktop", profile.app_id())) };
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

/// Print a configuration file padded out with every option, its default and its explanation, with what the real file
/// sets switched on: the application file, a profile's (`--profile=ID`) or a workspace's (`--profile=ID --workspace=ID`).
fn config_full(args: &[String]) -> i32 {
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned));
    let (level, title, file) = match (value("--profile"), value("--workspace").or_else(|| value("--app"))) {
        (None, None) => {
            let file = value("--config").map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
            (fullconfig::Level::Application, "browsah".to_owned(), file)
        }
        (id, ws) => {
            let profile = match profile::Profile::resolve(&id.unwrap_or_else(|| profile::DEFAULT_ID.to_owned())) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("browsah: {e}");
                    return 2;
                }
            };
            match ws {
                None => (fullconfig::Level::Profile, profile.id.clone(), profile.dir.join("profile.yaml")),
                Some(w) => match workspace::Workspace::resolve_any(&profile, &w) {
                    Ok(w) => (if w.is_app() { fullconfig::Level::App } else { fullconfig::Level::Workspace }, w.id.clone(), w.config_file()),
                    Err(e) => {
                        eprintln!("browsah: {e}");
                        return 2;
                    }
                },
            }
        }
    };
    let existing = std::fs::read_to_string(&file).ok();
    match fullconfig::generate(level, &title, existing.as_deref()) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("browsah: {}: {e}", file.display());
            1
        }
    }
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
    let ws = match value("--workspace").or_else(|| value("--app")).map(|w| workspace::Workspace::resolve_any(&profile, &w)) {
        Some(Err(e)) => {
            eprintln!("browsah: {e}");
            return 2;
        }
        Some(Ok(w)) => Some(w),
        None => None,
    };
    let app_file = value("--config").map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
    let profile_file = profile.dir.join("profile.yaml");
    let ws_file = ws.as_ref().map(|w| w.config_file());
    match config::load_window(&app_file, &profile_file, ws_file.as_deref()) {
        Ok(l) => {
            println!("# profile: {id}{}", ws.as_ref().map(|w| format!(", {}: {}", if w.is_app() { "web app" } else { "workspace" }, w.id)).unwrap_or_default());
            println!("# application file: {}\n# profile file: {}", app_file.display(), profile_file.display());
            if let Some(f) = &ws_file {
                println!("# {} file: {}", if ws.as_ref().is_some_and(|w| w.is_app()) { "web app" } else { "workspace" }, f.display());
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
