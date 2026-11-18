//! CEF runtime: process start-up, settings, and the handles shared with the rest of the app.
//!
//! CEF runs on **its own UI thread** (`multi_threaded_message_loop`). Pumping it from the GTK
//! loop pins the main thread at 100 % CPU (see docs/research/cef-multi-tab-cost.md), so all CEF
//! callbacks arrive on CEF's thread and must hand work to the GTK thread explicitly.

use cef::*;
use gtk::glib;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// `BrowserHost` is a ref-counted CEF handle; only its thread-safe methods are used through it.
pub struct HostHandle(pub BrowserHost);
unsafe impl Send for HostHandle {}

/// A CEF `Browser` handle, used from the GTK thread for navigation (these methods may be called
/// on any thread in the browser process).
pub struct BrowserHandle(pub Browser);
unsafe impl Send for BrowserHandle {}

wrap_app! {
    struct BrowsahApp {
        flags: Vec<String>,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(cl) = command_line else { return };
            for flag in &self.flags {
                let flag = flag.trim_start_matches("--");
                match flag.split_once('=') {
                    Some((k, v)) => {
                        cl.append_switch_with_value(
                            Some(&CefString::from(k)),
                            Some(&CefString::from(v)),
                        );
                    }
                    None => cl.append_switch(Some(&CefString::from(flag))),
                }
            }
        }

    }
}

wrap_task! {
    struct UiTask { f: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>> }

    impl Task {
        fn execute(&self) {
            if let Some(f) = self.f.lock().unwrap().take() {
                f();
            }
        }
    }
}

/// Run `f` on CEF's UI thread. For CEF calls that must run there (DevTools, the frame-rate
/// getter and setter, context-menu callbacks).
pub fn on_ui(f: impl FnOnce() + Send + 'static) {
    let mut task = UiTask::new(Arc::new(Mutex::new(Some(Box::new(f)))));
    post_task(ThreadId::UI, Some(&mut task));
}

/// What `init` found out about this process.
pub enum Init {
    /// A CEF child process (renderer, GPU, utility): it has already run to completion.
    Subprocess(i32),
    /// The browser process, with CEF initialised.
    Browser(Runtime),
}

/// Keeps the arguments and app object CEF refers to alive until shutdown.
pub struct Runtime {
    _args: cef::args::Args,
    _app: App,
}

/// Run as a CEF subprocess if this is one, otherwise initialise CEF in the browser process.
pub fn init(flags: Vec<String>, cache: Option<std::path::PathBuf>) -> Init {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = cef::args::Args::new();
    let mut app = BrowsahApp::new(flags);

    // CEF re-executes this binary for its renderer / GPU / utility processes.
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if code >= 0 {
        return Init::Subprocess(code);
    }

    // Chromium keeps cookies, storage and caches here: one directory per profile.
    let cache = cache.unwrap_or_else(|| glib::user_cache_dir().join("browsah"));
    let settings = Settings {
        // TODO(M6): enable the Linux sandbox (setuid helper or user namespaces).
        no_sandbox: 1,
        windowless_rendering_enabled: 1,
        multi_threaded_message_loop: 1,
        log_file: CefString::from(cache.join("cef.log").to_string_lossy().as_ref()),
        log_severity: LogSeverity::WARNING,
        root_cache_path: CefString::from(cache.to_string_lossy().as_ref()),
        ..Default::default()
    };
    let ok = initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut());
    assert_eq!(ok, 1, "CEF failed to initialise");
    Init::Browser(Runtime { _args: args, _app: app })
}

/// Browsers that exist right now (created, not yet closed), counted by the app's life-span handler.
static LIVE_BROWSERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Whether anything has been counted (the benchmarks create browsers without counting them).
static COUNTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A browser was created (any thread).
pub fn browser_created() {
    COUNTING.store(true, std::sync::atomic::Ordering::SeqCst);
    LIVE_BROWSERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// A browser has closed (any thread).
pub fn browser_closed() {
    if LIVE_BROWSERS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        LIVE_BROWSERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Runtime {
    /// Call after every browser has been asked to close. Waits until they have said they are closed (a few milliseconds
    /// usually, at most two seconds), then shuts CEF down. Without counted browsers (the benchmarks) it waits a fixed half second.
    pub fn shutdown(self) {
        use std::sync::atomic::Ordering;
        if COUNTING.load(Ordering::SeqCst) {
            let start = std::time::Instant::now();
            while LIVE_BROWSERS.load(Ordering::SeqCst) > 0 && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(2));
            }
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: browsers closed after {:?} ({} left)", start.elapsed(), LIVE_BROWSERS.load(Ordering::SeqCst));
            }
        } else {
            std::thread::sleep(Duration::from_millis(500));
        }
        let start = std::time::Instant::now();
        shutdown();
        if std::env::var_os("BROWSAH_DEBUG").is_some() {
            eprintln!("browsah: CEF shut down in {:?}", start.elapsed());
        }
    }
}

/// Forget the per-site zoom levels Chromium saved in the profile's `Preferences`. Earlier versions kept pages at the right
/// size on HiDPI displays by setting the browser zoom (about 3.8 at scale 2), and Chromium saves a zoom level per host, so
/// hosts browsah never sets one for (the developer tools, Chromium's error pages) stayed zoomed and now showed twice as
/// large as they should. browsah sets a page's zoom itself on every load (from its own per-site zoom), so nothing is lost.
/// Returns the new file text, or `None` if there is nothing to remove (or it is not the expected JSON).
pub fn without_saved_zoom(preferences: &str) -> Option<String> {
    let mut v: serde_json::Value = serde_json::from_str(preferences).ok()?;
    let partition = v.get_mut("partition")?.as_object_mut()?;
    let levels = partition.get("per_host_zoom_levels")?;
    // Chromium keeps `{"x": {}}` when nothing is saved: only act when some host has an entry.
    let any = levels.as_object().is_some_and(|o| o.values().any(|group| group.as_object().is_some_and(|g| !g.is_empty())));
    if !any {
        return None;
    }
    partition.remove("per_host_zoom_levels");
    serde_json::to_string(&v).ok()
}

/// Apply `without_saved_zoom` to the profile's `Preferences` file, before Chromium starts.
pub fn forget_saved_zoom(cache: &std::path::Path) {
    let file = cache.join("Default").join("Preferences");
    if let Some(new) = std::fs::read_to_string(&file).ok().and_then(|t| without_saved_zoom(&t)) {
        if std::fs::write(&file, new).is_ok() {
            eprintln!("browsah: cleared the zoom levels Chromium had saved for sites (older versions set them to scale pages)");
        }
    }
}

/// The Chromium switches browsah always starts with, added to what the user asked for: native Wayland unless a
/// platform was chosen, and (on Wayland) the fractional-scale feature off. With `WaylandFractionalScaleV1` on,
/// Chromium ignores the device scale factor a windowless browser reports (measured: frames stay at 1x and
/// `devicePixelRatio` stays 1 whatever `GetScreenInfo` says; with `--ozone-platform=x11` or the feature off it is
/// honoured, and scrollbars, `devicePixelRatio` and text all follow the scale). See docs/research/hidpi-scale.md.
pub fn with_default_flags(mut flags: Vec<String>, scale_fix: bool) -> Vec<String> {
    if !flags.iter().any(|f| f.starts_with("--ozone-platform")) {
        flags.insert(0, "--ozone-platform=wayland".into());
    }
    let wayland = flags.iter().rev().find_map(|f| f.strip_prefix("--ozone-platform=")).is_some_and(|p| p == "wayland");
    if wayland && scale_fix {
        const OFF: &str = "WaylandFractionalScaleV1";
        match flags.iter().rposition(|f| f.starts_with("--disable-features=")) {
            // Chromium keeps only the last `--disable-features`: add ours to it.
            Some(i) if !flags[i].split(['=', ',']).any(|x| x == OFF) => flags[i].push_str(&format!(",{OFF}")),
            Some(_) => {}
            None => flags.push(format!("--disable-features={OFF}")),
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn saved_zoom_levels_are_removed_and_nothing_else() {
        let prefs = r#"{"profile":{"name":"x"},"partition":{"per_host_zoom_levels":{"x":{"devtools":{"zoom_level":3.8}}},"other":1}}"#;
        let out = without_saved_zoom(prefs).expect("changed");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["partition"].get("per_host_zoom_levels").is_none());
        assert_eq!(v["partition"]["other"], 1);
        assert_eq!(v["profile"]["name"], "x");
        // Nothing saved, no partition, or not JSON: nothing to do.
        assert!(without_saved_zoom(r#"{"partition":{"per_host_zoom_levels":{}}}"#).is_none());
        assert!(without_saved_zoom(r#"{"partition":{"per_host_zoom_levels":{"x":{}}}}"#).is_none(), "an empty group is nothing saved");
        assert!(without_saved_zoom(r#"{"profile":{}}"#).is_none());
        assert!(without_saved_zoom("not json").is_none());
    }

    #[test]
    fn wayland_and_the_scale_fix_are_added_by_default() {
        let f = with_default_flags(Vec::new(), true);
        assert_eq!(f, v(&["--ozone-platform=wayland", "--disable-features=WaylandFractionalScaleV1"]));
    }

    #[test]
    fn the_users_own_disable_features_is_extended_not_duplicated() {
        let f = with_default_flags(v(&["--disable-features=Foo,Bar"]), true);
        assert_eq!(f.iter().filter(|x| x.starts_with("--disable-features=")).count(), 1);
        assert!(f.contains(&"--disable-features=Foo,Bar,WaylandFractionalScaleV1".to_string()));
        let again = with_default_flags(v(&["--disable-features=WaylandFractionalScaleV1"]), true);
        assert_eq!(again.iter().filter(|x| x.contains("WaylandFractionalScaleV1")).count(), 1);
    }

    #[test]
    fn the_scale_fix_can_be_switched_off() {
        assert_eq!(with_default_flags(Vec::new(), false), v(&["--ozone-platform=wayland"]));
    }

    #[test]
    fn another_platform_is_left_alone() {
        let f = with_default_flags(v(&["--ozone-platform=x11"]), true);
        assert_eq!(f, v(&["--ozone-platform=x11"]));
    }
}
