//! `--latency`: how long from an input event to the next frame arriving at GTK.
//!
//! The test page is static, so Chromium produces no frames until something changes; the first
//! frame delivered after an input is therefore the one that shows its effect. Each trial waits
//! for quiet, sends the input through the real GTK controllers (as GTK does for a user), and
//! times until the frame hook fires on the GTK thread.
//!
//! What this does **not** include: GTK drawing that frame (up to one GTK frame, ~8 ms at 120 Hz),
//! the compositor, and display scan-out. Add roughly one to two display frames for the full
//! input-to-photon latency.

use super::Shell;
use adw::prelude::*;
use gtk::gdk::{Key, ModifierType};
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub fn start_url() -> String {
    let dir = std::env::current_dir().expect("current dir");
    format!("file://{}/bench/pages/latency.html", dir.display())
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

struct Stats {
    label: &'static str,
    samples: Vec<f64>,
    timeouts: u32,
}

impl Stats {
    fn print(&mut self) {
        self.samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let s = &self.samples;
        let mean = s.iter().sum::<f64>() / s.len().max(1) as f64;
        println!(
            "{:<8} n={:<3} mean {:5.1}  p50 {:5.1}  p95 {:5.1}  p99 {:5.1}  max {:5.1} ms{}",
            self.label,
            s.len(),
            mean,
            percentile(s, 0.5),
            percentile(s, 0.95),
            percentile(s, 0.99),
            s.last().copied().unwrap_or(f64::NAN),
            if self.timeouts > 0 { format!("   ({} timed out)", self.timeouts) } else { String::new() }
        );
    }
}

/// Run `n` trials of `action`, each timed to the next frame.
async fn measure(shell: &Rc<Shell>, label: &'static str, n: usize, action: impl Fn(usize)) -> Stats {
    let last_frame: Rc<Cell<Instant>> = Rc::new(Cell::new(Instant::now()));
    let first_after: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let armed = Rc::new(Cell::new(false));
    {
        let (last_frame, first_after, armed) = (last_frame.clone(), first_after.clone(), armed.clone());
        shell.view().set_frame_hook(Some(Rc::new(move |_| {
            let now = Instant::now();
            last_frame.set(now);
            if armed.get() && first_after.get().is_none() {
                first_after.set(Some(now));
            }
        })));
    }
    let mut stats = Stats { label, samples: Vec::new(), timeouts: 0 };
    for i in 0..n {
        // Wait until no frame has arrived for a while.
        while last_frame.get().elapsed() < Duration::from_millis(250) {
            glib::timeout_future(Duration::from_millis(10)).await;
        }
        first_after.set(None);
        armed.set(true);
        let t0 = Instant::now();
        action(i);
        let start = Instant::now();
        while first_after.get().is_none() && start.elapsed() < Duration::from_secs(1) {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
        armed.set(false);
        match first_after.get() {
            Some(t) => stats.samples.push(t.duration_since(t0).as_secs_f64() * 1000.0),
            None => stats.timeouts += 1,
        }
    }
    shell.view().set_frame_hook(None);
    stats
}

pub fn spawn(app: adw::Application, shell: Rc<Shell>, frame_rate: i32) {
    glib::MainContext::default().spawn_local(async move {
        let ready = super::selftest::wait_until(15_000, || {
            let st = shell.state();
            let s = st.borrow();
            s.ready && !s.loading && shell.view().frames() > 0
        })
        .await;
        if !ready {
            println!("latency: the page did not load");
            app.quit();
            return;
        }
        // Settle: first frames, zoom, layout.
        glib::timeout_future(Duration::from_millis(1500)).await;
        let c = &shell.view().controllers;
        let refresh = gtk::prelude::WidgetExt::display(&shell.window)
            .monitors()
            .item(0)
            .and_then(|m| m.downcast::<gtk::gdk::Monitor>().ok())
            .map(|m| m.refresh_rate() as f64 / 1000.0);
        println!(
            "\n=== input latency (input event to the next frame arriving at GTK) ===\nCEF frame rate cap {frame_rate} fps, display refresh {} Hz, mode {}",
            refresh.map(|r| format!("{r:.0}")).unwrap_or_else(|| "?".into()),
            if cfg!(debug_assertions) { "debug build" } else { "release build" }
        );

        let n = 60;
        shell.view().widget.grab_focus();
        let mut key = measure(&shell, "key", n, |_| {
            let k = &c.key;
            k.emit_by_name::<bool>("key-pressed", &[&Key::a, &38u32, &ModifierType::empty()]);
            k.emit_by_name::<()>("key-released", &[&Key::a, &38u32, &ModifierType::empty()]);
        })
        .await;
        key.print();

        let mut click = measure(&shell, "click", n, |_| {
            c.motion.emit_by_name::<()>("motion", &[&200.0f64, &200.0f64]);
            c.click.emit_by_name::<()>("pressed", &[&1i32, &200.0f64, &200.0f64]);
            c.click.emit_by_name::<()>("released", &[&1i32, &200.0f64, &200.0f64]);
        })
        .await;
        click.print();

        let mut wheel = measure(&shell, "wheel", n, |i| {
            // Alternate directions so the page keeps moving within its range.
            let dy = if i % 2 == 0 { 3.0f64 } else { -3.0f64 };
            c.scroll.emit_by_name::<bool>("scroll", &[&0.0f64, &dy]);
        })
        .await;
        wheel.print();
        println!("(add about one GTK frame and the compositor/display for photon latency; see src/app/latency.rs)");

        shell.close_all();
        glib::timeout_future(Duration::from_millis(300)).await;
        app.quit();
    });
}
