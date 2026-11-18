//! Single-browser benchmark (`--bench=single`): one windowless browser in a GTK window, compared
//! across the readback (`OnPaint`) and dmabuf (`OnAcceleratedPaint`) paths. Prints paint rate,
//! callback time, bytes copied and GTK frame pacing every two seconds. See docs/research/.

use crate::cef_rt::HostHandle;
use crate::cli::{Mode, Options};
use crate::frames::{import_frame, DmabufFrame, View};
use cef::*;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Default)]
struct Stats {
    window_start: Option<Instant>,
    paints: u32,
    paint_cb_ns: u128,
    bytes: u64,
    dmabuf_failures: u32,
    gtk_frames: u32,
    last_frame_us: Option<i64>,
    frame_intervals_ms: Vec<f64>,
}

struct Ui {
    mode: Mode,
    picture: gtk::Picture,
    display: gdk::Display,
    /// Logical size and scale of the page area.
    size: (i32, i32),
    scale: f32,
    stats: Stats,
    totals: Stats,
    backing: bool,
    last_texture: Option<gdk::Texture>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.borrow_mut().as_mut().map(f))
}


static VIEW: Mutex<View> = Mutex::new(View { size: (1280, 800), scale: 1.0, text: 1.0, backing: true });

// Paint statistics, updated from whichever thread CEF paints on and drained by the GTK tick.
static PAINTS: AtomicU64 = AtomicU64::new(0);
static PAINT_NS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static DMABUF_FAIL: AtomicU64 = AtomicU64::new(0);

static HOST: Mutex<Option<HostHandle>> = Mutex::new(None);

fn with_host(f: impl FnOnce(&BrowserHost)) {
    if let Some(h) = HOST.lock().unwrap().as_ref() {
        f(&h.0);
    }
}

/// GTK thread: show a finished frame.
fn present(tex: gdk::Texture) {
    with_ui(|u| {
        u.last_texture = Some(tex.clone());
        u.picture.set_paintable(Some(&tex));
    });
}

wrap_render_handler! {
    struct PageRender;

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (w, h) = VIEW.lock().unwrap().device_size();
            if let Some(rect) = rect {
                *rect = Rect { x: 0, y: 0, width: w, height: h };
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            let Some(info) = screen_info else { return 0 };
            let v = *VIEW.lock().unwrap();
            let (w, h) = v.device_size();
            let r = Rect { x: 0, y: 0, width: w, height: h };
            info.device_scale_factor = v.dsf();
            info.rect = r.clone();
            info.available_rect = r;
            1
        }

        // Path 1: pixels in CPU memory. The buffer is only valid during the call, so it has to
        // be copied; the texture is built and uploaded on the GTK thread.
        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if type_ != PaintElementType::VIEW || buffer.is_null() {
                return; // popups (<select>, etc.) are ignored in this spike
            }
            let t0 = Instant::now();
            let len = width as usize * height as usize * 4;
            let bytes = glib::Bytes::from(unsafe { std::slice::from_raw_parts(buffer, len) });
            glib::MainContext::default().invoke(move || {
                let tex = gdk::MemoryTexture::new(
                    width,
                    height,
                    gdk::MemoryFormat::B8g8r8a8Premultiplied,
                    &bytes,
                    width as usize * 4,
                );
                present(tex.upcast());
            });
            PAINTS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(len as u64, Ordering::Relaxed);
            PAINT_NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }

        // Path 2: a dmabuf handle to the texture Chromium rendered. Nothing is copied; only the
        // plane descriptors cross to the GTK thread.
        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else { return };
            if type_ != PaintElementType::VIEW {
                return;
            }
            let t0 = Instant::now();
            let Some(frame) = DmabufFrame::from_info(info) else {
                DMABUF_FAIL.fetch_add(1, Ordering::Relaxed);
                return;
            };
            glib::MainContext::default().invoke(move || {
                match with_ui(|u| u.display.clone()).and_then(|d| import_frame(&d, frame)) {
                    Some(tex) => present(tex),
                    None => {
                        DMABUF_FAIL.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
            PAINTS.fetch_add(1, Ordering::Relaxed);
            PAINT_NS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }
}

wrap_life_span_handler! {
    struct PageLife;

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(host) = browser.and_then(|b| b.host()) {
                host.was_hidden(0);
                host.set_focus(1);
                host.was_resized();
                *HOST.lock().unwrap() = Some(HostHandle(host));
            }
        }
    }
}

wrap_client! {
    struct PageClient {
        render: RenderHandler,
        life: LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life.clone())
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn describe(label: &str, mode: Mode, s: &Stats, secs: f64) -> String {
    let mut iv = s.frame_intervals_ms.clone();
    iv.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_cb_ms = if s.paints > 0 { s.paint_cb_ns as f64 / s.paints as f64 / 1e6 } else { 0.0 };
    format!(
        "[{label} {mode:?}] cef paints {:5.1}/s | paint cb {:5.2} ms avg | copied {:7.1} MB/s | \
         gtk frames {:5.1}/s | frame interval p50 {:5.1} p95 {:5.1} p99 {:5.1} ms | dmabuf failures {}",
        s.paints as f64 / secs,
        mean_cb_ms,
        s.bytes as f64 / secs / 1e6,
        s.gtk_frames as f64 / secs,
        percentile(&iv, 0.5),
        percentile(&iv, 0.95),
        percentile(&iv, 0.99),
        s.dmabuf_failures,
    )
}

/// Called every GTK frame: tracks frame pacing, size changes, and prints the periodic report.
fn on_tick(widget: &gtk::Picture, clock: &gdk::FrameClock, seconds: u64, app: &gtk::Application) {
    let now_us = clock.frame_time();
    let (w, h) = (widget.width(), widget.height());
    let scale = widget
        .native()
        .and_then(|n| n.surface())
        .map(|s| s.scale() as f32)
        .unwrap_or(1.0);

    let mut resized = false;
    let mut report = None;
    let mut finished = false;
    with_ui(|u| {
        if (w, h) != u.size || scale != u.scale {
            u.size = (w, h);
            u.scale = scale;
            resized = true;
        }
        let st = &mut u.stats;
        st.paints += PAINTS.swap(0, Ordering::Relaxed) as u32;
        st.paint_cb_ns += PAINT_NS.swap(0, Ordering::Relaxed) as u128;
        st.bytes += BYTES.swap(0, Ordering::Relaxed);
        st.dmabuf_failures += DMABUF_FAIL.swap(0, Ordering::Relaxed) as u32;
        st.gtk_frames += 1;
        if let Some(last) = st.last_frame_us.replace(now_us) {
            st.frame_intervals_ms.push((now_us - last) as f64 / 1000.0);
        }
        let start = *st.window_start.get_or_insert_with(Instant::now);
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= 2.0 {
            report = Some(describe("2s", u.mode, st, elapsed));
            // Fold into totals (skip the first window: page load and warm-up).
            let done = std::mem::take(st);
            if u.totals.window_start.is_some() {
                let t = &mut u.totals;
                t.paints += done.paints;
                t.paint_cb_ns += done.paint_cb_ns;
                t.bytes += done.bytes;
                t.dmabuf_failures += done.dmabuf_failures;
                t.gtk_frames += done.gtk_frames;
                t.frame_intervals_ms.extend(done.frame_intervals_ms);
            } else {
                u.totals.window_start = Some(Instant::now());
            }
            if seconds > 0 && u.totals.window_start.unwrap().elapsed().as_secs() >= seconds {
                report = Some(format!(
                    "\n{}",
                    describe("TOTAL", u.mode, &u.totals, u.totals.window_start.unwrap().elapsed().as_secs_f64())
                ));
                finished = true;
                if let (Ok(path), Some(t)) = (std::env::var("BROWSAH_DUMP"), &u.last_texture) {
                    eprintln!("dumping final frame {}x{} px (view {}x{} logical, scale {})", t.width(), t.height(), u.size.0, u.size.1, u.scale);
                    let _ = t.save_to_png(&path);
                }
            }
        }
    });

    if resized {
        if let Some(view) = with_ui(|u| View { size: u.size, scale: u.scale, text: 1.0, backing: u.backing }) {
            *VIEW.lock().unwrap() = view;
        }
        if w > 0 && h > 0 {
            with_host(|host| host.was_resized());
        }
    }
    if let Some(r) = report {
        println!("{r}");
    }
    if finished {
        app.quit();
    }
}


// ---------------------------------------------------------------------------------------------
// entry
// ---------------------------------------------------------------------------------------------

pub fn run(opts: Rc<Options>) {
    let mode = opts.mode.unwrap_or(Mode::Accel);
    let backing = opts.backing.unwrap_or(true);
    let url = opts.url.clone().unwrap_or_else(|| {
        let page = std::env::current_dir().unwrap().join("bench/scroll.html");
        format!("file://{}", page.display())
    });
    let app = gtk::Application::builder().application_id("be.geee.browsah.bench").build();
    app.connect_activate(move |app| {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(format!("browsah benchmark ({mode:?})"))
            .default_width(1280)
            .default_height(800)
            .child(&picture)
            .build();

        UI.with(|u| {
            *u.borrow_mut() = Some(Ui {
                mode,
                picture: picture.clone(),
                display: WidgetExt::display(&window),
                size: (1280, 800),
                scale: 1.0,
                stats: Stats::default(),
                totals: Stats::default(),
                backing,
                last_texture: None,
            });
        });

        let seconds = opts.seconds;
        let app_for_tick = app.clone();
        picture.add_tick_callback(move |w, clock| {
            on_tick(w, clock, seconds, &app_for_tick);
            glib::ControlFlow::Continue
        });

        let window_info = WindowInfo {
            windowless_rendering_enabled: 1,
            shared_texture_enabled: (mode == Mode::Accel) as _,
            runtime_style: RuntimeStyle::from(sys::cef_runtime_style_t::CEF_RUNTIME_STYLE_ALLOY),
            ..Default::default()
        };
        let browser_settings = BrowserSettings { windowless_frame_rate: 60, ..Default::default() };
        let mut client = PageClient::new(PageRender::new(), PageLife::new());
        *VIEW.lock().unwrap() = View { size: (1280, 800), scale: 1.0, text: 1.0, backing };
        browser_host_create_browser(
            Some(&window_info),
            Some(&mut client),
            Some(&CefString::from(url.as_str())),
            Some(&browser_settings),
            None,
            None,
        );
        window.present();
    });

    app.run_with_args::<&str>(&[]);
    with_host(|host| host.close_browser(1));
}
