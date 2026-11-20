//! History menus for the navigation toolbar.
use super::{Shell, Tab};
use crate::navhistory::{self, Entry};
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy)]
enum Direction {
    Back,
    Forward,
}

#[derive(Clone, Copy)]
enum Target {
    Browser(usize),
    Saved(usize),
}

impl Direction {
    fn saved(self, tab: &Tab) -> &std::cell::RefCell<Vec<Entry>> {
        match self {
            Self::Back => &tab.hist_back,
            Self::Forward => &tab.hist_fwd,
        }
    }
}

impl Shell {
    pub(super) fn connect_navigation_menus(self: &Rc<Self>) {
        for (button, direction) in [(&self.back, Direction::Back), (&self.forward, Direction::Forward)] {
            let hold = gtk::GestureLongPress::builder().button(1).propagation_phase(gtk::PropagationPhase::Capture).build();
            hold.connect_pressed({
                let shell = Rc::downgrade(self);
                move |gesture, _, _| {
                    if let Some(shell) = shell.upgrade() {
                        // Cancel GtkButton's click gesture before the release can take a single step.
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        shell.show_navigation_menu(direction);
                    }
                }
            });
            button.add_controller(hold);
        }
    }

    fn show_navigation_menu(self: &Rc<Self>, direction: Direction) {
        let Some(tab) = self.current() else { return };
        let anchor = match direction { Direction::Back => &self.back, Direction::Forward => &self.forward };
        if anchor.last_child().is_some_and(|child| child.is::<gtk::Popover>()) {
            return;
        }
        let view = tab.view();
        let snapshot = Rc::new(view.nav_entries());
        let (entries, current) = snapshot.as_ref();
        let address = tab.state.borrow().address.clone();
        let browser_rows: Vec<_> = match direction {
            Direction::Back => entries.iter().enumerate().take(*current).rev().collect(),
            Direction::Forward => entries.iter().enumerate().skip(current + 1).collect(),
        };
        let saved = direction.saved(&tab).borrow();
        let rows = browser_rows.into_iter().map(|(i, e)| (Target::Browser(i), e))
            .chain(saved.iter().enumerate().rev().map(|(i, e)| (Target::Saved(i), e)));

        let popover = gtk::Popover::builder().css_classes(["browsah-navigation-history"]).build();
        popover.set_parent(anchor);
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(360).build();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroller = gtk::ScrolledWindow::builder().child(&list).propagate_natural_height(true)
            .propagate_natural_width(true).max_content_height(360).hscrollbar_policy(gtk::PolicyType::Never).build();
        for (target, entry) in rows {
            let name = if entry.title.trim().is_empty() { &entry.url } else { &entry.title };
            let button = history_row(name, Some(&entry.url));
            button.set_tooltip_text(Some(&entry.url));
            let (shell, tab, popover) = (Rc::downgrade(self), Rc::downgrade(&tab), popover.downgrade());
            let (snapshot, entry, address) = (snapshot.clone(), entry.clone(), address.clone());
            button.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                let (Some(shell), Some(tab)) = (shell.upgrade(), tab.upgrade()) else { return };
                if !shell.current().is_some_and(|current| Rc::ptr_eq(&current, &tab))
                    || tab.state.borrow().address != address || tab.view().nav_entries() != *snapshot {
                    return;
                }
                match target {
                    Target::Browser(index) => jump_browser(&tab, snapshot.1, index, &address, &entry.url),
                    Target::Saved(index) => {
                        let still_saved = direction.saved(&tab).borrow().get(index) == Some(&entry);
                        if still_saved {
                            jump_saved(&tab, direction, index, &snapshot, &entry.url);
                        }
                    }
                }
            });
            list.append(&button);
        }
        if list.first_child().is_some() {
            content.append(&scroller);
            content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        let more = super::menu::row("More history…", &super::menu::icon_widget(Some("rotate-ccw-clock")), None, false, true, false);
        more.connect_clicked({
            let (shell, popover) = (Rc::downgrade(self), popover.downgrade());
            move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                if let Some(shell) = shell.upgrade() {
                    shell.open_history();
                }
            }
        });
        content.append(&more);
        popover.set_child(Some(&content));
        popover.connect_closed(|p| p.unparent());
        popover.popup();
    }
}

/// A row of the history menu: the page's icon (remembered per site) or a generic one, then its title.
fn history_row(text: &str, url: Option<&str>) -> gtk::Button {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(10).build();
    match url.and_then(super::favicon::small_image) {
        Some(icon) => row.append(&icon),
        None => row.append(&super::menu::icon_widget(Some("link-2"))),
    }
    row.append(&gtk::Label::builder().label(text).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(64).build());
    gtk::Button::builder().child(&row).has_frame(false).build()
}

/// Use Chromium's entry ID, so a jump preserves the native history and works with repeated URLs.
fn jump_browser(tab: &Rc<Tab>, current: usize, target: usize, address: &str, url: &str) {
    let view = tab.view();
    let request = view.cdp("Page.getNavigationHistory");
    let (address, url) = (address.to_owned(), url.to_owned());
    let tab = Rc::downgrade(tab);
    glib::MainContext::default().spawn_local(async move {
        for _ in 0..50 {
            if let Some((ok, json)) = view.cdp_result(request) {
                let Some(tab) = tab.upgrade() else { return };
                if !ok || !Rc::ptr_eq(&view, &tab.view()) || tab.state.borrow().address != address {
                    return;
                }
                let Ok(history) = serde_json::from_str::<serde_json::Value>(&json) else { return };
                if history["currentIndex"].as_u64() == Some(current as u64)
                    && history["entries"][current]["url"].as_str() == Some(address.as_str())
                    && history["entries"][target]["url"].as_str() == Some(url.as_str()) {
                    if let Some(id) = history["entries"][target]["id"].as_i64().and_then(|id| i32::try_from(id).ok()) {
                        view.go_to_navigation_entry(id);
                    }
                }
                return;
            }
            glib::timeout_future(Duration::from_millis(20)).await;
        }
    });
}

/// A saved entry lives beyond Chromium's list. Move the intervening pages to the opposite stack.
fn jump_saved(tab: &Tab, direction: Direction, index: usize, native: &(Vec<Entry>, usize), url: &str) {
    let (mut back, mut forward) = navhistory::fold(&tab.hist_back.borrow(), &native.0, native.1, &tab.hist_fwd.borrow());
    let here = {
        let state = tab.state.borrow();
        Entry::new(&state.address, &state.title)
    };
    match direction {
        Direction::Back => {
            forward.push(here);
            forward.extend(back.drain(index + 1..).rev());
            back.pop();
        }
        Direction::Forward => {
            back.push(here);
            back.extend(forward.drain(index + 1..).rev());
            forward.pop();
        }
    }
    *tab.hist_back.borrow_mut() = back;
    *tab.hist_fwd.borrow_mut() = forward;
    tab.skip_navigation_starts();
    let view = tab.view();
    view.forget_other_entries();
    view.step_to(url, true);
}
