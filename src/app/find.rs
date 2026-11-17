//! Find in page (`Ctrl+F`): a bar at the bottom of the window with a search entry, the match count and
//! previous/next buttons. The search runs in the selected tab; matches are highlighted by Chromium.

use super::{Shell, Tab};
use gtk::prelude::*;
use std::rc::Rc;

pub struct Bar {
    pub revealer: gtk::Revealer,
    pub entry: gtk::SearchEntry,
    label: gtk::Label,
    /// A new search has not yet been told to settle on its first match (Chromium reports match 0 until it is).
    needs_first: std::cell::Cell<bool>,
}

impl Bar {
    pub fn new() -> Bar {
        let entry = gtk::SearchEntry::builder().hexpand(true).placeholder_text("Find in page").build();
        let label = gtk::Label::builder().label("").width_chars(10).build();
        label.add_css_class("dim-label");
        let prev = gtk::Button::builder().icon_name("go-up-symbolic").tooltip_text("Previous match (Shift+Enter)").action_name("win.find-prev").build();
        let next = gtk::Button::builder().icon_name("go-down-symbolic").tooltip_text("Next match (Enter)").action_name("win.find-next").build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text("Close (Esc)").action_name("win.find-close").build();
        for b in [&prev, &next, &close] {
            b.add_css_class("flat");
        }
        let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).margin_top(6).margin_bottom(6).margin_start(8).margin_end(8).build();
        row.append(&entry);
        row.append(&label);
        row.append(&prev);
        row.append(&next);
        row.append(&close);
        let revealer = gtk::Revealer::builder().child(&row).reveal_child(false).transition_type(gtk::RevealerTransitionType::SlideUp).build();
        Bar { revealer, entry, label, needs_first: std::cell::Cell::new(false) }
    }

    pub fn is_open(&self) -> bool {
        self.revealer.reveals_child()
    }
}

/// The text beside the entry: `3/12`, `No matches`, or nothing before a search.
pub fn status_text(searching: bool, count: i32, active: i32) -> String {
    if !searching {
        String::new()
    } else if count <= 0 {
        "No matches".to_owned()
    } else {
        format!("{active}/{count}")
    }
}

impl Shell {
    /// Show the bar and put the cursor in it (searching again for what was there).
    pub(super) fn open_find(self: &Rc<Self>) {
        self.find.revealer.set_reveal_child(true);
        self.window.add_css_class("has-footer"); // the colour frame has something in its footer: no extra border below
        self.find.entry.grab_focus();
        self.find.entry.select_region(0, -1);
        let text = self.find.entry.text().to_string();
        if !text.is_empty() {
            self.find_run(true, false);
        }
    }

    /// Search for what is in the entry (`next`: the same search, moving on).
    pub(super) fn find_run(self: &Rc<Self>, forward: bool, next: bool) {
        let text = self.find.entry.text().to_string();
        let Some(tab) = self.current() else { return };
        if text.is_empty() {
            tab.view().stop_find();
            self.find.label.set_label("");
            return;
        }
        self.find.needs_first.set(!next);
        tab.view().find(&text, forward, false, next);
    }

    /// Step to the next or previous match (opens the bar if it is closed).
    pub(super) fn find_step(self: &Rc<Self>, forward: bool) {
        if !self.find.is_open() {
            self.open_find();
            return;
        }
        self.find_run(forward, true);
    }

    /// Close the bar and remove the highlights in every tab.
    pub(super) fn close_find(self: &Rc<Self>) {
        if !self.find.is_open() {
            return;
        }
        self.find.revealer.set_reveal_child(false);
        self.window.remove_css_class("has-footer");
        self.find.label.set_label("");
        for t in self.tabs.borrow().iter() {
            t.view().stop_find();
        }
        if let Some(t) = self.current() {
            t.view().widget.grab_focus();
        }
    }

    /// A tab reported a result: show it if it is the tab being searched.
    pub(super) fn find_result(self: &Rc<Self>, tab: &Rc<Tab>, count: i32, active: i32) {
        if self.current().is_some_and(|c| Rc::ptr_eq(&c, tab)) && self.find.is_open() {
            if count > 0 && active == 0 && self.find.needs_first.replace(false) {
                // Move onto the first match (what a person expects after typing).
                self.find_run(true, true);
                return;
            }
            let searching = !self.find.entry.text().is_empty();
            self.find.label.set_label(&status_text(searching, count, active));
        }
    }

    /// The label beside the entry (for the self-test).
    pub fn find_label(&self) -> String {
        self.find.label.label().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_says_where_you_are() {
        assert_eq!(status_text(false, 0, 0), "");
        assert_eq!(status_text(true, 0, 0), "No matches");
        assert_eq!(status_text(true, 12, 3), "3/12");
    }
}
