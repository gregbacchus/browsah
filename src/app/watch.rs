//! Watching config files for changes (`watch_config`).
//!
//! Uses GIO file monitors (inotify), which also see an editor replacing the file by renaming a
//! new one over it. Events are coalesced: one reload shortly after the last change.

use gtk::{gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// Keeps the monitors alive; dropping it stops watching.
pub struct ConfigWatch {
    monitors: RefCell<Vec<gio::FileMonitor>>,
    on_change: Rc<dyn Fn()>,
    pending: Rc<Cell<bool>>,
}

impl ConfigWatch {
    /// Call `on_change` (on the GTK thread) shortly after any watched file changes. The files need
    /// not exist yet.
    pub fn start(paths: &[PathBuf], on_change: impl Fn() + 'static) -> ConfigWatch {
        let w = ConfigWatch { monitors: RefCell::new(Vec::new()), on_change: Rc::new(on_change), pending: Rc::new(Cell::new(false)) };
        for p in paths {
            w.add(p);
        }
        w
    }

    /// Also watch `path` (a workspace's file, when its window opens).
    pub fn add(&self, path: &Path) {
        let file = gio::File::for_path(path);
        match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            Ok(m) => {
                let (on_change, pending) = (self.on_change.clone(), self.pending.clone());
                m.connect_changed(move |_, _, _, _| {
                    if !pending.replace(true) {
                        let (on_change, pending) = (on_change.clone(), pending.clone());
                        glib::timeout_add_local_once(Duration::from_millis(300), move || {
                            pending.set(false);
                            on_change();
                        });
                    }
                });
                self.monitors.borrow_mut().push(m);
            }
            Err(e) => eprintln!("config: cannot watch {}: {e}", path.display()),
        }
    }
}
