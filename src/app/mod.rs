//! The browser application: one window with a navigation bar and one page.

mod input;
mod latency;
mod nav;
mod selftest;
mod view;
mod watch;

use crate::cli::{Mode, Options};
use crate::config::{self, Config};
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
    /// The window (shell) the tab is in. Changes when a tab is dragged to another window.
    owner: RefCell<Weak<Shell>>,
}

thread_local! {
    /// Tabs detached from one window and not yet attached to another (a drag between windows).
    static IN_FLIGHT: RefCell<Vec<Rc<Tab>>> = const { RefCell::new(Vec::new()) };
}

impl Tab {
    pub fn owner(&self) -> Option<Rc<Shell>> {
        self.owner.borrow().upgrade()
    }

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
    frame_rate: Cell<i32>,
    search_url: RefCell<String>,
    toasts: adw::ToastOverlay,
    /// This window's effective configuration (application, profile, workspace layers) and the
    /// one it started with, against which restart-only options are judged.
    settings: RefCell<Config>,
    startup_settings: Config,
    /// The workspace this window shows; `None` is an ephemeral window (not remembered).
    pub workspace: Option<crate::workspace::Workspace>,
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
    /// Scrolling settings are process-wide statics, so the window being used sets them.
    pub fn apply_input_tuning(&self) {
        let c = self.settings.borrow();
        input::set_scroll_tuning(c.input.wheel_notch, c.input.touchpad_speed, c.input.touchpad_acceleration);
    }

    /// Show a short message over the page.
    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Apply the options that can change while running (see `config::changes`): search engine,
    /// frame rate, scrolling. Options fixed at start-up are applied by the next start.
    pub fn apply_config(&self, cfg: &Config) {
        *self.settings.borrow_mut() = cfg.clone();
        if self.window.is_active() {
            self.apply_input_tuning();
        }
        *self.search_url.borrow_mut() = cfg.search_url.clone();
        if self.frame_rate.get() != cfg.rendering.frame_rate {
            self.frame_rate.set(cfg.rendering.frame_rate);
            for t in self.tabs.borrow().iter() {
                t.view().set_frame_rate(cfg.rendering.frame_rate);
            }
        }
    }

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
            let url = nav::resolve(text, &self.search_url.borrow());
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
        let page_title = if s.title.is_empty() && address_text(&s.address).is_empty() { "browsah".to_owned() } else { tab_title(&s) };
        self.window.set_title(Some(&match &self.workspace {
            Some(w) => format!("{page_title} \u{2014} {}", w.display_name()),
            None => page_title,
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
        let cell = cell.clone();
        move |ev: ViewEvent| {
            // The owner is looked up per event: a tab can be dragged to another window.
            if let Some(tab) = cell.get().and_then(|w| w.upgrade()) {
                if let Some(shell) = tab.owner() {
                    shell.handle(&tab, ev);
                }
            }
        }
    }

    /// Create a view for a tab, wired to deliver its events to `tab`.
    fn make_view(self: &Rc<Self>, cell: &Rc<OnceCell<Weak<Tab>>>, url: &str) -> Rc<BrowserView> {
        let on_event = self.event_closure(cell);
        BrowserView::new(WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate.get(), url, on_event)
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
        let tab = Rc::new(Tab { view: RefCell::new(view), holder, sad, state, page: page.clone(), owner: RefCell::new(Rc::downgrade(self)) });
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

    /// A page left this window's tab view without being closed (dragged to another window): keep
    /// its tab until the other window adopts it.
    fn detach_page(&self, page: &adw::TabPage) {
        let tab = {
            let mut tabs = self.tabs.borrow_mut();
            tabs.iter().position(|t| &t.page == page).map(|i| tabs.remove(i))
        };
        if let Some(tab) = tab {
            IN_FLIGHT.with(|f| f.borrow_mut().push(tab));
        }
    }

    /// A page arrived from another window: adopt its tab. (A new page of our own is not in flight.)
    fn attach_page(self: &Rc<Self>, page: &adw::TabPage) {
        let tab = IN_FLIGHT.with(|f| {
            let mut f = f.borrow_mut();
            f.iter().position(|t| &t.page == page).map(|i| f.remove(i))
        });
        if let Some(tab) = tab {
            *tab.owner.borrow_mut() = Rc::downgrade(self);
            self.tabs.borrow_mut().push(tab);
            if self.tab_view.selected_page().as_ref() == Some(page) {
                self.on_selected();
            }
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

fn build(app: &adw::Application, opts: &Options, cfg: &Config, start_url: &str, workspace: Option<crate::workspace::Workspace>) -> Rc<Shell> {
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
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

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
        frame_rate: Cell::new(cfg.rendering.frame_rate),
        search_url: RefCell::new(cfg.search_url.clone()),
        toasts: toasts.clone(),
        settings: RefCell::new(cfg.clone()),
        startup_settings: cfg.clone(),
        workspace,
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
    tab_view.connect_page_detached({
        let weak = weak.clone();
        move |_, page, _| {
            if let Some(s) = weak.upgrade() {
                s.detach_page(page);
            }
        }
    });
    tab_view.connect_page_attached({
        let weak = weak.clone();
        move |_, page, _| {
            if let Some(s) = weak.upgrade() {
                s.attach_page(page);
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

/// Turn what a person typed (or another application passed) into a URL, as the address bar does.
pub fn resolve_address(text: &str, search_url: &str) -> String {
    nav::resolve(text, search_url)
}

// The open windows of this process: a workspace's window (`Some(id)`) or an ephemeral one
// (`None`). The registry owns the shells (everything the widgets call back into holds a shell
// weakly, so something must own it for as long as its window lives).
thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Shell>>> = const { RefCell::new(Vec::new()) };
}

/// The open windows (GTK thread).
pub fn windows() -> Vec<Rc<Shell>> {
    WINDOWS.with(|w| w.borrow().clone())
}

/// Run the browser in this process, which owns its profile's application id. `app` is already
/// registered (see `main`). Returns the process exit code.
pub fn run(app: adw::Application, opts: Rc<Options>, loaded: config::Loaded, profile: crate::profile::Profile) -> i32 {
    let cfg0 = loaded.config;
    input::set_scroll_tuning(cfg0.input.wheel_notch, cfg0.input.touchpad_speed, cfg0.input.touchpad_acceleration);
    let exit = Rc::new(Cell::new(0));
    // The live configuration: replaced when a config file changes (see `watch`).
    let cfg = Rc::new(RefCell::new(cfg0));
    let watcher: Rc<RefCell<Option<watch::ConfigWatch>>> = Rc::new(RefCell::new(None));

    // Open a window showing `workspace` (None: an ephemeral window) on `start`.
    let new_window = {
        let (opts, cfg, exit, watcher, profile) = (opts.clone(), cfg.clone(), exit.clone(), watcher.clone(), profile.clone());
        move |app: &adw::Application, workspace: Option<crate::workspace::Workspace>, start: &str| -> Rc<Shell> {
            let first = WINDOWS.with(|w| w.borrow().is_empty());
            let app_file = opts.config_path.as_deref().map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
            let profile_file = profile.dir.join("profile.yaml");
            // This window's configuration: application, profile, then the workspace's own file.
            let ws_file = workspace.as_ref().map(|w| w.dir.join("workspace.yaml"));
            let window_cfg = match ws_file.as_deref().map(|wf| config::load_window(&app_file, &profile_file, Some(wf))) {
                Some(Ok(l)) => {
                    for w in &l.warnings {
                        eprintln!("config: {w}");
                    }
                    l.config
                }
                Some(Err(e)) => {
                    eprintln!("config: {e}; using the profile's settings for this workspace");
                    cfg.borrow().clone()
                }
                None => cfg.borrow().clone(),
            };
            let s = build(app, &opts, &window_cfg, start, workspace.clone());
            s.window.connect_is_active_notify({
                let shell = Rc::downgrade(&s);
                move |w| {
                    if let (true, Some(s)) = (w.is_active(), shell.upgrade()) {
                        s.apply_input_tuning();
                    }
                }
            });
            if let (Some(wf), Some(w)) = (&ws_file, watcher.borrow().as_ref()) {
                w.add(wf);
            }
            WINDOWS.with(|w| w.borrow_mut().push(s.clone()));
            if let Some(w) = &workspace {
                remember_workspace(&profile, &w.id, true);
            }
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: window opened (workspace {:?}) on {start}", workspace.as_ref().map(|w| &w.id));
            }
            // A closed window leaves the registry at once (its widgets are only disposed of later, and
            // a closure must not hold the window strongly or it would never be).
            s.window.connect_close_request({
                let (window, profile) = (s.window.downgrade(), profile.clone());
                move |_| {
                    let id = window.upgrade().and_then(|w| windows_workspace_id(&w));
                    let more = WINDOWS.with(|w| w.borrow().len()) > 1;
                    WINDOWS.with(|w| w.borrow_mut().retain(|s| Some(&s.window) != window.upgrade().as_ref()));
                    // Closing one of several windows forgets its workspace; the last window's
                    // stays remembered, so the next plain start brings it back.
                    if let (Some(id), true) = (id, more) {
                        remember_workspace(&profile, &id, false);
                    }
                    glib::Propagation::Proceed
                }
            });
            s.window.present();
            if first {
                if opts.latency {
                    latency::spawn(app.clone(), s.clone(), cfg.borrow().rendering.frame_rate);
                }
                if opts.selftest {
                    selftest::spawn(app.clone(), s.clone(), exit.clone(), opts.mode.unwrap_or(cfg.borrow().rendering.mode.mode()) == Mode::Accel);
                }
                // Reload configuration when its files change (unless switched off).
                let watching = cfg.borrow().watch_config && !opts.no_watch && !opts.selftest && !opts.latency;
                if watching {
                    let (cfg, watcher2) = (cfg.clone(), watcher.clone());
                    let (app_file2, profile_file2) = (app_file.clone(), profile_file.clone());
                    let w = watch::ConfigWatch::start(&[app_file, profile_file], move || {
                        reload(&cfg, &app_file2, &profile_file2, &watcher2);
                    });
                    if let Some(wf) = &ws_file {
                        w.add(wf);
                    }
                    *watcher.borrow_mut() = Some(w);
                }
            }
            s
        }
    };

    // Show a workspace: raise its window if it is open, otherwise open one. `url` opens as a tab.
    let open_workspace = {
        let (cfg, profile, new_window) = (cfg.clone(), profile.clone(), new_window.clone());
        move |app: &adw::Application, id: &str, url: Option<&str>| -> Result<Rc<Shell>, String> {
            if let Some(s) = windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id)) {
                if let Some(u) = url {
                    s.open_tab(u, true);
                }
                s.window.present();
                return Ok(s);
            }
            let w = crate::workspace::Workspace::resolve(&profile, id)?;
            let start = match url {
                Some(u) => u.to_owned(),
                None => cfg.borrow().home_page.clone(),
            };
            Ok(new_window(app, Some(w), &start))
        }
    };

    // Another window with no workspace (Ctrl+Shift+N): nothing about it is remembered.
    let new_ephemeral = gio::SimpleAction::new("new-window", None);
    new_ephemeral.connect_activate({
        let (cfg, new_window, app) = (cfg.clone(), new_window.clone(), app.clone());
        move |_, _| {
            let home = cfg.borrow().home_page.clone();
            new_window(&app, None, &home);
        }
    });
    app.add_action(&new_ephemeral);
    app.set_accels_for_action("app.new-window", &["<Ctrl><Shift>N"]);

    // `browsah --workspace=ID` against a running profile arrives here.
    let open_ws_action = gio::SimpleAction::new("open-workspace", Some(glib::VariantTy::STRING));
    open_ws_action.connect_activate({
        let (open_workspace, app) = (open_workspace.clone(), app.clone());
        move |_, param| {
            if let Some(id) = param.and_then(|p| p.get::<String>()) {
                if let Err(e) = open_workspace(&app, &id, None) {
                    eprintln!("browsah: {e}");
                }
            }
        }
    });
    app.add_action(&open_ws_action);

    app.connect_activate({
        let (opts, cfg, profile, new_window, open_workspace) = (opts.clone(), cfg.clone(), profile.clone(), new_window.clone(), open_workspace.clone());
        move |app| {
            if let Some(s) = windows().last() {
                s.window.present(); // another launch of this profile: show a window
                return;
            }
            let test_run = opts.selftest || opts.latency;
            let url = if opts.selftest {
                Some(selftest::start_url())
            } else if opts.latency {
                Some(latency::start_url())
            } else {
                opts.url.as_deref().map(|u| nav::resolve(u, &cfg.borrow().search_url))
            };
            // Which workspaces to open: the one asked for, else those open at last exit, else the
            // profile's default; with none of those, an ephemeral window.
            let ids: Vec<String> = match (&opts.workspace, test_run) {
                (Some(id), _) => vec![id.clone()],
                (None, true) => Vec::new(),
                (None, false) => startup_workspaces(&profile),
            };
            let mut opened = false;
            for (i, id) in ids.iter().enumerate() {
                // A link given on the command line goes to the first workspace.
                match open_workspace(app, id, if i == 0 { url.as_deref() } else { None }) {
                    Ok(_) => opened = true,
                    Err(e) => eprintln!("browsah: {e}"),
                }
            }
            if !opened {
                let start = url.unwrap_or_else(|| cfg.borrow().home_page.clone());
                new_window(app, None, &start);
            }
        }
    });

    // Links handed over by another launch of this profile (or by the desktop, when this profile's
    // entry is the default browser): each opens as a tab in the workspace named by the hint, else
    // the active window.
    app.connect_open({
        let (cfg, new_window, open_workspace) = (cfg.clone(), new_window, open_workspace);
        move |app, files, hint| {
            let search = cfg.borrow().search_url.clone();
            let urls: Vec<String> = files.iter().map(|f| nav::resolve(f.uri().as_str(), &search)).collect();
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: asked to open {urls:?} (workspace hint {hint:?})");
            }
            if !hint.is_empty() {
                for (i, url) in urls.iter().enumerate() {
                    if let Err(e) = open_workspace(app, hint, Some(url)) {
                        eprintln!("browsah: {e}");
                        if i == 0 {
                            new_window(app, None, url);
                        }
                    }
                }
                return;
            }
            let active = app.active_window().and_then(|w| windows().into_iter().find(|s| s.window.upcast_ref::<gtk::Window>() == &w));
            let existing = active.or_else(|| windows().last().cloned());
            let (s, skip) = match existing {
                Some(s) => (s, 0),
                None => (new_window(app, None, urls.first().map(String::as_str).unwrap_or("about:blank")), 1),
            };
            for url in urls.iter().skip(skip) {
                s.open_tab(url, true);
            }
            s.window.present();
        }
    });

    app.run_with_args::<&str>(&[]);
    exit.get()
}

/// The workspace id of the window, if it shows one.
fn windows_workspace_id(window: &adw::ApplicationWindow) -> Option<String> {
    WINDOWS.with(|w| w.borrow().iter().find(|s| &s.window == window).and_then(|s| s.workspace.as_ref().map(|w| w.id.clone())))
}

/// Which workspaces a plain start opens: those open at last exit that still exist, else the
/// profile's `default_workspace`, else none.
fn startup_workspaces(profile: &crate::profile::Profile) -> Vec<String> {
    let exists = |id: &String| crate::workspace::Workspace::resolve(profile, id).is_ok();
    let remembered: Vec<String> = profile.load_state().open_workspaces.into_iter().filter(exists).collect();
    if !remembered.is_empty() {
        return remembered;
    }
    profile.config.default_workspace.clone().filter(exists).into_iter().collect()
}

/// Record that a workspace's window is open (or no longer).
fn remember_workspace(profile: &crate::profile::Profile, id: &str, open: bool) {
    let mut state = profile.load_state();
    state.open_workspaces.retain(|w| w != id);
    if open {
        state.open_workspaces.push(id.to_owned());
    }
    profile.save_state(&state);
}

/// A config file changed: load each window's effective configuration again and apply what can
/// change live. An invalid file is reported and the running configuration kept.
fn reload(cfg: &Rc<RefCell<Config>>, app_file: &std::path::Path, profile_file: &std::path::Path, watcher: &Rc<RefCell<Option<watch::ConfigWatch>>>) {
    let shells = windows();
    let debug = std::env::var_os("BROWSAH_DEBUG").is_some();
    let process = match config::load_effective(app_file, Some(profile_file)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("config: {e}; keeping the current settings");
            for s in &shells {
                s.toast("Configuration has an error; keeping the current settings");
            }
            return;
        }
    };
    let mut stop = false;
    if !process.config.watch_config {
        stop = true;
    }
    for s in &shells {
        let ws_file = s.workspace.as_ref().map(|w| w.dir.join("workspace.yaml"));
        let loaded = match config::load_window(app_file, profile_file, ws_file.as_deref()) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("config: {e}; keeping this window's settings");
                s.toast("Configuration has an error; keeping the current settings");
                continue;
            }
        };
        let new = loaded.config;
        let changes = config::changes(&s.settings.borrow(), &new);
        if changes == config::Changes::default() {
            continue; // saved without a relevant change for this window
        }
        // Options fixed at start-up are judged against what this window started with, so the
        // message stays right when a restart-only option is edited back.
        let pending_restart = config::changes(&s.startup_settings, &new).restart;
        for w in &loaded.warnings {
            eprintln!("config: {w}");
        }
        if debug {
            eprintln!("browsah: config reloaded for workspace {:?}: live {:?}, restart needed {:?}", s.workspace.as_ref().map(|w| &w.id), changes.live, pending_restart);
        }
        s.apply_config(&new);
        let mut msg = String::from("Configuration reloaded");
        if !pending_restart.is_empty() {
            msg.push_str(&format!(" (restart needed for {})", pending_restart.join(", ")));
        }
        if stop {
            msg.push_str("; watching is now off");
        }
        s.toast(&msg);
    }
    *cfg.borrow_mut() = process.config;
    if stop {
        *watcher.borrow_mut() = None;
    }
}
