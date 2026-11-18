//! The "recently swept" button: next to the reload button, shown while sweeping has closed tabs of this window's workspace
//! lately (`recently_swept`), with a list to bring them back. The model is `sweep::recently_swept`; the closed tabs are the
//! profile's history (`closed_tabs`, reason `sweep:<rule>`), so restoring one removes it from there as Ctrl+Shift+T does.

use super::{now_secs, store, Shell};
use crate::store::{ClosedTab, Scope};
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;

pub struct Ui {
    pub button: gtk::MenuButton,
    pub popover: gtk::Popover,
    /// One button per swept tab.
    pub list: gtk::Box,
    restore_all: gtk::Button,
}

impl Ui {
    pub fn new() -> Rc<Ui> {
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        let scroller = gtk::ScrolledWindow::builder().child(&list).propagate_natural_height(true).max_content_height(360).min_content_width(360).hscrollbar_policy(gtk::PolicyType::Never).build();
        let restore_all = gtk::Button::builder().label("Restore all").build();
        let heading = gtk::Label::builder().label("Recently swept tabs").xalign(0.0).css_classes(["heading"]).margin_start(6).build();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
        content.set_size_request(380, -1);
        content.append(&heading);
        content.append(&scroller);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&restore_all);
        let popover = gtk::Popover::builder().child(&content).build();
        let button = gtk::MenuButton::builder().tooltip_text("Recently swept tabs").popover(&popover).visible(false).build();
        crate::icons::bind(&button, crate::icons::SWEPT_BROOM);
        Rc::new(Ui { button, popover, list, restore_all })
    }
}

impl Shell {
    /// The tabs to list now: this window's workspace's swept tabs within the configured time, newest first.
    pub fn recently_swept_tabs(&self) -> Vec<ClosedTab> {
        let cfg = self.settings.borrow().recently_swept.clone();
        let Some(store) = store() else { return Vec::new() };
        let scope = match self.workspace_id() {
            Some(w) => Scope::Workspace(w),
            None => Scope::Ephemeral,
        };
        let all = store.closed_tabs(scope, 200);
        crate::sweep::recently_swept(&all, now_secs(), cfg.within_secs(), cfg.max_tabs).into_iter().cloned().collect()
    }

    /// Show or hide the button, and (when shown) fill its list. Called when a sweep ends, a tab is restored, the window is
    /// used again and the configuration changes.
    pub fn refresh_swept(&self) {
        let Some(me) = super::shell_of(&self.window) else { return };
        let ui = self.swept.clone();
        let tabs = if self.settings.borrow().recently_swept.enabled { self.recently_swept_tabs() } else { Vec::new() };
        ui.button.set_visible(!tabs.is_empty());
        while let Some(c) = ui.list.first_child() {
            ui.list.remove(&c);
        }
        let now = now_secs();
        for t in &tabs {
            let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
            if let Some(icon) = super::favicon::small_image(&t.url) {
                row.append(&icon);
            } else {
                row.append(&gtk::Image::from_icon_name("web-browser-symbolic"));
            }
            let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
            let title = if t.title.is_empty() { t.url.as_str() } else { t.title.as_str() };
            text.append(&gtk::Label::builder().label(title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(40).build());
            let rule = t.reason.strip_prefix("sweep:").unwrap_or(&t.reason);
            text.append(&gtk::Label::builder().label(format!("{} \u{b7} {rule}", crate::history::when(t.at, now))).xalign(0.0).css_classes(["dim-label", "caption"]).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(40).build());
            row.append(&text);
            let b = gtk::Button::builder().child(&row).has_frame(false).tooltip_text(t.url.as_str()).build();
            let (id, weak) = (t.id, Rc::downgrade(&me));
            b.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.restore_swept(&[id]);
                }
            });
            ui.list.append(&b);
        }
        let ids: Vec<i64> = tabs.iter().map(|t| t.id).collect();
        // (A fresh handler each time: the list changes, so the ids do.)
        ui.restore_all.set_sensitive(!ids.is_empty());
        if let Some(old) = self.swept_all_handler.take() {
            ui.restore_all.disconnect(old);
        }
        let weak = Rc::downgrade(&me);
        let handler = ui.restore_all.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.restore_swept(&ids);
            }
        });
        self.swept_all_handler.set(Some(handler));
    }

    /// Open the listed swept tabs (each leaves the history's closed list) in this window, the last one selected.
    pub fn restore_swept(self: &Rc<Self>, ids: &[i64]) {
        self.swept.popover.popdown();
        let Some(store) = store() else { return };
        let tabs: Vec<ClosedTab> = ids.iter().filter_map(|id| store.take_closed(*id)).collect();
        for (i, t) in tabs.iter().enumerate() {
            crate::log_info!("swept tab restored: {} (rule {})", t.url, t.reason);
            self.open_tab(&t.url, i + 1 == tabs.len());
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(s) = weak.upgrade() {
                s.refresh_swept();
            }
        });
    }
}
