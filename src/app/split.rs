//! Split tabs: one tab shows two pages side by side (left and right, never more, never stacked). The tab bar, its tab and the
//! address bar are the tab's; the panes are only the page area. The left pane is the tab's own view (`Tab::view`, its `state`);
//! the right pane is a second `BrowserView` with a state of its own (`Pane`). The address bar, the navigation buttons, find,
//! zoom and the other page commands act on the *focused* pane (`Tab::focused_view`).
//!
//! Every event of the right view arrives through `Shell::split_event_closure`, which resolves the pane: what a pane alone can
//! say (address, title, loading, closing) stays in the pane, the rest (popups, downloads, dialogs) is the shell's as for any page.
//! A split is not remembered across restarts, its right page is not written to the history, and a split tab is never swept or put to sleep.
//! docs/research/split-panes.md.

use super::*;

/// Which pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
    /// The tab's own view.
    #[default]
    Left,
    /// The second view.
    Right,
}

/// The radius of a pane's corners when the window has no colour frame to take it from.
pub const PANE_RADIUS: i32 = 10;
/// The focus ring: the card's own border, inside it (`MENU_ITEM_CSS`). With a frame of thickness `b` around the page the split's gap and edges stay `b`
/// wide: the outer padding is `b - RING` while a split tab is shown (`split-active` on the frame) and the separator is `b - 2 * RING` (`workspace_css`).
pub const RING: i32 = 2;

/// The right pane and the widgets that lay the two out.
pub struct Pane {
    pub view: Rc<BrowserView>,
    pub state: Rc<RefCell<State>>,
    /// The paned widget in the tab's holder.
    paned: gtk::Paned,
    /// What holds each view: the rounded, clipped card.
    left: gtk::Box,
    right: gtk::Box,
}

impl Tab {
    /// The right pane, if the tab is split.
    pub fn split_pane(&self) -> Option<Rc<Pane>> {
        self.split.borrow().clone()
    }

    pub fn is_split(&self) -> bool {
        self.split.borrow().is_some()
    }

    /// The pane the address bar and the page commands act on (always the left one in a tab that is not split).
    pub fn focused_side(&self) -> Side {
        if self.is_split() {
            self.focus.get()
        } else {
            Side::Left
        }
    }

    pub fn focused_view(&self) -> Rc<BrowserView> {
        match (self.focused_side(), self.split_pane()) {
            (Side::Right, Some(p)) => p.view.clone(),
            _ => self.view(),
        }
    }

    pub fn focused_state(&self) -> Rc<RefCell<State>> {
        match (self.focused_side(), self.split_pane()) {
            (Side::Right, Some(p)) => p.state.clone(),
            _ => self.state.clone(),
        }
    }

    /// Every view the tab has (the left one first).
    pub fn views(&self) -> Vec<Rc<BrowserView>> {
        let mut v = vec![self.view()];
        v.extend(self.split_pane().map(|p| p.view.clone()));
        v
    }
}

fn card() -> gtk::Box {
    let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).vexpand(true).css_classes(["browsah-pane"]).build();
    card.set_overflow(gtk::Overflow::Hidden); // the page is clipped to the card's rounded corners
    card
}

impl Shell {
    /// What the right view's events call: they reach the tab through `Shell::handle_split`.
    pub(super) fn split_event_closure(self: &Rc<Self>, cell: &Rc<OnceCell<Weak<Tab>>>) -> impl Fn(ViewEvent) + 'static {
        let cell = cell.clone();
        move |ev: ViewEvent| {
            if let Some(tab) = cell.get().and_then(|w| w.upgrade()) {
                if let Some(shell) = tab.owner() {
                    shell.handle_split(&tab, ev);
                }
            }
        }
    }

    /// Open `url` in the selected tab's right pane: a new split, or, if the tab is split already, the right page moves on.
    /// The right pane gets the focus either way.
    pub fn open_in_split(self: &Rc<Self>, url: &str) {
        let Some(tab) = self.current() else { return };
        self.open_split_in(&tab, url);
    }

    pub(super) fn open_split_in(self: &Rc<Self>, tab: &Rc<Tab>, url: &str) {
        if let Some(p) = tab.split_pane() {
            p.view.navigate(url);
            self.focus_pane(tab, Side::Right);
            p.view.widget.grab_focus();
            return;
        }
        let cell: Rc<OnceCell<Weak<Tab>>> = Rc::new(OnceCell::new());
        let _ = cell.set(Rc::downgrade(tab));
        self.prepare_storage();
        let view = BrowserView::new(WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate.get(), url, self.split_event_closure(&cell));
        view.set_workspace(self.workspace.as_ref().map(|w| w.id.clone()));
        view.set_popup_policy(self.popup_policy);
        view.set_scope(tab.view().scope());
        self.apply_view_settings(&view);
        let state = Rc::new(RefCell::new(State::default()));
        let (left, right) = (card(), card());
        let main = tab.view();
        tab.holder.remove(&main.widget);
        left.append(&main.widget);
        right.append(&view.widget);
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .wide_handle(true)
            .hexpand(true)
            .vexpand(true)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .css_classes(["browsah-split"])
            .build();
        paned.set_start_child(Some(&left));
        paned.set_end_child(Some(&right));
        let width = self.tab_view.width();
        paned.set_position(if width > 0 { width / 2 } else { 400 });
        tab.holder.prepend(&paned);
        let pane = Rc::new(Pane { view: view.clone(), state, paned, left: left.clone(), right: right.clone() });
        *tab.split.borrow_mut() = Some(pane);
        tab.focus.set(Side::Right);
        for (side, card) in [(Side::Left, &left), (Side::Right, &right)] {
            let weak = Rc::downgrade(tab);
            let shell = Rc::downgrade(self);
            let click = gtk::GestureClick::new();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            click.connect_pressed({
                let (weak, shell) = (weak.clone(), shell.clone());
                move |_, _, _, _| {
                    if let (Some(tab), Some(shell)) = (weak.upgrade(), shell.upgrade()) {
                        shell.focus_pane(&tab, side);
                    }
                }
            });
            card.add_controller(click);
            let focus = gtk::EventControllerFocus::new();
            focus.connect_enter(move |_| {
                if let (Some(tab), Some(shell)) = (weak.upgrade(), shell.upgrade()) {
                    shell.focus_pane(&tab, side);
                }
            });
            card.add_controller(focus);
        }
        self.mark_focused_pane(tab);
        view.widget.grab_focus();
        crate::log_info!("split: {url} opened beside {}", tab.state.borrow().address);
        self.refresh_split_button();
        if self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) {
            self.refresh();
        }
    }

    /// Open the page the focused pane shows in a new right pane (the window's split button).
    pub fn split_current(self: &Rc<Self>) {
        let Some(tab) = self.current() else { return };
        let address = tab.focused_state().borrow().address.clone();
        let url = if address.is_empty() { "about:blank".to_owned() } else { address };
        self.open_split_in(&tab, &url);
        if url == "about:blank" {
            self.entry.grab_focus(); // nothing to show yet: the address bar is the next thing the person wants
        }
    }

    /// Give a pane the focus: the address bar, the buttons and the page commands follow it.
    pub(super) fn focus_pane(&self, tab: &Rc<Tab>, side: Side) {
        if !tab.is_split() || tab.focus.get() == side {
            return;
        }
        tab.focus.set(side);
        self.mark_focused_pane(tab);
        self.editing.set(false);
        if self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) {
            self.refresh();
        }
    }

    /// Apply `split.inactive` to this window's panes (live: on start and whenever the configuration reloads).
    pub(crate) fn refresh_split_css(&self) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if self.split_css.borrow().is_none() {
            // One class per window, so each window's provider styles only its own panes.
            self.tab_view.add_css_class(&format!("browsah-split-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            let provider = gtk::CssProvider::new();
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }
            *self.split_css.borrow_mut() = Some(provider);
        }
        let class = self.tab_view.css_classes().iter().find(|c| c.starts_with("browsah-split-")).map(|c| c.to_string()).unwrap_or_default();
        if let Some(provider) = self.split_css.borrow().as_ref() {
            provider.load_from_string(&self.settings.borrow().split.inactive.css(&class));
        }
    }

    /// The focused pane's card is drawn marked.
    fn mark_focused_pane(&self, tab: &Rc<Tab>) {
        let Some(p) = tab.split_pane() else { return };
        for (side, card) in [(Side::Left, &p.left), (Side::Right, &p.right)] {
            if tab.focus.get() == side {
                card.add_css_class("focused");
                card.remove_css_class("inactive");
            } else {
                card.remove_css_class("focused");
                card.add_css_class("inactive");
            }
        }
    }

    /// Close one pane of the selected tab. The other pane becomes the tab's only page: closing the left pane promotes the right
    /// view (and its address, title and state) to the tab's own, so nothing of the tab bar changes but the page it shows.
    pub fn close_split_pane(self: &Rc<Self>, side: Side) {
        let Some(tab) = self.current() else { return };
        self.close_pane_of(&tab, side);
    }

    pub(super) fn close_pane_of(self: &Rc<Self>, tab: &Rc<Tab>, side: Side) {
        let Some(p) = tab.split_pane() else { return };
        let (dying, survivor) = match side {
            Side::Right => (p.view.clone(), tab.view()),
            Side::Left => (tab.view(), p.view.clone()),
        };
        crate::log_info!("split: closing the {side:?} pane ({})", if side == Side::Left { tab.state.borrow().address.clone() } else { p.state.borrow().address.clone() });
        *tab.split.borrow_mut() = None;
        // The closing browser still reports an address and a title; its events must reach nobody.
        dying.retire();
        let (dying_card, survivor_card) = if side == Side::Right { (&p.right, &p.left) } else { (&p.left, &p.right) };
        dying_card.remove(&dying.widget);
        survivor_card.remove(&survivor.widget);
        p.paned.set_start_child(None::<&gtk::Widget>);
        p.paned.set_end_child(None::<&gtk::Widget>);
        tab.holder.remove(&p.paned);
        tab.holder.prepend(&survivor.widget);
        if side == Side::Left {
            // The right page is the tab's page now: its events go the way the tab's own do, and its state is the tab's.
            let cell: Rc<OnceCell<Weak<Tab>>> = Rc::new(OnceCell::new());
            let _ = cell.set(Rc::downgrade(tab));
            survivor.set_event_handler(Rc::new(self.event_closure(&cell)));
            *tab.state.borrow_mut() = std::mem::take(&mut *p.state.borrow_mut());
            *tab.view.borrow_mut() = survivor.clone();
            tab.hist_back.borrow_mut().clear(); // they led to the page that went
            tab.hist_fwd.borrow_mut().clear();
            *tab.first_url.borrow_mut() = tab.state.borrow().address.clone();
            self.favicon_for_address(tab);
            let s = tab.state.borrow();
            tab.page.set_title(&tab_title(&s));
            tab.page.set_loading(s.loading);
        }
        tab.focus.set(Side::Left);
        if self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) {
            survivor.widget.grab_focus();
            self.refresh();
        }
        self.refresh_split_button();
        self.schedule_save();
    }

    /// Close the pane that has the focus.
    pub fn close_focused_pane(self: &Rc<Self>) {
        let Some(tab) = self.current() else { return };
        self.close_pane_of(&tab, tab.focused_side());
    }

    /// The right pane's events.
    fn handle_split(self: &Rc<Self>, tab: &Rc<Tab>, ev: ViewEvent) {
        let Some(p) = tab.split_pane() else { return }; // the pane has gone: a closing browser's last words
        match ev {
            ViewEvent::Ready => {
                p.state.borrow_mut().ready = true;
                p.view.sync_focus();
            }
            ViewEvent::ContextMenu(req) => {
                {
                    let mut s = p.state.borrow_mut();
                    s.menu = req.items.iter().map(|i| if i.separator { "-".into() } else { i.label.clone() }).collect();
                    s.menus += 1;
                }
                self.show_page_menu(&p.view, req, false);
            }
            ViewEvent::Address(a) => {
                p.state.borrow_mut().address = a.clone();
                self.restore_site_zoom_of(&p.view, &a);
                p.view.apply_zoom();
            }
            ViewEvent::Title(t) => p.state.borrow_mut().title = t,
            ViewEvent::NavStart { .. } | ViewEvent::Favicon(_) | ViewEvent::CloseCancelled => {}
            ViewEvent::Loading { loading, can_back, can_forward } => {
                if !loading {
                    p.view.refocus();
                    p.view.apply_zoom();
                }
                let mut s = p.state.borrow_mut();
                if s.loading && !loading {
                    s.load_cycles += 1;
                }
                s.loading = loading;
                s.can_back = can_back;
                s.can_forward = can_forward;
            }
            ViewEvent::Crashed(why) => {
                eprintln!("browsah: the split pane's renderer ended: {why}");
                p.state.borrow_mut().crashed = true;
            }
            // The page closed itself (`window.close()`) or Chromium closed it: the pane goes.
            ViewEvent::Closed => {
                if !self.all_closing.get() {
                    self.close_pane_of(tab, Side::Right);
                }
                return;
            }
            ViewEvent::ClipboardWrite(item) => p.view.write_page_clipboard(item),
            ViewEvent::Route { action, url } => {
                perform_route(&action, &url, Some(self));
            }
            // Popups, permissions, downloads, dialogs, new tabs and the rest are the window's, whichever pane they come from.
            other => self.handle(tab, other),
        }
        if tab.focus.get() == Side::Right && self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) {
            self.refresh();
        }
    }

    /// The split button: it offers a split in a tab that is not split and the two "close pane" choices in one that is.
    pub(super) fn refresh_split_button(&self) {
        let split = self.current().is_some_and(|t| t.is_split());
        self.split_button.set_tooltip_text(Some(if split { "Split panes" } else { "Split the page into two panes" }));
        if let Some(frame) = self.tab_view.parent() {
            if split {
                frame.add_css_class("split-active");
            } else {
                frame.remove_css_class("split-active");
            }
        }
        if split {
            self.split_button.add_css_class("active");
        } else {
            self.split_button.remove_css_class("active");
        }
    }

    /// The split button's popover while the tab is split.
    pub(super) fn show_split_menu(self: &Rc<Self>) {
        let menu = gio::Menu::new();
        menu.append_item(&menu::item("Close left pane", "win.close-left-pane", Some("x")));
        menu.append_item(&menu::item("Close right pane", "win.close-right-pane", Some("x")));
        let popover = menu::popover(&menu, &self.window, 220);
        popover.set_parent(&self.split_button);
        popover.set_has_arrow(false);
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }
}
