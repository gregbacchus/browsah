//! The workspace picker: a button at the start of a window's header bar with a list of the profile's workspaces, each with a
//! badge in its colour, ordered by last use or alphabetically (the choice is remembered in the profile). The model (ordering,
//! badges) is `crate::picker`.

use super::{store, windows, PROFILE};
use crate::picker::{self, Entry, Order};
use adw::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

/// The key of the order in the profile's preferences.
const ORDER_PREF: &str = "workspace_order";

pub struct Ui {
    pub button: gtk::MenuButton,
    pub popover: gtk::Popover,
    /// The workspace rows.
    pub list: gtk::Box,
    pub recent: gtk::ToggleButton,
    pub alpha: gtk::ToggleButton,
    /// The workspace this window shows, marked in the list.
    current: Option<String>,
    /// The toggles are being set from the remembered order: their signals are not the person's choice.
    syncing: std::cell::Cell<bool>,
}

/// The remembered order.
pub fn order() -> Order {
    Order::from_pref(store().and_then(|s| s.pref(ORDER_PREF)).as_deref())
}

/// Note that a workspace is being used now (its window opened or was focused), for the last-used order.
pub fn touch(workspace: &str) {
    if let Some(s) = store() {
        let _ = s.touch_workspace(workspace, super::now_secs());
    }
}

/// The profile's workspaces (not web apps), in the remembered order.
pub fn entries() -> Vec<Entry> {
    let Some(profile) = PROFILE.with(|p| p.borrow().clone()) else { return Vec::new() };
    let mut entries: Vec<Entry> = crate::workspace::Workspace::list(&profile).iter().map(|w| Entry { id: w.id.clone(), name: w.display_name().to_owned(), color: w.config.color.clone() }).collect();
    let used = store().map(|s| s.workspace_last_used()).unwrap_or_default();
    picker::sort(&mut entries, order(), &used);
    entries
}

fn badge(color: Option<&str>) -> gtk::Image {
    let icon = gio::BytesIcon::new(&glib::Bytes::from_owned(picker::badge_svg(color).into_bytes()));
    let image = gtk::Image::from_gicon(&icon);
    image.set_pixel_size(16);
    image
}

impl Ui {
    /// `current` is the workspace of the window this belongs to (marked in the list).
    pub fn new(current: Option<String>) -> Rc<Ui> {
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        let scroller = gtk::ScrolledWindow::builder().child(&list).propagate_natural_height(true).max_content_height(360).min_content_width(260).hscrollbar_policy(gtk::PolicyType::Never).build();
        let recent = gtk::ToggleButton::builder().label("Recent").build();
        let alpha = gtk::ToggleButton::builder().label("A\u{2013}Z").group(&recent).build();
        let switch = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).css_classes(["linked"]).halign(gtk::Align::Center).build();
        switch.append(&recent);
        switch.append(&alpha);
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
        content.set_size_request(300, -1);
        content.append(&scroller);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&switch);
        let popover = gtk::Popover::builder().child(&content).build();
        let button = gtk::MenuButton::builder().icon_name("view-app-grid-symbolic").tooltip_text("Workspaces").popover(&popover).build();
        let ui = Rc::new(Ui { button, popover, list, recent, alpha, current, syncing: std::cell::Cell::new(false) });
        // The list is built when it is opened, so it shows the workspaces and the last use as they are now.
        ui.popover.connect_show({
            let ui = Rc::downgrade(&ui);
            move |_| {
                if let Some(ui) = ui.upgrade() {
                    ui.rebuild();
                }
            }
        });
        for (button, wanted) in [(&ui.recent, Order::LastUsed), (&ui.alpha, Order::Alphabetical)] {
            button.connect_toggled({
                let ui = Rc::downgrade(&ui);
                move |b| {
                    if let (true, Some(ui)) = (b.is_active(), ui.upgrade()) {
                        if !ui.syncing.get() {
                            ui.set_order(wanted);
                        }
                    }
                }
            });
        }
        ui
    }

    /// Choose the order and remember it (for every window of the profile).
    pub fn set_order(&self, wanted: Order) {
        if order() != wanted {
            if let Some(s) = store() {
                let _ = s.set_pref(ORDER_PREF, wanted.as_str());
            }
        }
        self.rebuild();
    }

    /// Fill the list from the profile's workspaces in the remembered order.
    pub fn rebuild(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let (current_order, dark) = (order(), adw::StyleManager::default().is_dark());
        // Setting the toggles from the remembered order is not a choice (and must not rebuild the list again).
        self.syncing.set(true);
        for (b, o) in [(&self.recent, Order::LastUsed), (&self.alpha, Order::Alphabetical)] {
            b.set_active(current_order == o);
        }
        self.syncing.set(false);
        let open: Vec<String> = windows().iter().filter_map(|s| s.workspace.as_ref().map(|w| w.id.clone())).collect();
        let entries = entries();
        if entries.is_empty() {
            self.list.append(&gtk::Label::builder().label("No workspaces yet").css_classes(["dim-label"]).margin_top(8).margin_bottom(8).build());
        }
        for e in entries {
            let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
            row.append(&badge(picker::badge_color(e.color.as_ref(), dark)));
            row.append(&gtk::Label::builder().label(e.name.as_str()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let is_current = self.current.as_deref() == Some(e.id.as_str());
            if open.contains(&e.id) && !is_current {
                row.append(&gtk::Label::builder().label("open").css_classes(["dim-label", "caption"]).build());
            }
            if is_current {
                row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            let b = gtk::Button::builder().child(&row).has_frame(false).tooltip_text(e.id.as_str()).build();
            b.set_widget_name(&e.id);
            b.connect_clicked({
                let popover = self.popover.downgrade();
                move |button| {
                    // Release the popover's input grab before asking Wayland to raise the window.
                    if let Some(p) = popover.upgrade() {
                        p.popdown();
                    }
                    let _ = gtk::prelude::WidgetExt::activate_action(button, "app.open-workspace", Some(&e.id.to_variant()));
                }
            });
            self.list.append(&b);
        }
    }
}
