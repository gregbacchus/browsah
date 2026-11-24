//! Browsah's own tab strip, in place of `adw::TabBar`.
//!
//! libadwaita's tab bar never makes a tab narrower than about 118 px (the floor is in its private tab box and no CSS or property
//! moves it), so `tab_min_width` could only raise it. This widget draws the tabs of an `adw::TabView` itself: the view stays the
//! model (pages, selection, reordering, closing, transfer between windows), only the drawing and the pointer handling are ours.
//!
//! Structure: `tabstrip` (this widget) holds `[start action] [pinned tabs] [scroller > ordinary tabs] [end action]`. Each tab is a
//! `tabitem` (CSS node name) with an icon or spinner, an indicator button, the title and a close button. The style classes the rest
//! of the program paints (`browsah-group-*`, `browsah-pin-*`, `needs-attention`) go on the `tabitem`. Tabs share the room equally down to
//! the minimum width and the strip scrolls below that; a tab too narrow for its title shows only its icon (the tooltip has the title).

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

/// The narrowest an ordinary tab is by default (what libadwaita's strip used).
pub const DEFAULT_MIN_WIDTH: i32 = 130;
/// A pinned (icon only) tab's width.
const PINNED_WIDTH: i32 = 40;
/// Below this width a tab shows its icon only.
const TITLE_MIN_WIDTH: i32 = 72;
const GAP: i32 = 5;

thread_local! {
    /// The page being dragged and the view it came from (a drag between two windows of this process).
    static DRAG: RefCell<Option<(adw::TabPage, adw::TabView)>> = const { RefCell::new(None) };
}

/// One tab of the strip.
pub struct Item {
    page: adw::TabPage,
    pub widget: gtk::Box,
    icon: gtk::Image,
    spinner: gtk::Spinner,
    title: gtk::Label,
    indicator: gtk::Button,
    close: gtk::Button,
    closable: Cell<bool>,
    narrow: Cell<bool>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
}

impl Drop for Item {
    fn drop(&mut self) {
        for id in self.handlers.take() {
            self.page.disconnect(id);
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Parts {
        pub start: gtk::Box,
        pub end: gtk::Box,
        pub pinned: gtk::Box,
        pub regular: gtk::Box,
        pub scroller: gtk::ScrolledWindow,
    }

    #[derive(Default)]
    pub struct TabStrip {
        pub parts: OnceCell<Parts>,
        pub view: RefCell<Option<adw::TabView>>,
        pub items: RefCell<Vec<Rc<Item>>>,
        pub min_width: Cell<i32>,
        pub not_closable: RefCell<Vec<adw::TabPage>>,
        pub no_drag: RefCell<Vec<adw::TabPage>>,
        pub layout_pending: Cell<bool>,
        pub marked: RefCell<Option<gtk::Widget>>,
        /// libadwaita drops the pages model with its last reference and makes another on the next call: keep ours.
        pub pages: RefCell<Option<gtk::SelectionModel>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TabStrip {
        const NAME: &'static str = "BrowsahTabStrip";
        type Type = super::TabStrip;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("tabstrip");
        }
    }

    impl ObjectImpl for TabStrip {}
    impl WidgetImpl for TabStrip {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.obj().queue_layout();
        }
    }
    impl BoxImpl for TabStrip {}
}

glib::wrapper! {
    pub struct TabStrip(ObjectSubclass<imp::TabStrip>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl TabStrip {
    pub fn new(view: &adw::TabView) -> Self {
        let this: Self = glib::Object::builder().property("orientation", gtk::Orientation::Horizontal).property("spacing", 0).build();
        let imp = this.imp();
        install_css();
        imp.min_width.set(DEFAULT_MIN_WIDTH);
        *imp.view.borrow_mut() = Some(view.clone());
        let parts = imp::Parts::default();
        parts.pinned.set_spacing(GAP);
        parts.regular.set_spacing(GAP);
                parts.start.add_css_class("slot");
        parts.start.add_css_class("start");
        parts.end.add_css_class("slot");
        parts.end.add_css_class("end");
        parts.start.set_valign(gtk::Align::Center);
        parts.end.set_valign(gtk::Align::Center);
        parts.pinned.set_margin_end(GAP);
        parts.scroller.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        parts.scroller.set_hexpand(true);
        parts.scroller.set_child(Some(&parts.regular));
        this.append(&parts.start);
        this.append(&parts.pinned);
        this.append(&parts.scroller);
        this.append(&parts.end);
        imp.parts.set(parts).ok();
        this.set_focusable(true);
        this.set_focus_on_click(false);

        let weak = this.downgrade();
        let pages = view.pages();
        pages.connect_items_changed(move |_, _, _, _| {
            if let Some(s) = weak.upgrade() {
                s.sync();
            }
        });
        *imp.pages.borrow_mut() = Some(pages);
        // The pages model has been seen to stay silent when many pages go at once: the view's own signals are the safety net.
        let weak = this.downgrade();
        view.connect_page_detached(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                s.sync();
            }
        });
        let weak = this.downgrade();
        view.connect_page_attached(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                s.sync();
            }
        });
        let weak = this.downgrade();
        view.connect_page_reordered(move |_, _, _| {
            if let Some(s) = weak.upgrade() {
                s.sync();
            }
        });
        let weak = this.downgrade();
        view.connect_selected_page_notify(move |_| {
            if let Some(s) = weak.upgrade() {
                s.sync_selection();
            }
        });
        let weak = this.downgrade();
        this.parts().scroller.hadjustment().connect_page_size_notify(move |_| {
            if let Some(s) = weak.upgrade() {
                s.queue_layout();
            }
        });
        this.setup_drop();
        this.sync();
        this
    }

    fn parts(&self) -> &imp::Parts {
        self.imp().parts.get().expect("built in new")
    }

    fn view(&self) -> adw::TabView {
        self.imp().view.borrow().clone().expect("set in new")
    }

    pub fn set_start_action_widget(&self, w: Option<&impl IsA<gtk::Widget>>) {
        set_slot(&self.parts().start, w.map(|w| w.upcast_ref()));
    }

    pub fn set_end_action_widget(&self, w: Option<&impl IsA<gtk::Widget>>) {
        set_slot(&self.parts().end, w.map(|w| w.upcast_ref()));
    }

    pub fn start_action_widget(&self) -> Option<gtk::Widget> {
        self.parts().start.first_child()
    }

    pub fn end_action_widget(&self) -> Option<gtk::Widget> {
        self.parts().end.first_child()
    }

    /// The narrowest an ordinary tab gets (`tab_min_width`; `None` is the default).
    pub fn set_min_tab_width(&self, w: Option<i32>) {
        self.imp().min_width.set(w.unwrap_or(DEFAULT_MIN_WIDTH).max(24));
        self.queue_layout();
    }

    pub fn min_tab_width(&self) -> i32 {
        self.imp().min_width.get()
    }

    /// Hide (or show again) a page's close button: a workspace's full-size pin cannot be closed.
    pub fn set_page_closable(&self, page: &adw::TabPage, closable: bool) {
        let mut v = self.imp().not_closable.borrow_mut();
        v.retain(|p| p != page);
        if !closable {
            v.push(page.clone());
        }
        drop(v);
        if let Some(item) = self.item_of(page) {
            item.closable.set(closable);
            self.refresh(&item);
        }
    }

    /// Forbid (or allow) dragging a page.
    pub fn set_page_draggable(&self, page: &adw::TabPage, draggable: bool) {
        let mut v = self.imp().no_drag.borrow_mut();
        v.retain(|p| p != page);
        if !draggable {
            v.push(page.clone());
        }
    }

    fn item_of(&self, page: &adw::TabPage) -> Option<Rc<Item>> {
        self.imp().items.borrow().iter().find(|i| &i.page == page).cloned()
    }

    /// The widget that draws `page`'s tab.
    pub fn tab_widget(&self, page: &adw::TabPage) -> Option<gtk::Widget> {
        self.item_of(page).map(|i| i.widget.clone().upcast())
    }

    /// The page whose tab is under (x, y) of the strip.
    pub fn page_at(&self, x: f64, y: f64) -> Option<adw::TabPage> {
        let mut widget = self.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(w) = widget {
            if let Some(i) = self.imp().items.borrow().iter().find(|i| i.widget.upcast_ref::<gtk::Widget>() == &w) {
                return Some(i.page.clone());
            }
            widget = w.parent();
        }
        None
    }

    /// Bring the tabs in line with the view's pages: make widgets for new pages, drop the ones of pages that left, put them in order.
    fn sync(&self) {
        let imp = self.imp();
        let view = self.view();
        let parts = self.parts();
        let mut old: Vec<Rc<Item>> = imp.items.take();
        let mut items = Vec::new();
        for i in 0..view.n_pages() {
            let page = view.nth_page(i);
            let item = match old.iter().position(|it| it.page == page) {
                Some(at) => old.remove(at),
                None => self.make_item(&page),
            };
            items.push(item);
        }
        for gone in &old {
            if let Some(p) = gone.widget.parent() {
                if let Ok(b) = p.downcast::<gtk::Box>() {
                    b.remove(&gone.widget);
                }
            }
        }
        for it in &items {
            if let Some(p) = it.widget.parent() {
                if let Ok(b) = p.downcast::<gtk::Box>() {
                    b.remove(&it.widget);
                }
            }
            if it.page.is_pinned() {
                parts.pinned.append(&it.widget);
            } else {
                parts.regular.append(&it.widget);
            }
        }
        *imp.items.borrow_mut() = items.clone();
        for it in &items {
            self.refresh(it);
        }
        parts.pinned.set_visible(items.iter().any(|i| i.page.is_pinned()));
        self.sync_selection();
        self.queue_layout();
    }

    fn sync_selection(&self) {
        let selected = self.view().selected_page();
        for it in self.imp().items.borrow().iter() {
            if Some(&it.page) == selected.as_ref() {
                it.widget.set_state_flags(gtk::StateFlags::SELECTED, false);
            } else {
                it.widget.unset_state_flags(gtk::StateFlags::SELECTED);
            }
            self.refresh(it);
        }
        if let Some(p) = selected {
            let weak = self.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(s) = weak.upgrade() {
                    s.scroll_to(&p);
                }
            });
        }
    }

    /// Scroll the ordinary tabs so `page`'s tab is fully inside the strip.
    fn scroll_to(&self, page: &adw::TabPage) {
        let Some(item) = self.item_of(page) else { return };
        let parts = self.parts();
        let Some(b) = item.widget.compute_bounds(&parts.regular) else { return };
        let adj = parts.scroller.hadjustment();
        let (x, w) = (f64::from(b.x()), f64::from(b.width()));
        if x < adj.value() {
            adj.set_value(x);
        } else if x + w > adj.value() + adj.page_size() {
            adj.set_value(x + w - adj.page_size());
        }
    }

    /// Give the ordinary tabs their width (soon, once the strip has its size).
    fn queue_layout(&self) {
        let imp = self.imp();
        if imp.layout_pending.replace(true) {
            return;
        }
        let weak = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(s) = weak.upgrade() {
                s.imp().layout_pending.set(false);
                s.layout();
            }
        });
    }

    /// Give the tabs their widths: ordinary tabs share the strip equally, never narrower than `tab_min_width`; a tab narrower than
    /// `TITLE_MIN_WIDTH` is icon only. Runs (soon) after the tabs, the strip's size or the minimum change, and whenever the width of
    /// the visible part of the strip changes (the scroller's page size), so it is never left working from an old width.
    fn layout(&self) {
        let imp = self.imp();
        let parts = self.parts();
        let regular: Vec<Rc<Item>> = imp.items.borrow().iter().filter(|i| !i.page.is_pinned()).cloned().collect();
        for it in imp.items.borrow().iter().filter(|i| i.page.is_pinned()) {
            if it.widget.width_request() != PINNED_WIDTH {
                it.widget.set_size_request(PINNED_WIDTH, -1);
            }
            it.set_narrow(true);
            self.refresh(it);
        }
        let n = regular.len() as i32;
        if n == 0 {
            return;
        }
        let avail = parts.scroller.hadjustment().page_size().round() as i32;
        let avail = if avail > 0 { avail } else { parts.scroller.width() };
        if avail <= 0 {
            return;
        }
        let min = imp.min_width.get();
        let share = (avail - GAP * (n - 1)) / n;
        let w = share.max(min);
        let extra = if share >= min { (avail - GAP * (n - 1)) % n } else { 0 };
        for (i, it) in regular.iter().enumerate() {
            let width = w + i32::from((i as i32) < extra);
            if it.widget.width_request() != width {
                it.widget.set_size_request(width, -1);
            }
            it.set_narrow(width < TITLE_MIN_WIDTH);
            self.refresh(it);
        }
    }

    fn make_item(&self, page: &adw::TabPage) -> Rc<Item> {
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).css_name("tabitem").overflow(gtk::Overflow::Hidden).hexpand(false).build();
        let icon = gtk::Image::builder().pixel_size(16).build();
        let spinner = gtk::Spinner::new();
        let title = gtk::Label::builder().hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).width_chars(1).xalign(0.5).css_classes(["tab-title"]).build();
        let indicator = gtk::Button::builder().valign(gtk::Align::Center).css_classes(["flat", "tab-indicator"]).focusable(false).focus_on_click(false).build();
        let close = gtk::Button::builder().valign(gtk::Align::Center).icon_name("window-close-symbolic").css_classes(["flat", "tab-close-button"]).focusable(false).focus_on_click(false).tooltip_text("Close tab").build();
        widget.append(&icon);
        widget.append(&spinner);
        widget.append(&indicator);
        widget.append(&title);
        widget.append(&close);
        let item = Rc::new(Item {
            page: page.clone(),
            widget,
            icon,
            spinner,
            title,
            indicator,
            close,
            closable: Cell::new(!self.imp().not_closable.borrow().contains(page)),
            narrow: Cell::new(false),
            handlers: RefCell::new(Vec::new()),
        });
        // Page properties
        let weak_strip = self.downgrade();
        let weak_item = Rc::downgrade(&item);
        let id = page.connect_notify_local(None, move |_, _| {
            if let (Some(s), Some(it)) = (weak_strip.upgrade(), weak_item.upgrade()) {
                s.refresh(&it);
            }
        });
        item.handlers.borrow_mut().push(id);
        // Buttons
        let view = self.view();
        item.close.connect_clicked({
            let (view, page) = (view.clone(), page.clone());
            move |_| view.close_page(&page)
        });
        item.indicator.connect_clicked({
            let (view, page) = (view.clone(), page.clone());
            move |_| view.emit_by_name::<()>("indicator-activated", &[&page])
        });
        // Select on press, close on middle click.
        let press = gtk::GestureClick::new();
        press.set_button(0);
        press.connect_pressed({
            let (view, page) = (view.clone(), page.clone());
            move |g, _, _, _| match g.current_button() {
                1 => view.set_selected_page(&page),
                2 => {
                    if !page.is_pinned() {
                        view.close_page(&page);
                    }
                }
                _ => {}
            }
        });
        item.widget.add_controller(press);
        // Drag
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        source.connect_prepare({
            let (weak, page) = (self.downgrade(), page.clone());
            move |_, _, _| {
                let s = weak.upgrade()?;
                if s.imp().no_drag.borrow().contains(&page) {
                    return None;
                }
                Some(gdk::ContentProvider::for_value(&page.to_value()))
            }
        });
        source.connect_drag_begin({
            let (weak, page) = (self.downgrade(), page.clone());
            move |src, _| {
                let Some(s) = weak.upgrade() else { return };
                DRAG.with(|d| *d.borrow_mut() = Some((page.clone(), s.view())));
                if let Some(it) = s.item_of(&page) {
                    let paintable = gtk::WidgetPaintable::new(Some(&it.widget));
                    src.set_icon(Some(&paintable), it.widget.width() / 2, it.widget.height() / 2);
                }
            }
        });
        source.connect_drag_end(|_, _, _| DRAG.with(|d| *d.borrow_mut() = None));
        item.widget.add_controller(source);
        item
    }

    /// Show a page's properties in its tab.
    fn refresh(&self, it: &Item) {
        let page = &it.page;
        let selected = self.view().selected_page().as_ref() == Some(page);
        let pinned = page.is_pinned();
        let narrow = it.narrow.get();
        let icon = page.icon();
        let loading = page.is_loading();
        it.spinner.set_visible(loading);
        it.spinner.set_spinning(loading);
        it.icon.set_visible(!loading);
        match &icon {
            Some(i) => it.icon.set_from_gicon(i),
            None => it.icon.set_icon_name(Some("web-browser-symbolic")),
        }
        // A tab with no icon still has one when its title is hidden; with a title and no icon the space is not kept.
        it.icon.set_visible(!loading && (icon.is_some() || narrow || pinned));
        it.widget.set_hexpand(false);
        it.icon.set_hexpand(pinned || narrow);
        it.spinner.set_hexpand(pinned || narrow);
        it.title.set_label(&page.title());
        it.title.set_visible(!narrow && !pinned);
        let indicator = page.indicator_icon();
        it.indicator.set_visible(indicator.is_some() && !pinned && (!narrow || selected));
        if let Some(i) = &indicator {
            it.indicator.set_icon_name("");
            it.indicator.set_child(Some(&gtk::Image::builder().gicon(i).pixel_size(14).build()));
        }
        it.indicator.set_sensitive(page.is_indicator_activatable());
        it.indicator.set_tooltip_text(Some(page.indicator_tooltip().as_str()).filter(|t| !t.is_empty()));
        it.close.set_visible(!pinned && it.closable.get() && !narrow);
        let tip = page.tooltip();
        let tip = tip.filter(|t| !t.is_empty()).unwrap_or_else(|| page.title());
        it.widget.set_tooltip_text(Some(tip.as_str()).filter(|t| !t.is_empty()));
        if page.needs_attention() {
            it.widget.add_css_class("needs-attention");
        } else {
            it.widget.remove_css_class("needs-attention");
        }
        if pinned {
            it.widget.add_css_class("pinned");
        } else {
            it.widget.remove_css_class("pinned");
        }
    }

    // ---- drag and drop ------------------------------------------------------------------------------------------------------

    fn setup_drop(&self) {
        let target = gtk::DropTarget::new(adw::TabPage::static_type(), gdk::DragAction::MOVE);
        target.connect_motion({
            let weak = self.downgrade();
            move |_, x, y| {
                if let Some(s) = weak.upgrade() {
                    s.mark_drop(Some(s.gap_at(x, y)));
                }
                gdk::DragAction::MOVE
            }
        });
        target.connect_leave({
            let weak = self.downgrade();
            move |_| {
                if let Some(s) = weak.upgrade() {
                    s.mark_drop(None);
                }
            }
        });
        target.connect_drop({
            let weak = self.downgrade();
            move |_, value, x, y| {
                let Some(s) = weak.upgrade() else { return false };
                s.mark_drop(None);
                let Ok(page) = value.get::<adw::TabPage>() else { return false };
                let gap = s.gap_at(x, y);
                s.drop_page(&page, gap)
            }
        });
        self.add_controller(target);
    }

    /// The gap (0 = before the first tab, n = after the last) nearest to (x, y).
    fn gap_at(&self, x: f64, y: f64) -> i32 {
        let _ = y;
        let items = self.imp().items.borrow();
        for (i, it) in items.iter().enumerate() {
            let Some(b) = it.widget.compute_bounds(self) else { continue };
            if !it.widget.is_mapped() || b.width() <= 0.0 {
                continue;
            }
            if x < f64::from(b.x() + b.width() / 2.0) {
                return i as i32;
            }
        }
        items.len() as i32
    }

    fn mark_drop(&self, gap: Option<i32>) {
        let imp = self.imp();
        if let Some(old) = imp.marked.take() {
            old.remove_css_class("drop-before");
            old.remove_css_class("drop-after");
        }
        let Some(gap) = gap else { return };
        let items = imp.items.borrow();
        let widget = if (gap as usize) < items.len() { items.get(gap as usize).map(|i| (i.widget.clone(), "drop-before")) } else { items.last().map(|i| (i.widget.clone(), "drop-after")) };
        if let Some((w, class)) = widget {
            w.add_css_class(class);
            *imp.marked.borrow_mut() = Some(w.upcast());
        }
    }

    /// Put `page` (dragged from this or another window) into gap `gap` of this strip. Returns whether it moved.
    pub fn drop_page(&self, page: &adw::TabPage, gap: i32) -> bool {
        let view = self.view();
        let pinned = view.n_pinned_pages();
        let from = DRAG.with(|d| d.borrow().as_ref().filter(|(p, _)| p == page).map(|(_, v)| v.clone()));
        let from = from.or_else(|| (0..view.n_pages()).any(|i| &view.nth_page(i) == page).then(|| view.clone()));
        let Some(from) = from else { return false };
        let gap = if page.is_pinned() { gap.min(pinned) } else { gap.max(pinned) };
        if from == view {
            let pos = view.page_position(page);
            let at = if pos < gap { gap - 1 } else { gap };
            view.reorder_page(page, at)
        } else {
            let gap = gap.clamp(0, view.n_pages());
            from.transfer_page(page, &view, gap);
            true
        }
    }
}

fn set_slot(slot: &gtk::Box, w: Option<&gtk::Widget>) {
    while let Some(c) = slot.first_child() {
        slot.remove(&c);
    }
    // An empty slot, or one whose button is hidden, takes no room (its margin would leave a gap beside the tabs).
    slot.set_visible(w.is_some_and(|w| w.is_visible()));
    if let Some(w) = w {
        slot.append(w);
        let slot = slot.clone();
        w.connect_visible_notify(move |w| slot.set_visible(w.is_visible()));
    }
}

impl Item {
    fn set_narrow(&self, narrow: bool) {
        self.narrow.set(narrow);
    }
}

/// The strip's own style: look, spacing and the drop marks. The window colour, group and pin rules are in `workspace_css` and `groups.rs`.
fn install_css() {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "tabstrip { padding: 3px 10px; }\n\
         tabstrip:backdrop, tabstrip:backdrop tabitem { color: @headerbar_fg_color; }\n\
         tabstrip .slot.start { margin-right: 8px; }\n\
         tabstrip .slot.end { margin-left: 8px; }\n\
         tabstrip .slot button { min-height: 34px; min-width: 34px; padding: 0; }\n\
         tabstrip.inline { padding: 0 6px; min-height: 0; }\n\
         tabitem { color: inherit; min-height: 34px; padding: 0 8px; border-radius: 9px; }\n\
         tabitem:hover { background-color: alpha(currentColor, 0.08); }\n\
         tabitem:selected { background-color: alpha(currentColor, 0.18); }\n\
         tabitem.pinned { padding: 0; }\n\
         tabitem button { min-width: 20px; min-height: 20px; padding: 0; margin: 0; border-radius: 99px; background: none; box-shadow: none; color: inherit; }\n\
         tabitem button:hover { background-color: alpha(currentColor, 0.16); }\n\
         tabitem:not(:selected):not(:hover) button.tab-close-button { opacity: 0; }\n\
         tabitem.needs-attention .tab-title { font-style: italic; }\n\
         tabitem.drop-before { box-shadow: inset 3px 0 0 @accent_bg_color; }\n\
         tabitem.drop-after { box-shadow: inset -3px 0 0 @accent_bg_color; }\n",
    );
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION - 1);
}
