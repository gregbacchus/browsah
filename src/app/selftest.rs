//! `--selftest`: a scripted check of the navigation shell against two local pages
//! (`bench/pages/a.html`, `b.html`). It drives the same paths the user does (address entry,
//! window actions such as the back shortcut) and reads the result back from the widgets.
//! Prints PASS/FAIL per check and exits non-zero if any failed. No network needed.

use super::Shell;
use adw::prelude::*;
use gtk::gdk::{self, Key, ModifierType};
use gtk::gio;
use gtk::glib;
use gtk::gdk::prelude::PaintableExt;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

fn page(name: &str) -> String {
    let dir = std::env::current_dir().expect("current dir");
    format!("file://{}/bench/pages/{name}", dir.display())
}

pub fn start_url() -> String {
    page("a.html")
}

struct Report {
    failed: u32,
    total: u32,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: impl FnOnce() -> String) {
        self.total += 1;
        if ok {
            println!("PASS  {name}");
        } else {
            self.failed += 1;
            println!("FAIL  {name}: {}", detail());
        }
    }
}

/// Poll until `cond` holds or `ms` elapse.
pub async fn wait_until(ms: u64, cond: impl Fn() -> bool) -> bool {
    let start = std::time::Instant::now();
    while !cond() {
        if start.elapsed().as_millis() as u64 > ms {
            return false;
        }
        glib::timeout_future(Duration::from_millis(20)).await;
    }
    true
}

/// Render the whole window (headerbar and page) to a PNG, for checking the layout by eye.
fn screenshot(window: &adw::ApplicationWindow, path: &str) -> bool {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let (w, h) = (paintable.intrinsic_width(), paintable.intrinsic_height());
    if w <= 0 || h <= 0 {
        return false;
    }
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let Some(node) = snapshot.to_node() else { return false };
    let Some(renderer) = window.native().and_then(|n| n.renderer()) else { return false };
    renderer.render_texture(&node, None).save_to_png(path).is_ok()
}

/// With `BROWSAH_SHOTS=<directory>` set, save a picture of a widget (a popover, a dialog or a whole window) as
/// `<directory>/<name>.png`, for checking the layout by eye. Does nothing without it.
fn shot(widget: &impl IsA<gtk::Widget>, name: &str) {
    let Some(dir) = std::env::var_os("BROWSAH_SHOTS") else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = std::path::Path::new(&dir).join(format!("{name}.png"));
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let (w, h) = (paintable.intrinsic_width(), paintable.intrinsic_height());
    let ok = (|| {
        if w <= 0 || h <= 0 {
            return false;
        }
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w as f64, h as f64);
        let Some(node) = snapshot.to_node() else { return false };
        let Some(renderer) = widget.native().and_then(|n| n.renderer()) else { return false };
        renderer.render_texture(&node, None).save_to_png(&path).is_ok()
    })();
    println!("shot {name}: {}", if ok { "saved" } else { "failed" });
}

/// The colours (r, g, b) of the window's pixels at the given points (window coordinates), as GTK renders them.
fn pixels(widget: &impl IsA<gtk::Widget>, points: &[(i32, i32)]) -> Vec<(u8, u8, u8)> {
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let (w, h) = (paintable.intrinsic_width(), paintable.intrinsic_height());
    if w <= 0 || h <= 0 {
        return Vec::new();
    }
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let (Some(node), Some(renderer)) = (snapshot.to_node(), widget.native().and_then(|n| n.renderer())) else { return Vec::new() };
    let texture = renderer.render_texture(&node, None);
    let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
    downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    points
        .iter()
        .filter_map(|&(x, y)| {
            let i = y as usize * stride + x as usize * 4;
            Some((*bytes.get(i)?, *bytes.get(i + 1)?, *bytes.get(i + 2)?))
        })
        .collect()
}

/// The sections `BROWSAH_SELFTEST_ONLY` can name, in the order they run (a test keeps this in step with `run`).
const SECTIONS: &[&str] = &[
    "basic", "input", "scroll", "tabs", "opener", "windows", "restore", "locked", "bookmarks", "send_to_workspace", "routing", "modals", "history",
    "sweep", "closed_workspace_sweep", "new_tab_page", "permissions", "downloads", "cookies", "storage", "session_cookies", "session_scope", "webapp",
    "install", "suggest", "jsdialog", "zoom", "find", "fullscreen", "devtools", "print", "groups", "menu", "hidpi", "frame",
];

pub fn spawn(app: adw::Application, shell: Rc<Shell>, exit: Rc<Cell<i32>>, accel: bool) {
    if let Ok(list) = std::env::var("BROWSAH_SELFTEST_ONLY") {
        if list.trim() == "list" {
            println!("self-test sections (BROWSAH_SELFTEST_ONLY=name[,name...]):\n  {}", SECTIONS.join("\n  "));
            std::process::exit(0);
        }
        let unknown: Vec<&str> = list.split(',').map(str::trim).filter(|s| !SECTIONS.contains(s)).collect();
        if !unknown.is_empty() {
            eprintln!("BROWSAH_SELFTEST_ONLY: unknown section(s) {unknown:?}; BROWSAH_SELFTEST_ONLY=list shows them");
            std::process::exit(2);
        }
    }
    super::prompts::set_ui_test(true); // questions are held for the test to answer, not shown
    super::view::set_synthetic_pointer_only(true);
    glib::MainContext::default().spawn_local(async move {
        let mut r = Report { failed: 0, total: 0 };
        run(&shell, &mut r, accel).await;
        println!("\n{} of {} checks passed", r.total - r.failed, r.total);
        exit.set(if r.failed == 0 { 0 } else { 1 });
        shell.close_all();
        glib::timeout_future(Duration::from_millis(300)).await;
        app.quit();
    });
}

async fn run(shell: &Rc<Shell>, r: &mut Report, accel: bool) {
    let st = &shell.state();
    let title_is = |t: &'static str| {
        let st = st.clone();
        move || {
            let s = st.borrow();
            !s.loading && s.title == t
        }
    };

    // The widgets reach the shell through weak references; the window must own it or the address
    // bar and buttons die at the end of start-up (the test's own reference hides that).
    r.check("the window registry owns the shell", Rc::strong_count(shell) >= 2, || format!("strong count {}", Rc::strong_count(shell)));

    let ready = wait_until(15_000, || st.borrow().ready).await;
    r.check("browser is created", ready, || "no Ready event within 15 s".into());
    if !ready {
        return;
    }

    // First page, passed at creation.
    let ok = wait_until(15_000, title_is("Page A")).await;
    r.check("start page loads and its title arrives", ok, || format!("{:?}", st.borrow()));
    let a_url = st.borrow().address.clone();
    r.check("address is the page's URL", a_url.ends_with("/bench/pages/a.html"), || a_url.clone());

    let frames = wait_until(10_000, || shell.view().frames() > 0 && shell.view().has_frame()).await;
    r.check(if accel { "frames arrive (dmabuf)" } else { "frames arrive (readback)" }, frames, || {
        format!("frames={}", shell.view().frames())
    });

    // With the Mesa workaround missing (BROWSAH_NO_GPU_WORKAROUNDS=1) the dmabuf path stays silent
    // on AMD and the shell must have fallen back to readback; otherwise it must not have.
    let expect_fallback = accel && std::env::var_os("BROWSAH_NO_GPU_WORKAROUNDS").is_some();
    r.check(
        if expect_fallback { "silent dmabuf path falls back to readback" } else { "no fallback when frames arrive" },
        st.borrow().fell_back == expect_fallback,
        || format!("fell_back={}", st.borrow().fell_back),
    );

    if let Ok(path) = std::env::var("BROWSAH_SCREENSHOT") {
        // Let the page settle so the frame shown is the finished one.
        glib::timeout_future(Duration::from_millis(500)).await;
        println!("screenshot {}: {}", path, if screenshot(&shell.window, &path) { "saved" } else { "failed" });
    }
    r.check("window title follows the page title", shell.window.title().as_deref() == Some("Page A"), || {
        format!("{:?}", shell.window.title())
    });
    r.check("address bar shows the address", shell.entry.text() == a_url, || shell.entry.text().to_string());
    r.check("back and forward disabled on the first page", !shell.back.is_sensitive() && !shell.forward.is_sensitive(), || {
        format!("back={} forward={}", shell.back.is_sensitive(), shell.forward.is_sensitive())
    });
    r.check("reload shown, stop hidden when idle", shell.reload.is_visible() && !shell.stop.is_visible(), || {
        format!("reload={} stop={}", shell.reload.is_visible(), shell.stop.is_visible())
    });

    // Basic navigation (a few seconds; skipped when only other sections are asked for).
    'basic: {
        if !wants("basic") {
            break 'basic;
        }
        // Navigate by typing in the address bar, as a user would.
        shell.entry.set_text(&page("b.html"));
        shell.entry.emit_by_name::<()>("activate", &[]);
        let ok = wait_until(15_000, title_is("Page B")).await;
        r.check("typing an address and pressing Enter navigates", ok, || format!("{:?}", st.borrow()));
        r.check("back enabled, forward disabled after navigating", shell.back.is_sensitive() && !shell.forward.is_sensitive(), || {
            format!("back={} forward={}", shell.back.is_sensitive(), shell.forward.is_sensitive())
        });
        let b_url = st.borrow().address.clone();
        r.check("address bar updated to the new page", shell.entry.text() == b_url && b_url.ends_with("/b.html"), || {
            format!("entry={:?} address={b_url:?}", shell.entry.text())
        });

        // Back / forward through the window actions (what the shortcuts trigger).
        let _ = WidgetExt::activate_action(&shell.window, "win.back", None);
        let ok = wait_until(15_000, title_is("Page A")).await;
        r.check("back returns to the first page", ok, || format!("{:?}", st.borrow()));
        r.check("forward enabled, back disabled after going back", shell.forward.is_sensitive() && !shell.back.is_sensitive(), || {
            format!("back={} forward={}", shell.back.is_sensitive(), shell.forward.is_sensitive())
        });

        let _ = WidgetExt::activate_action(&shell.window, "win.forward", None);
        let ok = wait_until(15_000, title_is("Page B")).await;
        r.check("forward returns to the second page", ok, || format!("{:?}", st.borrow()));

        // Reload completes a load cycle.
        let before = st.borrow().load_cycles;
        let _ = WidgetExt::activate_action(&shell.window, "win.reload", None);
        let ok = wait_until(15_000, || {
            let s = st.borrow();
            s.load_cycles > before && !s.loading
        })
        .await;
        r.check("reload completes", ok, || format!("{:?}", st.borrow()));
        r.check("still on the second page after reload", st.borrow().title == "Page B", || format!("{:?}", st.borrow()));

        // about:blank shows an empty address bar rather than the literal text.
        shell.entry.set_text("about:blank");
        shell.entry.emit_by_name::<()>("activate", &[]);
        let ok = wait_until(15_000, || {
            let s = st.borrow();
            !s.loading && s.address == "about:blank"
        })
        .await;
        r.check("about:blank loads", ok, || format!("{:?}", st.borrow()));
        r.check("address bar is empty for about:blank", shell.entry.text().is_empty(), || shell.entry.text().to_string());

        // Typing in the address bar must not be overwritten by a page's address change.
        shell.entry.set_text("half-typed");
        let _ = WidgetExt::activate_action(&shell.window, "win.back", None);
        let _ = wait_until(15_000, title_is("Page B")).await;
        r.check("text being typed survives a page navigation", shell.entry.text() == "half-typed", || shell.entry.text().to_string());

    }

    if wants("input") {
        run_input(shell, r).await;
    }
    if wants("scroll") {
        run_scroll(shell, r).await;
    }
    if let Ok(url) = std::env::var("BROWSAH_MANIFEST_TEST") {
        // Spike: load a page and read its web manifest through the DevTools protocol.
        let st = shell.state();
        shell.entry.set_text(&url);
        shell.entry.emit_by_name::<()>("activate", &[]);
        let before = st.borrow().address.clone();
        let _ = wait_until(30_000, || { let s = st.borrow(); !s.loading && s.address != before }).await;
        glib::timeout_future(Duration::from_millis(1000)).await;
        let v = shell.view();
        let t0 = std::time::Instant::now();
        let id = v.cdp("Page.getAppManifest");
        let ok = wait_until(10_000, || v.cdp_result(id).is_some()).await;
        println!("manifest spike: answered={ok} in {} ms", t0.elapsed().as_millis());
        if let Some((success, json)) = v.cdp_result(id) {
            println!("manifest spike: success={success}\n{json}");
        }
        for icon in ["icon.svg", "icon-192.png", "missing.png"] {
            let u = url.replace("index.html", icon);
            let t0 = std::time::Instant::now();
            v.download_image(&u);
            let ok = wait_until(10_000, || v.image_result(&u).is_some()).await;
            println!("icon spike: {icon}: answered={ok} in {} ms, (status, w, h)={:?}", t0.elapsed().as_millis(), v.image_result(&u));
        }
    }
    if let Ok(list) = std::env::var("BROWSAH_NET_TEST") {
        // Optional: type each of these (space separated) into the address bar in turn and report
        // where the page ends up (needs network or a local server).
        let st = shell.state();
        for text in list.split_whitespace() {
            shell.entry.set_text(text);
            let before = (st.borrow().address.clone(), st.borrow().load_cycles);
            shell.entry.emit_by_name::<()>("activate", &[]);
            let _ = wait_until(30_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before.1 }).await;
            glib::timeout_future(Duration::from_millis(std::env::var("BROWSAH_NET_WAIT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500))).await; // let page scripts finish
            println!("net test: {:?} -> address {:?} title {:?}", text, st.borrow().address, st.borrow().title);
        }
    }
    if wants("tabs") {
        run_tabs(shell, r).await;
    }
    if wants("opener") {
        run_opener(shell, r).await;
    }
    if wants("windows") {
        run_windows(shell, r).await;
    }
    if wants("restore") {
        run_restore(shell, r).await;
    }
    if wants("locked") {
        run_locked(shell, r).await;
    }
    if wants("bookmarks") {
        run_bookmarks(shell, r).await;
    }
    if wants("send_to_workspace") {
        run_send_to_workspace(shell, r).await;
    }
    if wants("routing") {
        run_routing(shell, r).await;
    }
    if wants("modals") {
        run_modals(shell, r).await;
    }
    if wants("history") {
        run_history(shell, r).await;
    }
    if wants("sweep") {
        run_sweep(shell, r).await;
    }
    if wants("closed_workspace_sweep") {
        run_closed_workspace_sweep(shell, r).await;
    }
    if wants("new_tab_page") {
        run_new_tab_page(shell, r).await;
    }
    if wants("permissions") {
        run_permissions(shell, r).await;
    }
    if wants("downloads") {
        run_downloads(shell, r).await;
    }
    if wants("cookies") {
        run_cookies(shell, r).await;
    }
    if wants("storage") {
        run_storage(shell, r).await;
    }
    if wants("session_cookies") {
        run_session_cookies(shell, r).await;
    }
    if wants("session_scope") {
        run_session_scope(shell, r).await;
    }
    if wants("webapp") {
        run_webapp(shell, r).await;
    }
    if wants("install") {
        run_install(shell, r).await;
    }
    if wants("suggest") {
        run_suggest(shell, r).await;
    }
    if wants("jsdialog") {
        run_jsdialog(shell, r).await;
    }
    if wants("zoom") {
        run_zoom(shell, r).await;
    }
    if wants("find") {
        run_find(shell, r).await;
    }
    if wants("fullscreen") {
        run_fullscreen(shell, r).await;
    }
    if wants("devtools") {
        run_devtools(shell, r).await;
    }
    if wants("print") {
        run_print(shell, r).await;
    }
    if wants("groups") {
        run_groups(shell, r).await;
    }
    if wants("menu") {
        run_menu(shell, r).await;
    }
    if wants("hidpi") {
        run_hidpi(shell, r).await;
    }
    if wants("frame") {
        run_frame(shell, r).await;
    }
}

// ---------------------------------------------------------------------------------------------
// Input: drive the real GTK controllers by emitting their signals, as GTK does for a user.
// ---------------------------------------------------------------------------------------------

/// Whether a section of the self-test should run. `BROWSAH_SELFTEST_ONLY=install,cookies` runs only those
/// (the name is the function's, without `run_`); without it everything runs (about four minutes).
fn wants(section: &str) -> bool {
    match std::env::var("BROWSAH_SELFTEST_ONLY") {
        Ok(list) => list.split(',').any(|s| s.trim() == section),
        Err(_) => true,
    }
}

/// A field of the test page's title (`name:value | name:value`...).
fn field(title: &str, name: &str) -> String {
    title
        .split(" | ")
        .find_map(|f| f.strip_prefix(&format!("{name}:")))
        .unwrap_or("")
        .to_owned()
}

struct Driver<'a> {
    shell: &'a Rc<Shell>,
}

impl Driver<'_> {
    fn mv(&self, x: f64, y: f64) {
        self.shell.view().controllers.motion.emit_by_name::<()>("motion", &[&x, &y]);
    }

    fn press(&self, x: f64, y: f64) {
        self.mv(x, y);
        self.shell.view().controllers.click.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
    }

    fn release(&self, x: f64, y: f64) {
        self.shell.view().controllers.click.emit_by_name::<()>("released", &[&1i32, &x, &y]);
    }

    fn click(&self, x: f64, y: f64) {
        self.press(x, y);
        self.release(x, y);
    }

    fn scroll(&self, dy: f64) {
        self.shell.view().controllers.scroll.emit_by_name::<bool>("scroll", &[&0.0f64, &dy]);
    }

    /// Press and release a key. Returns whether the press was consumed (false = propagated).
    fn key(&self, key: Key, mods: ModifierType) -> bool {
        let keycode = 38u32;
        let k = &self.shell.view().controllers.key;
        let consumed = k.emit_by_name::<bool>("key-pressed", &[&key, &keycode, &mods]);
        k.emit_by_name::<()>("key-released", &[&key, &keycode, &mods]);
        consumed
    }

    fn type_str(&self, s: &str) {
        for c in s.chars() {
            let shift = c.is_uppercase() || "!@#$%^&*()_+{}|:\"<>?~".contains(c);
            let key = unsafe { <Key as glib::translate::FromGlib<u32>>::from_glib(gdk::unicode_to_keyval(c as u32)) };
            self.key(key, if shift { ModifierType::SHIFT_MASK } else { ModifierType::empty() });
        }
    }
}

/// The popover a context menu is shown in (a child of the view's overlay).
fn find_popover(v: &Rc<super::view::BrowserView>) -> Option<gtk::Popover> {
    let mut child = v.widget.first_child();
    while let Some(w) = child {
        if let Ok(p) = w.clone().downcast::<gtk::Popover>() {
            return Some(p);
        }
        child = w.next_sibling();
    }
    None
}

/// A button anywhere below `root` whose label text is `label` (case-insensitive).
fn find_button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Ok(b) = root.clone().downcast::<gtk::Button>() {
        let text = b.child().and_then(|c| c.downcast::<gtk::Label>().ok()).map(|l| l.text().to_string());
        if text.is_some_and(|t| t.eq_ignore_ascii_case(label)) {
            return Some(b);
        }
    }
    let mut child = root.first_child();
    while let Some(w) = child {
        if let Some(b) = find_button(&w, label) {
            return Some(b);
        }
        child = w.next_sibling();
    }
    None
}

async fn run_input(shell: &Rc<Shell>, r: &mut Report) {
    let st = &shell.state();
    let d = Driver { shell };
    let v = &shell.view();
    let title = || st.borrow().title.clone();
    let none = ModifierType::empty();
    let ctrl = ModifierType::CONTROL_MASK;

    shell.entry.set_text(&page("input.html"));
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || {
        let s = st.borrow();
        !s.loading && s.title.starts_with("c:0")
    })
    .await;
    r.check("input test page loads", ok, || title());
    if !ok {
        return;
    }
    // Let the first frames and the zoom settle so coordinates line up with the layout.
    glib::timeout_future(Duration::from_millis(800)).await;

    // --- mouse: click a button, hover for events and cursors
    d.click(80.0, 40.0);
    let ok = wait_until(5_000, || field(&title(), "c") == "1").await;
    r.check("mouse click reaches the page (button onclick)", ok, || title());

    // The real pointer may be resting over the window and moving the cursor on again, so look
    // for the cursor having been asked for since the move rather than for its value right now.
    let mark = v.cursor_mark();
    d.mv(260.0, 40.0);
    let ok = wait_until(5_000, || field(&title(), "h") == "1" && v.cursor_seen(mark, "pointer")).await;
    r.check("hovering a link-like area gives mouseenter and the pointer cursor", ok, || format!("{} cursor={}", title(), v.cursor()));
    let mark = v.cursor_mark();
    d.mv(170.0, 130.0);
    let ok = wait_until(5_000, || v.cursor_seen(mark, "text")).await;
    r.check("hovering a text area gives the text cursor", ok, || v.cursor());
    let mark = v.cursor_mark();
    d.mv(700.0, 500.0);
    let ok = wait_until(5_000, || v.cursor_seen(mark, "default")).await;
    r.check("leaving returns the default cursor", ok, || v.cursor());

    // --- wheel
    d.scroll(3.0);
    let ok = wait_until(5_000, || field(&title(), "w").parse::<i64>().unwrap_or(0) > 50).await;
    r.check("wheel scrolling moves the page", ok, || title());
    d.scroll(-3.0);
    let ok = wait_until(5_000, || field(&title(), "w") == "0").await;
    r.check("scrolling back returns to the top", ok, || title());

    // --- focus and keyboard
    d.click(170.0, 130.0);
    let ok = wait_until(5_000, || field(&title(), "f") == "ta" && field(&title(), "hf") == "true").await;
    r.check("clicking a text area focuses it and the page", ok, || title());
    r.check("the page widget takes GTK focus when clicked", v.widget.has_focus(), || "no focus".into());

    d.type_str("Hello, World!");
    let ok = wait_until(5_000, || field(&title(), "t") == "Hello, World!").await;
    r.check("typing text (letters, capitals, punctuation, space)", ok, || title());
    r.check("typed keys produce keydown and keypress events", field(&title(), "kd").parse::<u32>().unwrap_or(0) >= 13 && field(&title(), "kp").parse::<u32>().unwrap_or(0) >= 13, || title());

    d.key(Key::BackSpace, none);
    d.key(Key::BackSpace, none);
    let ok = wait_until(5_000, || field(&title(), "t") == "Hello, Worl").await;
    r.check("Backspace deletes", ok, || title());

    d.key(Key::Left, none);
    d.key(Key::Left, none);
    d.type_str("X");
    let ok = wait_until(5_000, || field(&title(), "t") == "Hello, WoXrl").await;
    r.check("arrow keys move the caret", ok, || title());

    d.key(Key::End, none);
    d.key(Key::Return, none);
    d.type_str("2");
    let ok = wait_until(5_000, || field(&title(), "t") == "Hello, WoXrl\u{23ce}2").await;
    r.check("Enter inserts a newline", ok, || title());

    // Composed text from the input method (an accent, a symbol) arrives as a commit.
    v.controllers.im.emit_by_name::<()>("commit", &[&"é✓"]);
    let ok = wait_until(5_000, || field(&title(), "t").ends_with("2é✓")).await;
    r.check("input-method (composed) text is inserted", ok, || title());

    // --- shell shortcuts do not reach the page
    let kd = field(&title(), "kd");
    let consumed = d.key(Key::l, ctrl);
    glib::timeout_future(Duration::from_millis(300)).await;
    r.check("Ctrl+L is left to the shell and not sent to the page", !consumed && field(&title(), "kd") == kd, || {
        format!("consumed={consumed} kd {} -> {}", kd, field(&title(), "kd"))
    });

    // --- selection and clipboard
    d.click(170.0, 130.0);
    d.key(Key::a, ctrl);
    let ok = wait_until(5_000, || v.selection().contains("Hello, WoXrl")).await;
    r.check("select-all selects the text and CEF reports the selection", ok, || format!("{:?}", v.selection()));

    let clipboard = gdk::Display::default().expect("display").clipboard();
    d.key(Key::c, ctrl);
    glib::timeout_future(Duration::from_millis(500)).await;
    let copied = clipboard.read_text_future().await.ok().flatten().map(|s| s.to_string()).unwrap_or_default();
    r.check("Ctrl+C puts the selection on the clipboard", copied.contains("Hello, WoXrl"), || format!("clipboard={copied:?}"));

    d.click(170.0, 230.0);
    d.key(Key::v, ctrl);
    let ok = wait_until(3_000, || field(&title(), "t2").contains("Hello, WoXrl")).await;
    r.check("Ctrl+V pastes what was copied into another field", ok, || title());

    // Text that came from outside the browser pastes too.
    clipboard.set_text("from outside");
    d.click(170.0, 230.0);
    d.key(Key::a, ctrl);
    d.key(Key::v, ctrl);
    let ok = wait_until(3_000, || field(&title(), "t2") == "from outside").await;
    r.check("Ctrl+V pastes text from the system clipboard", ok, || title());

    // --- context menu (right click), driving the real popover buttons
    d.click(170.0, 130.0);
    let menus = st.borrow().menus;
    v.click(3, 1, 170.0, 130.0, none, false);
    v.click(3, 1, 170.0, 130.0, none, true);
    let ok = wait_until(5_000, || st.borrow().menus > menus).await;
    let items = st.borrow().menu.clone();
    let has = |name: &str| items.iter().any(|i| i.eq_ignore_ascii_case(name));
    r.check("right-click shows a context menu with edit commands", ok && has("Paste") && has("Select all"), || format!("{items:?}"));
    let clicked = find_popover(v).and_then(|p| find_button(p.upcast_ref(), "Select all")).map(|b| b.emit_clicked()).is_some();
    let ok = clicked && wait_until(5_000, || v.selection().contains("Hello, WoXrl")).await;
    r.check("choosing 'Select all' from the menu works", ok, || format!("clicked={clicked} selection={:?}", v.selection()));

    clipboard.set_text("old clipboard");
    let menus = st.borrow().menus;
    v.click(3, 1, 170.0, 130.0, none, false);
    v.click(3, 1, 170.0, 130.0, none, true);
    let _ = wait_until(5_000, || st.borrow().menus > menus).await;
    let clicked = find_popover(v).and_then(|p| find_button(p.upcast_ref(), "Copy")).map(|b| b.emit_clicked()).is_some();
    glib::timeout_future(Duration::from_millis(500)).await;
    let copied = clipboard.read_text_future().await.ok().flatten().map(|s| s.to_string()).unwrap_or_default();
    r.check("choosing 'Copy' from the menu puts the selection on the clipboard", clicked && copied.contains("Hello, WoXrl"), || {
        format!("clicked={clicked} clipboard={copied:?}")
    });

    // --- tooltip
    d.mv(430.0, 110.0);
    let ok = wait_until(5_000, || v.tooltip() == "tooltip text").await;
    r.check("page tooltips are shown", ok, || format!("{:?}", v.tooltip()));
    d.mv(700.0, 500.0);

    // --- <select> popup
    d.click(430.0, 36.0);
    let ok = wait_until(5_000, || v.popup_visible() && v.popup_has_frame()).await;
    r.check("a <select> opens its popup and the popup is drawn", ok, || format!("visible={} frame={}", v.popup_visible(), v.popup_has_frame()));
    if let Ok(path) = std::env::var("BROWSAH_SCREENSHOT") {
        glib::timeout_future(Duration::from_millis(400)).await;
        println!("screenshot {path}.popup.png: {}", if screenshot(&shell.window, &format!("{path}.popup.png")) { "saved" } else { "failed" });
    }
    d.key(Key::Down, none);
    d.key(Key::Return, none);
    let ok = wait_until(5_000, || field(&title(), "s") == "b" && !v.popup_visible()).await;
    r.check("choosing an option with the keyboard changes the value and closes the popup", ok, || {
        format!("{} popup={}", title(), v.popup_visible())
    });

    // --- focus moves back to the shell
    let _ = WidgetExt::activate_action(&shell.window, "win.focus-address", None);
    // A GtkEntry hands keyboard focus to an inner text widget, so look for focus inside it.
    let entry_focused = || gtk::prelude::GtkWindowExt::focus(&shell.window).is_some_and(|w| w == shell.entry.clone().upcast::<gtk::Widget>() || w.is_ancestor(&shell.entry));
    let ok = wait_until(5_000, || entry_focused() && !v.widget.has_focus() && field(&title(), "hf") == "false").await;
    r.check("Ctrl+L moves focus to the address bar and the page loses focus", ok, || {
        format!("entry={} page={} {}", entry_focused(), v.widget.has_focus(), title())
    });
}

// ---------------------------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------------------------

async fn run_tabs(shell: &Rc<Shell>, r: &mut Report) {
    let d = Driver { shell };
    r.check("one tab to start with", shell.tab_count() == 1, || shell.tab_count().to_string());
    let first = shell.current().expect("first tab");

    // A page whose button opens a window, and which also tries to open one without a gesture.
    shell.entry.set_text(&page("popup.html"));
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || {
        let s = first.state.borrow();
        !s.loading && s.title == "Popup test"
    })
    .await;
    r.check("popup test page loads", ok, || format!("{:?}", first.state.borrow()));
    glib::timeout_future(Duration::from_millis(1200)).await;
    r.check("a window opened without a user gesture is blocked", shell.tab_count() == 1, || shell.tab_count().to_string());

    d.click(80.0, 40.0);
    let ok = wait_until(10_000, || shell.tab_count() == 2).await;
    r.check("window.open from a click opens a new tab", ok, || shell.tab_count().to_string());
    let second = shell.tabs().get(1).cloned();
    let Some(second) = second else { return };
    let ok = wait_until(15_000, || {
        let s = second.state.borrow();
        !s.loading && s.title == "Page B"
    })
    .await;
    r.check("the new tab loads the target page", ok, || format!("{:?}", second.state.borrow()));
    r.check("the new tab is selected", shell.current().is_some_and(|c| Rc::ptr_eq(&c, &second)), || "not selected".into());
    r.check("tab titles follow their pages", first.page.title() == "Popup test" && second.page.title() == "Page B", || {
        format!("{:?} {:?}", first.page.title(), second.page.title())
    });
    r.check("the window title and address bar follow the selected tab", shell.window.title().as_deref() == Some("Page B") && shell.entry.text().ends_with("/b.html"), || {
        format!("{:?} {:?}", shell.window.title(), shell.entry.text())
    });
    r.check("back is enabled only where there is history", {
        let a = shell.back.is_sensitive();
        shell.step_tab_for_test(-1);
        a != shell.back.is_sensitive() || !a
    }, || "back button state did not follow the tab".into());
    shell.step_tab_for_test(1);

    // Background tabs are hidden and receive no frames.
    let _ = wait_until(5_000, || second.view().has_frame()).await;
    r.check("the selected tab is visible and has a frame", !second.view().is_hidden() && second.view().has_frame(), || {
        format!("hidden={} frame={}", second.view().is_hidden(), second.view().has_frame())
    });
    r.check("the background tab is hidden", first.view().is_hidden(), || "not hidden".into());
    glib::timeout_future(Duration::from_millis(400)).await;
    let (f0, g0) = (first.view().frames(), second.view().frames());
    glib::timeout_future(Duration::from_millis(1000)).await;
    r.check("a hidden tab receives no frames", first.view().frames() == f0, || format!("{} -> {}", f0, first.view().frames()));
    let _ = g0;

    // Switching wakes the hidden tab with a fresh frame.
    let f1 = first.view().frames();
    let _ = WidgetExt::activate_action(&shell.window, "win.prev-tab", None);
    let ok = wait_until(3_000, || first.view().frames() > f1 && !first.view().is_hidden() && second.view().is_hidden()).await;
    r.check("switching tabs wakes the hidden tab and hides the other", ok, || {
        format!("frames {} -> {}, hidden first={} second={}", f1, first.view().frames(), first.view().is_hidden(), second.view().is_hidden())
    });
    r.check("window title follows the switch", shell.window.title().as_deref() == Some("Popup test"), || format!("{:?}", shell.window.title()));

    // New tab: empty address bar with focus in it, then navigate.
    let _ = WidgetExt::activate_action(&shell.window, "win.new-tab", None);
    let ok = wait_until(5_000, || shell.tab_count() == 3).await;
    r.check("new tab action adds a tab", ok, || shell.tab_count().to_string());
    let third = shell.tabs().get(2).cloned();
    let Some(third) = third else { return };
    let focused = || gtk::prelude::GtkWindowExt::focus(&shell.window).is_some_and(|w| w == shell.entry.clone().upcast::<gtk::Widget>() || w.is_ancestor(&shell.entry));
    r.check("the new tab is selected with an empty, focused address bar", shell.current().is_some_and(|c| Rc::ptr_eq(&c, &third)) && shell.entry.text().is_empty() && focused(), || {
        format!("entry={:?} focused={}", shell.entry.text(), focused())
    });
    shell.entry.set_text(&page("a.html"));
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || {
        let s = third.state.borrow();
        !s.loading && s.title == "Page A"
    })
    .await;
    r.check("an address typed in the new tab loads there only", ok && first.state.borrow().title == "Popup test", || {
        format!("{:?} / {:?}", third.state.borrow(), first.state.borrow().title)
    });

    // Close tabs.
    let _ = WidgetExt::activate_action(&shell.window, "win.close-tab", None);
    let ok = wait_until(5_000, || shell.tab_count() == 2).await;
    r.check("closing a tab removes it", ok, || shell.tab_count().to_string());
    let _ = WidgetExt::activate_action(&shell.window, "win.close-tab", None);
    let ok = wait_until(5_000, || shell.tab_count() == 1).await;
    r.check("closing another leaves one", ok, || shell.tab_count().to_string());
    r.check("the remaining tab is the first and is shown", shell.current().is_some_and(|c| Rc::ptr_eq(&c, &first)) && !first.view().is_hidden(), || {
        format!("hidden={}", first.view().is_hidden())
    });
    let n0 = first.view().frames();
    d.click(80.0, 40.0);
    let ok = wait_until(10_000, || shell.tab_count() == 2).await;
    r.check("the surviving tab still works after the others closed", ok && first.view().frames() >= n0, || shell.tab_count().to_string());

    // A dead renderer shows a crashed page, leaves other tabs alone, and a new load recovers.
    let _ = WidgetExt::activate_action(&shell.window, "win.prev-tab", None);
    let other = shell.tabs().get(1).cloned();
    let cur = shell.current().expect("tab");
    shell.entry.set_text("chrome://crash");
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || cur.state.borrow().crashed).await;
    r.check("a crashed renderer is reported and the crashed page shown", ok && cur.sad_visible() && !cur.view().widget.is_visible(), || format!("{:?}", cur.state.borrow()));
    // The other tab is a popup of this one, so it keeps `window.opener` and therefore shares the
    // renderer process: it goes down with its opener (an unrelated tab would not).
    let ok = wait_until(5_000, || other.as_ref().is_some_and(|o| o.state.borrow().crashed)).await;
    r.check("a popup shares its opener's renderer and crashes with it", ok, || "other tab did not crash".into());
    shell.entry.set_text(&page("a.html"));
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || {
        let s = cur.state.borrow();
        !s.crashed && !s.loading && s.title == "Page A"
    })
    .await;
    r.check("loading an address again recovers the crashed tab", ok && !cur.sad_visible() && cur.view().widget.is_visible(), || format!("{:?}", cur.state.borrow()));
    let _ = wait_until(5_000, || cur.view().has_frame()).await;
}

/// Wheel scrolling must cover the distance asked for, however fast the notches arrive (a cap
/// or lagging animation would show as a shortfall).
async fn run_scroll(shell: &Rc<Shell>, r: &mut Report) {
    let st = shell.state();
    let v = shell.view();
    shell.entry.set_text(&page("tall.html"));
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || !st.borrow().loading && st.borrow().title.starts_with("y:")).await;
    r.check("scroll test page loads", ok, || format!("{:?}", st.borrow()));
    glib::timeout_future(Duration::from_millis(800)).await;
    let y = || st.borrow().title.trim_start_matches("y:").parse::<f64>().unwrap_or(0.0);
    let d = Driver { shell };
    d.mv(300.0, 300.0);
    println!("scroll unit for emitted signals: {:?}", shell.view().controllers.scroll.unit());
    let t0 = std::time::Instant::now();
    for _ in 0..20 {
        d.scroll(1.0); // one wheel notch (100 px)
        glib::timeout_future(Duration::from_millis(10)).await;
    }
    let _ = wait_until(5_000, || y() >= 1990.0).await;
    let took = t0.elapsed().as_millis();
    r.check("20 fast wheel notches scroll 2000 px (no speed cap)", (y() - 2000.0).abs() <= 20.0, || format!("scrollY={} after {took} ms", y()));
    println!("scroll: 20 notches reached y={} in {took} ms", y());
    // One big event (a fast trackpad swipe delivers large deltas): must not be clamped.
    glib::timeout_future(Duration::from_millis(500)).await;
    let y0 = y();
    d.scroll(10.0);
    let _ = wait_until(3_000, || y() >= y0 + 990.0).await;
    // Blink limits one wheel event to about a viewport of scrolling; real swipes are many small events.
    println!("scroll: a single 1000 px wheel event moved {} px (limited to about a viewport)", y() - y0);
    // Sustained small events at a given rate, as a trackpad delivers them: the page must keep up
    // (record how far behind the input it falls).
    for (per_event_px, interval_ms) in [(10.0, 8u64), (30.0, 8), (60.0, 8)] {
        shell.entry.emit_by_name::<()>("activate", &[]);
        glib::timeout_future(Duration::from_millis(100)).await;
        let _ = WidgetExt::activate_action(&shell.window, "win.focus-address", None);
        shell.entry.set_text(&page("tall.html"));
        shell.entry.emit_by_name::<()>("activate", &[]);
        let _ = wait_until(10_000, || !st.borrow().loading && st.borrow().title == "y:0").await;
        glib::timeout_future(Duration::from_millis(700)).await;
        d.mv(300.0, 300.0);
        let n = 125;
        let t0 = std::time::Instant::now();
        let mut samples = Vec::new();
        for i in 0..n {
            d.scroll(per_event_px / 100.0);
            glib::timeout_future(Duration::from_millis(interval_ms)).await;
            if i % 25 == 24 {
                samples.push((t0.elapsed().as_millis(), y()));
            }
        }
        let sent = n as f64 * per_event_px;
        glib::timeout_future(Duration::from_millis(1500)).await;
        println!("scroll rate test: {per_event_px} px every {interval_ms} ms (sent {sent} px over {} ms): samples {:?}, final y={}", t0.elapsed().as_millis() - 1500, samples, y());
        r.check(&format!("sustained {per_event_px} px events every {interval_ms} ms are all applied"), (y() - sent).abs() <= sent * 0.02 + 5.0, || format!("sent {sent}, scrolled {}", y()));
    }
    let _ = &v;
}

/// A popup opened by a click keeps `window.opener`: the child posts a message to its opener.
async fn run_opener(shell: &Rc<Shell>, r: &mut Report) {
    let d = Driver { shell };
    // Back to one tab, on the opener page.
    while shell.tab_count() > 1 {
        let _ = WidgetExt::activate_action(&shell.window, "win.close-tab", None);
        glib::timeout_future(Duration::from_millis(300)).await;
    }
    let parent = shell.current().expect("tab");
    shell.entry.set_text(&page("opener.html"));
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || {
        let s = parent.state.borrow();
        !s.loading && s.title == "Opener test"
    })
    .await;
    r.check("opener test page loads", ok, || format!("{:?}", parent.state.borrow()));
    glib::timeout_future(Duration::from_millis(800)).await;
    d.click(80.0, 40.0);
    let ok = wait_until(10_000, || parent.state.borrow().title == "msg:from-child").await;
    r.check("a popup keeps window.opener (postMessage reaches the opener)", ok, || {
        format!("tabs={} parent title={:?}", shell.tab_count(), parent.state.borrow().title)
    });
}

/// A second (ephemeral) window, and a tab dragged between windows changing owner.
async fn run_windows(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    r.check("one window to start with", super::windows().len() == 1, || super::windows().len().to_string());
    ActionGroupExt::activate_action(&app, "new-window", None);
    let ok = wait_until(10_000, || super::windows().len() == 2).await;
    r.check("new-window opens a second window", ok, || super::windows().len().to_string());
    let second = super::windows().into_iter().find(|s| !Rc::ptr_eq(s, shell));
    let Some(second) = second else { return };
    r.check("the new window is ephemeral (no workspace)", second.workspace.is_none(), || format!("{:?}", second.workspace));
    let ok = wait_until(15_000, || second.current().is_some_and(|t| t.state.borrow().ready)).await;
    r.check("the new window's tab starts", ok, || "no browser".into());

    // Move the first window's tab to the second window, as dragging it would.
    let tab = shell.current().expect("tab");
    shell.open_tab("about:blank", true); // keep the first window non-empty
    shell.tab_view.transfer_page(&tab.page, &second.tab_view, 0);
    r.check("a transferred tab changes owner", tab.owner().is_some_and(|o| Rc::ptr_eq(&o, &second)), || "owner not updated".into());
    r.check("tab lists follow the transfer", second.tabs().iter().any(|t| Rc::ptr_eq(t, &tab)) && !shell.tabs().iter().any(|t| Rc::ptr_eq(t, &tab)), || {
        format!("first={} second={}", shell.tab_count(), second.tab_count())
    });
    // Events from the moved tab reach its new window: its title shows up there.
    let (page_title_before, _) = (tab.page.title(), ());
    tab.view().navigate(&page("a.html"));
    let ok = wait_until(15_000, || tab.page.title() == "Page A").await;
    r.check("a moved tab still updates its page in the new window", ok, || format!("{:?} (was {page_title_before:?})", tab.page.title()));

    second.window.close();
    let ok = wait_until(5_000, || super::windows().len() == 1).await;
    r.check("closing a window removes it from the registry", ok, || format!("{} windows; visible={} tabs={}", super::windows().len(), second.window.is_visible(), second.tab_count()));
}

/// A workspace remembers its tabs: they come back as sleeping tabs with a screenshot, the selected
/// one live, and a sleeping tab wakes when shown.
async fn run_restore(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let ws_dir = profile.workspaces_dir().join("wstest");
    let db = profile.dir.join("data.sqlite");
    // Start from nothing.
    let _ = std::fs::remove_dir_all(&ws_dir);
    std::fs::create_dir_all(&ws_dir).expect("workspace dir");
    std::fs::write(ws_dir.join("workspace.yaml"), "name: Test workspace\n").expect("workspace.yaml");
    if let Ok(s) = crate::store::Store::open(&db) {
        let _ = s.delete_workspace("wstest");
    }
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "wstest"));

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"wstest".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().ready))).await;
    r.check("open-workspace opens the workspace's window", ok, || format!("{} windows", super::windows().len()));
    let Some(w) = find() else { return };
    r.check("the window title names the workspace", w.window.title().is_some_and(|t| t.ends_with("Test workspace")), || format!("{:?}", w.window.title()));

    // Three tabs: A, B, C, with B selected last.
    let a = w.current().expect("tab");
    a.view().navigate(&page("a.html"));
    let _ = wait_until(15_000, || a.state.borrow().title == "Page A").await;
    let _ = wait_until(10_000, || a.view().has_frame()).await; // a page that is left before its first frame has no screenshot
    let b = w.open_tab(&page("b.html"), true);
    let _ = wait_until(15_000, || b.state.borrow().title == "Page B" && b.view().has_frame()).await;
    let c = w.open_tab(&page("popup.html"), true);
    let _ = wait_until(15_000, || c.state.borrow().title == "Popup test" && c.view().has_frame()).await;
    let _ = WidgetExt::activate_action(&w.window, "win.prev-tab", None); // back to B
    let ok = wait_until(5_000, || w.current().is_some_and(|t| Rc::ptr_eq(&t, &b))).await;
    r.check("B is selected before closing", ok, || "not selected".into());
    glib::timeout_future(Duration::from_millis(1200)).await; // screenshots are taken after the switch
    let shots = ws_dir.join("shots");
    let (ua, ub, uc) = (a.uid, b.uid, c.uid);
    w.window.close();
    let ok = wait_until(5_000, || find().is_none()).await;
    r.check("closing the workspace window removes it", ok, || "still open".into());

    let records = crate::store::Store::open(&db).map(|s| s.load_tabs("wstest")).unwrap_or_default();
    r.check("the tabs were remembered in order with the selection", records.len() == 3 && records[1].selected && !records[0].selected && records[0].title == "Page A" && records[2].title == "Popup test", || format!("{records:?}"));
    let shot = |uid: u64| shots.join(format!("{uid}.png")).is_file();
    r.check("screenshots were saved for the tabs", shot(ua) && shot(ub) && shot(uc), || format!("a={} b={} c={}", shot(ua), shot(ub), shot(uc)));

    // Reopen.
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"wstest".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.tab_count() == 3)).await;
    r.check("reopening restores the three tabs", ok, || format!("{:?}", find().map(|s| s.tab_count())));
    let Some(w) = find() else { return };
    if let Ok(path) = std::env::var("BROWSAH_SCREENSHOT") {
        // The selected tab shows its blurred screenshot until its first frame.
        println!("screenshot {path}.restored.png: {}", if screenshot(&w.window, &format!("{path}.restored.png")) { "saved" } else { "failed" });
    }
    let tabs = w.tabs();
    let titles: Vec<String> = tabs.iter().map(|t| t.page.title().to_string()).collect();
    r.check("restored tabs show their remembered titles at once", titles == ["Page A", "Page B", "Popup test"], || format!("{titles:?}"));
    r.check("the previously selected tab is selected and live", w.current().is_some_and(|t| t.uid == ub && !t.view().is_dormant()), || "wrong selection".into());
    r.check("the other tabs are asleep behind a blurred screenshot", tabs[0].view().is_dormant() && tabs[0].view().has_frame() && tabs[0].view().showing_placeholder() && tabs[2].view().is_dormant(), || {
        format!("a dormant={} frame={} blurred={}", tabs[0].view().is_dormant(), tabs[0].view().has_frame(), tabs[0].view().showing_placeholder())
    });
    let ok = wait_until(15_000, || w.current().is_some_and(|t| t.state.borrow().title == "Page B")).await;
    r.check("the live tab loads its page", ok, || "B did not load".into());

    // Showing a sleeping tab wakes it.
    let _ = WidgetExt::activate_action(&w.window, "win.prev-tab", None); // to A
    let ok = wait_until(15_000, || tabs[0].state.borrow().ready && tabs[0].state.borrow().title == "Page A" && !tabs[0].view().showing_placeholder()).await;
    r.check("a sleeping tab wakes when shown and replaces its screenshot", ok && !tabs[0].view().is_dormant(), || format!("{:?} blurred={}", tabs[0].state.borrow(), tabs[0].view().showing_placeholder()));
    r.check("a tab that was never shown is still asleep", tabs[2].view().is_dormant(), || "woken".into());

    w.window.close();
    let _ = wait_until(5_000, || find().is_none()).await;
    if let Ok(s) = crate::store::Store::open(&db) {
        let _ = s.delete_workspace("wstest");
    }
    let _ = std::fs::remove_dir_all(&ws_dir);
}

/// A tiny local web server for the scope test: `localhost` and `127.0.0.1` are two different hosts
/// served by the same socket. Returns its port.
fn start_test_server() -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
          // One thread per connection: the browser opens idle connections ahead of use.
          std::thread::spawn(move || {
            let mut stream = stream;
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_owned();
            if path == "/slow" {
                // A download that takes a few seconds: 1 MB in ten pieces.
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"slow.bin\"\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n");
                for _ in 0..10 {
                    if stream.write_all(&[0u8; 100_000]).is_err() {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
                return;
            }
            let (status, extra, body) = match path.as_str() {
                "/scope.html" => (
                    "200 OK",
                    String::new(),
                    format!("<!doctype html><title>Scope test</title><a href=\"http://127.0.0.1:{port}/b.html\" style=\"position:absolute;left:20px;top:20px;font-size:28px\">out of scope</a><a target=\"_blank\" href=\"http://127.0.0.1:{port}/b.html\" style=\"position:absolute;left:20px;top:90px;font-size:28px\">in a new tab</a>"),
                ),
                "/form.html" => ("200 OK", String::new(), "<!doctype html><title>Form</title><input id=i value=\"\"><textarea id=t></textarea>".to_owned()),
                "/file.txt" => ("200 OK", "Content-Disposition: attachment; filename=\"note.txt\"\r\n".to_owned(), "hello".to_owned()),
                "/pwa.html" => ("200 OK", String::new(), "<!doctype html><title>PWA page</title><link rel=manifest href=/manifest.json>".to_owned()),
                "/manifest.json" => ("200 OK", "Content-Type: application/manifest+json\r\n".to_owned(), "{\"name\":\"Test Manifest App\",\"start_url\":\"/pwa.html\",\"scope\":\"/\",\"display\":\"minimal-ui\",\"theme_color\":\"#336699\",\"icons\":[{\"src\":\"/icon.svg\",\"sizes\":\"any\",\"type\":\"image/svg+xml\",\"purpose\":\"any\"}]}".to_owned()),
                "/icon.svg" => ("200 OK", "Content-Type: image/svg+xml\r\n".to_owned(), "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\"><rect width=\"64\" height=\"64\" fill=\"#336699\"/></svg>".to_owned()),
                "/find.html" => ("200 OK", String::new(), "<!doctype html><title>Find test</title><p>one needle here</p><p>another Needle there</p><p>and a third needle</p>".to_owned()),
                "/white.html" => ("200 OK", String::new(), "<!doctype html><title>White</title><body style=\"background:#fff;color:#000\">white page".to_owned()),
                "/tall.html" => ("200 OK", String::new(), "<!doctype html><title>Tall</title><body style=\"background:#fff;color:#000;margin:0\"><div style=\"height:3000px\">tall page <button>button</button> <input value=text></div>".to_owned()),
                "/b.html" => ("200 OK", String::new(), "<!doctype html><title>Page B</title><p>b</p>".to_owned()),
                "/cookie-set" => ("200 OK", "Set-Cookie: ck=1; Path=/\r\n".to_owned(), "<!doctype html><title>Cookie set</title>".to_owned()),
                "/cookie-show" => {
                    let c = req.lines().find(|l| l.to_ascii_lowercase().starts_with("cookie:")).map(|l| l[7..].trim().to_owned()).unwrap_or_default();
                    ("200 OK", String::new(), format!("<!doctype html><title>Cookies [{c}]</title>"))
                }
                "/redir" => ("302 Found", format!("Location: http://127.0.0.1:{port}/b.html\r\n"), String::new()),
                _ => ("404 Not Found", String::new(), "<title>Not found</title>".to_owned()),
            };
            let content_type = if extra.contains("Content-Type") { "" } else { "Content-Type: text/html\r\n" };
            let _ = write!(stream, "HTTP/1.1 {status}\r\n{content_type}{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
          });
        }
    });
    port
}

/// A locked workspace: controls hidden or read-only, no new tabs, a pinned tab limited to its
/// scope that cannot be closed, and the workspace colour.
async fn run_locked(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let ws_dir = profile.workspaces_dir().join("locked");
    let port = start_test_server();
    let _ = std::fs::remove_dir_all(&ws_dir);
    std::fs::create_dir_all(&ws_dir).expect("workspace dir");
    std::fs::write(
        ws_dir.join("workspace.yaml"),
        format!(
            "name: Locked\ncolor: '#2a6f97 #61a5c2'\nnav: {{ address_bar: readonly, forward: false, new_tab_button: false }}\ntabs:\n  allow_new: false\n  pinned:\n    - url: http://localhost:{port}/scope.html\n      title: Scoped\n      scope: [localhost]\n"
        ),
    )
    .expect("workspace.yaml");
    let before = super::windows().len();
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "locked"));

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"locked".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test"))).await;
    r.check("a workspace with a pinned tab opens on it (and nothing else)", ok, || format!("{:?}", find().map(|s| (s.tab_count(), s.current().map(|t| t.state.borrow().title.clone())))));
    let Some(w) = find() else { return };
    let pinned = w.current().expect("tab");
    r.check("the tab is pinned and it is the only tab", pinned.page.is_pinned() && w.tab_count() == 1, || format!("pinned={} tabs={}", pinned.page.is_pinned(), w.tab_count()));
    r.check("the address bar is read-only and forward is hidden", !w.entry.is_editable() && !w.forward.is_visible() && w.back.is_visible(), || {
        format!("editable={} forward={} back={}", w.entry.is_editable(), w.forward.is_visible(), w.back.is_visible())
    });
    let new_tab = w.window.lookup_action("new-tab").and_then(|a| a.downcast::<gio::SimpleAction>().ok());
    r.check("new tabs are not allowed (action disabled)", new_tab.is_some_and(|a| !a.is_enabled()), || "enabled".into());
    glib::timeout_future(Duration::from_millis(600)).await;
    shot(&w.window, "workspace-colour");
    // The colour covers the whole header and tab bar, with no dark line above or below (against a white page).
    if let Some(t) = w.current() {
        t.view().navigate(&format!("http://localhost:{port}/white.html"));
    }
    let _ = wait_until(15_000, || w.current().is_some_and(|t| t.state.borrow().title == "White" && !t.state.borrow().loading)).await;
    glib::timeout_future(Duration::from_millis(600)).await;
    shot(&w.window, "workspace-colour-white-page");
    let bar_height = w.tab_view.compute_point(&w.window, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| p.y() as i32).unwrap_or(86);
    let ys = [0, 1, 2, 3, 20, 44, bar_height / 2, bar_height - 3, bar_height - 2, bar_height - 1];
    let points: Vec<(i32, i32)> = ys.iter().map(|y| (200, *y)).collect();
    let px = pixels(&w.window, &points);
    let reference = px.get(4).copied().unwrap_or((0, 0, 0));
    let near = |a: (u8, u8, u8), b: (u8, u8, u8)| (a.0 as i32 - b.0 as i32).abs() < 8 && (a.1 as i32 - b.1 as i32).abs() < 8 && (a.2 as i32 - b.2 as i32).abs() < 8;
    let off: Vec<String> = ys.iter().zip(&px).filter(|(_, p)| !near(**p, reference)).map(|(y, p)| format!("y={y}: {p:?}")).collect();
    r.check("the workspace colour covers the header and tab bar edge to edge (no dark line at the top or bottom)", px.len() == ys.len() && off.is_empty(), || format!("reference {reference:?}; differing: {off:?}"));
    if let Some(t) = w.current() {
        t.view().navigate(&format!("http://localhost:{port}/scope.html"));
    }
    let _ = wait_until(15_000, || w.current().is_some_and(|t| t.state.borrow().title == "Scope test" && !t.state.borrow().loading)).await;
    r.check("the workspace colour is applied to the window", w.window.has_css_class("ws-locked"), || "no colour class".into());

    if let Ok(path) = std::env::var("BROWSAH_SCREENSHOT") {
        glib::timeout_future(Duration::from_millis(1200)).await;
        println!("screenshot {path}.locked.png: {}", if screenshot(&w.window, &format!("{path}.locked.png")) { "saved" } else { "failed" });
    }

    // A pinned tab cannot be closed.
    let _ = WidgetExt::activate_action(&w.window, "win.close-tab", None);
    glib::timeout_future(Duration::from_millis(500)).await;
    r.check("a pinned tab cannot be closed", w.tab_count() == 1, || w.tab_count().to_string());

    // A link out of scope leaves the pinned tab alone and opens in an ephemeral window (no new tabs here).
    let _ = wait_until(10_000, || pinned.view().has_frame()).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    let d = Driver { shell: &w };
    d.click(40.0, 45.0);
    let ok = wait_until(10_000, || super::windows().len() == before + 2).await;
    r.check("a link out of scope opens in an ephemeral window", ok, || format!("{} windows (was {before})", super::windows().len()));
    let find_other = || super::windows().into_iter().find(|s| s.workspace.is_none() && !Rc::ptr_eq(s, shell) && !Rc::ptr_eq(s, &w));
    let ok = wait_until(15_000, || find_other().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && t.state.borrow().address.contains("127.0.0.1")))).await;
    let other = find_other();
    r.check("the out-of-scope page loads there", ok, || "not loaded".into());
    r.check("the pinned tab stayed where it was", pinned.state.borrow().title == "Scope test" && pinned.state.borrow().address.contains("localhost"), || format!("{:?}", pinned.state.borrow()));
    if let Some(o) = other {
        o.window.close();
    }

    // Inside the scope, navigation is free.
    pinned.view().navigate(&format!("http://localhost:{port}/b.html"));
    let ok = wait_until(15_000, || pinned.state.borrow().title == "Page B").await;
    r.check("navigation inside the scope is allowed", ok, || format!("{:?}", pinned.state.borrow()));

    // A redirect out of the scope (a sign-in flow) is allowed.
    pinned.view().navigate(&format!("http://localhost:{port}/redir"));
    let ok = wait_until(15_000, || pinned.state.borrow().address.contains("127.0.0.1")).await;
    r.check("a redirect out of the scope is followed in the pinned tab", ok && w.tab_count() == 1, || format!("{:?} tabs={}", pinned.state.borrow().address, w.tab_count()));

    w.window.close();
    let _ = wait_until(5_000, || find().is_none()).await;
    let _ = std::fs::remove_dir_all(&ws_dir);
}

/// Bookmarks belong to a workspace: the star and Ctrl+D toggle them, the bar lists them, clicking
/// one opens it, and they are still there when the workspace reopens.
async fn run_bookmarks(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let ws_dir = profile.workspaces_dir().join("bm");
    let db = profile.dir.join("data.sqlite");
    let _ = std::fs::remove_dir_all(&ws_dir);
    std::fs::create_dir_all(&ws_dir).expect("workspace dir");
    std::fs::write(ws_dir.join("workspace.yaml"), "name: Bookmarks\n").expect("workspace.yaml");
    if let Ok(s) = crate::store::Store::open(&db) {
        let _ = s.delete_workspace("bm");
    }
    let marks = || crate::store::Store::open(&db).map(|s| s.bookmarks("bm")).unwrap_or_default();
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "bm"));

    r.check("an ephemeral window has no bookmark star", !shell.star.is_visible(), || "star visible".into());
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"bm".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().ready))).await;
    r.check("the bookmarks workspace opens", ok, || "not open".into());
    let Some(w) = find() else { return };
    r.check("a workspace window shows the star and no bar yet", w.star.is_visible() && !w.bookmark_revealer.reveals_child(), || format!("star={} bar={}", w.star.is_visible(), w.bookmark_revealer.reveals_child()));

    let tab = w.current().expect("tab");
    tab.view().navigate(&page("b.html"));
    let ok = wait_until(15_000, || tab.state.borrow().title == "Page B" && w.star.icon_name().as_deref() == Some("non-starred-symbolic")).await;
    r.check("a page that is not bookmarked shows an empty star", ok, || format!("{:?}", w.star.icon_name()));
    let _ = WidgetExt::activate_action(&w.window, "win.bookmark-page", None);
    let ok = wait_until(5_000, || marks().len() == 1).await;
    r.check("the star bookmarks the page with its title", ok && marks()[0].title == "Page B" && marks()[0].url.ends_with("/b.html"), || format!("{:?}", marks()));
    r.check("the star fills and the bar appears with the bookmark", w.star.icon_name().as_deref() == Some("starred-symbolic") && w.bookmark_revealer.reveals_child() && w.bookmark_buttons().len() == 1, || {
        format!("{:?} bar={} buttons={}", w.star.icon_name(), w.bookmark_revealer.reveals_child(), w.bookmark_buttons().len())
    });

    // Leave the page, then use the bookmark.
    tab.view().navigate(&page("a.html"));
    let ok = wait_until(15_000, || tab.state.borrow().title == "Page A" && w.star.icon_name().as_deref() == Some("non-starred-symbolic")).await;
    r.check("another page shows an empty star again", ok, || format!("{:?}", w.star.icon_name()));
    if let Some(b) = w.bookmark_buttons().first() {
        b.emit_clicked();
    }
    let ok = wait_until(15_000, || tab.state.borrow().title == "Page B").await;
    r.check("clicking a bookmark opens it in the current tab", ok, || format!("{:?}", tab.state.borrow().title));

    // Still there after closing and reopening.
    w.window.close();
    let _ = wait_until(5_000, || find().is_none()).await;
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"bm".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.bookmark_buttons().len() == 1 && s.bookmark_revealer.reveals_child())).await;
    r.check("bookmarks are still there when the workspace reopens", ok, || "bar empty".into());
    let Some(w) = find() else { return };

    // Ctrl+D on a bookmarked page removes the bookmark and hides the empty bar.
    let tab = w.current().expect("tab");
    let ok = wait_until(15_000, || tab.state.borrow().title == "Page B" && w.star.icon_name().as_deref() == Some("starred-symbolic")).await;
    r.check("the restored bookmarked page shows a filled star", ok, || format!("{:?} {:?}", tab.state.borrow().title, w.star.icon_name()));
    let _ = WidgetExt::activate_action(&w.window, "win.bookmark-page", None);
    let ok = wait_until(5_000, || marks().is_empty() && !w.bookmark_revealer.reveals_child()).await;
    r.check("the star removes the bookmark and hides the empty bar", ok, || format!("{:?} bar={}", marks(), w.bookmark_revealer.reveals_child()));

    w.window.close();
    let _ = wait_until(5_000, || find().is_none()).await;
    if let Ok(s) = crate::store::Store::open(&db) {
        let _ = s.delete_workspace("bm");
    }
    let _ = std::fs::remove_dir_all(&ws_dir);
}

/// The tab's right-click menu offers "Send to workspace", and sending moves the live tab.
async fn run_send_to_workspace(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let home = page("a.html");
    for (id, name) in [("s1", "Sender"), ("s2", "Receiver")] {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        std::fs::write(dir.join("workspace.yaml"), format!("name: {name}\nhome_page: '{home}'\n")).expect("workspace.yaml");
        if let Ok(s) = crate::store::Store::open(&db) {
            let _ = s.delete_workspace(id);
        }
    }
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"s1".to_variant()));
    let ok = wait_until(15_000, || find("s1").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page A"))).await;
    r.check("the sending workspace opens on its home page", ok, || "not open".into());
    let Some(s1) = find("s1") else { return };
    let moving = s1.open_tab(&page("b.html"), true);
    let _ = wait_until(15_000, || moving.state.borrow().title == "Page B").await;

    // The menu for that tab.
    s1.tab_view.emit_by_name::<()>("setup-menu", &[&moving.page]);
    let menu = s1.tab_menu.clone();
    let submenu_labels: Vec<String> = (0..menu.n_items())
        .filter_map(|i| menu.item_link(i, "submenu"))
        .flat_map(|sub| (0..sub.n_items()).filter_map(move |j| sub.item_attribute_value(j, "label", None).and_then(|v| v.get::<String>())).collect::<Vec<_>>())
        .collect();
    r.check("the tab menu has a send-to-workspace submenu listing the other workspaces and a new window", submenu_labels.contains(&"Receiver".to_string()) && submenu_labels.contains(&"New window".to_string()) && !submenu_labels.contains(&"Sender".to_string()), || format!("{submenu_labels:?}"));
    let has_close = (0..menu.n_items()).filter_map(|i| menu.item_link(i, "section")).any(|sec| (0..sec.n_items()).any(|j| sec.item_attribute_value(j, "label", None).and_then(|v| v.get::<String>()).as_deref() == Some("Close tab")));
    r.check("the tab menu also offers closing the tab", has_close, || "no close item".into());

    // Send it.
    ActionGroupExt::activate_action(&s1.window, "send-tab", Some(&"s2".to_variant()));
    let ok = wait_until(15_000, || find("s2").is_some_and(|s| s.tab_count() == 1 && s.current().is_some_and(|t| t.uid == moving.uid))).await;
    r.check("sending moves the tab to the other workspace's window", ok, || format!("{:?}", find("s2").map(|s| s.tab_count())));
    r.check("the sent tab keeps running and belongs to its new window", moving.owner().zip(find("s2")).is_some_and(|(o, s)| Rc::ptr_eq(&o, &s)) && !moving.view().is_dormant(), || "owner not updated".into());
    r.check("the sender no longer has it", s1.tab_count() == 1 && !s1.tabs().iter().any(|t| t.uid == moving.uid), || s1.tab_count().to_string());
    moving.view().navigate(&page("popup.html"));
    let ok = wait_until(15_000, || moving.page.title() == "Popup test").await;
    r.check("the sent tab still updates in its new window", ok, || format!("{:?}", moving.page.title()));

    // Send the sender's last tab to a new window: the emptied window closes.
    let last = s1.current().expect("last tab");
    s1.tab_view.emit_by_name::<()>("setup-menu", &[&last.page]);
    let before = super::windows().len();
    ActionGroupExt::activate_action(&s1.window, "send-tab", Some(&"".to_variant()));
    let ok = wait_until(10_000, || find("s1").is_none() && super::windows().len() == before).await;
    r.check("sending the last tab to a new window closes the emptied window", ok, || format!("{} windows (was {before})", super::windows().len()));
    let ephemeral = super::windows().into_iter().find(|s| s.workspace.is_none() && s.tabs().iter().any(|t| t.uid == last.uid));
    r.check("the tab is in a new ephemeral window", ephemeral.is_some(), || "not found".into());

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    for id in ["s1", "s2"] {
        if let Ok(s) = crate::store::Store::open(&db) {
            let _ = s.delete_workspace(id);
        }
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// Links go through the router: a clicked link and a new-tab link, from a workspace whose rules send
/// them elsewhere, and links from other applications (the application's rules).
async fn run_routing(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::{ActionGroupExt, ApplicationExt};
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    for id in ["ra", "rb", "rc"] {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        if let Ok(s) = crate::store::Store::open(&db) {
            let _ = s.delete_workspace(id);
        }
    }
    // ra: links to 127.0.0.1 from its tabs go to workspace rb.
    std::fs::write(
        profile.workspaces_dir().join("ra/workspace.yaml"),
        format!("name: A\nhome_page: 'http://localhost:{port}/scope.html'\nrouting:\n  rules:\n    - name: to b\n      match: {{ url: ['127.0.0.1'] }}\n      open: workspace:rb\n"),
    )
    .expect("ra");
    std::fs::write(profile.workspaces_dir().join("rb/workspace.yaml"), "name: B\n").expect("rb");
    std::fs::write(profile.workspaces_dir().join("rc/workspace.yaml"), "name: C\n").expect("rc");
    // The application's rules, from a throwaway file: localhost links from other applications go to rc, one host is blocked.
    let app_rules = std::env::temp_dir().join(format!("browsah-routing-{}.yaml", std::process::id()));
    std::fs::write(&app_rules, "routing:\n  rules:\n    - match: { url: ['localhost'] }\n      open: workspace:rc\n    - match: { url: ['blocked.invalid'] }\n      open: block\n").expect("rules");
    let router = crate::router::Router::load(&app_rules, Some(&profile));
    r.check("the routing rules load without warnings", router.warnings.is_empty(), || format!("{:?}", router.warnings));
    crate::router::install(router);
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"ra".to_variant()));
    let ok = wait_until(15_000, || find("ra").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && t.view().has_frame()))).await;
    r.check("workspace A opens on its home page", ok, || "not open".into());
    let Some(ra) = find("ra") else { return };
    glib::timeout_future(Duration::from_millis(800)).await;
    let d = Driver { shell: &ra };

    // A clicked link: routed to workspace rb; the tab in ra stays where it was.
    d.click(40.0, 45.0);
    let ok = wait_until(15_000, || find("rb").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B"))).await;
    r.check("a clicked link is routed to another workspace", ok, || format!("{:?}", find("rb").map(|s| s.tab_count())));
    r.check("the clicked tab did not navigate and no tab was added to its window", ra.tab_count() == 1 && ra.current().is_some_and(|t| t.state.borrow().title == "Scope test"), || format!("tabs={} {:?}", ra.tab_count(), ra.current().map(|t| t.state.borrow().title.clone())));

    // A link that asks for a new tab goes the same way.
    let Some(rb) = find("rb") else { return };
    let before = rb.tab_count();
    d.click(40.0, 110.0);
    let ok = wait_until(15_000, || rb.tab_count() == before + 1).await;
    r.check("a new-tab link is routed too (and does not open a tab in its own window)", ok && ra.tab_count() == 1, || format!("rb tabs {} (was {before}), ra tabs {}", rb.tab_count(), ra.tab_count()));

    // Links from other applications: the application's rules.
    let windows_before = super::windows().len();
    app.open(&[gio::File::for_uri(&format!("http://localhost:{port}/b.html"))], "");
    let ok = wait_until(15_000, || find("rc").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B"))).await;
    r.check("an external link matching an application rule opens in that workspace", ok, || "workspace rc did not open".into());
    let n = super::windows().len();
    app.open(&[gio::File::for_uri("http://blocked.invalid/x")], "");
    glib::timeout_future(Duration::from_millis(800)).await;
    r.check("a blocked external link opens nothing", super::windows().len() == n && n == windows_before + 1, || format!("{} windows (was {windows_before}+1)", super::windows().len()));

    // A sign-in another application started (an OAuth address that redirects back to this machine)
    // gets an ephemeral window of its own, whatever the page is.
    let n = super::windows().len();
    app.open(&[gio::File::for_uri(&format!("http://127.0.0.1:{port}/b.html?client_id=cli&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A9%2Fcallback"))], "");
    let find_signin = || super::windows().into_iter().find(|s| s.workspace.is_none() && !Rc::ptr_eq(s, shell) && s.tabs().iter().any(|t| t.state.borrow().address.contains("response_type=code")));
    let ok = wait_until(15_000, || super::windows().len() == n + 1 && find_signin().is_some()).await;
    let signin = find_signin();
    r.check("a sign-in started by another application opens in an ephemeral window", ok && signin.is_some(), || format!("{} windows (was {n})", super::windows().len()));
    if let Some(s) = signin {
        let ok = wait_until(15_000, || s.current().is_some_and(|t| t.state.borrow().title == "Page B")).await;
        r.check("the sign-in page loads there", ok, || "not loaded".into());
    }

    // A workspace named on the command line is an explicit choice: no routing.
    let rc_tabs = find("rc").map(|s| s.tab_count()).unwrap_or(0);
    app.open(&[gio::File::for_uri(&format!("http://localhost:{port}/b.html"))], "rb");
    let ok = wait_until(15_000, || rb.tab_count() == before + 2).await;
    r.check("--workspace with a link skips the router", ok && find("rc").map(|s| s.tab_count()).unwrap_or(0) == rc_tabs, || format!("rb {} rc {}", rb.tab_count(), find("rc").map(|s| s.tab_count()).unwrap_or(0)));

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    crate::router::install(crate::router::Router::default());
    let _ = std::fs::remove_file(&app_rules);
    for id in ["ra", "rb", "rc"] {
        if let Ok(s) = crate::store::Store::open(&db) {
            let _ = s.delete_workspace(id);
        }
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// Modals: a routing rule opens a link as one, the move button asks the router where the page really
/// belongs, and workspaces decide what a script popup becomes.
async fn run_modals(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let ids = ["ma", "mb", "mp", "mq", "mw"];
    for id in ids {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        if let Ok(s) = crate::store::Store::open(&db) {
            let _ = s.delete_workspace(id);
        }
    }
    let wd = |id: &str| profile.workspaces_dir().join(id).join("workspace.yaml");
    let scope = format!("http://localhost:{port}/scope.html");
    let modal_rules = "routing:\n  rules:\n    - match: { url: ['127.0.0.1'] }\n      open: modal\n    - match: { url: ['127.0.0.1'] }\n      open: workspace:mb\n";
    std::fs::write(wd("ma"), format!("name: A\nhome_page: '{scope}'\n{modal_rules}")).expect("ma");
    std::fs::write(wd("mb"), "name: B\n").expect("mb");
    std::fs::write(wd("mp"), format!("name: P\nhome_page: '{}'\ntabs: {{ popups: modal }}\n", page("popup.html"))).expect("mp");
    std::fs::write(wd("mq"), format!("name: Q\nhome_page: '{}'\ntabs: {{ popups: block }}\n", page("popup.html"))).expect("mq");
    std::fs::write(wd("mw"), format!("name: W\nhome_page: '{scope}'\nmodals: {{ style: window }}\n{modal_rules}")).expect("mw");
    crate::router::install(crate::router::Router::load(&std::env::temp_dir().join("browsah-no-such-config.yaml"), Some(&profile)));
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    let open = |id: &str| ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));

    // A routing rule opens a link as a modal.
    open("ma");
    let ok = wait_until(15_000, || find("ma").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && t.view().has_frame()))).await;
    r.check("workspace A opens on its home page", ok, || "not open".into());
    let Some(ma) = find("ma") else { return };
    glib::timeout_future(Duration::from_millis(800)).await;
    let d = Driver { shell: &ma };
    d.click(40.0, 45.0);
    let ok = wait_until(15_000, || ma.modal_list().len() == 1 && ma.modal_list()[0].state.borrow().title == "Page B").await;
    r.check("a link whose rule says modal opens in a modal over the window", ok, || format!("{} modals", ma.modal_list().len()));
    let Some(modal) = ma.modal_list().first().cloned() else { return };
    r.check("the modal's address bar is read-only and shows the address", !modal.entry.is_editable() && modal.entry.text().contains("127.0.0.1"), || format!("editable={} text={:?}", modal.entry.is_editable(), modal.entry.text()));
    r.check("the tab under the modal did not move and the modal is an overlay", ma.tab_count() == 1 && ma.current().is_some_and(|t| t.state.borrow().title == "Scope test") && modal.dialog.is_some(), || "tab changed".into());

    if let Ok(path) = std::env::var("BROWSAH_SCREENSHOT") {
        glib::timeout_future(Duration::from_millis(1200)).await;
        println!("screenshot {path}.modal.png: {}", if screenshot(&ma.window, &format!("{path}.modal.png")) { "saved" } else { "failed" });
    }

    // Move: the router (skipping the modal rule) names workspace B.
    modal.move_via_router();
    let ok = wait_until(15_000, || find("mb").is_some_and(|s| s.tab_count() == 1 && s.current().is_some_and(|t| t.page.title() == "Page B"))).await;
    r.check("moving a modal sends its page to the workspace the router names", ok, || format!("{:?}", find("mb").map(|s| s.tab_count())));
    let ok = wait_until(5_000, || ma.modal_list().is_empty() && find("mb").and_then(|s| s.current()).is_some_and(|t| !t.view().is_dormant() && t.view().has_frame())).await;
    r.check("the modal is gone and the page kept running as a tab", ok, || format!("modals {} live {:?}", ma.modal_list().len(), find("mb").and_then(|s| s.current()).map(|t| (t.view().is_dormant(), t.view().has_frame()))));
    if let Some(t) = find("mb").and_then(|s| s.current()) {
        t.view().navigate(&page("popup.html"));
        let ok = wait_until(15_000, || t.page.title() == "Popup test").await;
        r.check("the moved page is wired to its new window (it updates its tab)", ok, || format!("{:?}", t.page.title()));
    }

    // Closing a modal closes its page.
    d.click(40.0, 45.0);
    let ok = wait_until(15_000, || ma.modal_list().len() == 1).await;
    if ok {
        ma.modal_list()[0].close();
    }
    let ok2 = wait_until(5_000, || ma.modal_list().is_empty()).await;
    r.check("closing a modal removes it", ok && ok2, || format!("{} modals", ma.modal_list().len()));

    // Popup policy: modal.
    open("mp");
    let ok = wait_until(15_000, || find("mp").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Popup test" && t.view().has_frame()))).await;
    r.check("workspace P (popups as modals) opens", ok, || "not open".into());
    if let Some(mp) = find("mp") {
        glib::timeout_future(Duration::from_millis(600)).await;
        Driver { shell: &mp }.click(80.0, 40.0);
        let ok = wait_until(15_000, || mp.modal_list().len() == 1 && mp.modal_list()[0].state.borrow().title == "Page B").await;
        r.check("a script popup opens as a modal when the workspace says so", ok && mp.tab_count() == 1, || format!("{} modals, {} tabs", mp.modal_list().len(), mp.tab_count()));
    }

    // Popup policy: block.
    open("mq");
    let ok = wait_until(15_000, || find("mq").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Popup test" && t.view().has_frame()))).await;
    r.check("workspace Q (popups blocked) opens", ok, || "not open".into());
    if let Some(mq) = find("mq") {
        glib::timeout_future(Duration::from_millis(600)).await;
        let windows = super::windows().len();
        Driver { shell: &mq }.click(80.0, 40.0);
        glib::timeout_future(Duration::from_millis(2500)).await;
        r.check("a script popup is not opened when the workspace blocks them", mq.tab_count() == 1 && mq.modal_list().is_empty() && super::windows().len() == windows, || format!("{} tabs, {} modals, {} windows (was {windows})", mq.tab_count(), mq.modal_list().len(), super::windows().len()));
    }

    // Modal style: window.
    open("mw");
    let ok = wait_until(15_000, || find("mw").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && t.view().has_frame()))).await;
    r.check("workspace W (modals as windows) opens", ok, || "not open".into());
    if let Some(mw) = find("mw") {
        glib::timeout_future(Duration::from_millis(800)).await;
        Driver { shell: &mw }.click(40.0, 45.0);
        let ok = wait_until(15_000, || mw.modal_list().len() == 1).await;
        r.check("with modals.style window a modal is a window of its own", ok && mw.modal_list()[0].window.is_some() && mw.modal_list()[0].dialog.is_none(), || "not a window".into());
        if let Some(m) = mw.modal_list().first() {
            m.close();
        }
    }

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    crate::router::install(crate::router::Router::default());
    for id in ids {
        if let Ok(s) = crate::store::Store::open(&db) {
            let _ = s.delete_workspace(id);
        }
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// History: visits are recorded per workspace (not reloads, excluded addresses, disabled workspaces or
/// sign-in windows), the history window lists and searches them, closed tabs can be reopened, and a
/// closed ephemeral window comes back whole.
async fn run_history(shell: &Rc<Shell>, r: &mut Report) {
    use crate::store::Scope;
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let store = || crate::store::Store::open(&db).expect("store");
    let _ = store().clear_history(None);
    for id in ["hw", "hn"] {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = store().delete_workspace(id);
    }
    let home = format!("http://localhost:{port}/scope.html");
    std::fs::write(profile.workspaces_dir().join("hw/workspace.yaml"), format!("name: History\nhome_page: '{home}'\nhistory: {{ exclude: ['127.0.0.1'] }}\n")).expect("hw");
    std::fs::write(profile.workspaces_dir().join("hn/workspace.yaml"), format!("name: NoHistory\nhome_page: '{home}'\nhistory: {{ enabled: false }}\n")).expect("hn");
    crate::router::install(crate::router::Router::default());
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    let visits = |ws: &str| store().search_visits("", Scope::Workspace(ws), 100);

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"hw".to_variant()));
    let ok = wait_until(15_000, || find("hw").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && !t.state.borrow().loading))).await;
    r.check("a workspace opens for the history test", ok, || "not open".into());
    let Some(hw) = find("hw") else { return };
    let tab = hw.current().expect("tab");
    let ok = wait_until(5_000, || visits("hw").len() == 1).await;
    r.check("a page visit is recorded with its workspace and title", ok && visits("hw")[0].title == "Scope test" && visits("hw")[0].url == home, || format!("{:?}", visits("hw")));
    tab.view().navigate(&format!("http://localhost:{port}/b.html"));
    let ok = wait_until(15_000, || tab.state.borrow().title == "Page B" && visits("hw").len() == 2).await;
    r.check("a second page is a second visit", ok, || format!("{:?}", visits("hw")));
    tab.view().reload();
    glib::timeout_future(Duration::from_millis(2500)).await;
    r.check("reloading is not a new visit", visits("hw").len() == 2, || format!("{} visits", visits("hw").len()));
    tab.view().navigate(&format!("http://127.0.0.1:{port}/b.html"));
    let ok = wait_until(15_000, || tab.state.borrow().address.contains("127.0.0.1") && !tab.state.borrow().loading).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    r.check("an excluded address is not recorded", ok && visits("hw").len() == 2, || format!("{:?}", visits("hw")));
    tab.view().navigate(&format!("http://localhost:{port}/b.html?again=1"));
    let _ = wait_until(15_000, || visits("hw").len() == 3).await;

    // The history window.
    let _ = WidgetExt::activate_action(&hw.window, "win.history", None);
    let ok = wait_until(5_000, || hw.history.borrow().is_some()).await;
    r.check("Ctrl+H opens the history window", ok, || "not open".into());
    let Some(hv) = hw.history.borrow().clone() else { return };
    let count = |l: &gtk::ListBox| (0..).take_while(|i| l.row_at_index(*i).is_some()).count();
    r.check("it lists this workspace's visits", count(&hv.visits) == 3, || format!("{} rows", count(&hv.visits)));
    hv.search.set_text("again");
    let ok = wait_until(5_000, || count(&hv.visits) == 1).await;
    r.check("searching narrows the list", ok, || format!("{} rows", count(&hv.visits)));
    hv.search.set_text("");
    hv.visits.row_at_index(count(&hv.visits).saturating_sub(1) as i32).map(|row| row.emit_by_name::<()>("activate", &[]));
    let ok = wait_until(15_000, || tab.state.borrow().address == home).await;
    r.check("activating a visit opens it in the current tab", ok, || format!("{:?}", tab.state.borrow().address));
    hw.close_history();

    // Closed tabs.
    let extra = hw.open_tab(&format!("http://localhost:{port}/b.html?extra=1"), true);
    let _ = wait_until(15_000, || extra.state.borrow().title == "Page B" && !extra.state.borrow().loading).await;
    let _ = WidgetExt::activate_action(&hw.window, "win.close-tab", None);
    let ok = wait_until(5_000, || store().closed_tabs(Scope::Workspace("hw"), 10).len() == 1).await;
    let closed = store().closed_tabs(Scope::Workspace("hw"), 10);
    r.check("a tab closed by the user is kept with its reason", ok && closed[0].reason == "user" && closed[0].url.contains("extra=1"), || format!("{closed:?}"));
    let before = hw.tab_count();
    let _ = WidgetExt::activate_action(&hw.window, "win.reopen-tab", None);
    let ok = wait_until(15_000, || hw.tab_count() == before + 1 && hw.current().is_some_and(|t| t.state.borrow().address.contains("extra=1"))).await;
    r.check("Ctrl+Shift+T reopens it, once", ok && store().closed_tabs(Scope::Workspace("hw"), 10).is_empty(), || format!("{} tabs (was {before})", hw.tab_count()));

    // Not recorded: history turned off for a workspace; a sign-in another application started.
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"hn".to_variant()));
    let ok = wait_until(15_000, || find("hn").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && !t.state.borrow().loading))).await;
    glib::timeout_future(Duration::from_millis(1500)).await;
    r.check("a workspace with history off records nothing", ok && visits("hn").is_empty(), || format!("{:?}", visits("hn")));
    let n = super::windows().len();
    app.open(&[gio::File::for_uri(&format!("http://localhost:{port}/b.html?client_id=cli&response_type=code&redirect_uri=http%3A%2F%2Flocalhost%3A9%2Fcb"))], "");
    let ok = wait_until(15_000, || super::windows().len() == n + 1).await;
    glib::timeout_future(Duration::from_millis(2500)).await;
    r.check("a sign-in window for another application is never recorded", ok && store().search_visits("response_type", Scope::All, 10).is_empty(), || format!("{:?}", store().search_visits("response_type", Scope::All, 10)));

    // An ephemeral window closed with tabs open can be reopened whole.
    ActionGroupExt::activate_action(&app, "new-window", None);
    let find_eph = || super::windows().into_iter().rev().find(|s| s.workspace.is_none() && !Rc::ptr_eq(s, shell) && !s.private.get());
    let ok = wait_until(15_000, || find_eph().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().ready))).await;
    r.check("an ephemeral window opens", ok, || "not open".into());
    if let Some(e) = find_eph() {
        let first = e.current().expect("tab");
        first.view().navigate(&format!("http://localhost:{port}/b.html?eph=1"));
        let _ = wait_until(15_000, || first.state.borrow().title == "Page B" && !first.state.borrow().loading).await;
        let second = e.open_tab(&format!("http://localhost:{port}/b.html?eph=2"), true);
        let _ = wait_until(15_000, || second.state.borrow().title == "Page B" && !second.state.borrow().loading).await;
        e.window.close();
        let ok = wait_until(5_000, || store().closed_tabs(Scope::Ephemeral, 10).iter().filter(|t| t.reason == "window").count() >= 2).await;
        r.check("closing an ephemeral window keeps its tabs as a group", ok, || format!("{:?}", store().closed_tabs(Scope::Ephemeral, 10)));
        let windows = super::windows().len();
        let _ = WidgetExt::activate_action(&shell.window, "win.reopen-tab", None);
        let ok = wait_until(15_000, || super::windows().len() == windows + 1 && find_eph().is_some_and(|s| s.tab_count() == 2)).await;
        r.check("Ctrl+Shift+T in an ephemeral window reopens the last closed window with its tabs", ok, || format!("{} windows (was {windows})", super::windows().len()));
    }

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = store().clear_history(None);
    for id in ["hw", "hn"] {
        let _ = store().delete_workspace(id);
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// Tab sweeping: tabs idle past their rule are closed (recorded as swept and reopenable) or discarded,
/// while the selected tab and a tab with unsaved form input are left alone.
async fn run_sweep(shell: &Rc<Shell>, r: &mut Report) {
    use crate::store::Scope;
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let store = || crate::store::Store::open(&db).expect("store");
    let dir = profile.workspaces_dir().join("sw");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    let _ = store().delete_workspace("sw");
    let _ = store().clear_history(None);
    std::fs::write(
        dir.join("workspace.yaml"),
        format!(
            "name: Sweep\nhome_page: 'http://localhost:{port}/scope.html'\nsweep:\n  enabled: true\n  interval: 1d\n  rules:\n    - name: Stale\n      match: {{ url: ['localhost/b.html*'] }}\n      idle_for: 2s\n    - name: Sleepy\n      match: {{ url: ['127.0.0.1'] }}\n      idle_for: 2s\n      action: discard\n    - name: Forms\n      match: {{ url: ['localhost/form.html'] }}\n      idle_for: 2s\n    - name: Everything\n      idle_for: 2s\ntabs:\n  pinned:\n    - url: 'http://localhost:{port}/b.html?pinned=1'\n"
        ),
    )
    .expect("workspace.yaml");
    crate::router::install(crate::router::Router::default());
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "sw"));

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"sw".to_variant()));
    // (A workspace with pinned tabs opens on them, not on its home page.)
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    r.check("a workspace with sweeping rules opens", ok, || "not open".into());
    let Some(sw) = find() else { return };
    r.check("its rules are loaded", sw.sweeper.as_ref().is_some_and(|s| s.rules.len() == 4), || format!("{:?}", sw.sweeper));
    let pinned = sw.tabs().into_iter().find(|t| t.page.is_pinned());
    r.check("the workspace has its pinned tab", pinned.is_some(), || "no pinned tab".into());
    let home = sw.open_tab(&format!("http://localhost:{port}/scope.html"), true);
    let _ = wait_until(15_000, || home.state.borrow().title == "Scope test").await;
    let stale = sw.open_tab(&format!("http://localhost:{port}/b.html?stale=1"), true);
    let sleepy = sw.open_tab(&format!("http://127.0.0.1:{port}/b.html?sleepy=1"), true);
    let form = sw.open_tab(&format!("http://localhost:{port}/form.html"), true);
    let ok = wait_until(20_000, || [&stale, &sleepy, &form].iter().all(|t| !t.state.borrow().loading && !t.state.borrow().title.is_empty())).await;
    r.check("the tabs to sweep load", ok, || "not loaded".into());
    // Type into the form (set by script, as a page would see it), then look at the home tab so nothing else is selected.
    let id = form.view().cdp_eval("document.getElementById('i').value = 'half typed'; 'ok'");
    let _ = wait_until(5_000, || form.view().cdp_result(id).is_some()).await;
    sw.tab_view.set_selected_page(&home.page);
    glib::timeout_future(Duration::from_millis(3500)).await; // idle for longer than the rules' 2 s

    let before = sw.tab_count();
    sw.sweep_now();
    let ok = wait_until(15_000, || sw.sweeps_done() == 1).await;
    r.check("a sweep runs", ok, || "no sweep".into());
    r.check("an idle tab matching a close rule is closed", sw.tab_count() == before - 1 && !sw.tabs().iter().any(|t| t.uid == stale.uid), || format!("{} tabs (was {before}); {:?}", sw.tab_count(), sw.last_sweep_notes()));
    let closed = store().closed_tabs(Scope::Workspace("sw"), 10);
    r.check("the closed tab is recorded as swept by its rule", closed.len() == 1 && closed[0].reason == "sweep:Stale" && closed[0].url.contains("stale=1"), || format!("{closed:?}"));
    r.check("an idle tab matching a discard rule stays but is put to sleep", sw.tabs().iter().any(|t| t.uid == sleepy.uid) && sleepy.view().is_dormant(), || format!("dormant={}", sleepy.view().is_dormant()));
    r.check("a tab with unsaved form input is left alone and marked", sw.tabs().iter().any(|t| t.uid == form.uid) && form.dirty.get() && form.page.indicator_icon().is_some(), || format!("dirty={}", form.dirty.get()));
    r.check("the tab being looked at is never swept", sw.tabs().iter().any(|t| t.uid == home.uid), || "home swept".into());
    r.check("a pinned tab is never swept, however idle (a rule matches every tab)", pinned.as_ref().is_some_and(|p| sw.tabs().iter().any(|t| t.uid == p.uid)), || "pinned tab swept".into());

    // A swept tab comes back with Ctrl+Shift+T.
    let n = sw.tab_count();
    let _ = WidgetExt::activate_action(&sw.window, "win.reopen-tab", None);
    let ok = wait_until(15_000, || sw.tab_count() == n + 1 && sw.current().is_some_and(|t| t.state.borrow().address.contains("stale=1"))).await;
    r.check("reopening brings the swept tab back", ok, || format!("{} tabs (was {n})", sw.tab_count()));

    // Use is remembered with the tabs.
    sw.window.close();
    let _ = wait_until(5_000, || find().is_none()).await;
    let saved = store().load_tabs("sw");
    r.check("tab use is saved with the remembered tabs", !saved.is_empty() && saved.iter().all(|t| !t.usage.is_empty()), || format!("{saved:?}"));

    let _ = store().delete_workspace("sw");
    let _ = store().clear_history(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A workspace whose window is closed is still swept: its remembered tabs go by their remembered use.
async fn run_closed_workspace_sweep(shell: &Rc<Shell>, r: &mut Report) {
    use crate::store::{Scope, TabRecord};
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let store = || crate::store::Store::open(&db).expect("store");
    let dir = profile.workspaces_dir().join("sx");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    let _ = store().delete_workspace("sx");
    let _ = store().clear_history(None);
    std::fs::write(dir.join("workspace.yaml"), "name: Closed\nsweep:\n  enabled: true\n  interval: 1d\n  rules:\n    - name: Gone\n      match: { url: ['localhost'] }\n      idle_for: 1h\n").expect("workspace.yaml");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let tab = |uid: u64, url: &str, used: i64, pinned: bool| TabRecord { uid, url: url.into(), title: format!("T{uid}"), pinned, selected: uid == 1, usage: format!("{used}:1"), group: 0 };
    let old = now - 3 * 86_400;
    store().save_tabs("sx", &[
        tab(1, "http://localhost:1/old-selected", old, false),
        tab(2, "http://localhost:1/fresh", now - 60, false),
        tab(3, "http://localhost:1/pinned", old, true),
        tab(4, "https://example.org/other-site", old, false),
    ]).expect("save");
    let app_file = std::env::temp_dir().join("browsah-no-such-config.yaml");

    // No window for the workspace is open. A dry run says what would go and changes nothing.
    let ws = crate::workspace::Workspace::resolve(&profile, "sx").expect("workspace");
    let dry = crate::sweep::sweep_stored(&profile, &ws, &store(), &app_file, now, false);
    r.check("a dry run reports what would be swept and changes nothing", dry.len() == 1 && dry[0].starts_with("would close") && store().load_tabs("sx").len() == 4, || format!("{dry:?}"));

    // The process's own pass does it.
    let notes = super::sweep::background_sweep(&profile, &app_file, true);
    r.check("the background pass sweeps a workspace that is not open", notes.len() == 1 && notes[0].contains("old-selected"), || format!("{notes:?}"));
    let kept: Vec<u64> = store().load_tabs("sx").iter().map(|t| t.uid).collect();
    r.check("only idle tabs matching the rule are removed (not fresh, pinned or other-site ones)", kept == [2, 3, 4], || format!("{kept:?}"));
    let closed = store().closed_tabs(Scope::Workspace("sx"), 10);
    r.check("the swept tab is recorded as swept by its rule", closed.len() == 1 && closed[0].reason == "sweep:Gone", || format!("{closed:?}"));
    r.check("a second pass has nothing left to sweep", super::sweep::background_sweep(&profile, &app_file, true).is_empty(), || "swept again".into());

    // Opening the workspace shows what is left.
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"sx".to_variant()));
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "sx"));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.tab_count() == 2)).await; // the pinned record comes from the configuration, not the database
    r.check("the workspace opens with the remaining tabs", ok, || format!("{:?}", find().map(|s| s.tab_count())));
    // While its window is open the background pass leaves it to the window.
    store().save_tabs("sx", &[tab(9, "http://localhost:1/x", old, false)]).expect("save");
    r.check("the background pass skips a workspace whose window is open", super::sweep::background_sweep(&profile, &app_file, true).is_empty(), || "swept an open workspace".into());
    if let Some(w) = find() {
        w.window.close();
    }
    let _ = wait_until(5_000, || find().is_none()).await;
    let _ = store().delete_workspace("sx");
    let _ = store().clear_history(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What a new tab opens is configurable: blank by default, the home page with `new_tab_page: home`.
async fn run_new_tab_page(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let port = start_test_server();
    let db = profile.dir.join("data.sqlite");
    for id in ["nh", "nb"] {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
    }
    let home = format!("http://localhost:{port}/scope.html");
    std::fs::write(profile.workspaces_dir().join("nh/workspace.yaml"), format!("home_page: '{home}'\nnew_tab_page: home\n")).expect("nh");
    std::fs::write(profile.workspaces_dir().join("nb/workspace.yaml"), format!("home_page: '{home}'\n")).expect("nb");
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));

    for (id, wants_home) in [("nh", true), ("nb", false)] {
        ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
        let ok = wait_until(15_000, || find(id).is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && !t.state.borrow().loading))).await;
        let Some(w) = find(id) else { r.check("a workspace opens for the new-tab test", false, || format!("{id} not open")); continue };
        r.check("a workspace opens for the new-tab test", ok, || format!("{id} not loaded"));
        let _ = WidgetExt::activate_action(&w.window, "win.new-tab", None);
        let ok = wait_until(10_000, || w.tab_count() == 2).await;
        let new = w.current().expect("tab");
        if wants_home {
            let ok2 = wait_until(15_000, || new.state.borrow().title == "Scope test").await;
            r.check("with new_tab_page: home a new tab opens the home page", ok && ok2, || format!("{:?}", new.state.borrow().address));
        } else {
            glib::timeout_future(Duration::from_millis(800)).await;
            r.check("by default a new tab is blank", ok && new.state.borrow().address.is_empty() || new.state.borrow().address == "about:blank", || format!("{:?}", new.state.borrow().address));
        }
        w.window.close();
        let _ = wait_until(5_000, || find(id).is_none()).await;
    }
    for id in ["nh", "nb"] {
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// Run JavaScript in a tab and return the string it produced (a promise is awaited).
async fn eval_string(tab: &Rc<super::Tab>, expression: &str) -> Option<String> {
    let v = tab.view();
    let id = v.cdp_eval(expression);
    let _ = wait_until(15_000, || v.cdp_result(id).is_some()).await;
    let (_, json) = v.cdp_result(id)?;
    let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&json).ok()?;
    parsed.get("result")?.get("value")?.as_str().map(str::to_owned)
}

/// Permissions: the configuration allows or denies (per kind and per site) without asking; `ask` puts a
/// question to the person, and a remembered answer stands in for the question.
async fn run_permissions(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let store = || crate::store::Store::open(&db).expect("store");
    let page = format!("http://localhost:{port}/b.html");
    for id in ["pm", "pa"] {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = store().delete_workspace(id);
    }
    let _ = store().forget_permissions(None);
    std::fs::write(profile.workspaces_dir().join("pm/workspace.yaml"), format!("home_page: '{page}'\npermissions:\n  default: ask\n  notifications: allow\n  location: deny\n  sites:\n    - match: ['127.0.0.1']\n      notifications: deny\n")).expect("pm");
    std::fs::write(profile.workspaces_dir().join("pa/workspace.yaml"), format!("home_page: '{page}'\npermissions:\n  default: ask\n")).expect("pa");
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    let geo = "new Promise(r => navigator.geolocation.getCurrentPosition(() => r('ok'), e => r('err' + e.code), { timeout: 3000 }))";

    // The configuration decides.
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"pm".to_variant()));
    let ok = wait_until(15_000, || find("pm").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    r.check("a workspace with permission rules opens", ok, || "not open".into());
    let Some(pm) = find("pm") else { return };
    let tab = pm.current().expect("tab");
    let granted = eval_string(&tab, "Notification.requestPermission()").await;
    r.check("a kind set to allow is granted without asking", granted.as_deref() == Some("granted") && pm.pending_permission().is_none(), || format!("{granted:?}"));
    let loc = eval_string(&tab, geo).await;
    r.check("a kind set to deny is refused without asking", loc.as_deref() == Some("err1") && pm.pending_permission().is_none(), || format!("{loc:?}"));
    tab.view().navigate(&format!("http://127.0.0.1:{port}/b.html"));
    let _ = wait_until(15_000, || tab.state.borrow().address.contains("127.0.0.1") && !tab.state.borrow().loading).await;
    let denied = eval_string(&tab, "Notification.requestPermission()").await;
    r.check("a site rule beats the kind's setting", denied.as_deref() == Some("denied") && pm.pending_permission().is_none(), || format!("{denied:?}"));

    // Asking. (Chromium keeps the answers it was given for an origin, so each scenario below uses an
    // origin of its own: another server, another port.)
    let port = start_test_server();
    std::fs::write(profile.workspaces_dir().join("pa/workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\npermissions:\n  default: ask\n")).expect("pa");
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"pa".to_variant()));
    let ok = wait_until(15_000, || find("pa").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    r.check("a workspace that asks opens", ok, || "not open".into());
    let Some(pa) = find("pa") else { return };
    let tab = pa.current().expect("tab");
    let _ = eval_string(&tab, "window.__p = Notification.requestPermission(); 'started'").await;
    let ok = wait_until(10_000, || pa.pending_permission().is_some()).await;
    let q = pa.pending_permission();
    r.check("a request that needs asking becomes a question for the person", ok && q.as_ref().is_some_and(|q| q.unresolved() == [crate::permissions::Kind::Notifications] && q.req.origin == format!("http://localhost:{port}")), || format!("{:?}", q.map(|q| q.req.origin.clone())));
    pa.answer_pending_permission(true, true);
    let result = eval_string(&tab, "window.__p").await;
    r.check("allowing answers the page", result.as_deref() == Some("granted"), || format!("{result:?}"));
    let kept = store().remembered_permission(&format!("http://localhost:{port}"), "notifications");
    r.check("the answer is remembered for the site and kind", kept.as_deref() == Some("allow"), || format!("{kept:?}"));

    // A permission lasts as long as the page it was given to. Chromium would go on answering the same origin
    // itself for good (making the configuration, "forget" and "do not remember" meaningless), so what it kept is
    // cleared when a page of the origin starts loading: the next load reaches us again, and the remembered answer
    // stands in for the question.
    let samepage = eval_string(&tab, "Notification.requestPermission()").await;
    r.check("while the page stays, Chromium goes on answering it", samepage.as_deref() == Some("granted"), || format!("{samepage:?}"));
    tab.view().navigate(&format!("http://localhost:{port}/b.html?second=1"));
    let _ = wait_until(15_000, || tab.state.borrow().address.contains("second=1") && !tab.state.borrow().loading).await;
    let before = super::view::PERMISSION_REQUESTS.load(std::sync::atomic::Ordering::Relaxed);
    let again = eval_string(&tab, "Notification.requestPermission()").await;
    let seen = super::view::PERMISSION_REQUESTS.load(std::sync::atomic::Ordering::Relaxed) - before;
    r.check("the next load of the origin reaches us again and is answered from the remembered answer", seen == 1 && again.as_deref() == Some("granted") && pa.pending_permission().is_none(), || format!("{again:?}, handler reached {seen} time(s)"));

    // A remembered answer stands in for the question.
    let port = start_test_server();
    tab.view().navigate(&format!("http://localhost:{port}/b.html"));
    let _ = wait_until(15_000, || tab.state.borrow().address.contains(&port.to_string()) && !tab.state.borrow().loading).await;
    let origin = format!("http://localhost:{port}");
    store().remember_permission(&origin, "location", "deny", 1).expect("remember");
    let loc = eval_string(&tab, geo).await;
    r.check("a remembered deny answers without a question", loc.as_deref() == Some("err1") && pa.pending_permission().is_none(), || format!("{loc:?}"));
    let port = start_test_server();
    tab.view().navigate(&format!("http://localhost:{port}/b.html"));
    let _ = wait_until(15_000, || tab.state.borrow().address.contains(&port.to_string()) && !tab.state.borrow().loading).await;
    store().remember_permission(&format!("http://localhost:{port}"), "location", "allow", 2).expect("remember");
    let loc = eval_string(&tab, geo).await;
    r.check("a remembered allow lets the request through without a question", loc.as_deref() != Some("err1") && pa.pending_permission().is_none(), || format!("{loc:?}"));
    // Blocking without remembering keeps nothing.
    let port = start_test_server();
    tab.view().navigate(&format!("http://localhost:{port}/b.html"));
    let _ = wait_until(15_000, || tab.state.borrow().address.contains(&port.to_string()) && !tab.state.borrow().loading).await;
    let _ = store().forget_permissions(None);
    let _ = eval_string(&tab, "window.__q = new Promise(r => navigator.geolocation.getCurrentPosition(() => r('ok'), e => r('err' + e.code), { timeout: 5000 })); 'started'").await;
    let ok = wait_until(10_000, || pa.pending_permission().is_some()).await;
    pa.answer_pending_permission(false, false);
    let result = eval_string(&tab, "window.__q").await;
    r.check("blocking without remembering refuses this time and keeps nothing", ok && result.as_deref() == Some("err1") && store().remembered_permissions().is_empty(), || format!("{result:?} {:?}", store().remembered_permissions()));

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    for id in ["pm", "pa"] {
        let _ = store().delete_workspace(id);
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
    let _ = store().forget_permissions(None);
}

/// Downloads go to the configured folder without overwriting; `ask` puts a Save-as question first.
async fn run_downloads(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let tmp = std::env::temp_dir().join(format!("browsah-dl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    for id in ["dl", "da"] {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
    }
    let home = format!("http://localhost:{port}/b.html");
    std::fs::write(profile.workspaces_dir().join("dl/workspace.yaml"), format!("home_page: '{home}'\ndownloads:\n  directory: '{}'\n", tmp.join("plain").display())).expect("dl");
    std::fs::write(profile.workspaces_dir().join("da/workspace.yaml"), format!("home_page: '{home}'\ndownloads:\n  directory: '{}'\n  ask: true\n", tmp.join("asked").display())).expect("da");
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    let file_url = format!("http://localhost:{port}/file.txt");

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"dl".to_variant()));
    let ok = wait_until(15_000, || find("dl").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    r.check("a workspace with a download folder opens", ok, || "not open".into());
    let Some(dl) = find("dl") else { return };
    let tab = dl.current().expect("tab");
    tab.view().navigate(&file_url);
    let first = tmp.join("plain/note.txt");
    let ok = wait_until(15_000, || first.is_file() && dl.completed_downloads().len() == 1).await;
    r.check("a download is saved in the configured folder under the suggested name", ok && std::fs::read_to_string(&first).ok().as_deref() == Some("hello"), || format!("{:?}", std::fs::read_dir(tmp.join("plain")).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())));
    tab.view().navigate(&file_url);
    let second = tmp.join("plain/note (1).txt");
    let ok = wait_until(15_000, || second.is_file() && dl.completed_downloads().len() == 2).await;
    r.check("a second download of the same name does not overwrite the first", ok && first.is_file(), || "no second file".into());

    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"da".to_variant()));
    let ok = wait_until(15_000, || find("da").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    r.check("a workspace that asks where to save opens", ok, || "not open".into());
    let Some(da) = find("da") else { return };
    let tab = da.current().expect("tab");
    tab.view().navigate(&file_url);
    let ok = wait_until(15_000, || da.pending_download_question().is_some()).await;
    let q = da.pending_download_question();
    r.check("with ask the download waits for a place to save, suggesting the name and folder", ok && q.as_ref().is_some_and(|q| q.suggested == "note.txt" && q.dir == tmp.join("asked")), || format!("{q:?}"));
    if let Some(q) = da.take_pending_download() {
        q.answer(Some(tmp.join("asked/chosen.txt")));
    }
    let chosen = tmp.join("asked/chosen.txt");
    let ok = wait_until(15_000, || chosen.is_file()).await;
    r.check("a chosen place is used", ok && std::fs::read_to_string(&chosen).ok().as_deref() == Some("hello"), || "not saved".into());
    tab.view().navigate(&file_url);
    let ok = wait_until(15_000, || da.pending_download_question().is_some()).await;
    if let Some(q) = da.take_pending_download() {
        q.answer(None);
    }
    glib::timeout_future(Duration::from_millis(1500)).await;
    let files = std::fs::read_dir(tmp.join("asked")).map(|d| d.flatten().count()).unwrap_or(0);
    r.check("cancelling the question saves nothing", ok && files == 1, || format!("{files} files"));

    // The downloads list.
    {
        let pop = da.downloads.button.popover().expect("popover");
        pop.popup();
        glib::timeout_future(Duration::from_millis(400)).await;
        shot(&pop, "downloads-finished");
        pop.popdown();
    }
    let rows = super::downloads::rows();
    let done: Vec<&str> = rows.iter().filter(|(_, s)| *s == super::downloads::RowState::Done).map(|(n, _)| n.as_str()).collect();
    r.check("finished downloads are in the list", ["note.txt", "note (1).txt", "chosen.txt"].iter().all(|n| done.contains(n)), || format!("{rows:?}"));
    r.check("the list shows a row for each and the button is shown", da.downloads.shown_rows() == rows.len() && da.downloads.button.is_visible(), || format!("{} rows widgets vs {}", da.downloads.shown_rows(), rows.len()));
    let remembered = super::store().map(|s| s.downloads(50)).unwrap_or_default();
    r.check("finished downloads are remembered in the database", remembered.iter().filter(|d| d.state == "done").count() >= 3 && remembered.iter().any(|d| d.path.ends_with("note.txt")), || format!("{remembered:?}"));
    super::downloads::clear_all();
    r.check("clearing the list empties it, the database and hides the button (the files stay)", super::downloads::rows().is_empty() && super::store().is_some_and(|s| s.downloads(50).is_empty()) && !da.downloads.button.is_visible() && first.is_file(), || format!("{:?}", super::downloads::rows()));

    // A download in progress is listed with its progress and can be cancelled from the list.
    dl.current().expect("tab").view().navigate(&format!("http://localhost:{port}/slow"));
    let ok = wait_until(15_000, || super::downloads::rows().iter().any(|(n, s)| n == "slow.bin" && matches!(s, super::downloads::RowState::Active { received, .. } if *received > 0))).await;
    r.check("a download in progress is listed with the bytes received so far", ok, || format!("{:?}", super::downloads::rows()));
    {
        // (The `da` window is the one in front.)
        let pop = da.downloads.button.popover().expect("popover");
        pop.popup();
        glib::timeout_future(Duration::from_millis(500)).await;
        shot(&pop, "downloads-running");
        pop.popdown();
        shot(&da.window, "window-with-downloads-button");
    }
    r.check("the button shows while it runs", dl.downloads.button.is_visible() && dl.downloads.shown_rows() == 1, || format!("{}", dl.downloads.shown_rows()));
    super::downloads::cancel_active();
    let ok = wait_until(15_000, || super::downloads::rows().iter().any(|(n, s)| n == "slow.bin" && *s == super::downloads::RowState::Cancelled)).await;
    r.check("cancelling it from the list stops it", ok, || format!("{:?}", super::downloads::rows()));
    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = std::fs::remove_dir_all(&tmp);
    for id in ["dl", "da"] {
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// Cookie rules: a site a rule denies can neither save nor send cookies; others can.
async fn run_cookies(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let dir = profile.workspaces_dir().join("ck");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("ck"));
    std::fs::write(dir.join("workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\ncookies:\n  rules:\n    - match: ['127.0.0.1']\n      action: deny\n")).expect("ck");
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "ck"));
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"ck".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    r.check("a workspace with cookie rules opens", ok, || "not open".into());
    let Some(ws) = find() else { return };
    let tab = ws.current().expect("tab");
    for (host, expect, what) in [("localhost", "Cookies [ck=1]", "an allowed site keeps its cookies"), ("127.0.0.1", "Cookies []", "a denied site neither saves nor sends cookies")] {
        tab.view().navigate(&format!("http://{host}:{port}/cookie-set"));
        let ok = wait_until(15_000, || tab.state.borrow().title == "Cookie set" && !tab.state.borrow().loading).await;
        tab.view().navigate(&format!("http://{host}:{port}/cookie-show"));
        let got = wait_until(15_000, || tab.state.borrow().title.starts_with("Cookies [") && !tab.state.borrow().loading).await;
        let title = tab.state.borrow().title.clone();
        r.check(what, ok && got && title == expect, || format!("title {title:?}"));
    }
    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("ck"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Isolated storage: a workspace with `storage: isolated` has cookies of its own; shared ones see each other's.
async fn run_storage(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let ids = ["sh1", "sh2", "is1", "is2"];
    for id in ids {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let storage = if id.starts_with("is") { "isolated" } else { "shared" };
        std::fs::write(dir.join("workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\nstorage: {storage}\n")).expect("ws");
    }
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    let open = |id: &'static str| {
        let app = app.clone();
        async move {
        ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
        wait_until(15_000, || find(id).is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await
        }
    };
    let show = |id: &'static str| async move {
        let tab = find(id)?.current()?;
        tab.view().navigate(&format!("http://localhost:{port}/cookie-show"));
        wait_until(15_000, || tab.state.borrow().title.starts_with("Cookies [") && !tab.state.borrow().loading).await;
        let t = tab.state.borrow().title.clone();
        Some(t)
    };
    let set = |id: &'static str| async move {
        let Some(tab) = find(id).and_then(|s| s.current()) else { return };
        tab.view().navigate(&format!("http://localhost:{port}/cookie-set"));
        wait_until(15_000, || tab.state.borrow().title == "Cookie set" && !tab.state.borrow().loading).await;
    };
    let mut all_open = true;
    for id in ids {
        all_open &= open(id).await;
    }
    r.check("four workspaces (two shared, two isolated) open", all_open, || "not all open".into());
    set("sh1").await;
    let a = show("sh2").await;
    r.check("shared workspaces see each other's cookies", a.as_deref() == Some("Cookies [ck=1]"), || format!("{a:?}"));
    let b = show("is1").await;
    r.check("an isolated workspace does not see the profile's cookies", b.as_deref() == Some("Cookies []"), || format!("{b:?}"));
    set("is1").await;
    let c = show("is1").await;
    r.check("an isolated workspace keeps its own cookies", c.as_deref() == Some("Cookies [ck=1]"), || format!("{c:?}"));
    let d = show("is2").await;
    r.check("two isolated workspaces do not see each other's cookies", d.as_deref() == Some("Cookies []"), || format!("{d:?}"));
    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    for id in ids {
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// `session` cookies work while the window is open and are gone when it has been closed and reopened.
async fn run_session_cookies(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let ids = ["se1", "ke1"];
    for id in ids {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let action = if id == "se1" { "session" } else { "allow" };
        std::fs::write(dir.join("workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\nstorage: isolated\ncookies:\n  rules:\n    - match: ['localhost']\n      action: {action}\n")).expect("ws");
    }
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    for (id, kept) in [("se1", false), ("ke1", true)] {
        ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
        let ok = wait_until(15_000, || find(id).is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
        let Some(ws) = find(id) else { r.check("a session-cookie workspace opens", ok, || "not open".into()); continue };
        let tab = ws.current().expect("tab");
        tab.view().navigate(&format!("http://localhost:{port}/cookie-set"));
        wait_until(15_000, || tab.state.borrow().title == "Cookie set" && !tab.state.borrow().loading).await;
        tab.view().navigate(&format!("http://localhost:{port}/cookie-show"));
        wait_until(15_000, || tab.state.borrow().title.starts_with("Cookies [") && !tab.state.borrow().loading).await;
        let during = tab.state.borrow().title.clone();
        r.check("a session cookie works while the window is open", during == "Cookies [ck=1]", || during.clone());
        ws.window.close();
        let _ = wait_until(5_000, || find(id).is_none()).await;
        glib::timeout_future(Duration::from_millis(1000)).await;
        ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
        wait_until(15_000, || find(id).is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
        let Some(ws) = find(id) else { continue };
        let tab = ws.current().expect("tab");
        tab.view().navigate(&format!("http://localhost:{port}/cookie-show"));
        wait_until(15_000, || tab.state.borrow().title.starts_with("Cookies [") && !tab.state.borrow().loading).await;
        let after = tab.state.borrow().title.clone();
        let want = if kept { "Cookies [ck=1]" } else { "Cookies []" };
        r.check(if kept { "an allowed cookie is still there after the window is reopened" } else { "a session cookie is gone after the window is closed and reopened" }, after == want, || after.clone());
    }
    // A cookie an earlier run (here: the `ke1` rules that allowed it) left behind is never sent once its site is a
    // `session` one, even though nothing has deleted it yet.
    if let Some(w) = find("ke1") {
        w.window.close();
        let _ = wait_until(5_000, || find("ke1").is_none()).await;
    }
    let dir = profile.workspaces_dir().join("ke1");
    std::fs::write(dir.join("workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\nstorage: isolated\ncookies:\n  rules:\n    - match: ['localhost']\n      action: session\n")).expect("ws");
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"ke1".to_variant()));
    wait_until(15_000, || find("ke1").is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    if let Some(tab) = find("ke1").and_then(|s| s.current()) {
        tab.view().navigate(&format!("http://localhost:{port}/cookie-show"));
        wait_until(15_000, || tab.state.borrow().title.starts_with("Cookies [") && !tab.state.borrow().loading).await;
        let t = tab.state.borrow().title.clone();
        r.check("a cookie from an earlier run is not sent once its site is a session one", t == "Cookies []", || t.clone());
    }
    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    for id in ids {
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// Session cookies belong to the workspace that saved them (even in the profile's shared store), and blocked
/// cookie sites are listed so allow rules can be copied.
async fn run_session_scope(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let ids = ["sa1", "sb1", "bl1"];
    for id in ids {
        let dir = profile.workspaces_dir().join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let rules = if id == "bl1" { "default: deny" } else { "rules:\n    - match: ['localhost']\n      action: session" };
        std::fs::write(dir.join("workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\ncookies:\n  {rules}\n")).expect("ws");
    }
    let find = |id: &str| super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id));
    let open = |id: &'static str| {
        let app = app.clone();
        async move {
            ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
            wait_until(15_000, || find(id).is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await
        }
    };
    let visit = |id: &'static str, path: &'static str, title: &'static str| async move {
        let tab = find(id)?.current()?;
        tab.view().navigate(&format!("http://localhost:{port}/{path}"));
        wait_until(15_000, || tab.state.borrow().title.starts_with(title) && !tab.state.borrow().loading).await;
        let t = tab.state.borrow().title.clone();
        Some(t)
    };
    open("sa1").await;
    visit("sa1", "cookie-set", "Cookie set").await;
    let a = visit("sa1", "cookie-show", "Cookies [").await;
    r.check("a session cookie works in the workspace that saved it", a.as_deref() == Some("Cookies [ck=1]"), || format!("{a:?}"));
    open("sb1").await;
    let b = visit("sb1", "cookie-show", "Cookies [").await;
    r.check("another workspace in the same store is not sent that session cookie", b.as_deref() == Some("Cookies []"), || format!("{b:?}"));
    if let Some(w) = find("sa1") {
        w.window.close();
    }
    let _ = wait_until(5_000, || find("sa1").is_none()).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    open("sa1").await;
    let c = visit("sa1", "cookie-show", "Cookies [").await;
    r.check("closing the workspace deletes its session cookie from the shared store", c.as_deref() == Some("Cookies []"), || format!("{c:?}"));

    open("bl1").await;
    visit("bl1", "cookie-set", "Cookie set").await;
    let sites = find("bl1").and_then(|s| s.current()).map(|t| t.view().blocked_cookie_sites()).unwrap_or_default();
    r.check("cookies blocked by the rules are listed by site", sites == vec!["localhost".to_owned()], || format!("{sites:?}"));
    visit("bl1", "b.html", "Page B").await;
    let sites = find("bl1").and_then(|s| s.current()).map(|t| t.view().blocked_cookie_sites()).unwrap_or_default();
    r.check("the list starts again with the next page", sites.is_empty(), || format!("{sites:?}"));

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    for id in ids {
        let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(id));
        let _ = std::fs::remove_dir_all(profile.workspaces_dir().join(id));
    }
}

/// A web app: a workspace window with only its page, no new tabs and popups as modals; it is left out of the
/// tab menu's list of workspaces.
async fn run_webapp(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let dir = profile.workspaces_dir().join("wa1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("wa1"));
    std::fs::write(dir.join("workspace.yaml"), format!("name: Web app\napp:\n  url: 'http://localhost:{port}/scope.html'\n  display: standalone\n")).expect("ws");
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "wa1"));
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"wa1".to_variant()));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Scope test" && !t.state.borrow().loading))).await;
    r.check("a web app opens on its start page", ok, || format!("{:?}", find().map(|s| s.current().map(|t| t.state.borrow().title.clone()))));
    let Some(w) = find() else { return };
    glib::timeout_future(Duration::from_millis(600)).await;
    if let Some(w) = find() {
        shot(&w.window, "web-app-window");
    }
    r.check("a web app shows only its page: one pinned tab, no address bar, no new tabs", w.tab_count() == 1 && w.current().is_some_and(|t| t.page.is_pinned()) && gtk::prelude::WidgetExt::parent(&w.entry).is_none() && !w.allow_new_tabs, || format!("tabs={} entry={}", w.tab_count(), gtk::prelude::WidgetExt::is_visible(&w.entry)));
    r.check("a web app keeps back and reload, and shows popups as modals", w.back.is_visible() && w.popup_policy == crate::workspace::PopupPolicy::Modal, || "controls or popup policy".into());
    // Out of scope: the default scope is the start page's own site, so the link to 127.0.0.1 leaves the app.
    let before = super::windows().len();
    if let Some(t) = w.current() {
        t.view().navigate(&format!("http://127.0.0.1:{port}/b.html"));
    }
    let ok = wait_until(15_000, || super::windows().len() > before).await;
    r.check("a link outside the app's site opens in a window of its own", ok, || format!("{} windows (was {before})", super::windows().len()));
    for x in super::windows() {
        if !Rc::ptr_eq(&x, shell) {
            x.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("wa1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Installing a page as a web app: its manifest is read, and the app, with its icon, is created.
async fn run_install(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let st = shell.state();
    let before = st.borrow().load_cycles;
    let view = shell.view();
    view.navigate(&format!("http://localhost:{port}/pwa.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before && s.title == "PWA page" }).await;
    glib::timeout_future(Duration::from_millis(500)).await;
    let info = super::install::read_manifest(&view).await;
    let ok = info.as_ref().is_some_and(|i| i.name == "Test Manifest App" && i.start_url.ends_with("/pwa.html") && i.display == crate::workspace::AppDisplay::Minimal && i.color.as_deref() == Some("#336699") && i.scope == ["localhost"] && i.icons.len() == 1);
    r.check("the page's web manifest is read through DevTools", ok, || format!("{info:?}"));
    let Some(info) = info else { return };
    {
        let dialog = shell.confirm_install(view.clone(), info.clone());
        glib::timeout_future(Duration::from_millis(600)).await;
        shot(&shell.window, "install-dialog");
        dialog.force_close();
    }
    let id = super::install::save_app(&view, &info, "Test Manifest App").await;
    r.check("installing creates the web app's workspace", id.as_deref() == Ok("test-manifest-app"), || format!("{id:?}"));
    let Ok(id) = id else { return };
    let dir = profile.workspaces_dir().join(&id);
    let icon = std::fs::read(dir.join("icon.png")).unwrap_or_default();
    r.check("its icon was downloaded as a PNG", icon.starts_with(&[0x89, b'P', b'N', b'G']), || format!("{} bytes", icon.len()));
    let ws = crate::workspace::Workspace::resolve(&profile, &id);
    r.check("the saved configuration is the manifest's, with the icon", ws.as_ref().is_ok_and(|w| w.config.app.as_ref().is_some_and(|a| a.icon.as_deref() == Some("icon.png") && a.scope == ["localhost"]) && w.config.name.as_deref() == Some("Test Manifest App")), || format!("{ws:?}"));
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "test-manifest-app"));
    let ok = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "PWA page" && !t.state.borrow().loading))).await;
    r.check("the installed app opens on its start page", ok, || "not open".into());
    for x in super::windows() {
        if !Rc::ptr_eq(&x, shell) {
            x.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace(&id));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_section_list_matches_the_sections_that_run() {
        let src = include_str!("selftest.rs");
        let mut run: Vec<&str> = src.lines().filter_map(|l| l.trim().strip_prefix("if wants(\"").and_then(|r| r.split('"').next())).collect();
        run.insert(0, "basic"); // a labelled block rather than a call
        run.dedup();
        let mut listed: Vec<&str> = super::SECTIONS.to_vec();
        run.sort_unstable();
        listed.sort_unstable();
        assert_eq!(run, listed, "SECTIONS must name exactly the sections `run` checks with wants()");
    }
}

/// Address-bar suggestions: typing offers matching history, the arrow keys choose, Enter opens the choice.
async fn run_suggest(shell: &Rc<Shell>, r: &mut Report) {
    let Some(st) = super::store() else {
        r.check("the profile database is open", false, || "no store".into());
        return;
    };
    let port = start_test_server();
    let url = format!("http://localhost:{port}/b.html");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let _ = st.clear_history(None); // the test profile's own database: start from nothing
    let _ = st.record_visit(None, &url, "Page B suggestion", now);
    let _ = st.record_visit(None, "http://other.invalid/", "Something else", now);
    shell.window.present();
    glib::timeout_future(Duration::from_millis(500)).await;
    shell.entry.grab_focus();
    glib::timeout_future(Duration::from_millis(200)).await;
    shell.entry.set_text("page b sugg");
    let ok = wait_until(5_000, || shell.suggestion_urls().contains(&url)).await;
    let offered = shell.suggestion_urls();
    r.check("typing offers a matching history entry", ok && !offered.iter().any(|u| u.contains("other.invalid")), || format!("{offered:?}"));
    r.check("nothing is highlighted until an arrow key is used", shell.chosen_suggestion().is_none(), || format!("{:?}", shell.chosen_suggestion()));
    let _ = shell.move_suggestion(1);
    glib::timeout_future(Duration::from_millis(300)).await;
    shot(shell.suggestion_popover(), "suggestions");
    let _ = shell.move_suggestion(-1);
    let moved = shell.move_suggestion(1);
    r.check("the down arrow highlights the first suggestion", moved && shell.chosen_suggestion().as_deref() == Some(url.as_str()), || format!("{:?}", shell.chosen_suggestion()));
    let moved = shell.move_suggestion(-1);
    r.check("the up arrow returns to what was typed", moved && shell.chosen_suggestion().is_none(), || format!("{:?}", shell.chosen_suggestion()));
    let _ = shell.move_suggestion(1);
    let before = shell.state().borrow().load_cycles;
    shell.entry.emit_by_name::<()>("activate", &[]);
    let ok = wait_until(15_000, || { let s = shell.state(); let s = s.borrow(); !s.loading && s.load_cycles > before && s.address == url }).await;
    r.check("Enter opens the highlighted suggestion", ok, || format!("{:?}", shell.state().borrow()));
    r.check("the suggestions are gone afterwards", shell.suggestion_urls().is_empty(), || format!("{:?}", shell.suggestion_urls()));
    // Nothing is offered when the address bar is not being edited by a person.
    shell.entry.set_text("");
    glib::timeout_future(Duration::from_millis(200)).await;
    r.check("an empty address bar offers nothing", shell.suggestion_urls().is_empty(), || String::new());
    let _ = st.clear_history(None);
}

/// Dialogs a page opens with script: held for the test (as the permission questions are), answered, and the page
/// goes on with the answer.
async fn run_jsdialog(shell: &Rc<Shell>, r: &mut Report) {
    let port = start_test_server();
    let st = shell.state();
    let before = st.borrow().load_cycles;
    let view = shell.view();
    view.navigate(&format!("http://localhost:{port}/b.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before && s.title == "Page B" }).await;
    let run = |script: &str| {
        view.cdp_eval(&format!("setTimeout(() => {{ {script} }}, 0); 1"));
    };
    let title_is = |t: &'static str| {
        let st = st.clone();
        async move { wait_until(10_000, move || st.borrow().title == t).await }
    };

    run("document.title = 'got:' + prompt('name?', 'def');");
    let ok = wait_until(10_000, || shell.pending_js_dialog().is_some()).await;
    let q = shell.pending_js_dialog();
    r.check("a prompt is held with its message and default text", ok && q.as_ref().is_some_and(|q| q.kind == super::view::JsDialogKind::Prompt && q.message == "name?" && q.default_text == "def" && q.origin.contains("localhost")), || format!("{q:?}"));
    shell.answer_pending_js_dialog(true, "Ann");
    r.check("the page receives what was typed", title_is("got:Ann").await, || st.borrow().title.clone());

    run("document.title = 'c:' + confirm('sure?');");
    let _ = wait_until(10_000, || shell.pending_js_dialog().is_some()).await;
    r.check("a confirm is held", shell.pending_js_dialog().is_some_and(|q| q.kind == super::view::JsDialogKind::Confirm), || format!("{:?}", shell.pending_js_dialog()));
    shell.answer_pending_js_dialog(false, "");
    r.check("cancelling a confirm gives the page false", title_is("c:false").await, || st.borrow().title.clone());
    run("document.title = 'd:' + confirm('again?');");
    let _ = wait_until(10_000, || shell.pending_js_dialog().is_some()).await;
    shell.answer_pending_js_dialog(true, "");
    r.check("accepting a confirm gives the page true", title_is("d:true").await, || st.borrow().title.clone());

    run("alert('hello'); document.title = 'after alert';");
    let _ = wait_until(10_000, || shell.pending_js_dialog().is_some()).await;
    let held = shell.pending_js_dialog().is_some_and(|q| q.kind == super::view::JsDialogKind::Alert && q.message == "hello");
    r.check("an alert is held and the page waits for it", held && st.borrow().title != "after alert", || format!("{:?}", st.borrow().title));
    shell.answer_pending_js_dialog(true, "");
    r.check("the page continues after the alert", title_is("after alert").await, || st.borrow().title.clone());

    // The real dialog, for a picture: shown instead of held.
    super::prompts::set_ui_test(false);
    run("document.title = 'typed:' + prompt('What should we call you?', 'Ann');");
    let shown = wait_until(10_000, || shell.js_showing.borrow().is_some()).await;
    glib::timeout_future(Duration::from_millis(600)).await;
    shot(&shell.window, "js-prompt");
    r.check("a prompt is shown as a dialog over the window when not under test", shown, || "no dialog".into());
    let dialog = shell.js_showing.borrow().as_ref().map(|(_, d)| d.clone());
    if let Some(d) = dialog {
        d.force_close(); // closing with Esc cancels the prompt
    }
    super::prompts::set_ui_test(true);
    r.check("closing the dialog answers the page (a cancelled prompt gives null)", title_is("typed:null").await, || st.borrow().title.clone());
}

/// Page zoom: the keyboard actions change the page's layout width, the zoom is remembered per site and put back,
/// and Ctrl with the wheel zooms too.
async fn run_zoom(shell: &Rc<Shell>, r: &mut Report) {
    let port = start_test_server();
    let st = shell.state();
    let go = |url: String| {
        let st = st.clone();
        let view = shell.view();
        async move {
            let before = st.borrow().load_cycles;
            view.navigate(&url);
            wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before }).await;
            glib::timeout_future(Duration::from_millis(300)).await;
        }
    };
    let width = || async { eval_string(&shell.current().expect("tab"), "String(window.innerWidth)").await.and_then(|w| w.parse::<f64>().ok()).unwrap_or(0.0) };
    if let Some(s) = super::store() {
        let _ = s.set_site_zoom("localhost", 1.0);
        let _ = s.set_site_zoom("127.0.0.1", 1.0);
    }
    go(format!("http://localhost:{port}/b.html")).await;
    let w100 = width().await;
    let _ = WidgetExt::activate_action(&shell.window, "win.zoom-in", None);
    let _ = WidgetExt::activate_action(&shell.window, "win.zoom-in", None);
    glib::timeout_future(Duration::from_millis(400)).await;
    let w125 = width().await;
    r.check("two zoom-in steps make the page 125 % (its layout width shrinks to 80 %)", (shell.view().user_zoom() - 1.25).abs() < 0.001 && w100 > 0.0 && ((w125 / w100) - 0.8).abs() < 0.03, || format!("{w100} -> {w125}, zoom {}", shell.view().user_zoom()));
    r.check("the zoom is remembered for the site", super::store().and_then(|s| s.site_zoom("localhost")) == Some(1.25), || String::new());
    go(format!("http://127.0.0.1:{port}/b.html")).await;
    r.check("another site is not zoomed", (shell.view().user_zoom() - 1.0).abs() < 0.001 && (width().await - w100).abs() < 2.0, || format!("{}", shell.view().user_zoom()));
    go(format!("http://localhost:{port}/b.html?again")).await;
    r.check("coming back to the site puts its zoom back", (shell.view().user_zoom() - 1.25).abs() < 0.001 && ((width().await / w100) - 0.8).abs() < 0.03, || format!("{}", shell.view().user_zoom()));
    let _ = WidgetExt::activate_action(&shell.window, "win.zoom-out", None);
    r.check("zoom out steps back down", (shell.view().user_zoom() - 1.1).abs() < 0.001, || format!("{}", shell.view().user_zoom()));
    let _ = WidgetExt::activate_action(&shell.window, "win.zoom-reset", None);
    glib::timeout_future(Duration::from_millis(300)).await;
    r.check("reset returns to 100 % and forgets the site's zoom", (shell.view().user_zoom() - 1.0).abs() < 0.001 && super::store().and_then(|s| s.site_zoom("localhost")).is_none() && (width().await - w100).abs() < 2.0, || format!("{}", shell.view().user_zoom()));
}

/// Find in page: the bar opens, typing counts the matches, the buttons move between them, closing clears.
async fn run_find(shell: &Rc<Shell>, r: &mut Report) {
    let port = start_test_server();
    let st = shell.state();
    let before = st.borrow().load_cycles;
    shell.view().navigate(&format!("http://localhost:{port}/find.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before && s.title == "Find test" }).await;
    glib::timeout_future(Duration::from_millis(500)).await;
    let _ = WidgetExt::activate_action(&shell.window, "win.find", None);
    glib::timeout_future(Duration::from_millis(300)).await;
    r.check("Ctrl+F opens the find bar", shell.find.is_open(), || "closed".into());
    shell.find.entry.set_text("needle");
    let ok = wait_until(10_000, || shell.find_label() == "1/3").await;
    r.check("typing finds every match, ignoring case, and starts on the first", ok, || shell.find_label());
    let _ = WidgetExt::activate_action(&shell.window, "win.find-next", None);
    let ok = wait_until(10_000, || shell.find_label() == "2/3").await;
    glib::timeout_future(Duration::from_millis(500)).await;
    shot(&shell.window, "find-bar");
    r.check("next moves to the second match", ok, || shell.find_label());
    let _ = WidgetExt::activate_action(&shell.window, "win.find-prev", None);
    let ok = wait_until(10_000, || shell.find_label() == "1/3").await;
    r.check("previous moves back", ok, || shell.find_label());
    shell.find.entry.set_text("zebra");
    let ok = wait_until(10_000, || shell.find_label() == "No matches").await;
    r.check("a word that is not there says so", ok, || shell.find_label());
    let _ = WidgetExt::activate_action(&shell.window, "win.find-close", None);
    glib::timeout_future(Duration::from_millis(300)).await;
    r.check("closing hides the bar and clears the label", !shell.find.is_open() && shell.find_label().is_empty() && shell.view().find_result() == (0, 0), || shell.find_label());
}

/// Fullscreen: F11 fills the screen and hides the bars; a page's own request (an element's fullscreen) does too
/// and ends when the page leaves it.
async fn run_fullscreen(shell: &Rc<Shell>, r: &mut Report) {
    let port = start_test_server();
    let st = shell.state();
    let before = st.borrow().load_cycles;
    shell.view().navigate(&format!("http://localhost:{port}/b.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before && s.title == "Page B" }).await;
    glib::timeout_future(Duration::from_millis(300)).await;
    r.check("a window starts out not fullscreen, with its bars", !shell.is_fullscreen() && shell.bars_shown(), || String::new());
    let _ = WidgetExt::activate_action(&shell.window, "win.fullscreen", None);
    r.check("F11 fills the screen and hides the bars", shell.is_fullscreen() && !shell.bars_shown(), || format!("{} {}", shell.is_fullscreen(), shell.bars_shown()));
    let _ = WidgetExt::activate_action(&shell.window, "win.fullscreen", None);
    r.check("F11 again restores them", !shell.is_fullscreen() && shell.bars_shown(), || String::new());

    shell.view().cdp_eval_as("document.documentElement.requestFullscreen()", true);
    let ok = wait_until(10_000, || shell.is_fullscreen()).await;
    r.check("a page's fullscreen request fills the screen and hides the bars", ok && !shell.bars_shown(), || format!("{} {}", shell.is_fullscreen(), shell.bars_shown()));
    shell.view().cdp_eval("document.exitFullscreen()");
    let ok = wait_until(10_000, || !shell.is_fullscreen()).await;
    r.check("when the page leaves fullscreen the window does too", ok && shell.bars_shown(), || format!("{} {}", shell.is_fullscreen(), shell.bars_shown()));

    shell.view().cdp_eval_as("document.documentElement.requestFullscreen()", true);
    let _ = wait_until(10_000, || shell.is_fullscreen()).await;
    let _ = WidgetExt::activate_action(&shell.window, "win.fullscreen", None);
    let mut page_left = false;
    for _ in 0..30 {
        glib::timeout_future(Duration::from_millis(200)).await;
        let id = shell.view().cdp_eval("String(!!document.fullscreenElement)");
        let _ = wait_until(5_000, || shell.view().cdp_result(id).is_some()).await;
        if shell.view().cdp_result(id).is_some_and(|(_, j)| j.contains("false")) {
            page_left = true;
            break;
        }
    }
    r.check("leaving with F11 also makes the page leave its fullscreen", !shell.is_fullscreen() && page_left, || format!("{} {page_left}", shell.is_fullscreen()));
}

/// Developer tools: the action opens them for the page and toggles them closed.
async fn run_devtools(shell: &Rc<Shell>, r: &mut Report) {
    let view = shell.view();
    r.check("developer tools start closed", !view.has_dev_tools(), || String::new());
    let _ = WidgetExt::activate_action(&shell.window, "win.devtools", None);
    let ok = wait_until(10_000, || view.has_dev_tools()).await;
    r.check("F12 opens the developer tools", ok, || "not open".into());
    // (BROWSAH_DEVTOOLS_HOLD_MS keeps the tools open longer, to inspect them from outside.)
    let hold = std::env::var("BROWSAH_DEVTOOLS_HOLD_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
    glib::timeout_future(Duration::from_millis(hold)).await;
    let _ = WidgetExt::activate_action(&shell.window, "win.devtools", None);
    let ok = wait_until(10_000, || !view.has_dev_tools()).await;
    r.check("F12 again closes them", ok, || "still open".into());
}

/// Saving a page as a PDF: the file is written, is a PDF, and is on A4 paper by default.
async fn run_print(shell: &Rc<Shell>, r: &mut Report) {
    let port = start_test_server();
    let st = shell.state();
    let before = st.borrow().load_cycles;
    shell.view().navigate(&format!("http://localhost:{port}/find.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before && s.title == "Find test" }).await;
    glib::timeout_future(Duration::from_millis(300)).await;
    let path = std::env::temp_dir().join(format!("browsah-selftest-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    shell.write_pdf(&path, super::print::PdfJob::Save);
    let ok = wait_until(15_000, || std::fs::metadata(&path).map(|m| m.len() > 500).unwrap_or(false)).await;
    let bytes = std::fs::read(&path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    r.check("the page is written as a PDF file", ok && bytes.starts_with(b"%PDF"), || format!("{} bytes", bytes.len()));
    r.check("the paper is A4 by default", text.contains("/MediaBox [ 0 0 595") || text.contains("/MediaBox [0 0 595"), || text.lines().find(|l| l.contains("MediaBox")).unwrap_or("no MediaBox").to_owned());
    let done = wait_until(5_000, || shell.pdf_jobs_pending() == 0).await;
    r.check("the job is finished when the file is reported", done, || format!("{} pending", shell.pdf_jobs_pending()));
    let _ = std::fs::remove_file(&path);
}

/// Tab groups: making one, putting tabs in it (they move next to each other), collapsing to one tab and opening again,
/// taking tabs out, closing a group, and remembering groups (collapsed or not) with the workspace's tabs.
async fn run_groups(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let dir = profile.workspaces_dir().join("gr1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("gr1"));
    std::fs::write(dir.join("workspace.yaml"), format!("home_page: 'http://localhost:{port}/b.html'\n")).expect("ws");
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "gr1"));
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"gr1".to_variant()));
    let _ = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "Page B" && !t.state.borrow().loading))).await;
    let Some(ws) = find() else { return };
    let t1 = ws.current().expect("tab");
    let t2 = ws.open_tab(&format!("http://localhost:{port}/b.html?2"), false);
    let t3 = ws.open_tab(&format!("http://localhost:{port}/b.html?3"), false);
    let t4 = ws.open_tab(&format!("http://localhost:{port}/b.html?4"), false);
    glib::timeout_future(Duration::from_millis(800)).await;
    let pos = |t: &Rc<super::Tab>| ws.tab_view.page_position(&t.page);

    let gid = ws.create_group("News", None);
    ws.assign_group(&t2, gid);
    ws.assign_group(&t4, gid);
    r.check("tabs put in a group sit next to each other", (pos(&t2) - pos(&t4)).abs() == 1 && pos(&t3) > pos(&t4).max(pos(&t2)), || format!("{} {} {} {}", pos(&t1), pos(&t2), pos(&t3), pos(&t4)));
    r.check("a grouped tab shows the group's colour as its icon and its name as tooltip", t2.page.icon().is_some() && t2.page.tooltip().is_some_and(|t| t.contains("News")) && t1.page.icon().is_none(), || format!("{:?}", t2.page.tooltip()));
    ws.rename_group(gid, "Daily news");
    ws.set_group_color(gid, "#e01b24");
    r.check("renaming and recolouring a group updates its tabs", t4.page.tooltip().is_some_and(|t| t.contains("Daily news")) && ws.group(gid).is_some_and(|g| g.color == "#e01b24"), || format!("{:?}", t4.page.tooltip()));

    glib::timeout_future(Duration::from_millis(600)).await;
    shot(&ws.window, "groups-expanded");
    let pages_before = ws.tab_view.n_pages();
    ws.tab_view.set_selected_page(&t3.page);
    ws.collapse_group(gid);
    r.check("collapsing replaces the group's tabs by one stand-in tab", ws.tab_view.n_pages() == pages_before - 1 && ws.group(gid).is_some_and(|g| g.collapsed) && t2.parked.get() && t4.parked.get(), || format!("{} pages (was {pages_before})", ws.tab_view.n_pages()));
    let title = (0..ws.tab_view.n_pages()).map(|i| ws.tab_view.nth_page(i).title().to_string()).find(|t| t.contains("Daily news")).unwrap_or_default();
    r.check("the stand-in says which group it is and how many tabs it holds", title.contains("(2)"), || title.clone());
    r.check("the shown tab is a real one, not the stand-in", ws.selected_placeholder().is_none() && ws.current().is_some(), || String::new());
    glib::timeout_future(Duration::from_millis(600)).await;
    shot(&ws.window, "groups-collapsed");
    let saved = ws.tab_records();
    ws.save_now();
    let groups = super::store().map(|s| s.load_groups("gr1")).unwrap_or_default();
    r.check("a collapsed group's tabs are still remembered, with the group", saved.len() == 4 && saved.iter().filter(|t| t.group == gid).count() == 2 && groups.len() == 1 && groups[0].collapsed && groups[0].name == "Daily news", || format!("{} records, {groups:?}", saved.len()));
    ws.expand_group(gid);
    r.check("expanding puts the tabs back, together, and removes the stand-in", ws.tab_view.n_pages() == pages_before && !t2.parked.get() && (pos(&t2) - pos(&t4)).abs() == 1 && ws.selected_placeholder().is_none(), || format!("{} pages", ws.tab_view.n_pages()));

    // Collapsed when the window closes: it comes back collapsed, with its tabs asleep behind it.
    ws.collapse_group(gid);
    ws.window.close();
    let _ = wait_until(5_000, || find().is_none()).await;
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"gr1".to_variant()));
    let _ = wait_until(15_000, || find().is_some_and(|s| s.tab_count() >= 4)).await;
    let Some(ws) = find() else { return };
    let stand_in = (0..ws.tab_view.n_pages()).any(|i| ws.tab_view.nth_page(i).title().contains("Daily news"));
    r.check("a collapsed group comes back collapsed after a restart", stand_in && ws.group(gid).is_some_and(|g| g.collapsed && g.color == "#e01b24") && ws.tab_count() == 4, || format!("{} tabs, {:?}", ws.tab_count(), ws.groups()));
    ws.expand_group(gid);
    let members = ws.group_tabs(gid);
    r.check("its tabs are the group's again after expanding", members.len() == 2 && members.iter().all(|t| t.page.icon().is_some()), || format!("{}", members.len()));

    // Taking tabs out; an empty group disappears.
    let (a, b) = (members[0].clone(), members[1].clone());
    ws.remove_from_group(&a);
    r.check("a tab taken out of a group is plain again", a.group.get() == 0 && a.page.icon().is_none() && ws.group(gid).is_some(), || String::new());
    ws.remove_from_group(&b);
    r.check("a group with no tabs left is forgotten", ws.group(gid).is_none(), || format!("{:?}", ws.groups()));

    // Closing a group closes its tabs.
    let g2 = ws.create_group("Temp", None);
    let tabs = ws.tabs();
    ws.assign_group(&tabs[1], g2);
    ws.assign_group(&tabs[2], g2);
    let before = ws.tab_count();
    ws.close_group(g2);
    let ok = wait_until(5_000, || ws.tab_count() == before - 2).await;
    r.check("closing a group closes its tabs", ok && ws.group(g2).is_none(), || format!("{} tabs (was {before})", ws.tab_count()));

    for w in super::windows() {
        if !Rc::ptr_eq(&w, shell) {
            w.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("gr1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The main menu: it lists a window with no workspace, the profile's workspaces and the page tools.
async fn run_menu(shell: &Rc<Shell>, r: &mut Report) {
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let dir = profile.workspaces_dir().join("mn1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    std::fs::write(dir.join("workspace.yaml"), "name: Menu test\n").expect("ws");
    shell.menu_button.popup();
    glib::timeout_future(Duration::from_millis(600)).await;
    let model = shell.menu_button.menu_model();
    let mut labels = Vec::new();
    if let Some(m) = &model {
        for i in 0..m.n_items() {
            if let Some(section) = m.item_link(i, "section") {
                for j in 0..section.n_items() {
                    labels.extend(section.item_attribute_value(j, "label", None).and_then(|v| v.str().map(str::to_owned)));
                }
            }
        }
    }
    shot(&shell.window, "main-menu");
    if let Some(p) = shell.menu_button.popover() {
        shot(&p, "main-menu-popover");
    }
    shell.menu_button.popdown();
    r.check("the main menu offers a new window (no workspace) and the page tools", ["New window", "Open workspace", "Find in page", "Developer tools", "History"].iter().all(|l| labels.iter().any(|x| x == l)), || format!("{labels:?}"));
    let sub_has_workspace = model.as_ref().is_some_and(|m| {
        (0..m.n_items()).filter_map(|i| m.item_link(i, "section")).any(|s| (0..s.n_items()).filter_map(|j| s.item_link(j, "submenu")).any(|sub| (0..sub.n_items()).any(|k| sub.item_attribute_value(k, "label", None).and_then(|v| v.str().map(str::to_owned)).as_deref() == Some("Menu test"))))
    });
    r.check("it lists the profile's workspaces to open", sub_has_workspace, || format!("{labels:?}"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// HiDPI: a page's device pixel ratio, frame size and scrollbar thickness follow the display's scale. Run with
/// `BROWSAH_FORCE_SCALE=2` to test a scale-2 display on a 1x screen.
async fn run_hidpi(shell: &Rc<Shell>, r: &mut Report) {
    let port = start_test_server();
    let st = shell.state();
    let before = st.borrow().load_cycles;
    shell.view().navigate(&format!("http://localhost:{port}/tall.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before && s.title == "Tall" }).await;
    glib::timeout_future(Duration::from_millis(1500)).await;
    let tab = shell.current().expect("tab");
    let num = |js: &'static str| {
        let tab = tab.clone();
        async move { eval_string(&tab, js).await.and_then(|v| v.parse::<f64>().ok()).unwrap_or(-1.0) }
    };
    let dpr = num("String(window.devicePixelRatio)").await;
    let bar_css = num("String(window.innerWidth - document.documentElement.clientWidth)").await;
    let scale = std::env::var("BROWSAH_FORCE_SCALE").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| shell.window.native().and_then(|n| n.surface()).map(|s| s.scale()).unwrap_or(1.0));
    println!("hidpi: scale {scale}, devicePixelRatio {dpr}, scrollbar {bar_css} CSS px, so {} device px (a logical 15 px bar is {} device px)", bar_css * dpr, 15.0 * scale);
    let sw = num("String(screen.width)").await;
    let sh = num("String(screen.height)").await;
    println!("hidpi: page sees screen {sw}x{sh}, depth {}", num("String(screen.colorDepth)").await);
    let iw = num("String(window.innerWidth)").await;
    let ih = num("String(window.innerHeight)").await;
    let fs = num("String(parseFloat(getComputedStyle(document.body).fontSize))").await;
    let (pw, ph) = shell.view().widget_size();
    let frame = shell.view().frame_size();
    println!("hidpi: view {pw}x{ph} logical; page innerWidth x innerHeight {iw}x{ih} CSS px; body font {fs} CSS px; frame {frame:?} px (expected about {}x{} px)", (pw as f64 * scale).round(), (ph as f64 * scale).round());
    r.check("the page's layout width is the window's logical width", (iw - pw as f64).abs() < 20.0, || format!("innerWidth {iw}, view {pw}"));
    r.check("frames are rendered at device resolution (not blurry)", frame.is_some_and(|(w, h)| (w as f64 - pw as f64 * scale).abs() < 4.0 && (h as f64 - ph as f64 * scale).abs() < 4.0), || format!("{frame:?} vs {}x{}", pw as f64 * scale, ph as f64 * scale));
    r.check("the page's devicePixelRatio is the display scale", (dpr - scale).abs() < 0.05, || format!("{dpr} vs {scale}"));
    r.check("the scrollbar is as thick in device pixels as on a 1x screen scaled up (15 logical px)", (bar_css * dpr - 15.0 * scale).abs() < 1.5, || format!("{} device px, expected {}", bar_css * dpr, 15.0 * scale));
    shot(&shell.window, "hidpi-tall-page");
    // The page frame exactly as CEF delivered it (device pixels), to look at the sharpness of text.
    if let (Some(dir), Some(tex)) = (std::env::var_os("BROWSAH_SHOTS"), shell.view().picture_texture()) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = tex.save_to_png(std::path::Path::new(&dir).join("hidpi-frame.png"));
        // A magnified (4x, nearest neighbour) crop of the top-left corner, where the text is.
        let mut dl = gtk::gdk::TextureDownloader::new(&tex);
        dl.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = dl.download_bytes();
        let (cw, ch, z) = (220usize, 50usize, 4usize);
        let mut out = vec![0u8; cw * z * ch * z * 4];
        for y in 0..ch * z {
            for x in 0..cw * z {
                let src = (y / z) * stride + (x / z) * 4;
                let dst = (y * cw * z + x) * 4;
                out[dst..dst + 4].copy_from_slice(&bytes[src..src + 4]);
            }
        }
        let crop = gtk::gdk::MemoryTexture::new((cw * z) as i32, (ch * z) as i32, gtk::gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(out), cw * z * 4);
        let _ = crop.save_to_png(std::path::Path::new(&dir).join("hidpi-crop.png"));
    }
    // Another site (another process): the scale is still right.
    let before = st.borrow().load_cycles;
    shell.view().navigate(&format!("http://127.0.0.1:{port}/tall.html"));
    let _ = wait_until(15_000, || { let s = st.borrow(); !s.loading && s.load_cycles > before }).await;
    glib::timeout_future(Duration::from_millis(1500)).await;
    let dpr2 = num("String(window.devicePixelRatio)").await;
    let bar2 = num("String(window.innerWidth - document.documentElement.clientWidth)").await;
    r.check("another site gets the same scale and scrollbar", (dpr2 - scale).abs() < 0.05 && (bar2 * dpr2 - 15.0 * scale).abs() < 1.5, || format!("dpr {dpr2}, bar {bar2} CSS px"));
}

/// A coloured window shows its colour around the page (sides and bottom, with rounded page corners), maximized or not; in
/// fullscreen the page fills the screen.
async fn run_frame(shell: &Rc<Shell>, r: &mut Report) {
    use gtk::prelude::ActionGroupExt;
    let app = shell.window.application().expect("application");
    let profile = crate::profile::Profile::resolve(crate::profile::TEST_ID).expect("selftest profile");
    let db = profile.dir.join("data.sqlite");
    let port = start_test_server();
    let dir = profile.workspaces_dir().join("fr1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace dir");
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("fr1"));
    std::fs::write(dir.join("workspace.yaml"), format!("name: Frame\ncolor: '#2a6f97 #61a5c2'\nhome_page: 'http://localhost:{port}/white.html'\n")).expect("ws");
    let find = || super::windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == "fr1"));
    ActionGroupExt::activate_action(&app, "open-workspace", Some(&"fr1".to_variant()));
    let _ = wait_until(15_000, || find().is_some_and(|s| s.current().is_some_and(|t| t.state.borrow().title == "White" && !t.state.borrow().loading))).await;
    let Some(w) = find() else { return };
    let dark = adw::StyleManager::default().is_dark();
    let tint = {
        let c = crate::profile::ColorPair::parse("#2a6f97 #61a5c2").expect("colour");
        let h = if dark { c.dark } else { c.light };
        let n = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0);
        (((n >> 16) & 255) as u8, ((n >> 8) & 255) as u8, (n & 255) as u8)
    };
    let near = |a: (u8, u8, u8), b: (u8, u8, u8)| (a.0 as i32 - b.0 as i32).abs() < 10 && (a.1 as i32 - b.1 as i32).abs() < 10 && (a.2 as i32 - b.2 as i32).abs() < 10;
    let white = (255u8, 255u8, 255u8);

    // Not maximized: the colour frames the page.
    // Start from a maximized window (the compositor may have remembered either state), then un-maximize it.
    w.window.maximize();
    let _ = wait_until(10_000, || w.window.has_css_class("maximized")).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    let full = w.window.width();
    w.window.unmaximize();
    let ok = wait_until(10_000, || !w.window.has_css_class("maximized") && w.window.width() < full - 20).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    println!("frame: classes {:?}, window {}x{}, maximized {} fullscreen {}", w.window.css_classes(), w.window.width(), w.window.height(), w.window.is_maximized(), w.window.is_fullscreen());
    shot(&w.window, "frame-unmaximized");
    let (ww, wh) = (w.window.width(), w.window.height());
    let tv = w.tab_view.compute_point(&w.window, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| (p.x() as i32, p.y() as i32)).unwrap_or((0, 0));
    let tvw = w.tab_view.width();
    let px = pixels(&w.window, &[(3, wh / 2), (ww - 4, wh / 2), (ww / 2, wh - 3), (ww / 2, wh / 2), (tv.0 + 1, tv.1 + 1), (tv.0 + 12, tv.1 + 12)]);
    r.check("the window can be un-maximized for the test", ok, || "still maximized".into());
    r.check("the page is inset from the left and right edges (the colour shows at the sides)", px.len() == 6 && near(px[0], tint) && near(px[1], tint) && tv.0 > 0 && tvw < ww, || format!("{px:?} tint {tint:?} tabview x {} width {tvw} of {ww}", tv.0));
    r.check("the colour shows below the page too, and the page itself is not tinted", px.len() == 6 && near(px[2], tint) && near(px[3], white), || format!("{px:?} tint {tint:?}"));
    r.check("the page's corners are rounded (the colour shows in the corner, the page just inside it)", px.len() == 6 && near(px[4], tint) && near(px[5], white), || format!("{px:?} tint {tint:?}"));
    // Something in the footer (the find bar): no extra border below it.
    let _ = WidgetExt::activate_action(&w.window, "win.find", None);
    glib::timeout_future(Duration::from_millis(600)).await;
    r.check("with the find bar open the window is marked as having a footer", w.window.has_css_class("has-footer"), || String::new());
    let _ = WidgetExt::activate_action(&w.window, "win.find-close", None);
    glib::timeout_future(Duration::from_millis(300)).await;
    r.check("closing it removes the mark", !w.window.has_css_class("has-footer"), || String::new());

    // The options: the thickness is the border's width, and a window can turn it off.
    let th = w.tab_view.compute_point(&w.window, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| p.x() as i32).unwrap_or(-1);
    r.check("the border is 6 px thick by default", th == 6, || format!("page starts at x={th}"));
    let mut cfg = w.settings.borrow().clone();
    cfg.window.frame.thickness = 14;
    cfg.window.frame.radius = 0;
    w.apply_config(&cfg);
    glib::timeout_future(Duration::from_millis(600)).await;
    let th = w.tab_view.compute_point(&w.window, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| p.x() as i32).unwrap_or(-1);
    r.check("a changed thickness applies at once (14 px, square corners)", th == 14, || format!("page starts at x={th}"));
    cfg.window.frame.enabled = false;
    w.apply_config(&cfg);
    glib::timeout_future(Duration::from_millis(600)).await;
    let th = w.tab_view.compute_point(&w.window, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| p.x() as i32).unwrap_or(-1);
    let wh = w.window.height();
    let px = pixels(&w.window, &[(3, wh / 2)]);
    r.check("with the border off the page fills the width and the header keeps its colour", th == 0 && px.len() == 1 && near(px[0], white), || format!("x={th} {px:?}"));
    let head = pixels(&w.window, &[(200, 20)]);
    r.check("(and the header is still tinted)", head.len() == 1 && near(head[0], tint), || format!("{head:?} tint {tint:?}"));
    cfg.window.frame.enabled = true;
    cfg.window.frame.thickness = 6;
    cfg.window.frame.radius = 8;
    w.apply_config(&cfg);
    glib::timeout_future(Duration::from_millis(600)).await;

    // Maximized: the same frame.
    w.window.maximize();
    let ok = wait_until(10_000, || w.window.has_css_class("maximized") && w.window.width() >= full - 20).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    shot(&w.window, "frame-maximized");
    let (ww, wh) = (w.window.width(), w.window.height());
    let tv = w.tab_view.compute_point(&w.window, &gtk::graphene::Point::new(0.0, 0.0)).map(|p| (p.x() as i32, p.y() as i32)).unwrap_or((0, 0));
    let px = pixels(&w.window, &[(3, wh / 2), (ww - 4, wh / 2), (ww / 2, wh - 3), (ww / 2, wh / 2), (tv.0 + 1, tv.1 + 1), (tv.0 + 12, tv.1 + 12)]);
    r.check("maximized, the colour still frames the page (sides, bottom, rounded corners)", ok && px.len() == 6 && near(px[0], tint) && near(px[1], tint) && near(px[2], tint) && near(px[3], white) && near(px[4], tint) && near(px[5], white), || format!("{px:?} tint {tint:?}"));
    // Fullscreen: the page fills the screen (the bars are hidden too).
    let _ = WidgetExt::activate_action(&w.window, "win.fullscreen", None);
    let ok = wait_until(10_000, || w.window.has_css_class("fullscreen")).await;
    glib::timeout_future(Duration::from_millis(800)).await;
    let (ww, wh) = (w.window.width(), w.window.height());
    let px = pixels(&w.window, &[(3, wh / 2), (ww / 2, wh - 3), (ww / 2, 3)]);
    r.check("in fullscreen the page fills the screen with no colour around it", ok && px.len() == 3 && px.iter().all(|p| near(*p, white)), || format!("{px:?}"));
    let _ = WidgetExt::activate_action(&w.window, "win.fullscreen", None);
    let _ = wait_until(10_000, || !w.window.has_css_class("fullscreen")).await;
    for x in super::windows() {
        if !Rc::ptr_eq(&x, shell) {
            x.window.close();
        }
    }
    let _ = wait_until(5_000, || super::windows().len() == 1).await;
    let _ = crate::store::Store::open(&db).map(|s| s.delete_workspace("fr1"));
    let _ = std::fs::remove_dir_all(&dir);
}
