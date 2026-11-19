//! The workspace picker: a button at the start of a window's header bar with a list of the profile's workspaces, each with a
//! badge in its colour, ordered by last use or alphabetically (the choice is remembered in the profile). The model (ordering,
//! badges) is `crate::picker`.

use super::{shell_of, store, windows, PROFILE};
use crate::picker::{self, Entry, Order};
use adw::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

/// The key of the order in the profile's preferences.
const ORDER_PREF: &str = "workspace_order";

pub struct Ui {
    pub button: gtk::ToggleButton,
    pub panel: gtk::Box,
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
        let panel = gtk::Box::builder().css_classes(["browsah-workspace-picker"])
            .halign(gtk::Align::Start).valign(gtk::Align::Start).visible(false).build();
        panel.append(&content);
        let button = gtk::ToggleButton::builder().tooltip_text("Workspaces").build();
        crate::icons::bind(&button, crate::icons::SWATCH_BOOK);
        let ui = Rc::new(Ui { button, panel, list, recent, alpha, current, syncing: std::cell::Cell::new(false) });
        ui.button.connect_toggled({
            let ui = Rc::downgrade(&ui);
            move |button| {
                let Some(ui) = ui.upgrade() else { return };
                ui.panel.set_visible(button.is_active());
                if button.is_active() {
                    ui.rebuild();
                    ui.panel.child_focus(gtk::DirectionType::TabForward);
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

    /// Keep the picker on the window's Wayland surface so its input serial can activate another toplevel.
    pub fn attach(self: &Rc<Self>, overlay: &gtk::Overlay, header: &adw::HeaderBar) {
        overlay.add_overlay(&self.panel);
        overlay.connect_get_child_position({
            let header = header.downgrade();
            let panel = self.panel.downgrade();
            move |overlay, child| {
                if panel.upgrade().as_ref().map(|p| p.upcast_ref::<gtk::Widget>()) != Some(child) {
                    return None;
                }
                let bounds = header.upgrade()?.compute_bounds(overlay)?;
                let bottom = (bounds.y() + bounds.height()).ceil() as i32;
                let (_, width, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let width = width.min((overlay.width() - 16).max(1));
                let (_, height, _, _) = child.measure(gtk::Orientation::Vertical, width);
                let x = if overlay.direction() == gtk::TextDirection::Rtl { overlay.width() - width - 8 } else { 8 };
                Some(gtk::gdk::Rectangle::new(x, bottom + 4, width, height))
            }
        });

        let outside_click = gtk::GestureClick::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        outside_click.connect_pressed({
            let ui = Rc::downgrade(self);
            let overlay = overlay.downgrade();
            move |_, _, x, y| {
                let (Some(ui), Some(overlay)) = (ui.upgrade(), overlay.upgrade()) else { return };
                if ui.button.is_active() && !overlay.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|widget| {
                    widget == ui.panel || widget.is_ancestor(&ui.panel) || widget == ui.button || widget.is_ancestor(&ui.button)
                }) {
                    ui.button.set_active(false);
                }
            }
        });
        overlay.add_controller(outside_click);

        let keys = gtk::EventControllerKey::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        keys.connect_key_pressed({
            let ui = Rc::downgrade(self);
            move |_, key, _, _| {
                if let Some(ui) = ui.upgrade().filter(|ui| key == gtk::gdk::Key::Escape && ui.button.is_active()) {
                    ui.button.set_active(false);
                    ui.button.grab_focus();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        overlay.add_controller(keys);
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
        let open = windows();
        let entries = entries();
        let choices = std::iter::once((None, "No workspace", None))
            .chain(entries.iter().map(|e| (Some(e.id.as_str()), e.name.as_str(), e.color.as_ref())));
        for (id, name, color) in choices {
            let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
            row.append(&badge(picker::badge_color(color, dark)));
            row.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let is_current = self.current.as_deref() == id;
            if !is_current && open.iter().any(|s| !s.private.get() && s.workspace_id() == id) {
                row.append(&gtk::Label::builder().label("open").css_classes(["dim-label", "caption"]).build());
            }
            if is_current {
                row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            let b = gtk::Button::builder().child(&row).has_frame(false).tooltip_text(id.unwrap_or(name)).build();
            b.set_widget_name(id.unwrap_or("No workspace"));
            b.connect_clicked({
                let toggle = self.button.downgrade();
                let id = id.map(str::to_owned);
                move |button| {
                    let source = button.root().and_downcast::<adw::ApplicationWindow>();
                    let target = source.as_ref().and_then(|w| w.application()).and_then(|app| {
                        // GTK lists the windows in most-recently-focused order.
                        app.windows().into_iter().filter_map(|w| w.downcast::<adw::ApplicationWindow>().ok())
                            .filter_map(|w| shell_of(&w))
                            .find(|s| !s.private.get() && s.workspace_id() == id.as_deref())
                    });
                    if let Some(toggle) = toggle.upgrade() {
                        toggle.set_active(false);
                        toggle.grab_focus();
                    }
                    if let Some(target) = target {
                        if Some(&target.window) != source.as_ref() {
                            target.window.unminimize();
                            target.window.present();
                        }
                    } else if let Some(id) = &id {
                        let _ = gtk::prelude::WidgetExt::activate_action(button, "app.open-workspace", Some(&id.to_variant()));
                    } else {
                        let _ = gtk::prelude::WidgetExt::activate_action(button, "app.new-window", None);
                    }
                }
            });
            self.list.append(&b);
        }
    }
}
