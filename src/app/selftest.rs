//! `--selftest`: a scripted check of the navigation shell against two local pages
//! (`bench/pages/a.html`, `b.html`). It drives the same paths the user does (address entry,
//! window actions such as the back shortcut) and reads the result back from the widgets.
//! Prints PASS/FAIL per check and exits non-zero if any failed. No network needed.

use super::Shell;
use adw::prelude::*;
use gtk::gdk::{self, Key, ModifierType};
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

pub fn spawn(app: adw::Application, shell: Rc<Shell>, exit: Rc<Cell<i32>>, accel: bool) {
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

    run_input(shell, r).await;
    run_tabs(shell, r).await;
}

// ---------------------------------------------------------------------------------------------
// Input: drive the real GTK controllers by emitting their signals, as GTK does for a user.
// ---------------------------------------------------------------------------------------------

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
}
