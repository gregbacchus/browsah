//! Tab groups in the window. A group takes the room of one tab in the tab bar: the group's *front* tab, painted in the group's
//! colour and marked with a drop-down arrow. Clicking it opens a list of the group's tabs; choosing one swaps it into the
//! bar. The other tabs are detached from the tab view but kept alive (`parked`), so nothing is lost. The model and helpers
//! are `crate::groups`.

use super::{menu, tab_of_page, tab_title, Shell, Tab};
use crate::groups::{color_class, dot_svg, next_color, next_id, Group, PALETTE};
use crate::store::GroupRecord;
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

thread_local! {
    /// The colours the tab bar's style sheet already paints, and the sheet itself.
    static PAINTED: RefCell<(HashSet<String>, Option<gtk::CssProvider>)> = RefCell::new((HashSet::new(), None));
}

/// Make sure the display's style sheet paints tabs of `color` (the class is `color_class(color)`).
fn paint_rule(color: &str) {
    let Ok(rgba) = gtk::gdk::RGBA::parse(color) else { return };
    let class = color_class(color);
    PAINTED.with(|p| {
        let mut p = p.borrow_mut();
        if !p.0.insert(class.clone()) {
            return;
        }
        let Some(display) = gtk::gdk::Display::default() else { return };
        if p.1.is_none() {
            let provider = gtk::CssProvider::new();
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            p.1 = Some(provider);
        }
        let css: String = p
            .0
            .iter()
            .map(|c| {
                let hex = c.trim_start_matches("browsah-group-");
                let rgba = gtk::gdk::RGBA::parse(format!("#{hex}")).unwrap_or(rgba);
                let rgb = format!("rgb({}, {}, {})", (rgba.red() * 255.0).round(), (rgba.green() * 255.0).round(), (rgba.blue() * 255.0).round());
                format!("tab.{c} {{ background-color: alpha({rgb}, 0.42); border-bottom: 3px solid {rgb}; }}\ntab.{c}:selected {{ background-color: alpha({rgb}, 0.7); }}\n")
            })
            .collect();
        if let Some(provider) = &p.1 {
            provider.load_from_string(&css);
        }
    });
}

fn dot_icon(color: &str) -> gio::Icon {
    gio::BytesIcon::new(&glib::Bytes::from_owned(dot_svg(color).into_bytes())).upcast()
}

impl Shell {
    pub fn groups(&self) -> Vec<Group> {
        self.groups.borrow().clone()
    }

    pub fn group(&self, id: u32) -> Option<Group> {
        self.groups.borrow().iter().find(|g| g.id == id).cloned()
    }

    /// The tabs of a group: the ones in the tab bar in bar order, then the parked ones.
    pub fn group_tabs(&self, id: u32) -> Vec<Rc<Tab>> {
        let mut out = Vec::new();
        for i in 0..self.tab_view.n_pages() {
            let page = self.tab_view.nth_page(i);
            if let Some(t) = self.tab_for_page(&page).filter(|t| t.group.get() == id) {
                out.push(t);
            }
        }
        out.extend(self.parked.borrow().iter().filter(|(_, t)| t.group.get() == id).map(|(_, t)| t.clone()));
        out
    }

    /// The group's tab that is in the tab bar.
    pub fn front_of(&self, id: u32) -> Option<Rc<Tab>> {
        self.group_tabs(id).into_iter().find(|t| !t.parked.get())
    }

    /// Whether `page` is the tab a group shows in the bar.
    pub(super) fn is_group_front(&self, page: &adw::TabPage) -> bool {
        self.tab_for_page(page).is_some_and(|t| t.group.get() != 0 && !t.parked.get())
    }

    /// A web app has no tab groups (its window shows one page, or a few tabs that come from links).
    fn in_app(&self) -> bool {
        self.workspace.as_ref().is_some_and(|w| w.is_app())
    }

    /// Make a group (no tabs yet) and return its id.
    pub fn create_group(&self, name: &str, color: Option<&str>) -> u32 {
        let mut groups = self.groups.borrow_mut();
        let id = next_id(&groups);
        let color = color.map(str::to_owned).unwrap_or_else(|| next_color(&groups));
        groups.push(Group { id, name: name.trim().to_owned(), color });
        id
    }

    /// Take a tab out of the tab bar (kept alive).
    fn park(&self, tab: &Rc<Tab>) {
        self.collapsing.set(true);
        tab.parked.set(true);
        self.tab_view.transfer_page(&tab.page, &self.parking, 0);
        self.parked.borrow_mut().push((tab.group.get(), tab.clone()));
        self.collapsing.set(false);
    }

    /// Put a parked tab into the tab bar at `at`.
    fn unpark(&self, tab: &Rc<Tab>, at: i32) {
        self.collapsing.set(true);
        self.parked.borrow_mut().retain(|(_, t)| !Rc::ptr_eq(t, tab));
        self.parking.transfer_page(&tab.page, &self.tab_view, at.clamp(0, self.tab_view.n_pages()));
        tab.parked.set(false);
        self.collapsing.set(false);
    }

    /// A group whose front tab is gone shows its next tab, at `at` (the end by default). Returns the tab it brought.
    pub(super) fn ensure_front(&self, id: u32, at: Option<i32>) -> Option<Rc<Tab>> {
        if id == 0 || self.front_of(id).is_some() {
            return None;
        }
        let next = self.group_tabs(id).into_iter().next()?;
        self.unpark(&next, at.unwrap_or(self.tab_view.n_pages()));
        self.refresh_group_chrome();
        Some(next)
    }

    /// Show `tab` as its group's tab in the bar (the group's tab that was showing goes into the list) and select it.
    pub fn show_group_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        let gid = tab.group.get();
        if gid == 0 || !tab.parked.get() {
            self.tab_view.set_selected_page(&tab.page);
            return;
        }
        let front = self.front_of(gid);
        let at = front.as_ref().map(|f| self.tab_view.page_position(&f.page)).unwrap_or_else(|| self.tab_view.n_pages());
        self.unpark(tab, at + 1);
        if let Some(f) = &front {
            self.park(f);
        }
        self.tab_view.set_selected_page(&tab.page);
        self.refresh_group_chrome();
        self.schedule_save();
    }

    /// Paint the group tabs once the tab bar has made its widgets for a change.
    pub(super) fn schedule_group_chrome(&self) {
        let window = self.window.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(s) = window.upgrade().and_then(|w| super::shell_of(&w)) {
                s.refresh_group_chrome();
            }
        });
    }

    /// Paint the group tabs in the bar in their colour (the tab widgets are libadwaita's own, found by type name).
    pub(super) fn refresh_group_chrome(&self) {
        fn walk(shell: &Shell, widget: &gtk::Widget) {
            if widget.type_().name() == "AdwTab" {
                for class in widget.css_classes() {
                    if class.starts_with("browsah-group-") {
                        widget.remove_css_class(&class);
                    }
                }
                let group = widget.property::<Option<adw::TabPage>>("page").and_then(|p| shell.tab_for_page(&p)).and_then(|t| shell.group(t.group.get()));
                if let Some(g) = group {
                    paint_rule(&g.color);
                    widget.add_css_class(&color_class(&g.color));
                }
                return;
            }
            let mut child = widget.first_child();
            while let Some(c) = child {
                walk(shell, &c);
                child = c.next_sibling();
            }
        }
        walk(self, self.tab_bar.upcast_ref());
    }

    /// The icon, tooltip and drop-down arrow a tab shows for its group.
    pub(super) fn apply_group_look(&self, tab: &Rc<Tab>) {
        match self.group(tab.group.get()) {
            Some(g) => tab.page.set_tooltip(&if g.name.is_empty() { "Tab group".to_owned() } else { format!("Group: {}", g.name) }),
            None => tab.page.set_tooltip(""),
        }
        self.refresh_tab_icon(tab); // the favicon, with the group's dot on it
        self.mark_tab(tab); // the drop-down arrow
        self.refresh_group_chrome();
    }

    /// Put a tab in a group. It takes the group's place in the bar when it is the tab being used (the group's other tab goes
    /// into the list); otherwise it goes straight into the list.
    pub fn assign_group(self: &Rc<Self>, tab: &Rc<Tab>, id: u32) {
        if self.in_app() || self.group(id).is_none() || tab.is_pinned() || tab.parked.get() || tab.group.get() == id {
            return;
        }
        let old = tab.group.replace(id);
        let old_at = self.tab_view.page_position(&tab.page);
        let selected = self.tab_view.selected_page();
        if let Some(front) = self.front_of(id).filter(|f| !Rc::ptr_eq(f, tab)) {
            if selected.as_ref() == Some(&tab.page) || selected.as_ref() != Some(&front.page) {
                let at = self.tab_view.page_position(&front.page);
                self.tab_view.reorder_page(&tab.page, at);
                self.park(&front);
            } else {
                self.park(tab);
            }
        }
        self.apply_group_look(tab);
        if old != 0 {
            self.ensure_front(old, Some(old_at));
        }
        self.prune_groups();
        self.schedule_save();
    }

    /// Take a tab out of its group (it goes into the bar, after the group's tab).
    pub fn remove_from_group(self: &Rc<Self>, tab: &Rc<Tab>) {
        let id = tab.group.get();
        if id == 0 {
            return;
        }
        if tab.parked.get() {
            let at = self.front_of(id).map(|f| self.tab_view.page_position(&f.page) + 1).unwrap_or_else(|| self.tab_view.n_pages());
            self.unpark(tab, at);
        }
        tab.group.set(0);
        self.apply_group_look(tab);
        let at = self.tab_view.page_position(&tab.page) + 1;
        self.ensure_front(id, Some(at));
        self.prune_groups();
        self.schedule_save();
    }

    /// Forget groups that have no tab left.
    pub(super) fn prune_groups(&self) {
        let used: HashSet<u32> = self.tabs.borrow().iter().map(|t| t.group.get()).collect();
        self.groups.borrow_mut().retain(|g| used.contains(&g.id));
    }

    pub fn rename_group(&self, id: u32, name: &str) {
        if let Some(g) = self.groups.borrow_mut().iter_mut().find(|g| g.id == id) {
            g.name = name.trim().to_owned();
        }
        self.refresh_group_looks(id);
    }

    pub fn set_group_color(&self, id: u32, color: &str) {
        if let Some(g) = self.groups.borrow_mut().iter_mut().find(|g| g.id == id) {
            g.color = color.to_owned();
        }
        self.refresh_group_looks(id);
    }

    fn refresh_group_looks(&self, id: u32) {
        for t in self.group_tabs(id) {
            self.apply_group_look(&t);
        }
        self.schedule_save();
    }

    /// Take every tab out of a group (the tabs stay open, in the bar).
    pub fn ungroup(self: &Rc<Self>, id: u32) {
        let members = self.group_tabs(id);
        let mut at = self.front_of(id).map(|f| self.tab_view.page_position(&f.page) + 1).unwrap_or_else(|| self.tab_view.n_pages());
        for t in &members {
            if t.parked.get() {
                self.unpark(t, at);
                at += 1;
            }
            t.group.set(0);
        }
        for t in &members {
            self.apply_group_look(t);
        }
        self.prune_groups();
        self.schedule_save();
    }

    /// Close every tab of a group.
    pub fn close_group(self: &Rc<Self>, id: u32) {
        let members = self.group_tabs(id);
        let mut at = self.front_of(id).map(|f| self.tab_view.page_position(&f.page) + 1).unwrap_or_else(|| self.tab_view.n_pages());
        for t in members.iter().filter(|t| t.parked.get()) {
            self.unpark(t, at);
            at += 1;
        }
        for t in &members {
            self.tab_view.close_page(&t.page);
        }
    }

    /// Close one tab of a group, shown or not.
    pub fn close_group_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        if tab.parked.get() {
            let at = self.front_of(tab.group.get()).map(|f| self.tab_view.page_position(&f.page) + 1).unwrap_or_else(|| self.tab_view.n_pages());
            self.unpark(tab, at);
        }
        self.tab_view.close_page(&tab.page);
    }

    /// The groups, to be saved with the tabs.
    pub(super) fn group_records(&self) -> Vec<GroupRecord> {
        self.groups.borrow().iter().map(|g| GroupRecord { id: g.id, name: g.name.clone(), color: g.color.clone(), collapsed: false }).collect()
    }

    /// Put remembered groups back (before their tabs are restored).
    pub(super) fn restore_groups(&self, records: &[GroupRecord]) {
        let mut groups = self.groups.borrow_mut();
        groups.clear();
        groups.extend(records.iter().map(|r| Group { id: r.id, name: r.name.clone(), color: r.color.clone() }));
    }

    /// A restored tab belongs to a remembered group.
    pub(super) fn restore_tab_group(&self, tab: &Rc<Tab>, id: u32) {
        if id != 0 && self.group(id).is_some() {
            tab.group.set(id);
            self.apply_group_look(tab);
        }
    }

    /// After the tabs are back: every group shows one tab (the selected one, else its first) and lists the rest.
    pub(super) fn fold_restored_groups(self: &Rc<Self>) {
        let selected = self.tab_view.selected_page();
        for g in self.groups() {
            let members = self.group_tabs(g.id);
            let keep = members.iter().find(|t| Some(&t.page) == selected.as_ref()).or(members.first()).cloned();
            for t in members.iter().filter(|t| keep.as_ref().is_some_and(|k| !Rc::ptr_eq(k, t))) {
                self.park(t);
            }
        }
        self.refresh_group_chrome();
    }

    /// Ask for a group's name (and make or rename the group).
    fn ask_name(self: &Rc<Self>, heading: &str, initial: &str, done: impl Fn(&Rc<Shell>, String) + 'static) {
        let dialog = adw::AlertDialog::new(Some(heading), None);
        let entry = gtk::Entry::builder().text(initial).placeholder_text("Name").activates_default(true).build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Cancel"), ("ok", "OK")]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let window = self.window.downgrade();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |response| {
            if response == "ok" {
                if let Some(shell) = window.upgrade().and_then(|w| super::shell_of(&w)) {
                    done(&shell, entry.text().trim().to_owned());
                }
            }
        });
    }

    /// *Add to new group*: ask for a name, make the group and put the tab in it.
    pub(super) fn new_group_for(self: &Rc<Self>, tab: Rc<Tab>) {
        self.ask_name("New tab group", "", move |shell, name| {
            let id = shell.create_group(&name, None);
            shell.assign_group(&tab, id);
        });
    }

    pub(super) fn rename_group_dialog(self: &Rc<Self>, id: u32) {
        let current = self.group(id).map(|g| g.name).unwrap_or_default();
        self.ask_name("Rename group", &current, move |shell, name| shell.rename_group(id, &name));
    }

    /// What the tab menu was opened on: the tab and the group it is in.
    pub(super) fn menu_target(&self) -> (Option<Rc<Tab>>, u32) {
        let Some(page) = self.menu_page.borrow().clone().or_else(|| self.tab_view.selected_page()) else { return (None, 0) };
        let tab = self.tab_for_page(&page);
        let gid = tab.as_ref().map(|t| t.group.get()).unwrap_or(0);
        (tab, gid)
    }

    /// The tab-menu entries for groups, for the page the menu is on.
    pub(super) fn group_menu(&self, page: &adw::TabPage) -> gio::Menu {
        let menu = gio::Menu::new();
        if self.in_app() {
            return menu;
        }
        let gid = self.tab_for_page(page).map(|t| t.group.get()).filter(|g| *g != 0);
        if gid.is_some() {
            let own = gio::Menu::new();
            own.append_item(&menu::item("Rename group\u{2026}", "win.group-rename", Some("pen-line")));
            let colors = gio::Menu::new();
            for (name, hex) in PALETTE {
                let item = menu::targeted(name, "win.group-color", &(*hex).to_variant(), None);
                item.set_attribute_value(menu::DOT, Some(&(*hex).to_variant()));
                colors.append_item(&item);
            }
            own.append_submenu(Some("Group colour"), &colors);
            own.append_item(&menu::item("New tab in group", "win.group-new-tab", Some("plus")));
            own.append_item(&menu::item("Remove from group", "win.group-remove", Some("x")));
            own.append_item(&menu::item("Ungroup", "win.group-ungroup", None));
            own.append_item(&menu::item("Close group", "win.group-close", Some("x")));
            menu.append_section(None, &own);
        }
        if !self.page_is_pinned(page) {
            let add = gio::Menu::new();
            if gid.is_none() {
                add.append_item(&menu::item("Add to new group\u{2026}", "win.group-new", None));
            }
            let others: Vec<Group> = self.groups().into_iter().filter(|g| Some(g.id) != gid).collect();
            if !others.is_empty() {
                let list = gio::Menu::new();
                for g in &others {
                    let item = menu::targeted(if g.name.is_empty() { "Unnamed group" } else { &g.name }, "win.group-add", &g.id.to_string().to_variant(), None);
                    item.set_attribute_value(menu::DOT, Some(&g.color.to_variant()));
                    list.append_item(&item);
                }
                add.append_submenu(Some("Add to group"), &list);
            }
            menu.append_section(None, &add);
        }
        menu
    }

    /// Show `popover` under `parent`, replacing any earlier tab popover (one popover per click, docs/research/tab-menu-grab.md).
    ///
    /// The popover hangs on the tab *bar*, pointing at `anchor` (a tab): under the tab widget it would inherit the workspace
    /// colour's text colour (`workspace_css` sets one for `tabbar tab`), which is unreadable on the popover's own surface.
    fn adopt_popover(self: &Rc<Self>, popover: &gtk::Popover, anchor: &gtk::Widget) {
        let old = self.title_popover.borrow_mut().take();
        if let Some(old) = old {
            old.popdown();
        }
        popover.set_halign(gtk::Align::Start);
        popover.set_parent(&self.tab_bar);
        if let Some(b) = anchor.compute_bounds(&self.tab_bar) {
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32)));
        }
        popover.connect_closed({
            let weak = Rc::downgrade(self);
            move |p| {
                let p = p.clone();
                glib::idle_add_local_once({ let p = p.clone(); move || p.unparent() });
                if let Some(s) = weak.upgrade() {
                    let mine = s.title_popover.borrow().as_ref().is_some_and(|t| *t == p);
                    if mine {
                        s.title_popover.borrow_mut().take();
                        s.setup_tab_menu(None);
                    }
                }
            }
        });
        *self.title_popover.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    /// The group's drop-down: its name, its tabs (the one in the bar ticked), and what to do with the group. A right click on a row
    /// opens that tab's menu, on the heading the group's.
    pub(super) fn show_group_dropdown(self: &Rc<Self>, page: &adw::TabPage) {
        let Some(front) = self.tab_for_page(page) else { return };
        let Some(group) = self.group(front.group.get()) else { return };
        let Some(anchor) = tab_of_page(&self.tab_bar, page) else { return };
        let popover = gtk::Popover::builder().has_arrow(false).autohide(true).build();
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(260).build();
        let heading = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).margin_start(12).margin_end(12).margin_top(6).margin_bottom(6).build();
        let dot = gtk::Image::from_gicon(&dot_icon(&group.color));
        dot.set_pixel_size(16);
        heading.append(&dot);
        let name = if group.name.is_empty() { "Tab group".to_owned() } else { group.name.clone() };
        heading.append(&gtk::Label::builder().label(&name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label"]).build());
        let on_heading = gtk::GestureClick::new();
        on_heading.set_button(3);
        on_heading.connect_pressed({
            let (weak, anchor, page, popover) = (Rc::downgrade(self), anchor.clone(), page.clone(), popover.clone());
            move |_, _, _, _| {
                if let Some(s) = weak.upgrade() {
                    let (anchor, page) = (anchor.clone(), page.clone());
                    popover.popdown();
                    let x = anchor.compute_bounds(&s.tab_bar).map(|b| b.x() as i32 + 8).unwrap_or(8);
                    s.show_tab_menu(&s.tab_bar, x, anchor.height() - 2, Some(&page));
                }
            }
        });
        heading.add_controller(on_heading);
        list.append(&heading);
        list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        for member in self.group_tabs(group.id) {
            let (label, lead): (String, gtk::Widget) = {
                let s = member.state.borrow();
                let lead = match member.favicon.borrow().as_ref() {
                    Some(tex) => {
                        let image = gtk::Image::from_paintable(Some(tex));
                        image.set_pixel_size(16);
                        image.upcast()
                    }
                    None => menu::icon_widget(Some("link")),
                };
                (tab_title(&s), lead)
            };
            let row = menu::row(&label, &lead, None, !member.parked.get(), true, false);
            row.set_tooltip_text(Some(&member.state.borrow().address));
            row.connect_clicked({
                let (weak, member, popover) = (Rc::downgrade(self), member.clone(), popover.clone());
                move |_| {
                    if let Some(s) = weak.upgrade() {
                        s.show_group_tab(&member);
                    }
                    popover.popdown();
                }
            });
            let on_row = gtk::GestureClick::new();
            on_row.set_button(3);
            on_row.connect_pressed({
                let (weak, anchor, member, popover) = (Rc::downgrade(self), anchor.clone(), member.clone(), popover.clone());
                move |_, _, _, _| {
                    if let Some(s) = weak.upgrade() {
                        popover.popdown();
                        s.show_group_tab_menu(&anchor, &member);
                    }
                }
            });
            row.add_controller(on_row);
            list.append(&row);
        }
        list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let footer = |label: &str, icon: &str, run: Box<dyn Fn(&Rc<Shell>)>| {
            let row = menu::row(label, &menu::icon_widget(Some(icon)), None, false, true, false);
            let (weak, popover) = (Rc::downgrade(self), popover.clone());
            row.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    run(&s);
                }
                popover.popdown();
            });
            row
        };
        let gid = group.id;
        list.append(&footer("New tab in group", "plus", Box::new(move |s| s.open_tab_in_group(gid))));
        list.append(&footer("Rename group\u{2026}", "pen-line", Box::new(move |s| s.rename_group_dialog(gid))));
        popover.set_child(Some(&list));
        self.adopt_popover(&popover, &anchor);
    }

    /// The menu of one tab of a group's list.
    pub(super) fn show_group_tab_menu(self: &Rc<Self>, anchor: &gtk::Widget, member: &Rc<Tab>) {
        let model = gio::Menu::new();
        let uid = member.uid.to_string().to_variant();
        model.append_item(&menu::targeted("Show", "win.group-tab-show", &uid, None));
        model.append_item(&menu::targeted("Copy address", "win.group-tab-copy", &uid, Some("link")));
        model.append_item(&menu::targeted("Remove from group", "win.group-tab-remove", &uid, Some("x")));
        model.append_item(&menu::targeted("Close tab", "win.group-tab-close", &uid, Some("x")));
        let popover = menu::popover(&model, &self.window, 250);
        self.adopt_popover(&popover, anchor);
    }

    /// The tab with this id (shown or not).
    pub(super) fn tab_by_uid(&self, uid: &str) -> Option<Rc<Tab>> {
        self.tabs.borrow().iter().find(|t| t.uid.to_string() == uid).cloned()
    }

    /// A new tab in a group.
    pub(super) fn open_tab_in_group(self: &Rc<Self>, id: u32) {
        let url = self.settings.borrow().new_tab_url().map(|u| super::nav::resolve(&u, &self.search_url.borrow())).unwrap_or_else(|| "about:blank".to_owned());
        let tab = self.open_tab(&url, true);
        self.assign_group(&tab, id);
    }

}
