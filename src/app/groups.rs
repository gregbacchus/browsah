//! Tab groups in the window: assigning tabs, the coloured dot, collapsing a group to one tab, the menus, and
//! remembering groups with the tabs. The model and helpers are `crate::groups`.
//!
//! A collapsed group is one *placeholder* page in the tab bar; its tabs are detached from the tab view but kept
//! alive (`parked`), so nothing is lost. Selecting the placeholder expands the group.

use super::{Shell, Tab};
use crate::groups::{collapsed_title, dot_svg, join_position, next_color, next_id, Group, PALETTE};
use crate::store::GroupRecord;
use adw::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

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

    /// The tabs of a group, attached ones in tab-bar order, then the parked ones.
    pub fn group_tabs(&self, id: u32) -> Vec<Rc<Tab>> {
        let mut out = Vec::new();
        for i in 0..self.tab_view.n_pages() {
            let page = self.tab_view.nth_page(i);
            if let Some(t) = self.tab_for_page(&page).filter(|t| t.group.get() == id) {
                out.push(t);
            }
        }
        out.extend(self.parked.borrow().iter().filter(|(g, _)| *g == id).map(|(_, t)| t.clone()));
        out
    }

    /// The group a placeholder page stands for.
    pub(super) fn placeholder_group(&self, page: &adw::TabPage) -> Option<u32> {
        self.placeholders.borrow().iter().find(|(_, p)| p == page).map(|(g, _)| *g)
    }

    /// Whether the selected page is a collapsed group's placeholder.
    pub(super) fn selected_placeholder(&self) -> Option<u32> {
        self.tab_view.selected_page().and_then(|p| self.placeholder_group(&p))
    }

    /// Make a group (no tabs yet) and return its id.
    pub fn create_group(&self, name: &str, color: Option<&str>) -> u32 {
        let mut groups = self.groups.borrow_mut();
        let id = next_id(&groups);
        let color = color.map(str::to_owned).unwrap_or_else(|| next_color(&groups));
        groups.push(Group { id, name: name.trim().to_owned(), color, collapsed: false });
        id
    }

    /// The page order as group ids (a placeholder counts as its group), for placing a joining tab.
    fn page_groups(&self) -> Vec<u32> {
        (0..self.tab_view.n_pages())
            .map(|i| {
                let page = self.tab_view.nth_page(i);
                self.placeholder_group(&page).or_else(|| self.tab_for_page(&page).map(|t| t.group.get())).unwrap_or(0)
            })
            .collect()
    }

    /// The icon and tooltip a tab shows for its group.
    fn apply_group_look(&self, tab: &Rc<Tab>) {
        match self.group(tab.group.get()) {
            Some(g) => tab.page.set_tooltip(&if g.name.is_empty() { "Tab group".to_owned() } else { format!("Group: {}", g.name) }),
            None => tab.page.set_tooltip(""),
        }
        self.refresh_tab_icon(tab); // the favicon, with the group's dot on it
    }

    /// Put a tab in a group, next to the group's other tabs.
    pub fn assign_group(self: &Rc<Self>, tab: &Rc<Tab>, id: u32) {
        if self.group(id).is_none() || tab.page.is_pinned() {
            return;
        }
        if self.group(id).is_some_and(|g| g.collapsed) {
            self.expand_group(id);
        }
        let old = tab.group.replace(id);
        let order = self.page_groups();
        let from = self.tab_view.page_position(&tab.page) as usize;
        let to = join_position(&order, id, from, self.tab_view.n_pinned_pages() as usize);
        if to != from {
            self.tab_view.reorder_page(&tab.page, to as i32);
        }
        self.apply_group_look(tab);
        if old != id {
            self.prune_groups();
        }
        self.schedule_save();
    }

    /// Take a tab out of its group.
    pub fn remove_from_group(self: &Rc<Self>, tab: &Rc<Tab>) {
        if tab.group.replace(0) != 0 {
            self.apply_group_look(tab);
            self.prune_groups();
            self.schedule_save();
        }
    }

    /// Forget groups that have no tab left.
    pub(super) fn prune_groups(&self) {
        let used: std::collections::HashSet<u32> = self.tabs.borrow().iter().map(|t| t.group.get()).collect();
        let empty: Vec<u32> = self.groups.borrow().iter().map(|g| g.id).filter(|id| !used.contains(id)).collect();
        for id in empty {
            self.groups.borrow_mut().retain(|g| g.id != id);
            let placeholder = self.placeholders.borrow().iter().find(|(g, _)| *g == id).map(|(_, p)| p.clone());
            if let Some(p) = placeholder {
                self.placeholders.borrow_mut().retain(|(g, _)| *g != id);
                self.tab_view.close_page(&p);
                self.tab_view.close_page_finish(&p, true);
            }
        }
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
        let ph = self.placeholders.borrow().iter().find(|(g, _)| *g == id).map(|(_, p)| p.clone());
        if let (Some(p), Some(g)) = (ph, self.group(id)) {
            p.set_title(&collapsed_title(&g.name, self.group_tabs(id).len()));
            p.set_icon(Some(&dot_icon(&g.color)));
        }
        self.schedule_save();
    }

    /// Fold a group into one tab. Its tabs stay alive, out of the tab bar.
    pub fn collapse_group(self: &Rc<Self>, id: u32) {
        let Some(g) = self.group(id) else { return };
        let members: Vec<Rc<Tab>> = self.group_tabs(id).into_iter().filter(|t| !t.parked.get()).collect();
        if g.collapsed || members.is_empty() {
            return;
        }
        // Something has to stay in the bar to be shown.
        let others = (0..self.tab_view.n_pages()).map(|i| self.tab_view.nth_page(i)).filter(|p| self.placeholder_group(p).is_none() && !members.iter().any(|t| &t.page == p)).count();
        if others == 0 {
            self.toast("Open another tab first: a window cannot show only a collapsed group");
            return;
        }
        self.collapsing.set(true);
        let first = members.iter().map(|t| self.tab_view.page_position(&t.page)).min().unwrap_or(0).max(0);
        // The placeholder takes the group's place in the bar.
        let child = adw::StatusPage::builder().icon_name("view-list-symbolic").title(g.name.clone()).description("Select this tab to show the group's tabs again.").build();
        let page = self.tab_view.insert(&child, first);
        page.set_title(&collapsed_title(&g.name, members.len()));
        page.set_icon(Some(&dot_icon(&g.color)));
        page.set_tooltip(&format!("Group: {}", g.name));
        self.placeholders.borrow_mut().push((id, page.clone()));
        for t in &members {
            t.parked.set(true);
            self.tab_view.transfer_page(&t.page, &self.parking, 0);
            self.parked.borrow_mut().push((id, t.clone()));
        }
        // Show a tab that is still in the bar (never the stand-in, which would open the group again).
        let pos = self.tab_view.page_position(&page);
        let keep = (0..self.tab_view.n_pages()).map(|i| self.tab_view.nth_page(i)).filter(|p| self.placeholder_group(p).is_none()).min_by_key(|p| (self.tab_view.page_position(p) - pos).abs());
        if let Some(p) = keep {
            self.tab_view.set_selected_page(&p);
        }
        self.collapsing.set(false);
        self.on_selected();
        if let Some(g) = self.groups.borrow_mut().iter_mut().find(|g| g.id == id) {
            g.collapsed = true;
        }
        self.schedule_save();
    }

    /// Show a collapsed group's tabs again, in place of its placeholder.
    pub fn expand_group(self: &Rc<Self>, id: u32) {
        if !self.group(id).is_some_and(|g| g.collapsed) {
            return;
        }
        let placeholder = self.placeholders.borrow().iter().find(|(g, _)| *g == id).map(|(_, p)| p.clone());
        let Some(placeholder) = placeholder else { return };
        let at = self.tab_view.page_position(&placeholder).max(0);
        let members: Vec<Rc<Tab>> = {
            let mut parked = self.parked.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = parked.drain(..).partition(|(g, _)| *g == id);
            *parked = rest;
            mine.into_iter().map(|(_, t)| t).collect()
        };
        self.collapsing.set(true);
        for (i, t) in members.iter().enumerate() {
            self.parking.transfer_page(&t.page, &self.tab_view, at + i as i32);
            t.parked.set(false);
        }
        self.collapsing.set(false);
        self.placeholders.borrow_mut().retain(|(g, _)| *g != id);
        if let Some(g) = self.groups.borrow_mut().iter_mut().find(|g| g.id == id) {
            g.collapsed = false;
        }
        if let Some(first) = members.first() {
            self.tab_view.set_selected_page(&first.page);
        }
        self.tab_view.close_page(&placeholder);
        self.tab_view.close_page_finish(&placeholder, true);
        self.schedule_save();
    }

    /// Take every tab out of a group (the tabs stay open).
    pub fn ungroup(self: &Rc<Self>, id: u32) {
        self.expand_group(id);
        for t in self.group_tabs(id) {
            t.group.set(0);
            self.apply_group_look(&t);
        }
        self.prune_groups();
        self.schedule_save();
    }

    /// Close every tab of a group.
    pub fn close_group(self: &Rc<Self>, id: u32) {
        self.expand_group(id);
        for t in self.group_tabs(id) {
            self.tab_view.close_page(&t.page);
        }
    }

    /// The groups, to be saved with the tabs.
    pub(super) fn group_records(&self) -> Vec<GroupRecord> {
        self.groups.borrow().iter().map(|g| GroupRecord { id: g.id, name: g.name.clone(), color: g.color.clone(), collapsed: g.collapsed }).collect()
    }

    /// Put remembered groups back (before their tabs are restored).
    pub(super) fn restore_groups(&self, records: &[GroupRecord]) {
        let mut groups = self.groups.borrow_mut();
        groups.clear();
        groups.extend(records.iter().map(|r| Group { id: r.id, name: r.name.clone(), color: r.color.clone(), collapsed: false }));
    }

    /// A restored tab belongs to a remembered group.
    pub(super) fn restore_tab_group(&self, tab: &Rc<Tab>, id: u32) {
        if id != 0 && self.group(id).is_some() {
            tab.group.set(id);
            self.apply_group_look(tab);
        }
    }

    /// Collapse the groups that were collapsed when the window closed.
    pub(super) fn restore_collapsed(self: &Rc<Self>, records: &[GroupRecord]) {
        for r in records.iter().filter(|r| r.collapsed) {
            self.collapse_group(r.id);
        }
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

    /// What the tab menu was opened on: the tab (None for a collapsed group's stand-in) and the group it is in.
    pub(super) fn menu_target(&self) -> (Option<Rc<Tab>>, u32) {
        let Some(page) = self.menu_page.borrow().clone().or_else(|| self.tab_view.selected_page()) else { return (None, 0) };
        if let Some(gid) = self.placeholder_group(&page) {
            return (None, gid);
        }
        let tab = self.tab_for_page(&page);
        let gid = tab.as_ref().map(|t| t.group.get()).unwrap_or(0);
        (tab, gid)
    }

    /// The tab-menu entries for groups, for the page the menu is on.
    pub(super) fn group_menu(&self, page: &adw::TabPage) -> gio::Menu {
        let menu = gio::Menu::new();
        let gid = self.placeholder_group(page).or_else(|| self.tab_for_page(page).map(|t| t.group.get()).filter(|g| *g != 0));
        let is_placeholder = self.placeholder_group(page).is_some();
        if let Some(id) = gid {
            let own = gio::Menu::new();
            if is_placeholder {
                own.append(Some("Expand group"), Some("win.group-toggle"));
            } else {
                own.append(Some("Collapse group"), Some("win.group-toggle"));
                own.append(Some("Remove from group"), Some("win.group-remove"));
            }
            own.append(Some("Rename group\u{2026}"), Some("win.group-rename"));
            let colors = gio::Menu::new();
            for (name, hex) in PALETTE {
                let item = gio::MenuItem::new(Some(name), None);
                item.set_action_and_target_value(Some("win.group-color"), Some(&(*hex).to_variant()));
                colors.append_item(&item);
            }
            own.append_submenu(Some("Group colour"), &colors);
            own.append(Some("Ungroup"), Some("win.group-ungroup"));
            own.append(Some("Close group"), Some("win.group-close"));
            menu.append_section(None, &own);
            let _ = id;
        }
        if !is_placeholder && !page.is_pinned() {
            let add = gio::Menu::new();
            add.append(Some("Add to new group\u{2026}"), Some("win.group-new"));
            let others: Vec<Group> = self.groups().into_iter().filter(|g| Some(g.id) != gid).collect();
            if !others.is_empty() {
                let list = gio::Menu::new();
                for g in &others {
                    let item = gio::MenuItem::new(Some(if g.name.is_empty() { "Unnamed group" } else { &g.name }), None);
                    item.set_action_and_target_value(Some("win.group-add"), Some(&g.id.to_string().to_variant()));
                    list.append_item(&item);
                }
                add.append_submenu(Some("Add to group"), &list);
            }
            menu.append_section(None, &add);
        }
        menu
    }
}
