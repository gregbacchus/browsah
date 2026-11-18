//! Modals: a page opened over a window to be looked at and closed (a link in an email, say),
//! with a read-only address bar and a way to move it to a workspace.
//!
//! A modal is a browser view of its own, shown in an `adw::Dialog` over the window (or in a
//! window of its own, `modals.style: window`). Its page keeps running when it is moved: the view
//! is taken out of the dialog and put in a tab of the destination window.

use super::view::{BrowserView, PopupSpec, ViewEvent};
use super::{perform_route, Shell, State, NEW_EPHEMERAL, OPEN_WORKSPACE, PROFILE};
use crate::router::{self, Action, Kind, Request, Source, Target};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};

pub struct Modal {
    pub view: Rc<BrowserView>,
    pub state: Rc<RefCell<State>>,
    owner: Weak<Shell>,
    pub entry: gtk::Entry,
    toolbar: adw::ToolbarView,
    pub(super) dialog: Option<adw::Dialog>,
    pub(super) window: Option<adw::Window>,
    moved: Cell<bool>,
    closed: Cell<bool>,
}

/// Where "move" sends the page.
enum Dest {
    /// Wherever the router would send the link if it were not a modal.
    Router,
    Workspace(String),
}

impl Shell {
    /// Open `url` as a modal over this window.
    pub fn open_modal(self: &Rc<Self>, url: &str) {
        let cell: Rc<OnceCell<Weak<Modal>>> = Rc::new(OnceCell::new());
        self.prepare_storage();
        let view = BrowserView::new(gtk::prelude::WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate.get(), url, Self::modal_events(&cell));
        self.present_modal(view, cell, url);
    }

    /// Open a popup a page created (it keeps its opener) as a modal over this window.
    pub fn open_modal_popup(self: &Rc<Self>, spec: PopupSpec) {
        let cell: Rc<OnceCell<Weak<Modal>>> = Rc::new(OnceCell::new());
        let view = BrowserView::adopt(gtk::prelude::WidgetExt::display(&self.window), spec, Self::modal_events(&cell));
        self.present_modal(view, cell, "");
    }

    fn modal_events(cell: &Rc<OnceCell<Weak<Modal>>>) -> impl Fn(ViewEvent) + 'static {
        let cell = cell.clone();
        move |ev| {
            if let Some(m) = cell.get().and_then(|w| w.upgrade()) {
                m.handle(ev);
            }
        }
    }

    fn present_modal(self: &Rc<Self>, view: Rc<BrowserView>, cell: Rc<OnceCell<Weak<Modal>>>, url: &str) {
        view.set_workspace(self.workspace.as_ref().map(|w| w.id.clone()));
        self.apply_view_settings(&view);
        let entry = gtk::Entry::builder().editable(false).hexpand(true).text(url).build();
        let clamp = adw::Clamp::builder().maximum_size(800).tightening_threshold(500).child(&entry).build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&clamp));
        let state = Rc::new(RefCell::new(State { address: url.to_owned(), ..State::default() }));

        // Move: to where the router says, or to a workspace chosen from the menu.
        let move_button = gtk::Button::builder().label("Move to workspace").tooltip_text("Open this page where the routing rules send it, as a tab").build();
        let choose = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text("Choose a workspace").build();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let popover = gtk::Popover::new();
        header.pack_start(&move_button);
        header.pack_start(&choose);
        popover.set_child(Some(&list));
        choose.set_popover(Some(&popover));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&view.widget));

        let (dialog, window) = match self.modal_style {
            crate::workspace::ModalStyle::Overlay => {
                let d = adw::Dialog::builder().title("").child(&toolbar).content_width(1100).content_height(760).follows_content_size(false).build();
                (Some(d), None)
            }
            crate::workspace::ModalStyle::Window => {
                let w = adw::Window::builder().content(&toolbar).modal(true).transient_for(&self.window).default_width(1100).default_height(760).build();
                (None, Some(w))
            }
        };
        let modal = Rc::new(Modal { view, state, owner: Rc::downgrade(self), entry, toolbar, dialog, window, moved: Cell::new(false), closed: Cell::new(false) });
        let _ = cell.set(Rc::downgrade(&modal));

        // The workspaces to choose from.
        if let Some(profile) = PROFILE.with(|p| p.borrow().clone()) {
            let here = self.workspace.as_ref().map(|w| w.id.clone());
            for w in crate::workspace::Workspace::list(&profile) {
                let b = gtk::Button::builder().label(w.display_name()).has_frame(false).build();
                let (m, id, popover) = (Rc::downgrade(&modal), w.id.clone(), popover.downgrade());
                b.connect_clicked(move |_| {
                    if let Some(p) = popover.upgrade() {
                        p.popdown();
                    }
                    if let Some(m) = m.upgrade() {
                        m.move_to(Dest::Workspace(id.clone()));
                    }
                });
                if here.as_deref() != Some(w.id.as_str()) {
                    list.append(&b);
                }
            }
        }
        move_button.connect_clicked({
            let m = Rc::downgrade(&modal);
            move |_| {
                if let Some(m) = m.upgrade() {
                    m.move_to(Dest::Router);
                }
            }
        });

        self.modals.borrow_mut().push(modal.clone());
        if let Some(d) = &modal.dialog {
            d.connect_closed({
                let m = Rc::downgrade(&modal);
                move |_| {
                    if let Some(m) = m.upgrade() {
                        m.on_closed();
                    }
                }
            });
            d.present(Some(&self.window));
        }
        if let Some(w) = &modal.window {
            w.connect_close_request({
                let m = Rc::downgrade(&modal);
                move |_| {
                    if let Some(m) = m.upgrade() {
                        m.on_closed();
                    }
                    glib::Propagation::Proceed
                }
            });
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed({
                let w = w.downgrade();
                move |_, key, _, _| {
                    if key == gtk::gdk::Key::Escape {
                        if let Some(w) = w.upgrade() {
                            w.close();
                        }
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                }
            });
            w.add_controller(keys);
            w.present();
        }
    }

    /// The modals currently open over this window (the self-test).
    pub fn modal_list(&self) -> Vec<Rc<Modal>> {
        self.modals.borrow().clone()
    }
}

impl Modal {
    fn set_title(&self, title: &str) {
        if let Some(d) = &self.dialog {
            d.set_title(title);
        }
        if let Some(w) = &self.window {
            w.set_title(Some(title));
        }
    }

    /// Close the dialog or window.
    fn dismiss(&self) {
        if let Some(d) = &self.dialog {
            d.force_close();
        }
        if let Some(w) = &self.window {
            w.close();
        }
    }

    /// Close the modal and its page.
    pub fn close(&self) {
        self.dismiss();
    }

    fn on_closed(&self) {
        if self.closed.replace(true) {
            return;
        }
        if !self.moved.get() {
            self.view.close(); // the page goes with the modal
        }
        if let Some(owner) = self.owner.upgrade() {
            owner.modals.borrow_mut().retain(|m| !std::ptr::eq(m.as_ref(), self));
        }
    }

    fn handle(self: &Rc<Self>, ev: ViewEvent) {
        match ev {
            ViewEvent::Ready => self.view.sync_focus(),
            ViewEvent::Address(a) => {
                self.entry.set_text(if a == "about:blank" { "" } else { &a });
                self.state.borrow_mut().address = a;
                self.view.apply_zoom();
            }
            ViewEvent::Title(t) => {
                self.set_title(&t);
                self.state.borrow_mut().title = t;
            }
            ViewEvent::Loading { loading, .. } => {
                self.state.borrow_mut().loading = loading;
                if !loading {
                    self.view.apply_zoom();
                }
            }
            ViewEvent::ContextMenu(req) => self.view.show_context_menu(req),
            // Questions and downloads from a modal's page are the window's business.
            ViewEvent::Permission(req) => {
                if let Some(o) = self.owner.upgrade() {
                    o.prompt_permission(req);
                }
            }
            ViewEvent::PermissionDismissed(id) => {
                if let Some(o) = self.owner.upgrade() {
                    o.dismiss_permission(id);
                }
            }
            ViewEvent::Download { id, name, url, state, cancel } => {
                if let Some(o) = self.owner.upgrade() {
                    o.download_event(id, &name, &url, state, cancel);
                }
            }
            ViewEvent::Zoom(_) | ViewEvent::FindResult { .. } | ViewEvent::Fullscreen(_) | ViewEvent::PdfSaved { .. } | ViewEvent::Favicon(_) => {}
            // A copy button in a modal's page, if the person has just used it (see `Shell::handle`).
            ViewEvent::ClipboardWrite(text) => {
                if self.view.used_within(std::time::Duration::from_secs(5)) {
                    gtk::prelude::WidgetExt::display(&self.view.widget).clipboard().set_text(&text);
                }
            }
            ViewEvent::JsDialog(q) => {
                if let Some(o) = self.owner.upgrade() {
                    o.show_js_dialog(q);
                }
            }
            // The page closed itself (`window.close()`, as a sign-in popup does when it is done): the modal goes too,
            // otherwise its last frame stays frozen over the window.
            ViewEvent::Closed => self.dismiss(),
            ViewEvent::CloseCancelled => {}
            ViewEvent::FileDialog(q) => {
                if let Some(o) = self.owner.upgrade() {
                    o.show_file_dialog(q);
                } else {
                    q.cancel();
                }
            }
            ViewEvent::JsDialogDismissed(view) => {
                if let Some(o) = self.owner.upgrade() {
                    o.dismiss_js_dialog(view);
                }
            }
            ViewEvent::DownloadAsk(q) => {
                if let Some(o) = self.owner.upgrade() {
                    o.ask_download(q);
                }
            }
            // Links in a modal are routed like any link from its window.
            ViewEvent::Route { action, url } => {
                let owner = self.owner.upgrade();
                perform_route(&action, &url, owner.as_ref());
            }
            // A popup of a modal becomes a tab of the window the modal is over.
            ViewEvent::Popup(spec) => {
                if let Some(owner) = self.owner.upgrade() {
                    owner.adopt_tab(spec);
                }
            }
            _ => {}
        }
    }

    /// What the "Move to workspace" button does (the self-test presses it).
    pub(super) fn move_via_router(self: &Rc<Self>) {
        self.move_to(Dest::Router);
    }

    /// Move the page out of the modal into a tab (of this window, another workspace's, or a new window).
    fn move_to(self: &Rc<Self>, dest: Dest) {
        let Some(owner) = self.owner.upgrade() else { return };
        let url = {
            let s = self.state.borrow();
            s.address.clone()
        };
        let target: Option<Rc<Shell>> = match dest {
            Dest::Workspace(id) => OPEN_WORKSPACE.with(|f| f.borrow().clone()).and_then(|f| f(&id, "").ok()),
            Dest::Router => {
                let decision = router::route_ignoring_modal(&Request {
                    url: url.clone(),
                    source: Source::Tab(owner.workspace.as_ref().map(|w| w.id.clone())),
                    kind: Kind::Link,
                    target: Target::NewTab,
                });
                match decision.action {
                    Action::Workspace(id) => OPEN_WORKSPACE.with(|f| f.borrow().clone()).and_then(|f| f(&id, "").ok()),
                    Action::Ephemeral => NEW_EPHEMERAL.with(|f| f.borrow().clone()).map(|f| f("")),
                    Action::Block => None,
                    // Another profile or the desktop takes the address, not the running page.
                    // (and a web app is handed the address too: it delivers it as its own settings say)
                    Action::System | Action::Profile { .. } | Action::App(_) => {
                        perform_route(&decision.action, &url, Some(&owner));
                        self.dismiss();
                        return;
                    }
                    Action::Allow | Action::Modal => Some(owner.clone()),
                }
            }
        };
        let Some(target) = target else { return };
        // Take the page out of the dialog, put it in the destination's tabs, then close the dialog.
        self.moved.set(true);
        self.toolbar.set_content(None::<&gtk::Widget>);
        target.adopt_live_view(self.view.clone(), self.state.borrow().clone());
        target.window.present();
        self.dismiss();
    }
}
