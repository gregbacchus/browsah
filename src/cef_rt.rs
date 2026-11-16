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

impl Runtime {
    /// Call after every browser has been asked to close.
    pub fn shutdown(self) {
        std::thread::sleep(Duration::from_millis(500)); // CEF's own thread finishes the closes
        shutdown();
    }
}
