//! The address bar's suggestions: a popover under the entry that lists what `crate::suggest::rank` offers
//! while an address is typed. It never takes the keyboard focus: the arrow keys are forwarded from the entry.

use super::{store, Shell};
use crate::suggest::{rank, Kind, Suggestion};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct Popup {
    popover: gtk::Popover,
    list: gtk::ListBox,
    items: RefCell<Vec<Suggestion>>,
}

impl Popup {
    pub fn new(entry: &gtk::Entry) -> Popup {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).activate_on_single_click(true).focusable(false).build();
        list.add_css_class("suggest-list");
        // A highlighted suggestion must stand out in light and dark themes alike.
        let css = gtk::CssProvider::new();
        css.load_from_string(".suggest-list row:selected { background: alpha(@accent_bg_color, 0.35); }");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        let scroller = gtk::ScrolledWindow::builder().child(&list).propagate_natural_height(true).max_content_height(320).hscrollbar_policy(gtk::PolicyType::Never).build();
        let popover = gtk::Popover::builder().child(&scroller).autohide(false).has_arrow(false).position(gtk::PositionType::Bottom).focusable(false).build();
        popover.add_css_class("menu");
        popover.set_parent(entry);
        entry.connect_destroy({
            let popover = popover.clone();
            move |_| popover.unparent()
        });
        Popup { popover, list, items: RefCell::new(Vec::new()) }
    }

    fn clear(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        self.items.borrow_mut().clear();
    }
}

fn row_for(s: &Suggestion) -> gtk::ListBoxRow {
    let kind = match s.kind {
        Kind::Bookmark => "\u{2605} ",
        Kind::Tab => "\u{25a1} ",
        Kind::History => "",
    };
    let title = gtk::Label::builder().label(format!("{kind}{}", if s.title.is_empty() { &s.url } else { &s.title })).halign(gtk::Align::Start).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let url = gtk::Label::builder().label(&s.url).halign(gtk::Align::Start).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
    url.add_css_class("dim-label");
    url.add_css_class("caption");
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(4).margin_bottom(4).margin_start(8).margin_end(8).build();
    b.append(&title);
    b.append(&url);
    gtk::ListBoxRow::builder().child(&b).focusable(false).build()
}

impl Shell {
    /// Show the suggestions for what is in the address bar (or hide them when there are none).
    pub(super) fn update_suggestions(self: &Rc<Self>) {
        let text = self.entry.text().trim().to_owned();
        let s = self.settings.borrow().history.suggest;
        if !s || self.private.get() || !self.editing.get() || text.is_empty() {
            self.hide_suggestions();
            return;
        }
        let (bookmarks, visits) = match store() {
            Some(st) => (
                self.workspace.as_ref().map(|w| st.bookmarks(&w.id)).unwrap_or_default().into_iter().map(|b| (b.url, b.title)).collect::<Vec<_>>(),
                st.search_visits(&text.split_whitespace().next().unwrap_or("").to_owned(), crate::store::Scope::All, 300).into_iter().map(|v| (v.url, v.title)).collect::<Vec<_>>(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        let tabs: Vec<(String, String)> = self.tabs.borrow().iter().map(|t| (t.state.borrow().address.clone(), t.state.borrow().title.clone())).collect();
        let found = rank(&text, &bookmarks, &tabs, &visits, 6);
        let pop = &self.suggest;
        pop.clear();
        if found.is_empty() {
            pop.popover.popdown();
            return;
        }
        for f in &found {
            pop.list.append(&row_for(f));
        }
        *pop.items.borrow_mut() = found;
        pop.popover.set_size_request(self.entry.width().max(320), -1);
        pop.popover.popup();
    }

    pub(super) fn hide_suggestions(&self) {
        self.suggest.popover.popdown();
        self.suggest.clear();
    }

    /// Move the highlighted suggestion (`1` down, `-1` up). False when nothing is shown, so the key is not ours.
    pub(super) fn move_suggestion(&self, delta: i32) -> bool {
        let n = self.suggest.items.borrow().len() as i32;
        if n == 0 || !self.suggest.popover.is_visible() {
            return false;
        }
        let cur = self.suggest.list.selected_row().map(|r| r.index()).unwrap_or(-1);
        // One step past either end returns to "none" (Enter then uses what was typed).
        let next = cur + delta;
        if next < 0 || next >= n {
            self.suggest.list.unselect_all();
        } else if let Some(row) = self.suggest.list.row_at_index(next) {
            self.suggest.list.select_row(Some(&row));
        }
        true
    }

    /// The address of the highlighted suggestion, if one is.
    pub(super) fn chosen_suggestion(&self) -> Option<String> {
        let i = self.suggest.list.selected_row()?.index();
        self.suggest.items.borrow().get(i as usize).map(|s| s.url.clone())
    }

    /// The popover (for the self-test's pictures).
    pub fn suggestion_popover(&self) -> &gtk::Popover {
        &self.suggest.popover
    }

    /// The addresses now offered (for the self-test).
    pub fn suggestion_urls(&self) -> Vec<String> {
        self.suggest.items.borrow().iter().map(|s| s.url.clone()).collect()
    }

    /// Click a row: go there.
    pub(super) fn connect_suggestion_clicks(self: &Rc<Self>) {
        let shell = Rc::downgrade(self);
        self.suggest.list.connect_row_activated(move |_, row| {
            if let Some(shell) = shell.upgrade() {
                let url = shell.suggest.items.borrow().get(row.index() as usize).map(|s| s.url.clone());
                if let Some(url) = url {
                    shell.hide_suggestions();
                    shell.go(&url);
                }
            }
        });
    }
}
