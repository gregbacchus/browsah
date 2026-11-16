//! Watching config files for changes (`watch_config`).
//!
//! Uses GIO file monitors (inotify), which also see an editor replacing the file by renaming a
//! new one over it. Events are coalesced: one reload shortly after the last change.

use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Keeps the monitors alive; dropping it stops watching.
pub struct ConfigWatch {
    _monitors: Vec<gio::FileMonitor>,
}

impl ConfigWatch {
    /// Call `on_change` (on the GTK thread) shortly after any of `paths` changes. The files need
    /// not exist yet.
    pub fn start(paths: &[PathBuf], on_change: impl Fn() + 'static) -> ConfigWatch {
        let on_change = Rc::new(on_change);
        let pending = Rc::new(Cell::new(false));
        let mut monitors = Vec::new();
        for path in paths {
            let file = gio::File::for_path(path);
            match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
                Ok(m) => {
                    let (on_change, pending) = (on_change.clone(), pending.clone());
                    m.connect_changed(move |_, _, _, _| {
                        if !pending.replace(true) {
                            let (on_change, pending) = (on_change.clone(), pending.clone());
                            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                                pending.set(false);
                                on_change();
                            });
                        }
                    });
                    monitors.push(m);
                }
                Err(e) => eprintln!("config: cannot watch {}: {e}", path.display()),
            }
        }
        ConfigWatch { _monitors: monitors }
    }
}
