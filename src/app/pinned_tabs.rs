//! The close control for a full-size workspace pin, which uses an ordinary AdwTab.

use adw::prelude::*;

pub(super) fn protect_full_pin(bar: &adw::TabBar, page: &adw::TabPage) {
    let Some(tab) = tab_widget(bar.upcast_ref(), page) else { return };
    if let Some(button) = child_with_class(&tab, "tab-close-button") {
        button.set_visible(false);
    }

    // The tab box's drag gesture is in the bubble phase. Claim a drag before it
    // gets there, while leaving clicks, focus, and the context menu to libadwaita.
    let drag = gtk::GestureDrag::new();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    drag.connect_drag_update(|drag, dx, dy| {
        if let Some(widget) = drag.widget() {
            if widget.drag_check_threshold(0, 0, dx as i32, dy as i32) {
                drag.set_state(gtk::EventSequenceState::Claimed);
            }
        }
    });
    tab.add_controller(drag);
}

pub(super) fn tab_widget(root: &gtk::Widget, page: &adw::TabPage) -> Option<gtk::Widget> {
    if root.type_().name() == "AdwTab"
        && root.find_property("page").is_some_and(|p| p.value_type() == adw::TabPage::static_type())
        && root.property::<Option<adw::TabPage>>("page").as_ref() == Some(page)
    {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(tab) = tab_widget(&widget, page) {
            return Some(tab);
        }
        child = widget.next_sibling();
    }
    None
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
