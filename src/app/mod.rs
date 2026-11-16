//! The browser application: one window with a navigation bar and one page.

mod input;
mod latency;
mod nav;
mod selftest;
mod view;

use crate::cli::{Mode, Options};
use crate::config::Config;
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;
use view::{BrowserView, ViewEvent};

/// What the page last told the shell. Kept so the UI (and the self-test) can read it back.
#[derive(Debug, Default)]
pub struct State {
    pub ready: bool,
    pub address: String,
    pub title: String,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    /// Completed loads (loading went true then false).
    pub load_cycles: u32,
    /// Labels of the last context menu shown, and how many have been shown.
    pub menu: Vec<String>,
    pub menus: u32,
    /// The view was replaced by a readback one because no accelerated frame arrived.
    pub fell_back: bool,
    /// The renderer died; the tab shows a "crashed" page until it loads something again.
    pub crashed: bool,
}

/// One tab: a browser view, what it last told the shell, and its page in the tab bar.
pub struct Tab {
    /// The browser view. Replaced if the accelerated path produces no frames (see `fall_back`).
    view: RefCell<Rc<BrowserView>>,
    /// The page's child in the tab view; holds whichever view is current.
    holder: gtk::Box,
    /// Shown in place of the view while the page's renderer is dead.
    sad: adw::StatusPage,
    pub state: Rc<RefCell<State>>,
    pub page: adw::TabPage,
}

impl Tab {
    pub fn sad_visible(&self) -> bool {
        self.sad.is_visible()
    }

    pub fn view(&self) -> Rc<BrowserView> {
        self.view.borrow().clone()
    }
}

/// How long a view may take to deliver its first accelerated frame before the shell gives up on
/// the dmabuf path and uses readback instead.
const FIRST_FRAME_WATCHDOG: Duration = Duration::from_secs(3);

pub struct Shell {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    pub entry: gtk::Entry,
    pub back: gtk::Button,
    pub forward: gtk::Button,
    pub reload: gtk::Button,
    pub stop: gtk::Button,
    back_action: gio::SimpleAction,
    forward_action: gio::SimpleAction,
    tabs: RefCell<Vec<Rc<Tab>>>,
    mode: Cell<Mode>,
    backing: bool,
    frame_rate: i32,
    search_url: String,
    /// The user has typed in the address bar and not yet committed or cancelled.
    editing: Cell<bool>,
    /// Set while the shell itself changes the entry text, so that is not mistaken for typing.
    programmatic: Cell<bool>,
}

fn address_text(address: &str) -> &str {
    if address == "about:blank" { "" } else { address }
}

/// The label of a tab: the page title, else its address, else a placeholder.
fn tab_title(s: &State) -> String {
    if !s.title.is_empty() {
        s.title.clone()
    } else if !address_text(&s.address).is_empty() {
        s.address.clone()
    } else {
        "New tab".into()
    }
}

impl Shell {
    /// The selected tab.
    pub fn current(&self) -> Option<Rc<Tab>> {
        let page = self.tab_view.selected_page()?;
        self.tabs.borrow().iter().find(|t| t.page == page).cloned()
    }

    /// The selected tab's view and state. Panic if there is no tab (the self-test only).
    pub fn view(&self) -> Rc<BrowserView> {
        self.current().expect("a tab").view()
    }

    pub fn state(&self) -> Rc<RefCell<State>> {
        self.current().expect("a tab").state.clone()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.borrow().len()
    }

    pub fn tabs(&self) -> Vec<Rc<Tab>> {
        self.tabs.borrow().clone()
    }

    /// Resolve address-bar text and navigate the selected tab there.
    pub fn go(&self, text: &str) {
        self.editing.set(false);
        if let Some(t) = self.current() {
            let url = nav::resolve(text, &self.search_url);
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: address bar {text:?} -> {url:?}");
            }
            t.view().navigate(&url);
        }
    }

    fn set_entry_text(&self, text: &str) {
        self.programmatic.set(true);
        self.entry.set_text(text);
        self.programmatic.set(false);
    }

    /// Show the selected tab's state in the shared chrome (address bar, buttons, window title).
    fn refresh(&self) {
        let Some(tab) = self.current() else { return };
        let s = tab.state.borrow();
        if !self.editing.get() {
            self.set_entry_text(address_text(&s.address));
        }
        self.window.set_title(Some(&if s.title.is_empty() && address_text(&s.address).is_empty() {
            "browsah".to_owned()
        } else {
            tab_title(&s)
        }));
        self.back_action.set_enabled(s.can_back);
        self.forward_action.set_enabled(s.can_forward);
        self.reload.set_visible(!s.loading);
        self.stop.set_visible(s.loading);
    }

    /// Show a popup browser (one that keeps its opener) as a new selected tab.
    fn adopt_tab(self: &Rc<Self>, spec: view::PopupSpec) -> Rc<Tab> {
        let (cell, state) = (Rc::new(OnceCell::new()), Rc::new(RefCell::new(State::default())));
        let on_event = self.event_closure(&cell);
        let view = BrowserView::adopt(WidgetExt::display(&self.window), spec, on_event);
        self.add_tab(view, cell, state, true)
    }

    /// The closure that delivers a view's events to its tab.
    fn event_closure(self: &Rc<Self>, cell: &Rc<OnceCell<Weak<Tab>>>) -> impl Fn(ViewEvent) + 'static {
        let shell = Rc::downgrade(self);
        let cell = cell.clone();
        move |ev: ViewEvent| {
            if let (Some(shell), Some(tab)) = (shell.upgrade(), cell.get().and_then(|w| w.upgrade())) {
                shell.handle(&tab, ev);
            }
        }
    }

    /// Create a view for a tab, wired to deliver its events to `tab`.
    fn make_view(self: &Rc<Self>, cell: &Rc<OnceCell<Weak<Tab>>>, url: &str) -> Rc<BrowserView> {
        let on_event = self.event_closure(cell);
        BrowserView::new(WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate, url, on_event)
    }

    /// Open a tab on `url`. Background tabs start hidden (not rendering) until selected.
    pub fn open_tab(self: &Rc<Self>, url: &str, select: bool) -> Rc<Tab> {
        let state = Rc::new(RefCell::new(State::default()));
        let cell: Rc<OnceCell<Weak<Tab>>> = Rc::new(OnceCell::new());
        let view = self.make_view(&cell, url);
        self.add_tab(view, cell, state, select)
    }

    /// Put a view in a new tab.
    fn add_tab(self: &Rc<Self>, view: Rc<BrowserView>, cell: Rc<OnceCell<Weak<Tab>>>, state: Rc<RefCell<State>>, select: bool) -> Rc<Tab> {
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.append(&view.widget);
        let sad = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("This tab crashed")
            .hexpand(true)
            .vexpand(true)
            .visible(false)
            .child(&gtk::Button::builder().label("Reload").halign(gtk::Align::Center).action_name("win.reload").css_classes(["suggested-action", "pill"]).build())
            .build();
        holder.append(&sad);
        let page = self.tab_view.append(&holder);
        page.set_title("New tab");
        let tab = Rc::new(Tab { view: RefCell::new(view), holder, sad, state, page: page.clone() });
        let _ = cell.set(Rc::downgrade(&tab));
        self.tabs.borrow_mut().push(tab.clone());
        if select {
            self.tab_view.set_selected_page(&page);
        }
        if self.tab_view.selected_page().as_ref() == Some(&page) {
            self.on_selected();
        }
        tab
    }

    /// The tab's view delivered no accelerated frame in time: switch to readback (for this tab
    /// and every later one) by replacing the view with a new one on the same address.
    fn fall_back(self: &Rc<Self>, tab: &Rc<Tab>) {
        eprintln!("browsah: no accelerated frame within {FIRST_FRAME_WATCHDOG:?}; falling back to readback rendering");
        self.mode.set(Mode::Paint);
        let url = {
            let s = tab.state.borrow();
            if s.address.is_empty() { "about:blank".to_owned() } else { s.address.clone() }
        };
        // Find this tab's event cell again through the closure chain: a fresh cell is simplest.
        let cell: Rc<OnceCell<Weak<Tab>>> = Rc::new(OnceCell::new());
        let _ = cell.set(Rc::downgrade(tab));
        let old = tab.view();
        let new = self.make_view(&cell, &url);
        tab.holder.prepend(&new.widget);
        tab.holder.remove(&old.widget);
        *tab.view.borrow_mut() = new;
        old.close();
        {
            let mut s = tab.state.borrow_mut();
            s.ready = false;
            s.fell_back = true;
        }
        if self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) {
            tab.view().widget.grab_focus();
        }
    }

    /// The selected tab changed: show its state and give its page the keyboard focus.
    fn on_selected(&self) {
        self.editing.set(false);
        self.refresh();
        if let Some(t) = self.current() {
            t.view().widget.grab_focus();
        }
    }

    /// Close the tab whose page this is. Closing the last tab closes the window.
    fn close_tab(&self, page: &adw::TabPage) {
        let tab = {
            let mut tabs = self.tabs.borrow_mut();
            tabs.iter().position(|t| &t.page == page).map(|i| tabs.remove(i))
        };
        if let Some(tab) = tab {
            tab.view().close();
        }
        if self.tabs.borrow().is_empty() {
            self.window.close();
        }
    }

    /// Ask CEF to close every browser. Call before shutting CEF down.
    pub fn close_all(&self) {
        for t in self.tabs.borrow().iter() {
            t.view().close();
        }
    }

    /// Select the tab `delta` places along, wrapping around.
    fn step_tab(&self, delta: i32) {
        let n = self.tab_view.n_pages();
        if n < 2 {
            return;
        }
        if let Some(page) = self.tab_view.selected_page() {
            let pos = self.tab_view.page_position(&page);
            self.tab_view.set_selected_page(&self.tab_view.nth_page((pos + delta).rem_euclid(n)));
        }
    }

    /// Select the neighbouring tab (the self-test's stand-in for the shortcut).
    pub fn step_tab_for_test(&self, delta: i32) {
        self.step_tab(delta);
    }

    /// A tab's page told us something.
    fn handle(self: &Rc<Self>, tab: &Rc<Tab>, ev: ViewEvent) {
        match ev {
            ViewEvent::Ready => {
                tab.state.borrow_mut().ready = true;
                tab.view().sync_focus();
                if self.mode.get() == Mode::Accel {
                    let (shell, tab, view) = (Rc::downgrade(self), Rc::downgrade(tab), tab.view());
                    glib::timeout_add_local_once(FIRST_FRAME_WATCHDOG, move || {
                        if let (Some(shell), Some(tab)) = (shell.upgrade(), tab.upgrade()) {
                            // Still the same view, still visible or not, and nothing ever arrived.
                            if Rc::ptr_eq(&tab.view(), &view) && view.frames() == 0 && !view.is_hidden() {
                                shell.fall_back(&tab);
                            }
                        }
                    });
                }
            }
            ViewEvent::ContextMenu(req) => {
                {
                    let mut s = tab.state.borrow_mut();
                    s.menu = req.items.iter().map(|i| if i.separator { "-".into() } else { i.label.clone() }).collect();
                    s.menus += 1;
                }
                tab.view().show_context_menu(req);
            }
            ViewEvent::Popup(spec) => {
                self.adopt_tab(spec);
            }
            ViewEvent::Crashed(why) => {
                eprintln!("browsah: a page's renderer ended: {why}");
                tab.state.borrow_mut().crashed = true;
                tab.sad.set_description(Some(&why));
                tab.sad.set_visible(true);
                tab.view().widget.set_visible(false);
            }
            ViewEvent::Address(a) => {
                tab.state.borrow_mut().address = a;
                // Navigating to another host resets the zoom.
                tab.view().apply_zoom();
            }
            ViewEvent::Title(t) => tab.state.borrow_mut().title = t,
            ViewEvent::Loading { loading, can_back, can_forward } => {
                if loading && tab.state.borrow().crashed {
                    // Something is loading again (reload or a new address): show the page.
                    tab.state.borrow_mut().crashed = false;
                    tab.sad.set_visible(false);
                    tab.view().widget.set_visible(true);
                }
                {
                    let mut s = tab.state.borrow_mut();
                    if s.loading && !loading {
                        s.load_cycles += 1;
                    }
                    s.loading = loading;
                    s.can_back = can_back;
                    s.can_forward = can_forward;
                }
                if !loading {
                    tab.view().apply_zoom();
                }
            }
        }
        {
            let s = tab.state.borrow();
            tab.page.set_title(&tab_title(&s));
            tab.page.set_loading(s.loading);
        }
        if self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) {
            self.refresh();
        }
    }
}

fn add_action(window: &adw::ApplicationWindow, name: &str, f: impl Fn() + 'static) -> gio::SimpleAction {
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| f());
    window.add_action(&action);
    action
}

fn icon_button(icon: &str, tooltip: &str, action: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b.set_action_name(Some(action));
    b
}

fn build(app: &adw::Application, opts: &Options, cfg: &Config, start_url: &str) -> Rc<Shell> {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("browsah")
        .default_width(cfg.window.width)
        .default_height(cfg.window.height)
        .build();

    let back = icon_button("go-previous-symbolic", "Back (Alt+Left)", "win.back");
    let forward = icon_button("go-next-symbolic", "Forward (Alt+Right)", "win.forward");
    let reload = icon_button("view-refresh-symbolic", "Reload (Ctrl+R)", "win.reload");
    let stop = icon_button("process-stop-symbolic", "Stop", "win.stop");
    stop.set_visible(false);

    let entry = gtk::Entry::builder()
        .hexpand(true)
        .placeholder_text("Search or enter address")
        .input_purpose(gtk::InputPurpose::Url)
        .build();
    let clamp = adw::Clamp::builder().maximum_size(900).tightening_threshold(600).child(&entry).build();

    let header = adw::HeaderBar::new();
    header.pack_start(&back);
    header.pack_start(&forward);
    header.pack_start(&reload);
    header.pack_start(&stop);
    header.set_title_widget(Some(&clamp));

    let tab_view = adw::TabView::new();
    let tab_bar = adw::TabBar::builder().view(&tab_view).autohide(false).build();
    tab_bar.set_end_action_widget(Some(&icon_button("tab-new-symbolic", "New tab (Ctrl+T)", "win.new-tab")));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&tab_bar);
    toolbar.set_content(Some(&tab_view));
    window.set_content(Some(&toolbar));

    let back_action = gio::SimpleAction::new("back", None);
    let forward_action = gio::SimpleAction::new("forward", None);
    back_action.set_enabled(false);
    forward_action.set_enabled(false);
    window.add_action(&back_action);
    window.add_action(&forward_action);

    let shell = Rc::new(Shell {
        window: window.clone(),
        tab_view: tab_view.clone(),
        entry: entry.clone(),
        back,
        forward,
        reload,
        stop,
        back_action: back_action.clone(),
        forward_action: forward_action.clone(),
        tabs: RefCell::new(Vec::new()),
        mode: Cell::new(opts.mode.unwrap_or(cfg.rendering.mode.mode())),
        backing: opts.backing.unwrap_or(cfg.rendering.device_resolution),
        frame_rate: cfg.rendering.frame_rate,
        search_url: cfg.search_url.clone(),
        editing: Cell::new(false),
        programmatic: Cell::new(false),
    });
    let weak = Rc::downgrade(&shell);

    // Actions on the selected tab.
    let on_tab = |f: fn(&BrowserView)| {
        let weak = weak.clone();
        move || {
            if let Some(t) = weak.upgrade().and_then(|s| s.current()) {
                f(&t.view());
            }
        }
    };
    back_action.connect_activate({
        let f = on_tab(BrowserView::go_back);
        move |_, _| f()
    });
    forward_action.connect_activate({
        let f = on_tab(BrowserView::go_forward);
        move |_, _| f()
    });
    add_action(&window, "reload", on_tab(BrowserView::reload));
    add_action(&window, "stop", on_tab(BrowserView::stop));
    add_action(&window, "focus-address", {
        let entry = entry.clone();
        move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        }
    });

    // Tabs.
    add_action(&window, "new-tab", {
        let (weak, entry) = (weak.clone(), entry.clone());
        move || {
            if let Some(s) = weak.upgrade() {
                s.open_tab("about:blank", true);
                entry.grab_focus();
            }
        }
    });
    add_action(&window, "close-tab", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                if let Some(p) = s.tab_view.selected_page() {
                    s.tab_view.close_page(&p);
                }
            }
        }
    });
    add_action(&window, "next-tab", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.step_tab(1);
            }
        }
    });
    add_action(&window, "prev-tab", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.step_tab(-1);
            }
        }
    });
    tab_view.connect_close_page({
        let weak = weak.clone();
        move |tv, page| {
            if let Some(s) = weak.upgrade() {
                s.close_tab(page);
            }
            tv.close_page_finish(page, true);
            glib::Propagation::Stop
        }
    });
    tab_view.connect_selected_page_notify({
        let weak = weak.clone();
        move |_| {
            if let Some(s) = weak.upgrade() {
                s.on_selected();
            }
        }
    });

    app.set_accels_for_action("win.back", &["<Alt>Left"]);
    app.set_accels_for_action("win.forward", &["<Alt>Right"]);
    app.set_accels_for_action("win.reload", &["<Ctrl>R", "F5"]);
    app.set_accels_for_action("win.focus-address", &["<Ctrl>L", "F6"]);
    app.set_accels_for_action("win.new-tab", &["<Ctrl>T"]);
    app.set_accels_for_action("win.close-tab", &["<Ctrl>W"]);
    app.set_accels_for_action("win.next-tab", &["<Ctrl>Tab", "<Ctrl>Page_Down"]);
    app.set_accels_for_action("win.prev-tab", &["<Ctrl><Shift>Tab", "<Ctrl>ISO_Left_Tab", "<Ctrl>Page_Up"]);

    // Address bar: Enter navigates; typing marks the text as being edited so a page's address
    // change does not overwrite it; Escape or leaving the entry restores the current address.
    entry.connect_activate({
        let shell = Rc::downgrade(&shell);
        move |e| {
            if let Some(shell) = shell.upgrade() {
                shell.go(e.text().as_str());
            }
        }
    });
    entry.connect_changed({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(shell) = shell.upgrade() {
                if !shell.programmatic.get() {
                    shell.editing.set(true);
                }
            }
        }
    });
    let restore = {
        let shell = Rc::downgrade(&shell);
        move || {
            if let Some(shell) = shell.upgrade() {
                shell.editing.set(false);
                let a = shell.current().map(|t| t.state.borrow().address.clone()).unwrap_or_default();
                shell.set_entry_text(address_text(&a));
            }
        }
    };
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let restore = restore.clone();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                restore();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(keys);
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| restore());
    entry.add_controller(focus);

    window.connect_close_request({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(s) = shell.upgrade() {
                s.close_all();
            }
            glib::Propagation::Proceed
        }
    });

    if std::env::var_os("BROWSAH_DEBUG").is_some() {
        // Log every key press with the widget that has focus, to see where typing goes.
        let log = gtk::EventControllerKey::new();
        log.set_propagation_phase(gtk::PropagationPhase::Capture);
        log.connect_key_pressed({
            let window = window.clone();
            move |_, key, _, state| {
                let focus = gtk::prelude::GtkWindowExt::focus(&window).map(|w| w.type_().name().to_owned());
                eprintln!("browsah: key {:?} {:?} focus={:?}", key.name(), state, focus);
                glib::Propagation::Proceed
            }
        });
        window.add_controller(log);
    }

    shell.open_tab(start_url, true);
    shell
}

/// Run the browser. Returns the process exit code.
pub fn run(opts: Rc<Options>, cfg: Config) -> i32 {
    input::set_scroll_tuning(cfg.input.wheel_notch, cfg.input.touchpad_speed, cfg.input.touchpad_acceleration);
    let app = adw::Application::builder()
        .application_id("nz.co.bacchus.browsah")
        // Each launch is its own browser for now (no single-instance forwarding).
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let exit = Rc::new(Cell::new(0));

    app.connect_activate({
        let exit = exit.clone();
        move |app| {
            let start = if opts.selftest {
                selftest::start_url()
            } else if opts.latency {
                latency::start_url()
            } else {
                nav::resolve(opts.url.as_deref().unwrap_or(&cfg.home_page), &cfg.search_url)
            };
            let shell = build(app, &opts, &cfg, &start);
            // Everything the window's widgets call back into holds the shell weakly, so something
            // must own it for as long as the window lives. (Without this the address bar, buttons
            // and shortcuts silently did nothing once start-up ended.)
            shell.window.connect_destroy({
                let keep = shell.clone();
                move |_| {
                    let _ = &keep;
                }
            });
            shell.window.present();
            if opts.latency {
                latency::spawn(app.clone(), shell.clone(), cfg.rendering.frame_rate);
            }
            if opts.selftest {
                selftest::spawn(app.clone(), shell, exit.clone(), opts.mode.unwrap_or(cfg.rendering.mode.mode()) == Mode::Accel);
            }
        }
    });

    app.run_with_args::<&str>(&[]);
    exit.get()
}
