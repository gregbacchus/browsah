//! The one way to draw a menu (read AGENTS.md, "Menus", before adding one).
//!
//! Every menu is a `gtk::Popover` of left-aligned rows: a Lucide icon (or a 16 px spacer, so labels line up), the label in regular
//! weight, the shortcut dimmed on the right, a chevron on a submenu. GTK's own `PopoverMenu` cannot show an icon beside a label, so a
//! menu described as a `gio::MenuModel` is *drawn by us* with `popover`: the model carries each item's Lucide id in the `ICON`
//! attribute (and a group colour dot in `DOT`), sections become separators and submenus become pages of a `Stack`.
//!
//! A popover made here is an ordinary autohiding `Popover`: the caller parents it, shows it, and unparents it when it is closed.
//! Never open two popovers for one click (docs/research/tab-menu-grab.md).

use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::rc::Rc;

/// The menu item attribute that names the item's Lucide icon (`crate::icons`, e.g. `"printer"`).
pub(super) const ICON: &str = "browsah-icon";
/// The menu item attribute that holds a colour (`#rrggbb`), drawn as a dot in place of an icon (a tab group's colour).
pub(super) const DOT: &str = "browsah-dot";

/// A menu item that runs `action` (a detailed name such as `win.print`), with an optional Lucide icon.
pub(super) fn item(label: &str, action: &str, icon: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    if let Some(icon) = icon {
        item.set_attribute_value(ICON, Some(&icon.to_variant()));
    }
    item
}

/// A menu item that runs `action` with `target`.
pub(super) fn targeted(label: &str, action: &str, target: &glib::Variant, icon: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(target));
    if let Some(icon) = icon {
        item.set_attribute_value(ICON, Some(&icon.to_variant()));
    }
    item
}

/// The 16 px leading image of a row: the icon, or empty space of the same size.
pub(super) fn icon_widget(icon: Option<&str>) -> gtk::Widget {
    match icon.and_then(crate::icons::by_id) {
        Some(icon) => {
            let image = gtk::Image::new();
            image.set_pixel_size(16);
            crate::icons::bind(&image, icon);
            image.upcast()
        }
        None => gtk::Box::builder().width_request(16).build().upcast(),
    }
}

/// One row: `lead` (an icon, a badge or a dot), the label left-aligned, then the shortcut, a tick, and a chevron for a submenu.
pub(super) fn row(label: &str, lead: &impl IsA<gtk::Widget>, shortcut: Option<&str>, checked: bool, enabled: bool, chevron: bool) -> gtk::Button {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
    row.append(lead);
    row.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(36).build());
    if let Some(keys) = shortcut {
        row.append(&gtk::Label::builder().label(keys).css_classes(["dim-label", "caption"]).margin_start(18).build());
    }
    if checked {
        row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
    }
    if chevron {
        let image = gtk::Image::new();
        image.set_pixel_size(16);
        crate::icons::bind(&image, crate::icons::CHEVRON_RIGHT);
        row.append(&image);
    }
    gtk::Button::builder().child(&row).has_frame(false).sensitive(enabled).build()
}

fn dot(color: &str) -> gtk::Image {
    let icon = gio::BytesIcon::new(&glib::Bytes::from_owned(crate::groups::dot_svg(color).into_bytes()));
    let image = gtk::Image::from_gicon(&icon);
    image.set_pixel_size(16);
    image
}

/// The shortcut shown for an action that takes no target, from the application's accelerators.
fn shortcut(window: &adw::ApplicationWindow, action: &str) -> Option<String> {
    let accel = window.application()?.accels_for_action(action).into_iter().next()?;
    let (key, mods) = gtk::accelerator_parse(&accel)?;
    Some(gtk::accelerator_get_label(key, mods).to_string())
}

/// Whether the action exists and is enabled (`win.` and `app.` names); a missing one is shown dimmed, like GTK's menus do.
fn enabled(window: &adw::ApplicationWindow, action: &str) -> bool {
    let found = match action.split_once('.') {
        Some(("win", name)) => window.lookup_action(name),
        Some(("app", name)) => window.application().and_then(|a| a.lookup_action(name)),
        _ => None,
    };
    found.is_some_and(|a| a.is_enabled())
}

struct Ctx {
    window: glib::WeakRef<adw::ApplicationWindow>,
    popover: glib::WeakRef<gtk::Popover>,
    stack: gtk::Stack,
    pages: Cell<u32>,
    width: i32,
}

impl Ctx {
    fn label(model: &gio::MenuModel, i: i32, attribute: &str) -> Option<String> {
        model.item_attribute_value(i, attribute, None).and_then(|v| v.str().map(str::to_owned))
    }

    /// Fill `page` with the rows of `model`: sections are separated, submenus open their own page.
    fn fill(self: &Rc<Self>, model: &gio::MenuModel, page: &gtk::Box) {
        let mut last_was_section = None;
        for i in 0..model.n_items() {
            if let Some(section) = model.item_link(i, "section") {
                if section.n_items() == 0 {
                    continue;
                }
                if page.first_child().is_some() {
                    page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                }
                self.fill(&section, page);
                last_was_section = Some(true);
                continue;
            }
            // An item outside any section: set it apart from the sections around it.
            if last_was_section == Some(true) && page.first_child().is_some() {
                page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            last_was_section = Some(false);
            let label = Self::label(model, i, "label").unwrap_or_default();
            let icon = Self::label(model, i, ICON);
            let lead: gtk::Widget = match Self::label(model, i, DOT) {
                Some(color) => dot(&color).upcast(),
                None => icon_widget(icon.as_deref()),
            };
            if let Some(submenu) = model.item_link(i, "submenu") {
                page.append(&self.submenu_row(&label, &lead, &submenu));
                continue;
            }
            let action = Self::label(model, i, "action");
            let target = model.item_attribute_value(i, "target", None);
            let usable = action.as_deref().is_some_and(|a| self.window.upgrade().is_some_and(|w| enabled(&w, a)));
            let keys = match (&action, &target, self.window.upgrade()) {
                (Some(a), None, Some(w)) => shortcut(&w, a),
                _ => None,
            };
            let button = row(&label, &lead, keys.as_deref(), false, usable, false);
            if let Some(action) = action {
                let (window, popover) = (self.window.clone(), self.popover.clone());
                button.connect_clicked(move |_| {
                    // Run the action first, then close: the closing handler may clear what the action reads.
                    if let Some(window) = window.upgrade() {
                        let _ = WidgetExt::activate_action(&window, &action, target.as_ref());
                    }
                    if let Some(p) = popover.upgrade() {
                        p.popdown();
                    }
                });
            }
            page.append(&button);
        }
    }

    fn submenu_row(self: &Rc<Self>, label: &str, lead: &gtk::Widget, submenu: &gio::MenuModel) -> gtk::Button {
        self.pages.set(self.pages.get() + 1);
        let name = format!("sub{}", self.pages.get());
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.set_width_request(self.width);
        let back = row(label, &icon_widget(Some("chevron-left")), None, false, true, false);
        back.connect_clicked({
            let stack = self.stack.downgrade();
            move |_| {
                if let Some(s) = stack.upgrade() {
                    s.set_visible_child_name("main");
                }
            }
        });
        page.append(&back);
        page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        self.fill(submenu, &page);
        self.stack.add_named(&page, Some(&name));
        // The row that opens it needs its own copy of the lead image: a widget has one parent.
        let lead = match lead.downcast_ref::<gtk::Image>().and_then(|i| i.gicon()) {
            Some(gicon) => {
                let image = gtk::Image::from_gicon(&gicon);
                image.set_pixel_size(16);
                image.upcast()
            }
            None => lead.clone(),
        };
        let opener = row(label, &lead, None, false, true, true);
        opener.connect_clicked({
            let stack = self.stack.downgrade();
            move |_| {
                if let Some(s) = stack.upgrade() {
                    s.set_visible_child_name(&name);
                }
            }
        });
        opener
    }
}

/// A popover showing `model` as rows. Actions run on `window` (`win.` and `app.` names). The popover has no parent yet: the caller
/// sets it, positions it, unparents it from `closed`, and pops it up.
pub(super) fn popover(model: &impl IsA<gio::MenuModel>, window: &adw::ApplicationWindow, width: i32) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    let stack = gtk::Stack::builder().vhomogeneous(false).hhomogeneous(false).build();
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.set_width_request(width);
    let ctx = Rc::new(Ctx { window: window.downgrade(), popover: popover.downgrade(), stack: stack.clone(), pages: Cell::new(0), width });
    ctx.fill(model.upcast_ref(), &main);
    stack.add_named(&main, Some("main"));
    stack.set_visible_child_name("main");
    popover.set_child(Some(&stack));
    popover
}
