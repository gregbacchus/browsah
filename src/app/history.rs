//! The history window (`Ctrl+H`): search what was visited, reopen what was closed.

use super::{now_secs, store, Shell, NEW_EPHEMERAL};
use crate::store::{ClosedTab, Scope, Visit};
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;

/// The open history window; its lists are public to the self-test.
pub struct HistoryView {
    pub dialog: adw::Dialog,
    pub search: gtk::SearchEntry,
    pub all: gtk::CheckButton,
    pub visits: gtk::ListBox,
    pub closed: gtk::ListBox,
}

fn row(title: &str, subtitle: &str, url: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder().title(glib::markup_escape_text(if title.is_empty() { subtitle } else { title }).as_str()).subtitle(glib::markup_escape_text(subtitle).as_str()).activatable(true).build();
    r.set_title_lines(1);
    r.set_subtitle_lines(1);
    if let Some(icon) = super::favicon::small_image(url) {
        r.add_prefix(&icon);
    }
    r
}

fn clear(list: &gtk::ListBox) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
}

impl Shell {
    /// Open the history window over this window.
    pub fn open_history(self: &Rc<Self>) -> Option<Rc<HistoryView>> {
        let store = store()?;
        let search = gtk::SearchEntry::builder().hexpand(true).placeholder_text("Search history").build();
        let all = gtk::CheckButton::builder().label("All workspaces").active(self.workspace.is_none()).build();
        let visits = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let closed = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        let stack = adw::ViewStack::new();
        let scroll = |l: &gtk::ListBox| gtk::ScrolledWindow::builder().child(l).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        stack.add_titled(&scroll(&visits), Some("visits"), "Visited");
        stack.add_titled(&scroll(&closed), Some("closed"), "Recently closed");
        let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();

        let top = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).margin_start(12).margin_end(12).margin_top(8).margin_bottom(8).build();
        top.append(&search);
        top.append(&all);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).build();
        body.append(&top);
        stack.set_margin_start(12);
        stack.set_margin_end(12);
        stack.set_margin_bottom(12);
        body.append(&stack);
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&body));
        let dialog = adw::Dialog::builder().title("History").child(&toolbar).content_width(780).content_height(640).build();
        let view = Rc::new(HistoryView { dialog: dialog.clone(), search: search.clone(), all: all.clone(), visits: visits.clone(), closed: closed.clone() });

        let window = self.window.downgrade();
        let ws = self.workspace_id().map(str::to_owned);
        let fill = {
            let (view, store, ws) = (view.clone(), store.clone(), ws.clone());
            let window = window.clone();
            move || {
                let scope = match (&ws, view.all.is_active()) {
                    (Some(w), false) => Scope::Workspace(w),
                    (None, false) => Scope::Ephemeral,
                    (_, true) => Scope::All,
                };
                let now = now_secs();
                clear(&view.visits);
                for v in store.search_visits(view.search.text().as_str(), scope, 200) {
                    let Visit { url, title, at, workspace, .. } = v;
                    let r = row(&title, &format!("{url} \u{b7} {}{}", crate::history::when(at, now), workspace.as_deref().map(|w| format!(" \u{b7} {w}")).unwrap_or_default()), &url);
                    let open_here = {
                        let (window, url) = (window.clone(), url.clone());
                        move |new_tab: bool| {
                            if let Some(s) = window.upgrade().and_then(|w| super::shell_of(&w)) {
                                if new_tab {
                                    s.open_tab(&url, true);
                                } else if let Some(t) = s.current() {
                                    t.view().navigate(&url);
                                } else {
                                    s.open_tab(&url, true);
                                }
                            }
                        }
                    };
                    let new_tab = gtk::Button::builder().icon_name("tab-new-symbolic").valign(gtk::Align::Center).has_frame(false).tooltip_text("Open in a new tab").build();
                    new_tab.connect_clicked({
                        let open_here = open_here.clone();
                        move |_| open_here(true)
                    });
                    r.add_suffix(&new_tab);
                    r.connect_activated(move |_| open_here(false));
                    view.visits.append(&r);
                }
                // Recently closed: tabs, and ephemeral windows as one entry each.
                clear(&view.closed);
                let tabs: Vec<ClosedTab> = store.closed_tabs(scope, 200);
                let mut seen_groups = std::collections::HashSet::new();
                for t in tabs {
                    if let Some(g) = t.group {
                        if !seen_groups.insert(g) {
                            continue;
                        }
                        let members: Vec<ClosedTab> = store.closed_tabs(Scope::Ephemeral, 1000).into_iter().filter(|m| m.group == Some(g)).collect();
                        let r = row(&format!("Window with {} tab{}", members.len(), if members.len() == 1 { "" } else { "s" }), &format!("{} \u{b7} {}", members.first().map(|m| m.url.as_str()).unwrap_or(""), crate::history::when(t.at, now)), members.first().map(|m| m.url.as_str()).unwrap_or(""));
                        let (window, store, ids) = (window.clone(), store.clone(), members.iter().map(|m| m.id).collect::<Vec<_>>());
                        r.connect_activated(move |_| {
                            let mut tabs: Vec<ClosedTab> = ids.iter().filter_map(|id| store.take_closed(*id)).collect();
                            tabs.sort_by_key(|t| t.id);
                            if let (Some(first), Some(f)) = (tabs.first(), NEW_EPHEMERAL.with(|f| f.borrow().clone())) {
                                let shell = f(&first.url);
                                for t in tabs.iter().skip(1) {
                                    shell.open_tab(&t.url, false);
                                }
                            }
                            if let Some(s) = window.upgrade().and_then(|w| super::shell_of(&w)) {
                                s.close_history();
                            }
                        });
                        view.closed.append(&r);
                    } else {
                        let id = t.id;
                        let r = row(&t.title, &format!("{} \u{b7} {} \u{b7} closed by {}", t.url, crate::history::when(t.at, now), t.reason), &t.url);
                        let (window, store) = (window.clone(), store.clone());
                        r.connect_activated(move |_| {
                            if let (Some(t), Some(s)) = (store.take_closed(id), window.upgrade().and_then(|w| super::shell_of(&w))) {
                                s.open_tab(&t.url, true);
                                s.close_history();
                            }
                        });
                        view.closed.append(&r);
                    }
                }
            }
        };
        fill();
        search.connect_search_changed({
            let fill = fill.clone();
            move |_| fill()
        });
        all.connect_toggled(move |_| fill());
        *self.history.borrow_mut() = Some(view.clone());
        dialog.connect_closed({
            let window = self.window.downgrade();
            move |_| {
                if let Some(s) = window.upgrade().and_then(|w| super::shell_of(&w)) {
                    *s.history.borrow_mut() = None;
                }
            }
        });
        dialog.present(Some(&self.window));
        search.grab_focus();
        Some(view)
    }

    /// Close the history window if it is open.
    pub fn close_history(&self) {
        // (Copy it out first: closing runs a handler that clears this field.)
        let open = self.history.borrow().clone();
        if let Some(v) = open {
            v.dialog.force_close();
        }
    }
}
