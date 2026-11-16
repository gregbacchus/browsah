//! Multi-tab benchmark (`--bench=tabs --tabs=N`): resource cost of N windowless browsers, and
//! how long a tab takes to produce a fresh frame after waking from each sleep tier.
//!
//! CEF runs on its own UI thread (`multi_threaded_message_loop`), so all CEF callbacks arrive on
//! that thread. Shared state lives behind `SH`; GTK objects stay in the `G` thread-local and are
//! only touched on the GTK thread. Frames cross over with `MainContext::invoke`.
//!
//! Tiers: hidden (`was_hidden`), frozen (DevTools `Page.setWebLifecycleState`), discarded
//! (browser closed and recreated). Everything is driven by one async scenario on the GLib loop.

use super::{import_frame, DmabufFrame, Mode, Options, View};
use cef::*;
use gtk::{gdk, glib, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// CEF handles are ref-counted and only used through thread-safe methods.
struct Host(BrowserHost);
unsafe impl Send for Host {}
impl Host {
    // Method access (not field access) so closures capture the whole `Send` wrapper.
    fn get(&self) -> &BrowserHost {
        &self.0
    }
}

struct Tab {
    url: String,
    host: Option<Host>,
    /// Hidden state to apply as soon as the browser exists (`on_after_created`).
    want_hidden: bool,
    /// Wake bookkeeping: set before the action, filled by the first paint / load-finished event.
    mark: Option<Instant>,
    first_paint_ms: Option<f64>,
    load_ms: Option<f64>,
    paints: u64,
}

struct Shared {
    mode: Mode,
    tabs: Vec<Tab>,
    active: usize,
    view: View,
}

static SH: Mutex<Option<Shared>> = Mutex::new(None);
static CDP_ACCEPTED: AtomicUsize = AtomicUsize::new(0);

fn sh<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    f(SH.lock().unwrap().as_mut().expect("tabs state not initialised"))
}

/// GTK-thread-only state.
struct G {
    picture: gtk::Picture,
    display: gdk::Display,
    textures: Vec<Option<gdk::Texture>>,
}

thread_local! {
    static G: RefCell<Option<G>> = const { RefCell::new(None) };
}

fn g<R>(f: impl FnOnce(&mut G) -> R) -> Option<R> {
    G.with(|g| g.borrow_mut().as_mut().map(f))
}

/// GTK thread: store a tab's newest frame, and show it if that tab is the active one.
fn present(tab: usize, tex: gdk::Texture) {
    let active = sh(|s| s.active) == tab;
    g(|g| {
        g.textures[tab] = Some(tex.clone());
        if active {
            g.picture.set_paintable(Some(&tex));
        }
    });
}

/// Called on CEF's thread for every frame. Returns whether the tab is the displayed one.
fn note_paint(i: usize) -> bool {
    sh(|s| {
        let active = s.active == i;
        let t = &mut s.tabs[i];
        t.paints += 1;
        if let Some(m) = t.mark {
            if t.first_paint_ms.is_none() {
                t.first_paint_ms = Some(m.elapsed().as_secs_f64() * 1000.0);
            }
        }
        active
    })
}

wrap_render_handler! {
    struct TabRender { tab: usize }

    impl RenderHandler {
        fn view_rect(&self, _b: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (w, h) = sh(|s| s.view.device_size());
            if let Some(rect) = rect {
                *rect = Rect { x: 0, y: 0, width: w, height: h };
            }
        }

        fn screen_info(&self, _b: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            let Some(info) = info else { return 0 };
            let v = sh(|s| s.view);
            let (w, h) = v.device_size();
            let r = Rect { x: 0, y: 0, width: w, height: h };
            info.device_scale_factor = if v.k() > 1.0 { 1.0 } else { v.scale };
            info.rect = r.clone();
            info.available_rect = r;
            1
        }

        fn on_paint(
            &self,
            _b: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if type_ != PaintElementType::VIEW || buffer.is_null() {
                return;
            }
            // Only the visible tab is turned into a texture.
            if note_paint(self.tab) {
                let tab = self.tab;
                let len = width as usize * height as usize * 4;
                let bytes = glib::Bytes::from(unsafe { std::slice::from_raw_parts(buffer, len) });
                glib::MainContext::default().invoke(move || {
                    let tex = gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, width as usize * 4);
                    present(tab, tex.upcast());
                });
            }
        }

        fn on_accelerated_paint(
            &self,
            _b: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else { return };
            if type_ != PaintElementType::VIEW {
                return;
            }
            if note_paint(self.tab) {
                let tab = self.tab;
                if let Some(frame) = DmabufFrame::from_info(info) {
                    glib::MainContext::default().invoke(move || {
                        if let Some(d) = g(|g| g.display.clone()) {
                            if let Some(tex) = import_frame(&d, frame) {
                                present(tab, tex);
                            }
                        }
                    });
                }
            }
        }
    }
}

wrap_load_handler! {
    struct TabLoad { tab: usize }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _b: Option<&mut Browser>,
            is_loading: ::std::os::raw::c_int,
            _back: ::std::os::raw::c_int,
            _fwd: ::std::os::raw::c_int,
        ) {
            if is_loading == 0 {
                sh(|s| {
                    let t = &mut s.tabs[self.tab];
                    if let Some(m) = t.mark {
                        if t.load_ms.is_none() {
                            t.load_ms = Some(m.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                });
            }
        }
    }
}

wrap_life_span_handler! {
    struct TabLife { tab: usize }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(host) = browser.and_then(|b| b.host()) else { return };
            let hidden = sh(|s| s.tabs[self.tab].want_hidden);
            host.was_hidden(hidden as _);
            if !hidden {
                host.set_focus(1);
            }
            host.was_resized();
            sh(|s| s.tabs[self.tab].host = Some(Host(host)));
        }
    }
}

wrap_client! {
    struct TabClient { render: RenderHandler, load: LoadHandler, life: LifeSpanHandler }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(self.render.clone()) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(self.load.clone()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(self.life.clone()) }
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

/// Run `f` on CEF's UI thread.
fn on_ui(f: impl FnOnce() + Send + 'static) {
    let mut task = UiTask::new(Arc::new(Mutex::new(Some(Box::new(f)))));
    post_task(ThreadId::UI, Some(&mut task));
}

// ---- tab operations (GTK thread) --------------------------------------------------------------

fn host(i: usize) -> Option<BrowserHost> {
    sh(|s| s.tabs[i].host.as_ref().map(|h| h.get().clone()))
}

fn create_tab(i: usize, visible: bool, mark: bool) {
    let (url, accel) = sh(|s| {
        let accel = s.mode == Mode::Accel;
        let t = &mut s.tabs[i];
        t.host = None;
        t.want_hidden = !visible;
        t.first_paint_ms = None;
        t.load_ms = None;
        t.mark = mark.then(Instant::now);
        (t.url.clone(), accel)
    });
    let wi = WindowInfo {
        windowless_rendering_enabled: 1,
        shared_texture_enabled: accel as _,
        runtime_style: RuntimeStyle::from(sys::cef_runtime_style_t::CEF_RUNTIME_STYLE_ALLOY),
        ..Default::default()
    };
    let bs = BrowserSettings { windowless_frame_rate: 60, ..Default::default() };
    let mut client = TabClient::new(TabRender::new(i), TabLoad::new(i), TabLife::new(i));
    browser_host_create_browser(Some(&wi), Some(&mut client), Some(&CefString::from(url.as_str())), Some(&bs), None, None);
}

fn close_tab(i: usize) {
    let h = sh(|s| s.tabs[i].host.take());
    if let Some(h) = h {
        h.get().close_browser(1);
    }
    g(|g| g.textures[i] = None);
}

fn set_hidden(i: usize, hidden: bool) {
    sh(|s| s.tabs[i].want_hidden = hidden);
    if let Some(h) = host(i) {
        h.was_hidden(hidden as _);
    }
}

/// Switch the displayed tab, showing its stale last frame immediately if it has one.
fn set_active(i: usize) {
    sh(|s| s.active = i);
    g(|g| {
        if let Some(t) = g.textures[i].clone() {
            g.picture.set_paintable(Some(&t));
        }
    });
}

/// Freeze/unfreeze via the DevTools protocol, posted to CEF's UI thread.
fn lifecycle(i: usize, state: &'static str) {
    let Some(h) = host(i) else { return };
    let h = Host(h);
    on_ui(move || {
        let Some(mut params) = dictionary_value_create() else { return };
        params.set_string(Some(&CefString::from("state")), Some(&CefString::from(state)));
        let ok = h.get().execute_dev_tools_method(0, Some(&CefString::from("Page.setWebLifecycleState")), Some(&mut params));
        if ok != 0 {
            CDP_ACCEPTED.fetch_add(1, Ordering::Relaxed);
        }
    });
}

async fn sleep(ms: u64) {
    glib::timeout_future(Duration::from_millis(ms)).await;
}

/// Wait until the tab's `first_paint_ms` (and optionally `load_ms`) is set. Returns (paint, load).
async fn wait_events(i: usize, want_load: bool, timeout_ms: u64) -> (Option<f64>, Option<f64>) {
    let start = Instant::now();
    loop {
        let (p, l) = sh(|s| (s.tabs[i].first_paint_ms, s.tabs[i].load_ms));
        if (p.is_some() && (!want_load || l.is_some())) || start.elapsed().as_millis() as u64 > timeout_ms {
            return (p, l);
        }
        sleep(2).await;
    }
}

// ---- measurement ------------------------------------------------------------------------------

struct Row {
    label: String,
    renderers: usize,
    procs: usize,
    pss_mb: f64,
    cpu_main_pct: f64,
    cpu_child_pct: f64,
    power_w: Option<f64>,
    gpu_busy: Option<f64>,
    vram_mb: Option<f64>,
    gtt_mb: Option<f64>,
}

fn read_f64(path: &str) -> Option<f64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn amdgpu_hwmon_power() -> Option<String> {
    for e in std::fs::read_dir("/sys/class/hwmon").ok()? {
        let p = e.ok()?.path();
        if std::fs::read_to_string(p.join("name")).ok()?.trim() == "amdgpu" {
            return Some(p.join("power1_average").to_string_lossy().into_owned());
        }
    }
    None
}

fn drm_dev() -> Option<String> {
    for e in std::fs::read_dir("/sys/class/drm").ok()? {
        let p = e.ok()?.path().join("device");
        if p.join("mem_info_vram_used").exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

/// pid -> (ppid, cpu ticks) for every process.
fn proc_table() -> Vec<(u32, u32, u64)> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return v };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() > 12 {
            let ppid = f[1].parse().unwrap_or(0);
            let ticks = f[11].parse::<u64>().unwrap_or(0) + f[12].parse::<u64>().unwrap_or(0);
            v.push((pid, ppid, ticks));
        }
    }
    v
}

fn descendants(table: &[(u32, u32, u64)], root: u32) -> Vec<(u32, u64)> {
    let mut set = vec![root];
    loop {
        let before = set.len();
        for &(pid, ppid, _) in table {
            if set.contains(&ppid) && !set.contains(&pid) {
                set.push(pid);
            }
        }
        if set.len() == before {
            break;
        }
    }
    table.iter().filter(|(p, _, _)| set.contains(p)).map(|&(p, _, t)| (p, t)).collect()
}

fn pss_kb(pid: u32) -> f64 {
    std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Pss:")).and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok())))
        .unwrap_or(0.0)
}

fn cmdline(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline")).map(|c| String::from_utf8_lossy(&c).replace('\0', " ")).unwrap_or_default()
}

/// Renderers are forked from the general zygote and keep its `--type=zygote` command line, so
/// count the children of the zygote that is not the GPU one (`--no-zygote-sandbox`).
fn count_renderers(table: &[(u32, u32, u64)], tree: &[(u32, u64)]) -> usize {
    let zygotes: Vec<u32> = tree
        .iter()
        .map(|x| x.0)
        .filter(|&p| {
            let c = cmdline(p);
            c.contains("--type=zygote") && !c.contains("--no-zygote-sandbox")
        })
        .collect();
    table
        .iter()
        .filter(|(p, pp, _)| zygotes.contains(pp) && tree.iter().any(|x| x.0 == *p))
        .count()
}

async fn measure(label: impl Into<String>, secs: u64) -> Row {
    let me = std::process::id();
    let power_path = amdgpu_hwmon_power();
    let dev = drm_dev();
    let tab0 = proc_table();
    let tree0 = descendants(&tab0, me);
    let main0: u64 = tree0.iter().filter(|x| x.0 == me).map(|x| x.1).sum();
    let all0: u64 = tree0.iter().map(|x| x.1).sum();
    let (mut power, mut busy, mut n) = (0.0, 0.0, 0.0);
    let samples = secs * 4;
    for _ in 0..samples {
        sleep(250).await;
        if let Some(p) = power_path.as_deref().and_then(read_f64) {
            power += p / 1e6;
        }
        if let Some(d) = dev.as_deref() {
            busy += read_f64(&format!("{d}/gpu_busy_percent")).unwrap_or(0.0);
        }
        n += 1.0;
    }
    let tab1 = proc_table();
    let tree = descendants(&tab1, me);
    let main1: u64 = tree.iter().filter(|x| x.0 == me).map(|x| x.1).sum();
    let all1: u64 = tree.iter().map(|x| x.1).sum();
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    Row {
        label: label.into(),
        renderers: count_renderers(&tab1, &tree),
        procs: tree.len(),
        pss_mb: tree.iter().map(|(p, _)| pss_kb(*p)).sum::<f64>() / 1024.0,
        cpu_main_pct: main1.saturating_sub(main0) as f64 / hz / secs as f64 * 100.0,
        cpu_child_pct: (all1 - main1).saturating_sub(all0 - main0) as f64 / hz / secs as f64 * 100.0,
        power_w: power_path.as_ref().map(|_| power / n),
        gpu_busy: dev.as_ref().map(|_| busy / n),
        vram_mb: dev.as_deref().and_then(|d| read_f64(&format!("{d}/mem_info_vram_used"))).map(|b| b / 1048576.0),
        gtt_mb: dev.as_deref().and_then(|d| read_f64(&format!("{d}/mem_info_gtt_used"))).map(|b| b / 1048576.0),
    }
}

// ---- scenario ---------------------------------------------------------------------------------

fn median(v: &mut Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() { f64::NAN } else { v[v.len() / 2] }
}

fn fmt_opt(v: Option<f64>, prec: usize) -> String {
    v.map(|x| format!("{x:.prec$}")).unwrap_or_else(|| "-".into())
}

// ---- frame-rate sweep -------------------------------------------------------------------------

/// Cap a tab's frame rate (posted to CEF's UI thread).
fn set_rate(i: usize, fps: i32) {
    if let Some(h) = host(i) {
        let h = Host(h);
        on_ui(move || h.get().set_windowless_frame_rate(fps));
    }
}

fn total_paints() -> u64 {
    sh(|s| s.tabs.iter().map(|t| t.paints).sum())
}

struct SweepRow {
    row: Row,
    /// Frames CEF actually delivered per tab per second during the window.
    achieved_fps: f64,
    /// Same, for tab 0 only (the "focused" one in the mixed scenarios).
    tab0_fps: f64,
}

async fn sweep_row(label: String, secs: u64, n: usize) -> SweepRow {
    let p0 = total_paints();
    let t0 = sh(|s| s.tabs[0].paints);
    let row = measure(label, secs).await;
    let p1 = total_paints();
    let t1 = sh(|s| s.tabs[0].paints);
    SweepRow {
        row,
        achieved_fps: (p1 - p0) as f64 / n as f64 / secs as f64,
        tab0_fps: (t1 - t0) as f64 / secs as f64,
    }
}

async fn sweep(app: gtk::Application, n: usize, opts: Rc<Options>) {
    while sh(|s| s.view.size.0 <= 0) {
        sleep(50).await;
    }
    sleep(500).await;
    let (size, scale) = sh(|s| (s.view.size, s.view.scale));
    println!("\n=== frame-rate sweep ===");
    println!(
        "{n} tabs, all visible, mode {:?}, page mode {}, view {}x{} logical @ scale {scale}, CEF on its own UI thread",
        opts.mode, opts.tab_mode, size.0, size.1
    );

    for i in 0..n {
        create_tab(i, true, false);
        sleep(150).await;
    }
    set_active(0);
    sleep(6000).await;

    let mut rows: Vec<SweepRow> = Vec::new();
    for fps in [60, 30, 15, 5] {
        for i in 0..n {
            set_rate(i, fps);
        }
        sleep(3000).await;
        rows.push(sweep_row(format!("all {n} visible @ {fps} fps cap"), 6, n).await);
    }
    for low in [15, 5] {
        set_rate(0, 60);
        for i in 1..n {
            set_rate(i, low);
        }
        sleep(3000).await;
        rows.push(sweep_row(format!("tab 0 @ 60 + {} others @ {low}", n - 1), 6, n).await);
    }
    // Reference: everything but the focused tab hidden.
    for i in 1..n {
        set_hidden(i, true);
    }
    sleep(3000).await;
    rows.push(sweep_row(format!("tab 0 @ 60 + {} hidden", n - 1), 6, n).await);
    for i in 1..n {
        set_hidden(i, false);
    }

    // Ramp-up latency: tab 1 is capped low, then raised to 60 as if the user touched it.
    let mut ramp: Vec<(i32, Vec<(Option<f64>, f64)>)> = Vec::new();
    for from in [5, 15, 30] {
        let mut trials = Vec::new();
        for _ in 0..4 {
            set_rate(1, from);
            sleep(2500).await;
            sh(|s| {
                s.tabs[1].first_paint_ms = None;
                s.tabs[1].mark = Some(Instant::now());
            });
            let before = sh(|s| s.tabs[1].paints);
            set_rate(1, 60);
            let (first, _) = wait_events(1, false, 3000).await;
            sleep(500).await;
            let after = sh(|s| s.tabs[1].paints);
            trials.push((first, (after - before) as f64 / 0.5 + 0.0));
        }
        ramp.push((from, trials));
    }

    println!(
        "\n{:<46} {:>9} {:>9} {:>8} {:>8} {:>7} {:>6} {:>8}",
        "scenario", "fps/tab", "tab0 fps", "main CPU", "kids CPU", "pwr W", "gpu %", "PSS MB"
    );
    for r in &rows {
        println!(
            "{:<46} {:>9.1} {:>9.1} {:>8.1} {:>8.1} {:>7} {:>6} {:>8.0}",
            r.row.label, r.achieved_fps, r.tab0_fps, r.row.cpu_main_pct, r.row.cpu_child_pct,
            fmt_opt(r.row.power_w, 1), fmt_opt(r.row.gpu_busy, 0), r.row.pss_mb
        );
    }
    println!("\nramp-up: time from set_windowless_frame_rate(60) to the first frame, then frames in the next 500 ms");
    for (from, trials) in &ramp {
        let firsts: Vec<f64> = trials.iter().filter_map(|t| t.0).collect();
        let fps: Vec<f64> = trials.iter().map(|t| t.1).collect();
        let mut f = firsts.clone();
        println!(
            "  from {from:>2} fps: first frame median {:.0} ms (all {:?}); then {:.0} fps average over 500 ms",
            median(&mut f),
            firsts.iter().map(|x| x.round() as i64).collect::<Vec<_>>(),
            fps.iter().sum::<f64>() / fps.len().max(1) as f64
        );
    }
    println!("(power is whole-APU package power; other panes' frames are counted but not imported/composited by GTK here)");

    for i in 0..n {
        close_tab(i);
    }
    sleep(500).await;
    app.quit();
}

pub fn run(app: gtk::Application, window: gtk::ApplicationWindow, picture: gtk::Picture, opts: Rc<Options>) {
    let n = opts.tabs.max(2);
    let page = std::env::current_dir().unwrap().join("bench/tab.html");
    let tabs = (0..n)
        .map(|i| Tab {
            url: format!("file://{}?i={i}&mode={}", page.display(), opts.tab_mode),
            host: None,
            want_hidden: true,
            mark: None,
            first_paint_ms: None,
            load_ms: None,
            paints: 0,
        })
        .collect();
    *SH.lock().unwrap() = Some(Shared {
        mode: opts.mode,
        tabs,
        active: 0,
        view: View { size: (0, 0), scale: 1.0, backing: opts.backing },
    });
    G.with(|g| {
        *g.borrow_mut() = Some(G {
            picture: picture.clone(),
            display: WidgetExt::display(&window),
            textures: vec![None; n],
        })
    });

    picture.add_tick_callback(|w, _| {
        let (pw, ph) = (w.width(), w.height());
        let scale = w.native().and_then(|n| n.surface()).map(|s| s.scale() as f32).unwrap_or(1.0);
        let changed = sh(|s| {
            let c = s.view.size != (pw, ph) || s.view.scale != scale;
            s.view.size = (pw, ph);
            s.view.scale = scale;
            c
        });
        if changed && pw > 0 && ph > 0 {
            for i in 0..sh(|s| s.tabs.len()) {
                if let Some(h) = host(i) {
                    h.was_resized();
                }
            }
        }
        glib::ControlFlow::Continue
    });

    window.present();
    if opts.fps_sweep {
        glib::MainContext::default().spawn_local(sweep(app, n, opts));
        return;
    }
    let mode = opts.mode;
    glib::MainContext::default().spawn_local(async move {
        let sample = |k: usize| -> Vec<usize> { (1..n).take(k).collect() };
        let mut rows: Vec<Row> = Vec::new();
        let mut notes: Vec<String> = Vec::new();

        while sh(|s| s.view.size.0 <= 0) {
            sleep(50).await;
        }
        sleep(500).await;
        let (size, scale) = sh(|s| (s.view.size, s.view.scale));
        notes.push(format!(
            "{n} tabs, mode {mode:?}, page mode {}, view {}x{} logical @ scale {scale}, CEF on its own UI thread",
            opts.tab_mode, size.0, size.1
        ));

        rows.push(measure("app only, no tabs", 5).await);

        create_tab(0, true, false);
        set_active(0);
        sleep(4000).await;
        rows.push(measure("1 tab, visible", 5).await);

        for i in 1..n {
            create_tab(i, false, false);
            sleep(150).await;
        }
        sleep(8000).await;
        rows.push(measure(format!("{n} tabs: 1 visible, {} hidden", n - 1), 5).await);

        for i in 1..n {
            set_hidden(i, false);
        }
        sleep(3000).await;
        rows.push(measure(format!("{n} tabs: all visible"), 5).await);

        for i in 1..n {
            set_hidden(i, true);
        }
        sleep(5000).await;

        // Wake from hidden: time from was_hidden(false) to the first fresh frame delivered.
        let mut hidden_wake = Vec::new();
        for k in sample(5) {
            sh(|s| {
                s.tabs[k].first_paint_ms = None;
                s.tabs[k].mark = Some(Instant::now());
            });
            set_hidden(k, false);
            set_active(k);
            hidden_wake.push(wait_events(k, false, 3000).await.0);
            sleep(500).await;
            set_hidden(k, true);
            set_active(0);
            sleep(1000).await;
        }

        // Frozen tier.
        CDP_ACCEPTED.store(0, Ordering::Relaxed);
        for i in 1..n {
            lifecycle(i, "frozen");
        }
        sleep(5000).await;
        let freeze_ok = CDP_ACCEPTED.load(Ordering::Relaxed);
        rows.push(measure(format!("{n} tabs: {} hidden + frozen (CDP accepted {freeze_ok})", n - 1), 5).await);

        let mut frozen_wake = Vec::new();
        for k in sample(5) {
            sh(|s| {
                s.tabs[k].first_paint_ms = None;
                s.tabs[k].mark = Some(Instant::now());
            });
            lifecycle(k, "active");
            set_hidden(k, false);
            set_active(k);
            frozen_wake.push(wait_events(k, false, 3000).await.0);
            sleep(500).await;
            set_hidden(k, true);
            set_active(0);
            lifecycle(k, "frozen");
            sleep(1000).await;
        }
        for i in 1..n {
            lifecycle(i, "active");
        }
        sleep(500).await;

        // Discard tier: close the browser entirely, recreate on demand.
        for i in 1..n {
            close_tab(i);
        }
        sleep(4000).await;
        rows.push(measure(format!("{n} tabs: {} discarded (only tab 0 alive)", n - 1), 5).await);

        let mut disc_paint = Vec::new();
        let mut disc_load = Vec::new();
        for k in sample(5) {
            create_tab(k, true, true);
            set_active(k);
            let (p, l) = wait_events(k, true, 8000).await;
            disc_paint.push(p);
            disc_load.push(l);
            sleep(500).await;
            close_tab(k);
            set_active(0);
            sleep(1500).await;
        }

        // ---- report
        println!("\n=== multi-tab benchmark ===");
        for n in &notes {
            println!("{n}");
        }
        println!("{:<58} {:>5} {:>5} {:>9} {:>8} {:>8} {:>7} {:>6} {:>8} {:>8}", "scenario", "rndr", "procs", "PSS MB", "main CPU", "kids CPU", "pwr W", "gpu %", "VRAM MB", "GTT MB");
        for r in &rows {
            println!(
                "{:<58} {:>5} {:>5} {:>9.0} {:>8.1} {:>8.1} {:>7} {:>6} {:>8} {:>8}",
                r.label, r.renderers, r.procs, r.pss_mb, r.cpu_main_pct, r.cpu_child_pct,
                fmt_opt(r.power_w, 1), fmt_opt(r.gpu_busy, 0), fmt_opt(r.vram_mb, 0), fmt_opt(r.gtt_mb, 0)
            );
        }
        let ms = |v: &[Option<f64>]| -> String {
            let mut ok: Vec<f64> = v.iter().flatten().copied().collect();
            let missing = v.len() - ok.len();
            let all = ok.clone();
            format!(
                "median {:.0} ms, max {:.0} ms (n={}{}) all: {:?}",
                median(&mut ok),
                all.iter().cloned().fold(f64::NAN, f64::max),
                all.len(),
                if missing > 0 { format!(", {missing} timed out") } else { String::new() },
                all.iter().map(|x| x.round() as i64).collect::<Vec<_>>()
            )
        };
        println!("\ntime to first fresh frame after wake:");
        println!("  hidden    -> shown : {}", ms(&hidden_wake));
        println!("  frozen    -> shown : {}", ms(&frozen_wake));
        println!("  discarded -> shown : {}   (new browser)", ms(&disc_paint));
        println!("  discarded -> loaded: {}", ms(&disc_load));
        println!("(memory in PSS: shared pages counted proportionally; power is whole-APU package power\n from amdgpu hwmon, so it includes anything else running on the machine)");
        close_tab(0);
        sleep(500).await;
        app.quit();
    });
}
