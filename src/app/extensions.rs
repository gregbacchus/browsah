//! Chrome extensions in the window: a button in the header for each loaded extension that has a toolbar popup, which opens the
//! popup page (`chrome-extension://<id>/popup/index.html`) in a popover. The popup is a browser view like a modal's, made when
//! the popover opens and closed with it (a popup is meant to be short-lived: Chrome does the same).
//!
//! Chromium itself loads the extensions (`crate::extensions` decides which); here is only the popup's window. Extensions see no
//! tabs yet (`chrome.tabs.query` is empty), which 1Password does not need to fill a login (docs/research/chrome-extensions.md).

use super::view::{BrowserView, ViewEvent};
use super::{perform_route, Shell};
use crate::extensions::Extension;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// The size of a popup's view. Chrome sizes the popup to its page (up to 800 x 600); the extension pages browsah targets
/// (1Password's) lay themselves out for about this size.
pub const POPUP_SIZE: (i32, i32) = (400, 600);

/// One extension's button and popover.
pub struct Host {
    ext: Extension,
    pub button: gtk::MenuButton,
    pub popover: gtk::Popover,
    /// Fixes the popup's size: a page's picture is as big as its frame, which would otherwise make the popover as big as the frame.
    holder: gtk::ScrolledWindow,
    view: RefCell<Option<Rc<BrowserView>>>,
    shell: RefCell<Weak<Shell>>,
}

/// A window's extension buttons.
pub struct Ui {
    /// Holds the buttons, hidden when there are none.
    pub bar: gtk::Box,
    hosts: Vec<Rc<Host>>,
}

/// The address of an extension's popup page.
pub fn popup_url(ext: &Extension) -> Option<String> {
    ext.popup.as_ref().map(|p| format!("chrome-extension://{}/{}", ext.id, p.trim_start_matches('/')))
}

impl Host {
    fn new(ext: &Extension) -> Rc<Host> {
        let holder = gtk::ScrolledWindow::builder()
            // Not `Never`: GTK then ignores the minimum content size. The page's view can shrink, so no scrollbar ever shows.
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_width(POPUP_SIZE.0)
            .min_content_height(POPUP_SIZE.1)
            .propagate_natural_width(false)
            .propagate_natural_height(false)
            .build();
        let popover = gtk::Popover::builder().child(&holder).build();
        let button = gtk::MenuButton::builder().tooltip_text(ext.name.as_str()).popover(&popover).build();
        button.add_css_class("flat");
        match ext.icon.as_ref().and_then(|p| gtk::gdk::Texture::from_filename(p).ok()) {
            Some(texture) => button.set_child(Some(&gtk::Image::builder().paintable(&texture).pixel_size(20).build())),
            None => button.set_icon_name("application-x-addon-symbolic"),
        }
        Rc::new(Host { ext: ext.clone(), button, popover, holder, view: RefCell::new(None), shell: RefCell::new(Weak::new()) })
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.popover.connect_show(move |_| {
            if let Some(h) = weak.upgrade() {
                h.open_view();
            }
        });
        let weak = Rc::downgrade(self);
        self.popover.connect_closed(move |_| {
            if let Some(h) = weak.upgrade() {
                h.close_view();
            }
        });
    }

    /// Make the popup's view (the popover has just opened).
    fn open_view(self: &Rc<Self>) {
        let (Some(shell), Some(url)) = (self.shell.borrow().upgrade(), popup_url(&self.ext)) else { return };
        self.close_view();
        let weak = Rc::downgrade(self);
        let view = BrowserView::new(gtk::prelude::WidgetExt::display(&shell.window), shell.mode.get(), shell.backing, shell.frame_rate.get(), &url, move |ev| {
            if let Some(h) = weak.upgrade() {
                h.handle(ev);
            }
        });
        shell.apply_view_settings(&view);
        self.holder.set_child(Some(&view.widget));
        *self.view.borrow_mut() = Some(view);
    }

    /// The popover has closed: the page goes with it.
    fn close_view(&self) {
        let view = self.view.borrow_mut().take();
        if let Some(view) = view {
            view.close();
            self.holder.set_child(None::<&gtk::Widget>);
        }
    }

    fn handle(self: &Rc<Self>, ev: ViewEvent) {
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        let view = self.view.borrow().clone();
        match ev {
            ViewEvent::Ready => {
                if let Some(v) = &view {
                    v.sync_focus();
                }
            }
            ViewEvent::ContextMenu(req) => {
                if let Some(v) = &view {
                    v.show_context_menu(req);
                }
            }
            // Questions and downloads from the popup's page are the window's business, as for a modal.
            ViewEvent::Permission(req) => shell.prompt_permission(req),
            ViewEvent::PermissionDismissed(id) => shell.dismiss_permission(id),
            ViewEvent::Download { id, name, url, state, cancel } => shell.download_event(id, &name, &url, state, cancel),
            ViewEvent::DownloadAsk(q) => shell.ask_download(q),
            ViewEvent::JsDialog(q) => shell.show_js_dialog(q),
            ViewEvent::JsDialogDismissed(v) => shell.dismiss_js_dialog(v),
            ViewEvent::FileDialog(q) => shell.show_file_dialog(q),
            // A link the popup opens (1Password's "Open website") is a page of the window, not of the popup.
            ViewEvent::OpenTab { url, select } => {
                self.popover.popdown();
                if shell.allow_new_tabs {
                    shell.open_tab(&url, select);
                } else {
                    shell.open_modal(&url);
                }
            }
            ViewEvent::Route { action, url } => {
                self.popover.popdown();
                perform_route(&action, &url, Some(&shell));
            }
            ViewEvent::Popup(spec) => {
                self.popover.popdown();
                shell.adopt_tab(spec);
            }
            // The page closed itself (`window.close()`).
            ViewEvent::Closed => self.popover.popdown(),
            _ => {}
        }
    }
}

impl Ui {
    /// The buttons for the extensions this process loaded that have a popup.
    pub fn new() -> Rc<Ui> {
        let bar = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).build();
        let hosts: Vec<Rc<Host>> = crate::extensions::loaded().iter().filter(|e| e.popup.is_some()).map(Host::new).collect();
        for h in &hosts {
            bar.append(&h.button);
        }
        bar.set_visible(!hosts.is_empty());
        Rc::new(Ui { bar, hosts })
    }

    /// Connect the popovers to the window (once the window exists).
    pub fn attach(&self, shell: &Rc<Shell>) {
        for h in &self.hosts {
            *h.shell.borrow_mut() = Rc::downgrade(shell);
            h.connect();
        }
    }

    /// Show or hide each button for this window (`crate::extensions::shown_in_window`). A button that goes while its popup is open closes the popup.
    fn set_visible(&self, shown: impl Fn(&Extension) -> bool) {
        let mut any = false;
        for h in &self.hosts {
            let show = shown(&h.ext);
            any |= show;
            h.button.set_visible(show);
            if !show {
                h.popover.popdown();
            }
        }
        self.bar.set_visible(any);
    }

    /// The button of the extension with this id (the self-test opens its popup).
    #[allow(dead_code)] // the self-test
    pub fn host(&self, id: &str) -> Option<&Rc<Host>> {
        self.hosts.iter().find(|h| h.ext.id == id)
    }
}

impl Shell {
    /// Show the extension buttons this window's configuration allows: a workspace window shows all but `extensions.hide`, an app's
    /// window only `extensions.show`, and a window with its own storage none. Called when the window opens and when the
    /// configuration reloads.
    pub(super) fn refresh_extension_buttons(&self) {
        let cfg = self.settings.borrow();
        let is_app = self.workspace.as_ref().is_some_and(|w| w.is_app());
        let own_storage = self.workspace.is_some() && cfg.storage == crate::config::Storage::Isolated;
        self.extensions.set_visible(|e| crate::extensions::shown_in_window(&e.id, &cfg.extensions, is_app, own_storage));
    }
}

impl Host {
    /// The popup's view while the popover is open (the self-test reads the page).
    #[allow(dead_code)] // the self-test
    pub fn view(&self) -> Option<Rc<BrowserView>> {
        self.view.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_popup_address_is_the_extension_page() {
        let mut e = Extension { id: "a".repeat(32), name: "x".into(), version: "1".into(), dir: "/x".into(), popup: Some("popup/index.html".into()), icon: None, installed: true, native_messaging: false };
        assert_eq!(popup_url(&e).as_deref(), Some(format!("chrome-extension://{}/popup/index.html", "a".repeat(32)).as_str()));
        e.popup = Some("/p.html".into());
        assert!(popup_url(&e).unwrap().ends_with("/p.html") && !popup_url(&e).unwrap().contains("//p.html"));
        e.popup = None;
        assert_eq!(popup_url(&e), None);
    }
}
