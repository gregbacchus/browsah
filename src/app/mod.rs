//! The browser application: one window with a navigation bar and one page.

mod input;
mod history;
mod downloads;
mod find;
mod fullscreen;
mod install;
mod zoom;
mod jsdialog;
mod suggest;
mod latency;
mod modal;
mod prompts;
mod sweep;
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
use std::sync::Arc;
use std::time::Duration;
use crate::store::{Store, TabRecord};
use view::{BrowserView, ViewEvent};

/// What the page last told the shell. Kept so the UI (and the self-test) can read it back.
#[derive(Debug, Default, Clone)]
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
    /// Stable id; names the tab's screenshot and its row in the database.
    pub uid: u64,
    /// The address last written to the history (a reload is not a new visit).
    last_recorded: RefCell<String>,
    /// How the tab has been used, for sweeping.
    usage: RefCell<crate::sweep::Usage>,
    /// When the tab was created (unix seconds); a tab with no use counts as idle since then.
    created: std::cell::Cell<i64>,
    /// What the last probe found: unsaved form input, playing media, and the input count then.
    dirty: std::cell::Cell<bool>,
    playing: std::cell::Cell<bool>,
    input_seen: std::cell::Cell<u64>,
}

static NEXT_WINDOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

thread_local! {
    /// The profile's database (GTK thread).
    static STORE: RefCell<Option<Rc<Store>>> = const { RefCell::new(None) };
}

fn store() -> Option<Rc<Store>> {
    STORE.with(|s| s.borrow().clone())
}

/// A tab id that is unique across restarts (it names the tab's screenshot file).
fn new_uid() -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let micros = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0);
    (micros << 8) | (COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0xff)
}

thread_local! {
    /// Opens an ephemeral window on a URL (no tab at all for an empty one) and returns it. Set by
    /// `run`; used when a workspace cannot open tabs itself.
    static NEW_EPHEMERAL: RefCell<Option<Rc<dyn Fn(&str) -> Rc<Shell>>>> = const { RefCell::new(None) };
}

thread_local! {
    /// Shows a workspace's window (opening it if needed); an empty URL adds no tab. Set by `run`.
    static OPEN_WORKSPACE: RefCell<Option<Rc<dyn Fn(&str, &str) -> Result<Rc<Shell>, String>>>> = const { RefCell::new(None) };
    /// The profile this process runs (for listing its workspaces).
    static PROFILE: RefCell<Option<crate::profile::Profile>> = const { RefCell::new(None) };
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

/// Where a workspace window's tabs are remembered.
struct Persist {
    workspace: String,
    /// Directory of tab screenshots (`<uid>.png`).
    shots: std::path::PathBuf,
}

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
    /// Which controls are shown and whether tabs can be opened (a workspace's `nav` and `tabs`).
    nav: crate::workspace::NavConfig,
    allow_new_tabs: bool,
    /// This workspace's sweeping rules (None: off), how many sweeps have run, and the last one's notes.
    sweeper: Option<crate::sweep::Sweeper>,
    sweeps_done: Cell<u32>,
    last_sweep: RefCell<Vec<String>>,
    pending_close_reason: RefCell<Option<String>>,
    /// Nothing in this window is remembered (a sign-in another application started).
    pub private: Cell<bool>,
    /// Whose session cookies these are: the workspace's id, or a number for an ephemeral window.
    session_owner: String,
    /// The address bar's suggestions.
    suggest: suggest::Popup,
    /// The downloads button and its list.
    pub downloads: Rc<downloads::Ui>,
    /// Dialogs pages opened with script: waiting, the one showing, and (in tests) the one held.
    js_queue: RefCell<std::collections::VecDeque<Arc<view::JsDialogQuestion>>>,
    js_showing: RefCell<Option<(u64, adw::AlertDialog)>>,
    js_pending: RefCell<Option<Arc<view::JsDialogQuestion>>>,
    /// The find-in-page bar.
    find: find::Bar,
    /// The window's bars (hidden in fullscreen), and whether a page asked for the fullscreen.
    toolbar: adw::ToolbarView,
    page_fullscreen: Cell<bool>,
    /// Whether the window has been asked to fill the screen (the compositor's answer arrives later).
    fullscreen: Cell<bool>,
    /// The history window, while it is open.
    history: RefCell<Option<Rc<history::HistoryView>>>,
    /// Permission questions waiting, the one showing, and (in tests) the one held; downloads in progress,
    /// finished, and one waiting for a place to save.
    prompt_queue: RefCell<std::collections::VecDeque<Rc<prompts::Queued>>>,
    prompt_dialog: RefCell<Option<(u64, adw::AlertDialog)>>,
    pending_prompt: RefCell<Option<Rc<prompts::Queued>>>,
    downloads_seen: RefCell<std::collections::HashSet<u32>>,
    completed_downloads: RefCell<Vec<std::path::PathBuf>>,
    pending_download: RefCell<Option<Arc<view::DownloadQuestion>>>,
    /// Modals open over this window, how they look, and what a script popup becomes.
    modals: RefCell<Vec<Rc<modal::Modal>>>,
    modal_style: crate::workspace::ModalStyle,
    popup_policy: crate::workspace::PopupPolicy,
    /// The tab context menu (rebuilt each time it opens) and the page it was opened on.
    tab_menu: gio::Menu,
    menu_page: RefCell<Option<adw::TabPage>>,
    /// Bookmarks: the bar's container and the star button (workspace windows only).
    bookmark_bar: gtk::Box,
    bookmark_revealer: gtk::Revealer,
    star: gtk::Button,
    /// The workspace colour's style sheet, if it has one.
    color_css: RefCell<Option<gtk::CssProvider>>,
    /// Where this window's tabs are remembered (workspace windows only).
    persist: Option<Persist>,
    save_pending: Cell<bool>,
    /// True while remembered tabs are being put back, so sleeping ones are not woken.
    restoring: Cell<bool>,
    /// The tab that was selected before the current one (its screenshot is taken on the switch).
    last_selected: RefCell<Weak<Tab>>,
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
    /// Tint the window's header and tab bar with the workspace colour (light or dark to match the
    /// appearance, which can change while running).
    fn install_color(self: &Rc<Self>, color: Option<&(String, crate::profile::ColorPair)>) {
        let Some((id, pair)) = color else { return };
        let class = format!("ws-{id}");
        self.window.add_css_class(&class);
        let provider = gtk::CssProvider::new();
        let apply = {
            let (provider, class, pair) = (provider.clone(), class.clone(), pair.clone());
            move || provider.load_from_string(&workspace_css(&class, &pair, adw::StyleManager::default().is_dark()))
        };
        apply();
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        adw::StyleManager::default().connect_dark_notify(move |_| apply());
        *self.color_css.borrow_mut() = Some(provider);
    }

    // ---- history ----------------------------------------------------------------------------------

    /// Whether a page may be remembered: history is on for this window, the window is not private,
    /// the address is a web page and not excluded.
    fn remembers(&self, url: &str) -> bool {
        if self.private.get() || !(url.starts_with("http://") || url.starts_with("https://")) {
            return false;
        }
        let h = self.settings.borrow();
        h.history.enabled && !h.history.exclude.iter().any(|p| crate::workspace::scope_matches(p, url))
    }

    /// The window's workspace id (None: an ephemeral window).
    fn workspace_id(&self) -> Option<&str> {
        self.workspace.as_ref().map(|w| w.id.as_str())
    }

    fn record_visit(&self, tab: &Rc<Tab>) {
        let (url, title) = {
            let s = tab.state.borrow();
            (s.address.clone(), s.title.clone())
        };
        if !self.remembers(&url) || *tab.last_recorded.borrow() == url {
            return;
        }
        if let Some(store) = store() {
            if let Err(e) = store.record_visit(self.workspace_id(), &url, &title, now_secs()) {
                eprintln!("browsah: cannot record history: {e}");
            }
        }
        *tab.last_recorded.borrow_mut() = url;
    }

    /// Keep a closed tab so it can be reopened. `group` ties the tabs of one ephemeral window together.
    fn record_closed(&self, tab: &Rc<Tab>, reason: &str, group: Option<i64>) {
        let (url, title) = {
            let s = tab.state.borrow();
            (s.address.clone(), s.title.clone())
        };
        if !self.remembers(&url) {
            return;
        }
        if let Some(store) = store() {
            let _ = store.record_closed(self.workspace_id(), &url, &title, now_secs(), reason, group);
        }
    }

    /// An ephemeral window is closing with its tabs open: keep them together so the window can come back.
    fn record_window_closed(&self) {
        if self.workspace.is_some() {
            return; // a workspace's tabs are remembered as they are
        }
        let group = now_micros();
        for tab in self.tabs.borrow().iter() {
            self.record_closed(tab, "window", Some(group));
        }
    }

    /// Reopen the last closed tab (of this workspace; in an ephemeral window, the last closed window).
    pub fn reopen_closed(self: &Rc<Self>) {
        let Some(store) = store() else { return };
        match self.workspace_id() {
            Some(ws) => match store.closed_tabs(crate::store::Scope::Workspace(ws), 1).first().and_then(|t| store.take_closed(t.id)) {
                Some(t) => {
                    self.open_tab(&t.url, true);
                }
                None => self.toast("No closed tab to reopen"),
            },
            None => {
                let tabs = store.take_last_closed_window();
                let Some((first, rest)) = tabs.split_first() else {
                    self.toast("No closed window to reopen");
                    return;
                };
                if let Some(f) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                    let shell = f(&first.url);
                    for t in rest {
                        shell.open_tab(&t.url, false);
                    }
                }
            }
        }
    }

    // ---- the tab context menu -------------------------------------------------------------------

    /// Fill the right-click menu for `page` (None: the menu closed).
    fn setup_tab_menu(&self, page: Option<&adw::TabPage>) {
        *self.menu_page.borrow_mut() = page.cloned();
        self.tab_menu.remove_all();
        let Some(page) = page else { return };
        let blocked = self.tab_for_page(page).map(|t| t.view().blocked_cookie_sites().len()).unwrap_or(0);
        let cookies = gio::Menu::new();
        let label = if blocked == 0 { "No cookies blocked on this page".to_owned() } else { format!("Copy allow rules for {blocked} blocked cookie site{}", if blocked == 1 { "" } else { "s" }) };
        cookies.append(Some(&label), Some(if blocked == 0 { "win.none" } else { "win.copy-cookie-rules" }));
        cookies.append(Some("Install as web app\u{2026}"), Some("win.install-app"));
        self.tab_menu.append_section(None, &cookies);
        if page.is_pinned() {
            return; // a workspace's pinned tabs stay where they are
        }
        let send = gio::Menu::new();
        let here = self.workspace.as_ref().map(|w| w.id.as_str());
        if let Some(profile) = PROFILE.with(|p| p.borrow().clone()) {
            for w in crate::workspace::Workspace::list(&profile).into_iter().filter(|w| !w.is_app()) {
                if Some(w.id.as_str()) == here {
                    continue;
                }
                let item = gio::MenuItem::new(Some(w.display_name()), None);
                item.set_action_and_target_value(Some("win.send-tab"), Some(&w.id.to_variant()));
                send.append_item(&item);
            }
        }
        let new_window = gio::MenuItem::new(Some("New window"), None);
        new_window.set_action_and_target_value(Some("win.send-tab"), Some(&"".to_variant()));
        send.append_item(&new_window);
        self.tab_menu.append_submenu(Some("Send to workspace"), &send);
        let close = gio::Menu::new();
        if self.sweeper.is_some() {
            close.append(Some("Sweep unused tabs now"), Some("win.sweep-now"));
        }
        close.append(Some("Close tab"), Some("win.close-menu-tab"));
        self.tab_menu.append_section(None, &close);
    }

    /// Move the tab the menu was opened on to workspace `id` (an empty id: a window of its own).
    /// The tab keeps running: its page moves with its browser.
    fn send_menu_tab(&self, id: &str) {
        let Some(page) = self.menu_page.borrow().clone().or_else(|| self.tab_view.selected_page()) else { return };
        if page.is_pinned() || self.workspace.as_ref().is_some_and(|w| w.id == id) {
            return;
        }
        let target = if id.is_empty() {
            NEW_EPHEMERAL.with(|f| f.borrow().clone()).map(|f| f(""))
        } else {
            match OPEN_WORKSPACE.with(|f| f.borrow().clone()).map(|f| f(id, "")) {
                Some(Ok(s)) => Some(s),
                Some(Err(e)) => {
                    self.toast(&e);
                    None
                }
                None => None,
            }
        };
        let Some(target) = target else { return };
        self.tab_view.transfer_page(&page, &target.tab_view, target.tab_view.n_pages());
        target.tab_view.set_selected_page(&page);
        target.window.present();
    }

    // ---- bookmarks ------------------------------------------------------------------------------

    fn bookmark_workspace(&self) -> Option<&str> {
        self.workspace.as_ref().filter(|_| self.nav.bookmarks_bar).map(|w| w.id.as_str())
    }

    /// The address and title to bookmark for the selected tab (not blank pages).
    fn bookmarkable(&self) -> Option<(String, String)> {
        let tab = self.current()?;
        let st = tab.state.borrow();
        if st.address.is_empty() || st.address == "about:blank" {
            return None;
        }
        Some((st.address.clone(), if st.title.is_empty() { st.address.clone() } else { st.title.clone() }))
    }

    /// Bookmark the selected tab's page, or remove the bookmark if it is already there.
    fn toggle_bookmark(&self) {
        let (Some(ws), Some(store), Some((url, title))) = (self.bookmark_workspace(), store(), self.bookmarkable()) else { return };
        let result = if store.has_bookmark(ws, &url) { store.remove_bookmark(ws, &url) } else { store.add_bookmark(ws, &url, &title) };
        if let Err(e) = result {
            eprintln!("browsah: cannot change bookmarks: {e}");
        }
        self.rebuild_bookmarks();
        self.update_star();
    }

    /// The star shows whether the selected page is bookmarked.
    fn update_star(&self) {
        let (Some(ws), Some(store)) = (self.bookmark_workspace(), store()) else { return };
        let marked = self.bookmarkable().is_some_and(|(url, _)| store.has_bookmark(ws, &url));
        self.star.set_icon_name(if marked { "starred-symbolic" } else { "non-starred-symbolic" });
        self.star.set_tooltip_text(Some(if marked { "Remove bookmark (Ctrl+D)" } else { "Bookmark this page (Ctrl+D)" }));
    }

    /// Show the workspace's bookmarks in the bar (hidden when there are none).
    fn rebuild_bookmarks(&self) {
        let (Some(ws), Some(store)) = (self.bookmark_workspace(), store()) else { return };
        while let Some(child) = self.bookmark_bar.first_child() {
            self.bookmark_bar.remove(&child);
        }
        let marks = store.bookmarks(ws);
        let window = self.window.downgrade();
        for b in &marks {
            let label = gtk::Label::builder().label(b.title.as_str()).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(24).build();
            let button = gtk::Button::builder().child(&label).has_frame(false).tooltip_text(b.url.as_str()).build();
            button.add_css_class("bookmark");
            let url = b.url.clone();
            // Click: open here. Middle click or Ctrl+click: a new tab. Right click: a small menu.
            button.connect_clicked({
                let (window, url) = (window.clone(), url.clone());
                move |_| {
                    if let Some(s) = window.upgrade().and_then(|w| shell_of(&w)) {
                        s.open_bookmark(&url, false);
                    }
                }
            });
            let gesture = gtk::GestureClick::builder().button(0).build();
            gesture.connect_pressed({
                let (window, url, button) = (window.clone(), url.clone(), button.clone());
                move |g, _, _, _| {
                    let Some(s) = window.upgrade().and_then(|w| shell_of(&w)) else { return };
                    match g.current_button() {
                        2 => {
                            g.set_state(gtk::EventSequenceState::Claimed);
                            s.open_bookmark(&url, true);
                        }
                        3 => {
                            g.set_state(gtk::EventSequenceState::Claimed);
                            s.bookmark_menu(&button, &url);
                        }
                        _ => {}
                    }
                }
            });
            button.add_controller(gesture);
            self.bookmark_bar.append(&button);
        }
        self.bookmark_revealer.set_reveal_child(!marks.is_empty());
    }

    /// A bookmark was activated: open it in the selected tab, or in a new tab if asked (and allowed).
    fn open_bookmark(self: &Rc<Self>, url: &str, new_tab: bool) {
        if new_tab && self.allow_new_tabs {
            self.open_tab(url, true);
        } else if let Some(t) = self.current() {
            t.view().navigate(url);
        }
    }

    fn bookmark_menu(self: &Rc<Self>, button: &gtk::Button, url: &str) {
        let popover = gtk::Popover::new();
        popover.set_parent(button);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mut entries: Vec<(&str, bool)> = Vec::new();
        if self.allow_new_tabs {
            entries.push(("Open in new tab", false));
        }
        entries.push(("Remove bookmark", true));
        for (label, remove) in entries {
            let b = gtk::Button::builder().label(label).has_frame(false).build();
            b.connect_clicked({
                let (window, url, popover) = (self.window.downgrade(), url.to_owned(), popover.downgrade());
                move |_| {
                    if let Some(p) = popover.upgrade() {
                        p.popdown();
                    }
                    let Some(s) = window.upgrade().and_then(|w| shell_of(&w)) else { return };
                    if remove {
                        if let (Some(ws), Some(store)) = (s.bookmark_workspace(), store()) {
                            let _ = store.remove_bookmark(ws, &url);
                            s.rebuild_bookmarks();
                            s.update_star();
                        }
                    } else {
                        s.open_tab(&url, true);
                    }
                }
            });
            list.append(&b);
        }
        popover.set_child(Some(&list));
        popover.connect_closed(|p| p.unparent());
        popover.popup();
    }

    /// The bookmark buttons currently in the bar (for the self-test).
    pub fn bookmark_buttons(&self) -> Vec<gtk::Button> {
        let mut out = Vec::new();
        let mut child = self.bookmark_bar.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            if let Ok(b) = c.downcast::<gtk::Button>() {
                out.push(b);
            }
        }
        out
    }

    /// Open a workspace's pinned tabs: always present, first, not closable, and limited to their scope.
    fn add_pinned(self: &Rc<Self>, pinned: &[crate::workspace::PinnedTab]) {
        for (i, p) in pinned.iter().enumerate() {
            let tab = self.open_tab_scoped(&p.url, p.scope.clone());
            if let Some(title) = &p.title {
                tab.state.borrow_mut().title = title.clone();
                tab.page.set_title(title);
            }
            self.tab_view.set_page_pinned(&tab.page, true);
            tab.page.set_icon(Some(&gio::ThemedIcon::new("web-browser-symbolic")));
            tab.page.set_tooltip(p.title.as_deref().unwrap_or(&p.url));
            if i == 0 {
                self.tab_view.set_selected_page(&tab.page);
            }
        }
        if !pinned.is_empty() {
            self.on_selected();
        }
    }

    /// A tab that may only navigate inside `scope` (empty: anywhere).
    fn open_tab_scoped(self: &Rc<Self>, url: &str, scope: Vec<String>) -> Rc<Tab> {
        let tab = self.open_tab_with(url, new_uid(), false);
        tab.view().set_scope(scope);
        tab
    }

    // ---- remembered tabs ------------------------------------------------------------------------

    fn screenshot_path(&self, uid: u64) -> Option<std::path::PathBuf> {
        self.persist.as_ref().map(|p| p.shots.join(format!("{uid}.png")))
    }

    /// Save a tab's last frame as its screenshot (workspace windows; a sleeping tab keeps its old
    /// one). The frame is copied at once and the PNG encoded when the main loop is idle.
    fn snapshot(&self, tab: &Rc<Tab>) {
        self.snapshot_with(tab, false);
    }

    /// `now`: encode before returning (the window is closing, so there is no later).
    fn snapshot_with(&self, tab: &Rc<Tab>, now: bool) {
        let Some(path) = self.screenshot_path(tab.uid) else { return };
        let view = tab.view();
        if view.is_dormant() {
            return;
        }
        let Some(frame) = view.capture() else {
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: screenshot of tab {}: the view has no frame", tab.uid);
            }
            return;
        };
        let uid = tab.uid;
        let encode = move || {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let ok = frame.save_to_png(&path).is_ok();
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: screenshot of tab {uid}: {}", if ok { "saved" } else { "not saved" });
            }
        };
        if now {
            encode();
        } else {
            glib::idle_add_local_once(encode);
        }
    }

    /// The tabs to remember, in tab-bar order. Blank tabs are not worth keeping.
    fn tab_records(&self) -> Vec<TabRecord> {
        let selected = self.tab_view.selected_page();
        let tabs = self.tabs.borrow();
        (0..self.tab_view.n_pages())
            .filter_map(|i| {
                let page = self.tab_view.nth_page(i);
                let tab = tabs.iter().find(|t| t.page == page)?;
                let s = tab.state.borrow();
                if s.address.is_empty() || s.address == "about:blank" {
                    return None;
                }
                Some(TabRecord { uid: tab.uid, url: s.address.clone(), title: s.title.clone(), pinned: page.is_pinned(), selected: selected.as_ref() == Some(&page), usage: tab.usage.borrow().to_text() })
            })
            .collect()
    }

    /// Remember the tabs now (and forget screenshots of tabs that are gone).
    pub fn save_now(&self) {
        let (Some(p), Some(store)) = (&self.persist, store()) else { return };
        let records = self.tab_records();
        if let Err(e) = store.save_tabs(&p.workspace, &records) {
            eprintln!("browsah: cannot save tabs of workspace {}: {e}", p.workspace);
            return;
        }
        let keep: std::collections::HashSet<String> = records.iter().map(|r| format!("{}.png", r.uid)).collect();
        if let Ok(rd) = std::fs::read_dir(&p.shots) {
            for e in rd.flatten() {
                if !keep.contains(e.file_name().to_string_lossy().as_ref()) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    /// Remember the tabs soon (changes come in bursts).
    fn schedule_save(&self) {
        if self.persist.is_none() || self.save_pending.replace(true) {
            return;
        }
        let window = self.window.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(s) = window.upgrade().and_then(|w| shell_of(&w)) {
                s.save_pending.set(false);
                s.save_now();
            }
        });
    }

    /// Take the screenshot of the selected tab and save the tabs, for when the window closes.
    fn save_on_close(&self) {
        if let Some(t) = self.current() {
            self.snapshot_with(&t, true);
        }
        self.save_now();
    }

    /// Put remembered tabs back as sleeping tabs (no browser yet, their screenshot shown); the
    /// selected one wakes at once, the others when first shown.
    fn restore(self: &Rc<Self>, records: &[TabRecord]) {
        self.restoring.set(true);
        let mut target = None;
        for rec in records {
            // Every remembered tab starts without a browser (the selected one is woken below, and
            // shows its screenshot while it loads).
            let tab = self.open_tab_dormant(rec);
            if rec.selected {
                target = Some(tab);
            }
        }
        let target = target.or_else(|| self.tabs.borrow().first().cloned());
        self.restoring.set(false);
        if let Some(t) = target {
            self.tab_view.set_selected_page(&t.page);
            self.on_selected(); // wakes it if it was only remembered
        }
    }

    /// A tab for a remembered page: no browser until it is shown.
    fn open_tab_dormant(self: &Rc<Self>, rec: &TabRecord) -> Rc<Tab> {
        let state = Rc::new(RefCell::new(State { address: rec.url.clone(), title: rec.title.clone(), ..State::default() }));
        let cell: Rc<OnceCell<Weak<Tab>>> = Rc::new(OnceCell::new());
        let on_event = self.event_closure(&cell);
        let shot = self.screenshot_path(rec.uid);
        self.prepare_storage();
        let view = BrowserView::new_dormant(WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate.get(), &rec.url, shot.as_deref(), on_event);
        let tab = self.add_tab(view, cell, state, false, rec.uid);
        tab.page.set_title(&tab_title(&tab.state.borrow()));
        // Use is remembered across runs: a tab unused for a week while the browser was closed is idle for a week.
        if !rec.usage.is_empty() {
            *tab.usage.borrow_mut() = crate::sweep::Usage::from_text(&rec.usage);
        }
        tab
    }

    /// Scrolling settings are process-wide statics, so the window being used sets them.
    pub fn apply_input_tuning(&self) {
        let c = self.settings.borrow();
        input::set_scroll_tuning(c.input.wheel_notch, c.input.touchpad_speed, c.input.touchpad_acceleration);
    }

    /// Show a short message over the page.
    /// Tell the next view this window creates where to keep its site data: its own folder for an isolated
    /// workspace, otherwise the profile's.
    /// The folder of this window's own site data, if it has one.
    fn storage_dir(&self) -> Option<std::path::PathBuf> {
        match &self.workspace {
            Some(w) if self.settings.borrow().storage == config::Storage::Isolated => {
                PROFILE.with(|p| p.borrow().as_ref().map(|p| p.cef_dir().join("workspaces").join(&w.id)))
            }
            _ => None,
        }
    }

    /// The window is closing: delete the session cookies it saved.
    pub(crate) fn end_session_cookies(self: &Rc<Self>) {
        view::end_session_cookies(self.storage_dir().as_deref(), self.settings.borrow().cookies.clone(), &self.session_owner, true);
    }

    /// The window has opened: delete session cookies nobody owns (an earlier run left them).
    pub(crate) fn purge_stale_session_cookies(self: &Rc<Self>) {
        view::end_session_cookies(self.storage_dir().as_deref(), self.settings.borrow().cookies.clone(), &self.session_owner, false);
    }

    pub(crate) fn prepare_storage(&self) {
        let dir = self.storage_dir();
        view::set_next_storage(dir);
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Apply the options that can change while running (see `config::changes`): search engine,
    /// frame rate, scrolling. Options fixed at start-up are applied by the next start.
    pub fn apply_config(&self, cfg: &Config) {
        *self.settings.borrow_mut() = cfg.clone();
        for t in self.tabs.borrow().iter() {
            self.apply_view_settings(&t.view());
        }
        for m in self.modals.borrow().iter() {
            self.apply_view_settings(&m.view);
        }
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

    fn tab_for_page(&self, page: &adw::TabPage) -> Option<Rc<Tab>> {
        self.tabs.borrow().iter().find(|t| &t.page == page).cloned()
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
        drop(s);
        self.update_star();
        let s = tab.state.borrow();
        self.back_action.set_enabled(s.can_back && self.nav.back);
        self.forward_action.set_enabled(s.can_forward && self.nav.forward);
        self.reload.set_visible(!s.loading && self.nav.reload);
        self.stop.set_visible(s.loading && self.nav.reload);
    }

    /// Take over a running view (a modal's page moving to this window) as a new selected tab.
    fn adopt_live_view(self: &Rc<Self>, view: Rc<BrowserView>, state: State) -> Rc<Tab> {
        let (cell, state) = (Rc::new(OnceCell::new()), Rc::new(RefCell::new(state)));
        view.set_event_handler(Rc::new(self.event_closure(&cell)));
        let tab = self.add_tab(view, cell, state, true, new_uid());
        tab.page.set_title(&tab_title(&tab.state.borrow()));
        tab
    }

    /// Show a popup browser (one that keeps its opener) as a new selected tab.
    fn adopt_tab(self: &Rc<Self>, spec: view::PopupSpec) -> Rc<Tab> {
        let (cell, state) = (Rc::new(OnceCell::new()), Rc::new(RefCell::new(State::default())));
        let on_event = self.event_closure(&cell);
        let view = BrowserView::adopt(WidgetExt::display(&self.window), spec, on_event);
        self.add_tab(view, cell, state, true, new_uid())
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
        self.prepare_storage();
        let view = BrowserView::new(WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate.get(), url, on_event);
        self.apply_view_settings(&view);
        view.set_popup_policy(self.popup_policy);
        view
    }

    /// Open a tab on `url`. Background tabs start hidden (not rendering) until selected.
    pub fn open_tab(self: &Rc<Self>, url: &str, select: bool) -> Rc<Tab> {
        self.open_tab_with(url, new_uid(), select)
    }

    fn open_tab_with(self: &Rc<Self>, url: &str, uid: u64, select: bool) -> Rc<Tab> {
        let state = Rc::new(RefCell::new(State::default()));
        let cell: Rc<OnceCell<Weak<Tab>>> = Rc::new(OnceCell::new());
        let view = self.make_view(&cell, url);
        self.add_tab(view, cell, state, select, uid)
    }

    /// Put a view in a new tab.
    fn add_tab(self: &Rc<Self>, view: Rc<BrowserView>, cell: Rc<OnceCell<Weak<Tab>>>, state: Rc<RefCell<State>>, select: bool, uid: u64) -> Rc<Tab> {
        view.set_workspace(self.workspace.as_ref().map(|w| w.id.clone()));
        view.set_popup_policy(self.popup_policy);
        self.apply_view_settings(&view);
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
        let tab = Rc::new(Tab { view: RefCell::new(view), holder, sad, state, page: page.clone(), owner: RefCell::new(Rc::downgrade(self)), uid, last_recorded: RefCell::new(String::new()), usage: RefCell::new(crate::sweep::Usage::starting(now_secs())), created: std::cell::Cell::new(now_secs()), dirty: std::cell::Cell::new(false), playing: std::cell::Cell::new(false), input_seen: std::cell::Cell::new(0) });
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
        let current = self.current();
        // The tab we are leaving gets a screenshot.
        if let Some(prev) = self.last_selected.borrow().upgrade() {
            if current.as_ref().is_none_or(|c| !Rc::ptr_eq(c, &prev)) {
                self.snapshot(&prev);
            }
        }
        if let Some(t) = current {
            *self.last_selected.borrow_mut() = Rc::downgrade(&t);
            if !self.restoring.get() {
                t.usage.borrow_mut().record(now_secs(), 1.0); // being shown is use
                t.view().wake(); // a restored tab comes to life when it is first shown
            }
            t.view().widget.grab_focus();
        }
        self.schedule_save();
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
            // A window whose last tab was sent elsewhere has nothing left to show.
            if self.tabs.borrow().is_empty() {
                let window = self.window.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(w) = window.upgrade() {
                        w.close();
                    }
                });
            }
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
            tab.view().set_workspace(self.workspace.as_ref().map(|w| w.id.clone()));
            tab.view().set_popup_policy(self.popup_policy);
            self.apply_view_settings(&tab.view());
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
            let reason = self.pending_close_reason.borrow_mut().take().unwrap_or_else(|| "user".to_owned());
            self.record_closed(&tab, &reason, None);
            tab.view().close();
        }
        self.schedule_save();
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
                if self.popup_policy == crate::workspace::PopupPolicy::Modal {
                    self.open_modal_popup(spec);
                } else if self.allow_new_tabs {
                    self.adopt_tab(spec);
                } else if let Some(factory) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                    // Tabs cannot be opened here: the popup goes to a window of its own, still attached to its opener.
                    factory("").adopt_tab(spec);
                } else {
                    self.adopt_tab(spec);
                }
            }
            ViewEvent::Route { action, url } => {
                perform_route(&action, &url, Some(self));
            }
            ViewEvent::Permission(req) => self.prompt_permission(req),
            ViewEvent::PermissionDismissed(id) => self.dismiss_permission(id),
            ViewEvent::Download { id, name, url, state, cancel } => self.download_event(id, &name, &url, state, cancel),
            ViewEvent::DownloadAsk(q) => self.ask_download(q),
            ViewEvent::Zoom(dir) => self.zoom_step(dir),
            ViewEvent::Fullscreen(on) => self.set_fullscreen(on, true),
            ViewEvent::FindResult { count, active } => self.find_result(tab, count, active),
            ViewEvent::JsDialog(q) => self.show_js_dialog(q),
            ViewEvent::JsDialogDismissed(view) => self.dismiss_js_dialog(view),
            ViewEvent::ExternalLaunch { scheme, url, signin } => {
                // The sign-in handed its result to the application: an ephemeral window with only
                // this tab has nothing more to show.
                if std::env::var_os("BROWSAH_DEBUG").is_some() {
                    eprintln!("browsah: handing a {scheme}: address to the desktop");
                }
                if std::env::var_os("BROWSAH_NO_OS_LAUNCH").is_none() {
                    if let Err(e) = gio::AppInfo::launch_default_for_uri(&url, gio::AppLaunchContext::NONE) {
                        self.toast(&format!("Cannot open {url}: {e}"));
                    }
                }
                if signin && self.workspace.is_none() && self.tab_count() == 1 {
                    let window = self.window.downgrade();
                    glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                        if let Some(w) = window.upgrade() {
                            w.close();
                        }
                    });
                }
            }
            ViewEvent::OutOfScope(url) => {
                if self.allow_new_tabs {
                    self.open_tab(&url, true);
                } else if let Some(factory) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                    factory(&url);
                }
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
                // Navigating to another host resets the zoom: put back the person's zoom for this site.
                self.restore_site_zoom(tab);
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
                    self.record_visit(tab);
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
        self.schedule_save();
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

fn build(app: &adw::Application, opts: &Options, cfg: &Config, start_url: &str, workspace: Option<crate::workspace::Workspace>, remembered_tabs: Vec<TabRecord>) -> Rc<Shell> {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("browsah")
        .default_width(cfg.window.width)
        .default_height(cfg.window.height)
        .build();

    // A workspace window's navigation controls and tab rules; ephemeral windows have everything.
    let ws_config = workspace.as_ref().map(|w| w.config.clone()).unwrap_or_default();
    let workspace_color = workspace.as_ref().and_then(|w| w.config.color.clone().map(|c| (w.id.clone(), c)));
    let nav = ws_config.nav.clone();
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
    back.set_visible(nav.back);
    forward.set_visible(nav.forward);
    match nav.address_bar {
        crate::workspace::AddressBar::Editable => header.set_title_widget(Some(&clamp)),
        crate::workspace::AddressBar::Readonly => {
            entry.set_editable(false);
            header.set_title_widget(Some(&clamp));
        }
        // The header shows the window title (the page title) when it has no title widget.
        crate::workspace::AddressBar::Hidden => {}
    }

    // Bookmarks (workspace windows with `nav.bookmarks_bar`): a star in the header and a bar under the tabs.
    let bookmarks_on = workspace.is_some() && nav.bookmarks_bar;
    let star = icon_button("non-starred-symbolic", "Bookmark this page (Ctrl+D)", "win.bookmark-page");
    star.set_visible(bookmarks_on);
    let downloads_ui = downloads::Ui::new();
    header.pack_end(&downloads_ui.button);
    header.pack_end(&star);
    let bookmark_bar = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(2).margin_start(6).margin_end(6).margin_top(2).margin_bottom(2).build();
    let bookmark_scroll = gtk::ScrolledWindow::builder().child(&bookmark_bar).vscrollbar_policy(gtk::PolicyType::Never).hscrollbar_policy(gtk::PolicyType::Automatic).build();
    let bookmark_revealer = gtk::Revealer::builder().child(&bookmark_scroll).reveal_child(false).transition_type(gtk::RevealerTransitionType::SlideDown).build();

    let tab_view = adw::TabView::new();
    let tab_menu = gio::Menu::new();
    tab_view.set_menu_model(Some(&tab_menu));
    let tab_bar = adw::TabBar::builder().view(&tab_view).autohide(false).build();
    let new_tab_button = icon_button("tab-new-symbolic", "New tab (Ctrl+T)", "win.new-tab");
    new_tab_button.set_visible(nav.new_tab_button && ws_config.tabs.allow_new);
    tab_bar.set_end_action_widget(Some(&new_tab_button));
    tab_bar.set_visible(nav.tab_bar);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&tab_bar);
    toolbar.add_top_bar(&bookmark_revealer);
    let find_bar = find::Bar::new();
    toolbar.add_bottom_bar(&find_bar.revealer);
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
        nav: ws_config.nav.clone(),
        allow_new_tabs: ws_config.tabs.allow_new,
        sweeper: if workspace.is_some() {
            let sweeper = crate::sweep::Sweeper::from_config(&ws_config.sweep);
            for w in sweeper.iter().flat_map(|s| s.warnings.iter()) {
                eprintln!("sweep: {w}");
            }
            sweeper
        } else {
            None
        },
        sweeps_done: Cell::new(0),
        last_sweep: RefCell::new(Vec::new()),
        pending_close_reason: RefCell::new(None),
        private: Cell::new(false),
        suggest: suggest::Popup::new(&entry),
        downloads: downloads_ui.clone(),
        js_queue: RefCell::new(std::collections::VecDeque::new()),
        js_showing: RefCell::new(None),
        js_pending: RefCell::new(None),
        find: find_bar,
        toolbar: toolbar.clone(),
        page_fullscreen: Cell::new(false),
        fullscreen: Cell::new(false),
        session_owner: workspace.as_ref().map(|w| w.id.clone()).unwrap_or_else(|| format!("window-{}", NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed))),
        history: RefCell::new(None),
        prompt_queue: RefCell::new(Default::default()),
        prompt_dialog: RefCell::new(None),
        pending_prompt: RefCell::new(None),
        downloads_seen: RefCell::new(Default::default()),
        completed_downloads: RefCell::new(Vec::new()),
        pending_download: RefCell::new(None),
        modals: RefCell::new(Vec::new()),
        modal_style: ws_config.modals.style,
        popup_policy: ws_config.tabs.popups,
        tab_menu: tab_menu.clone(),
        menu_page: RefCell::new(None),
        bookmark_bar: bookmark_bar.clone(),
        bookmark_revealer: bookmark_revealer.clone(),
        star: star.clone(),
        color_css: RefCell::new(None),
        persist: workspace.as_ref().map(|w| Persist { workspace: w.id.clone(), shots: w.dir.join("shots") }),
        save_pending: Cell::new(false),
        restoring: Cell::new(false),
        last_selected: RefCell::new(Weak::new()),
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
    let new_tab_action = add_action(&window, "new-tab", {
        let (weak, entry) = (weak.clone(), entry.clone());
        move || {
            if let Some(s) = weak.upgrade() {
                // What a new tab opens is an option (`new_tab_page`): a blank tab with the address bar
                // ready, the home page, or an address.
                let page = s.settings.borrow().new_tab_url();
                match page {
                    Some(url) => {
                        s.open_tab(&nav::resolve(&url, &s.search_url.borrow()), true);
                    }
                    None => {
                        s.open_tab("about:blank", true);
                        entry.grab_focus();
                    }
                }
            }
        }
    });
    new_tab_action.set_enabled(ws_config.tabs.allow_new);
    let send_tab = gio::SimpleAction::new("send-tab", Some(glib::VariantTy::STRING));
    send_tab.connect_activate({
        let weak = weak.clone();
        move |_, param| {
            if let (Some(s), Some(id)) = (weak.upgrade(), param.and_then(|p| p.get::<String>())) {
                s.send_menu_tab(&id);
            }
        }
    });
    window.add_action(&send_tab);
    for (name, forward) in [("find-next", true), ("find-prev", false)] {
        add_action(&window, name, {
            let weak = weak.clone();
            move || {
                if let Some(s) = weak.upgrade() {
                    s.find_step(forward);
                }
            }
        });
    }
    add_action(&window, "fullscreen", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.toggle_fullscreen();
            }
        }
    });
    add_action(&window, "devtools", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                if let Some(t) = s.current() {
                    t.view().toggle_dev_tools();
                }
            }
        }
    });
    add_action(&window, "find", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.open_find();
            }
        }
    });
    add_action(&window, "find-close", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.close_find();
            }
        }
    });
    add_action(&window, "zoom-in", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.zoom_step(1);
            }
        }
    });
    add_action(&window, "zoom-out", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.zoom_step(-1);
            }
        }
    });
    add_action(&window, "zoom-reset", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.zoom_step(0);
            }
        }
    });
    add_action(&window, "install-app", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                let page = s.menu_page.borrow().clone().or_else(|| s.tab_view.selected_page());
                if let Some(tab) = page.and_then(|p| s.tab_for_page(&p)) {
                    s.offer_install_app(tab);
                }
            }
        }
    });
    add_action(&window, "copy-cookie-rules", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                let page = s.menu_page.borrow().clone().or_else(|| s.tab_view.selected_page());
                let hosts = page.and_then(|p| s.tab_for_page(&p)).map(|t| t.view().blocked_cookie_sites()).unwrap_or_default();
                if hosts.is_empty() {
                    s.toast("No cookies were blocked on this page");
                } else {
                    gtk::prelude::WidgetExt::display(&s.window).clipboard().set_text(&crate::cookies::allow_rules_yaml(&hosts));
                    s.toast(&format!("Copied allow rules for {} site{}; paste them into a config file", hosts.len(), if hosts.len() == 1 { "" } else { "s" }));
                }
            }
        }
    });
    add_action(&window, "close-menu-tab", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                let page = s.menu_page.borrow().clone().or_else(|| s.tab_view.selected_page());
                if let Some(p) = page {
                    s.tab_view.close_page(&p);
                }
            }
        }
    });
    tab_view.connect_setup_menu({
        let weak = weak.clone();
        move |_, page| {
            if let Some(s) = weak.upgrade() {
                s.setup_tab_menu(page);
            }
        }
    });
    add_action(&window, "sweep-now", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.sweep_now();
            }
        }
    });
    add_action(&window, "history", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.open_history();
            }
        }
    });
    add_action(&window, "reopen-tab", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.reopen_closed();
            }
        }
    });
    add_action(&window, "bookmark-page", {
        let weak = weak.clone();
        move || {
            if let Some(s) = weak.upgrade() {
                s.toggle_bookmark();
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
            // Pinned tabs (a workspace's always-present tabs) are not closable.
            if page.is_pinned() {
                tv.close_page_finish(page, false);
                return glib::Propagation::Stop;
            }
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
                s.close_find(); // a search belongs to the tab it was made in
                s.on_selected();
            }
        }
    });

    app.set_accels_for_action("win.back", &["<Alt>Left"]);
    app.set_accels_for_action("win.forward", &["<Alt>Right"]);
    app.set_accels_for_action("win.reload", &["<Ctrl>R", "F5"]);
    app.set_accels_for_action("win.focus-address", &["<Ctrl>L", "F6"]);
    app.set_accels_for_action("win.new-tab", &["<Ctrl>T"]);
    app.set_accels_for_action("win.bookmark-page", &["<Ctrl>D"]);
    app.set_accels_for_action("win.reopen-tab", &["<Ctrl><Shift>T"]);
    app.set_accels_for_action("win.history", &["<Ctrl>H"]);
    app.set_accels_for_action("win.devtools", &["F12", "<Ctrl><Shift>I"]);
    app.set_accels_for_action("win.fullscreen", &["F11"]);
    app.set_accels_for_action("win.find", &["<Ctrl>F"]);
    app.set_accels_for_action("win.find-next", &["<Ctrl>G", "F3"]);
    app.set_accels_for_action("win.find-prev", &["<Ctrl><Shift>G", "<Shift>F3"]);
    app.set_accels_for_action("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]);
    app.set_accels_for_action("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]);
    app.set_accels_for_action("win.zoom-reset", &["<Ctrl>0", "<Ctrl>KP_0"]);
    app.set_accels_for_action("win.close-tab", &["<Ctrl>W"]);
    app.set_accels_for_action("win.next-tab", &["<Ctrl>Tab", "<Ctrl>Page_Down"]);
    app.set_accels_for_action("win.prev-tab", &["<Ctrl><Shift>Tab", "<Ctrl>ISO_Left_Tab", "<Ctrl>Page_Up"]);

    // Address bar: Enter navigates; typing marks the text as being edited so a page's address
    // change does not overwrite it; Escape or leaving the entry restores the current address.
    entry.connect_activate({
        let shell = Rc::downgrade(&shell);
        move |e| {
            if let Some(shell) = shell.upgrade() {
                // A highlighted suggestion wins over what was typed.
                let target = shell.chosen_suggestion().unwrap_or_else(|| e.text().to_string());
                shell.hide_suggestions();
                shell.go(&target);
            }
        }
    });
    entry.connect_changed({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(shell) = shell.upgrade() {
                if !shell.programmatic.get() {
                    shell.editing.set(true);
                    shell.update_suggestions();
                }
            }
        }
    });
    let restore = {
        let shell = Rc::downgrade(&shell);
        move || {
            if let Some(shell) = shell.upgrade() {
                shell.editing.set(false);
                shell.hide_suggestions();
                let a = shell.current().map(|t| t.state.borrow().address.clone()).unwrap_or_default();
                shell.set_entry_text(address_text(&a));
            }
        }
    };
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let restore = restore.clone();
        let shell = Rc::downgrade(&shell);
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Down || key == gtk::gdk::Key::Up {
                let step = if key == gtk::gdk::Key::Down { 1 } else { -1 };
                if shell.upgrade().is_some_and(|s| s.move_suggestion(step)) {
                    return glib::Propagation::Stop;
                }
            }
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

    // Find bar: typing searches, Enter goes on, Shift+Enter goes back, Escape closes.
    shell.find.entry.connect_search_changed({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(shell) = shell.upgrade() {
                shell.find_run(true, false);
            }
        }
    });
    shell.find.entry.connect_activate({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(shell) = shell.upgrade() {
                shell.find_run(true, true);
            }
        }
    });
    shell.find.entry.connect_stop_search({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(shell) = shell.upgrade() {
                shell.close_find();
            }
        }
    });
    let find_keys = gtk::EventControllerKey::new();
    find_keys.connect_key_pressed({
        let shell = Rc::downgrade(&shell);
        move |_, key, _, state| {
            if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter) && state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                if let Some(shell) = shell.upgrade() {
                    shell.find_run(false, true);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    shell.find.entry.add_controller(find_keys);

    window.connect_close_request({
        let shell = Rc::downgrade(&shell);
        move |_| {
            if let Some(s) = shell.upgrade() {
                s.record_window_closed();
                s.end_session_cookies();
                s.save_on_close(); // before the browsers close
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

    shell.install_color(workspace_color.as_ref());
    shell.connect_suggestion_clicks();
    // A web app's window is its own application to the desktop: its own launcher entry, icon and place in the dash.
    // (GTK sets the profile's id when the window is shown, so this runs after that, when the window is mapped.)
    if shell.workspace.as_ref().is_some_and(|w| w.is_app()) {
        let id = shell.workspace.as_ref().map(|w| w.id.clone()).unwrap_or_default();
        window.connect_map(move |win| {
            use gtk::prelude::NativeExt;
            let Some(profile) = PROFILE.with(|p| p.borrow().clone()) else { return };
            if let Some(top) = win.surface().and_then(|s| s.downcast::<gdk_wayland::WaylandToplevel>().ok()) {
                top.set_application_id(&crate::webapp::app_id(&profile, &id));
            }
        });
    }
    shell.purge_stale_session_cookies();
    shell.rebuild_bookmarks();

    // A workspace's pinned tabs come first.
    let pinned = ws_config.tabs.pinned.clone();
    shell.add_pinned(&pinned);
    if remembered_tabs.is_empty() {
        if !start_url.is_empty() {
            shell.open_tab(start_url, true);
        }
    } else {
        shell.restore(&remembered_tabs);
        if !start_url.is_empty() {
            shell.open_tab(start_url, true); // a link given when opening: after the remembered tabs
        }
    }
    shell.start_sweeping();
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
    match Store::open(&profile.dir.join("data.sqlite")) {
        Ok(s) => STORE.with(|st| *st.borrow_mut() = Some(Rc::new(s))),
        Err(e) => eprintln!("browsah: cannot open the profile database ({e}); tabs will not be remembered"),
    }
    // Workspaces with sweeping rules are swept while their window is closed too.
    if !opts.selftest && !opts.latency {
        let app_file = opts.config_path.as_deref().map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
        sweep::start_background_sweeping(profile.clone(), app_file);
    }
    // Old history is deleted when the browser starts.
    if let (Some(store), days) = (store(), cfg0.history.retention_days) {
        if days > 0 {
            let _ = store.clear_history(Some(now_secs() - i64::from(days) * 86_400));
        }
    }
    // Restored tabs that are still asleep are shown blurred.
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(".dormant { filter: blur(8px); }");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
    // The live configuration: replaced when a config file changes (see `watch`).
    let cfg = Rc::new(RefCell::new(cfg0));
    let watcher: Rc<RefCell<Option<watch::ConfigWatch>>> = Rc::new(RefCell::new(None));

    // Open a window showing `workspace` (None: an ephemeral window) on `start`.
    let new_window = {
        let (opts, cfg, exit, watcher, profile) = (opts.clone(), cfg.clone(), exit.clone(), watcher.clone(), profile.clone());
        move |app: &adw::Application, workspace: Option<crate::workspace::Workspace>, start: &str, restore: Vec<TabRecord>| -> Rc<Shell> {
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
            let s = build(app, &opts, &window_cfg, start, workspace.clone(), restore);
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
        let (cfg, profile, new_window, opts) = (cfg.clone(), profile.clone(), new_window.clone(), opts.clone());
        move |app: &adw::Application, id: &str, url: Option<&str>| -> Result<Rc<Shell>, String> {
            if let Some(s) = windows().into_iter().find(|s| s.workspace.as_ref().is_some_and(|w| w.id == id)) {
                if let Some(u) = url.filter(|u| !u.is_empty()) {
                    s.open_tab(u, true);
                }
                s.window.present();
                return Ok(s);
            }
            let w = crate::workspace::Workspace::resolve(&profile, id)?;
            // A workspace opens as it was; with nothing remembered it opens on a link or its home page.
            // (Pinned tabs come from the configuration each time, not from what was remembered.)
            let remembered: Vec<TabRecord> = store().map(|s| s.load_tabs(&w.id)).unwrap_or_default().into_iter().filter(|r| !r.pinned).collect();
            let start = match url {
                Some(u) => u.to_owned(),
                None if remembered.is_empty() && w.config.tabs.pinned.is_empty() => {
                    // The workspace's own home page, if it sets one.
                    let app_file = opts.config_path.as_deref().map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
                    config::load_window(&app_file, &profile.dir.join("profile.yaml"), Some(&w.dir.join("workspace.yaml"))).map(|l| l.config.home_page).unwrap_or_else(|_| cfg.borrow().home_page.clone())
                }
                None => String::new(),
            };
            Ok(new_window(app, Some(w), &start, remembered))
        }
    };

    PROFILE.with(|p| *p.borrow_mut() = Some(profile.clone()));
    {
        let app_file = opts.config_path.as_deref().map(std::path::PathBuf::from).unwrap_or_else(config::default_path);
        install_router(&app_file, &profile);
    }

    // Lets a locked workspace (no new tabs) send a link or a popup to an ephemeral window.
    NEW_EPHEMERAL.with(|f| {
        let (new_window, app) = (new_window.clone(), app.clone());
        *f.borrow_mut() = Some(Rc::new(move |url: &str| new_window(&app, None, url, Vec::new())));
    });

    OPEN_WORKSPACE.with(|f| {
        let (open_workspace, app) = (open_workspace.clone(), app.clone());
        *f.borrow_mut() = Some(Rc::new(move |id: &str, url: &str| open_workspace(&app, id, Some(url))));
    });

    // Another window with no workspace (Ctrl+Shift+N): nothing about it is remembered.
    let new_ephemeral = gio::SimpleAction::new("new-window", None);
    new_ephemeral.connect_activate({
        let (cfg, new_window, app) = (cfg.clone(), new_window.clone(), app.clone());
        move |_, _| {
            let home = cfg.borrow().home_page.clone();
            new_window(&app, None, &home, Vec::new());
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
                new_window(app, None, &start, Vec::new());
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
            let mut urls: Vec<String> = files.iter().map(|f| nav::resolve(f.uri().as_str(), &search)).collect();
            if std::env::var_os("BROWSAH_DEBUG").is_some() {
                eprintln!("browsah: asked to open {urls:?} (workspace hint {hint:?})");
            }
            // Links from other applications go through the router (a workspace named on the command
            // line is an explicit choice and skips it).
            if hint.is_empty() {
                urls.retain(|url| {
                    let decision = crate::router::route(&crate::router::Request { url: url.clone(), source: crate::router::Source::External, kind: crate::router::Kind::External, target: crate::router::Target::SameTab });
                    if std::env::var_os("BROWSAH_DEBUG").is_some() {
                        eprintln!("browsah: router: {url} -> {:?} ({:?})", decision.action, decision.rule);
                    }
                    if decision.action == crate::router::Action::Allow {
                        return true;
                    }
                    let origin = windows().into_iter().next();
                    let opened = perform_route(&decision.action, url, origin.as_ref());
                    // A sign-in another application started: nothing from its window is remembered, and
                    // it may hand its result to that application through the application's own scheme.
                    if let Some(s) = opened {
                        if crate::router::looks_like_app_sign_in(url) {
                            s.private.set(true);
                            if let (Some(scheme), Some(t)) = (crate::router::sign_in_callback_scheme(url), s.tabs().last()) {
                                t.view().allow_scheme(&scheme);
                            }
                        }
                    }
                    false
                });
                if urls.is_empty() {
                    return;
                }
            }
            if !hint.is_empty() {
                for (i, url) in urls.iter().enumerate() {
                    if let Err(e) = open_workspace(app, hint, Some(url)) {
                        eprintln!("browsah: {e}");
                        if i == 0 {
                            new_window(app, None, url, Vec::new());
                        }
                    }
                }
                return;
            }
            let active = app.active_window().and_then(|w| windows().into_iter().find(|s| s.window.upcast_ref::<gtk::Window>() == &w));
            let existing = active.or_else(|| windows().last().cloned());
            let (s, skip) = match existing {
                Some(s) => (s, 0),
                None => (new_window(app, None, urls.first().map(String::as_str).unwrap_or("about:blank"), Vec::new()), 1),
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

/// Relative luminance of `#rrggbb` (0 dark, 1 light), to choose a readable foreground.
fn luminance(hex: &str) -> f64 {
    let c = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
}

/// The style sheet that gives a workspace window its colour: libadwaita's header colours, set for
/// just this window.
fn workspace_css(class: &str, pair: &crate::profile::ColorPair, dark: bool) -> String {
    let bg = if dark { &pair.dark } else { &pair.light };
    let fg = if luminance(bg) > 0.55 { "rgba(0, 0, 0, 0.8)" } else { "#ffffff" };
    format!(
        "window.{class} {{ --headerbar-bg-color: {bg}; --headerbar-fg-color: {fg}; --headerbar-backdrop-color: {bg}; }}\n\
         window.{class} headerbar, window.{class} headerbar:backdrop, window.{class} tabbar > revealer > box {{ background-color: {bg}; color: {fg}; }}\n\
         window.{class} headerbar button, window.{class} headerbar windowcontrols button {{ color: {fg}; }}"
    )
}

/// Carry out the router's decision for a link. `origin` is the window the link came from (for
/// messages), if any. `Allow` is not handled here: it means "carry on as usual".
fn perform_route(action: &crate::router::Action, url: &str, origin: Option<&Rc<Shell>>) -> Option<Rc<Shell>> {
    use crate::router::Action;
    let say = |text: &str| {
        if let Some(s) = origin {
            s.toast(text);
        }
    };
    match action {
        Action::Allow => {}
        Action::Modal => match origin {
            Some(shell) => shell.open_modal(url),
            None => {
                if let Some(f) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                    return Some(f(url));
                }
            }
        },
        Action::Block => say(&format!("Blocked a link to {}", url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url))),
        Action::Ephemeral => {
            if let Some(f) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                return Some(f(url));
            }
        }
        Action::Workspace(id) => match OPEN_WORKSPACE.with(|f| f.borrow().clone()).map(|f| f(id, url)) {
            Some(Ok(s)) => return Some(s),
            Some(Err(e)) => say(&e),
            None => {}
        },
        Action::System => {
            // A web address would come straight back here, so the desktop is only asked for other schemes.
            if url.starts_with("http://") || url.starts_with("https://") {
                if let Some(f) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                    f(url);
                }
            } else if let Err(e) = gio::AppInfo::launch_default_for_uri(url, gio::AppLaunchContext::NONE) {
                say(&format!("Cannot open {url}: {e}"));
            }
        }
        Action::Profile { profile, workspace } => {
            // The other profile's process takes the link (it is started if it is not running).
            let Ok(exe) = std::env::current_exe() else { return None };
            let mut cmd = std::process::Command::new(exe);
            cmd.arg(format!("--profile={profile}"));
            if let Some(w) = workspace {
                cmd.arg(format!("--workspace={w}"));
            }
            cmd.arg(url);
            if let Err(e) = cmd.spawn() {
                say(&format!("Cannot start profile {profile}: {e}"));
            }
        }
    }
    None
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn now_micros() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

/// The shell of a window.
fn shell_of(window: &adw::ApplicationWindow) -> Option<Rc<Shell>> {
    WINDOWS.with(|w| w.borrow().iter().find(|s| &s.window == window).cloned())
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

/// Load the routing rules (application, profile and its workspaces) and install them.
fn install_router(app_file: &std::path::Path, profile: &crate::profile::Profile) {
    let router = crate::router::Router::load(app_file, Some(profile));
    for w in &router.warnings {
        eprintln!("routing: {w}");
    }
    let known: Vec<String> = crate::workspace::Workspace::list(profile).into_iter().map(|w| w.id).collect();
    for w in router.referenced_workspaces() {
        if !known.contains(&w) {
            eprintln!("routing: a rule opens workspace {w:?}, which does not exist in this profile");
        }
    }
    crate::router::install(router);
}

/// A config file changed: load each window's effective configuration again and apply what can
/// change live. An invalid file is reported and the running configuration kept.
fn reload(cfg: &Rc<RefCell<Config>>, app_file: &std::path::Path, profile_file: &std::path::Path, watcher: &Rc<RefCell<Option<watch::ConfigWatch>>>) {
    let shells = windows();
    let debug = std::env::var_os("BROWSAH_DEBUG").is_some();
    if let Some(profile) = PROFILE.with(|p| p.borrow().clone()) {
        install_router(app_file, &profile); // routing rules reload with the rest
    }
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
