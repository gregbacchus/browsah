//! A watch on the GTK main loop: "Not Responding" (and "runs slow") means the thread that draws the window and answers the
//! compositor's pings was busy for seconds. A beat on the main loop every 100 ms, and a helper thread that notices when the
//! beat stops, so the journal says how long the loop was blocked and what the thread was doing (running, or waiting in which
//! system call) while it was. `browsah log` shows it. docs/research/2026-11-21-slow-start.md.

use gtk::glib;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A stall shorter than this is not worth a line.
const REPORT_AFTER: Duration = Duration::from_millis(750);

/// Start watching the calling (GTK main) thread. Not for tests and benchmarks.
pub(super) fn watch_main_loop() {
    let Ok(me) = std::fs::read_link("/proc/thread-self") else { return };
    let thread_dir = std::path::Path::new("/proc").join(me);
    let origin = Instant::now();
    let beat = Arc::new(AtomicU64::new(0));
    let b = beat.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        b.store(origin.elapsed().as_millis() as u64, Ordering::Relaxed);
        glib::ControlFlow::Continue
    });
    let _ = std::thread::Builder::new().name("stall-watch".into()).spawn(move || {
        let mut reported = false;
        let mut sampled = String::new();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = origin.elapsed().as_millis() as u64;
            let last = beat.load(Ordering::Relaxed);
            let gap = Duration::from_millis(now.saturating_sub(last));
            if gap >= REPORT_AFTER {
                // Keep the most recent look at what the thread is doing; the first is the most telling.
                if !reported {
                    sampled = describe(&thread_dir);
                    reported = true;
                    crate::log_warn!("the main loop has not run for {}ms: {sampled}", gap.as_millis());
                }
            } else if reported {
                reported = false;
                crate::log_warn!("the main loop is running again (it was blocked about {}ms more than it should: {sampled})", now.saturating_sub(last));
            }
        }
    });
}

/// What the thread is doing now: its scheduler state and the system call it is in (a futex is a lock or a wait on another thread).
fn describe(dir: &std::path::Path) -> String {
    let stat = std::fs::read_to_string(dir.join("stat")).unwrap_or_default();
    let state = stat.rsplit(')').next().and_then(|r| r.split_whitespace().next()).unwrap_or("?");
    let syscall = std::fs::read_to_string(dir.join("syscall")).unwrap_or_default();
    let wchan = std::fs::read_to_string(dir.join("wchan")).unwrap_or_default();
    let call = syscall.split_whitespace().next().unwrap_or("?");
    format!("state {state} ({}), system call {call}, waiting in {}", match state { "R" => "running", "S" => "sleeping", "D" => "disk wait", _ => "other" }, wchan.trim())
}
