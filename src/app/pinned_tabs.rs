//! The close control for a full-size workspace pin, which is an ordinary tab of the strip.

use super::tabstrip::TabStrip;
use adw::prelude::*;

/// A full-size pin shows no close button and cannot be dragged (the order of the pins is the workspace's).
pub(super) fn protect_full_pin(bar: &TabStrip, page: &adw::TabPage) {
    bar.set_page_closable(page, false);
    bar.set_page_draggable(page, false);
}

/// The widget of the strip that draws `page`'s tab.
pub(super) fn tab_widget(bar: &TabStrip, page: &adw::TabPage) -> Option<gtk::Widget> {
    bar.tab_widget(page)
}

pub(super) fn child_with_class(tab: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    let mut child = tab.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(class) {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}
