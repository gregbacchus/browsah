//! The page's context menu, drawn and run. What it offers is decided by `crate::ctxmenu::plan` from what the click was on (CEF's
//! `ContextMenuParams`, captured in `view.rs`) and what the window knows (bookmarks, workspaces, the router); this file fills in
//! that knowledge, draws the entries as a popover with icons, and carries out the chosen action.

use super::view::{BrowserView, MenuRequest};
use super::{store, Shell, NEW_EPHEMERAL, PROFILE};
use crate::ctxmenu::{self, Action, Context, Entry, How, Item, MediaCmd, PageCmd, Target, Targets};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::rc::Rc;
use std::sync::Arc;

/// A workspace's colour badge, as in the workspace picker.
fn badge(color: Option<&str>) -> gtk::Image {
    let icon = gio::BytesIcon::new(&glib::Bytes::from_owned(crate::picker::badge_svg(color).into_bytes()));
    let image = gtk::Image::from_gicon(&icon);
    image.set_pixel_size(16);
    image
}

/// The icon widget of an item: its Lucide icon, a workspace badge, or empty space of the same size so labels line up.
fn icon_widget(item: &Item) -> gtk::Widget {
    let dark = adw::StyleManager::default().is_dark();
    match &item.badge {
        Some(color) => badge(crate::picker::badge_color(color.as_ref(), dark)).upcast(),
        None => super::menu::icon_widget(item.icon),
    }
}

/// One row of the page menu (the shared row of `menu::row`).
pub(super) fn row_button(item: &Item, chevron: bool) -> gtk::Button {
    super::menu::row(&item.label, &icon_widget(item), item.shortcut, item.checked, item.enabled, chevron)
}

impl Shell {
    /// What the router would do with a link from this window: a workspace, web app or profile that is not simply "here".
    fn link_route(&self, url: &str, targets: &Targets) -> Option<(crate::router::Action, String)> {
        use crate::router::{route, Action as R, Kind, Request, Source, Target as T};
        let url = crate::router::unwrap_url(url);
        let decision = route(&Request { url, source: Source::Tab(self.workspace_id().map(str::to_owned)), kind: Kind::Link, target: T::NewTab });
        let find = |list: &[Target], id: &str| list.iter().find(|t| t.id == id).map(|t| t.name.clone());
        match &decision.action {
            R::Workspace(id) if Some(id.as_str()) != self.workspace_id() => find(&targets.workspaces, id).map(|n| (decision.action.clone(), n)),
            R::App(id) => find(&targets.apps, id).map(|n| (decision.action.clone(), n)),
            R::Profile { profile, workspace } => find(&targets.profiles, profile).map(|n| (decision.action.clone(), if workspace.is_some() { format!("{n} (workspace)") } else { n })),
            _ => None,
        }
    }

    /// The workspaces, web apps and profiles a link or tab can be sent to.
    fn menu_targets(&self) -> Targets {
        let here = self.workspace_id().map(str::to_owned);
        let workspaces: Vec<Target> = super::picker::entries().into_iter().filter(|e| Some(&e.id) != here.as_ref()).map(|e| Target { id: e.id, name: e.name, color: e.color }).collect();
        let Some(profile) = PROFILE.with(|p| p.borrow().clone()) else { return Targets { send_to: workspaces.clone(), workspaces, ..Targets::default() } };
        let apps = crate::workspace::Workspace::list_apps(&profile).into_iter().filter(|a| Some(&a.id) != here.as_ref()).map(|a| Target { name: a.display_name().to_owned(), id: a.id, color: None }).collect();
        let profiles = crate::profile::list().into_iter().filter(|p| p.id != profile.id).map(|p| Target { name: p.display_name().to_owned(), id: p.id, color: None }).collect();
        Targets { send_to: workspaces.clone(), workspaces, apps, profiles }
    }

    /// Show the menu for a click in `view` (a tab's page, or a modal's with `in_modal`).
    pub fn show_page_menu(self: &Rc<Self>, view: &Rc<BrowserView>, req: Arc<MenuRequest>, in_modal: bool) {
        let mut ctx: Context = req.ctx.clone();
        let targets = self.menu_targets();
        let tab = self.current().filter(|t| Rc::ptr_eq(&t.view(), view));
        ctx.in_modal = in_modal || tab.is_none();
        ctx.in_app = self.workspace.as_ref().is_some_and(|w| w.is_app());
        if let Some(tab) = &tab {
            let s = tab.state.borrow();
            ctx.loading = s.loading;
            ctx.can_back = s.can_back || !tab.hist_back.borrow().is_empty();
            ctx.can_forward = s.can_forward || !tab.hist_fwd.borrow().is_empty();
            ctx.can_send_tab = !self.page_is_pinned(&tab.page);
            if let (Some(ws), Some(st), Some((url, _))) = (self.bookmark_workspace(), store(), self.bookmarkable()) {
                ctx.can_bookmark = true;
                ctx.bookmarked = st.has_bookmark(ws, &url);
            }
        } else {
            ctx.can_back = true;
            ctx.can_forward = true;
        }
        if !ctx.link_url.is_empty() {
            ctx.link_route = self.link_route(&ctx.link_url, &targets);
        }
        let selection = ctx.selection.trim();
        if !selection.is_empty() && !selection.chars().any(char::is_whitespace) {
            let resolved = super::nav::resolve(selection, "SEARCH:{query}");
            if !resolved.starts_with("SEARCH:") && resolved != "about:blank" {
                ctx.selection_url = Some(resolved);
            }
        }
        let opts = self.settings.borrow().context_menu.options(&self.search_url.borrow());
        let plan = ctxmenu::plan(&ctx, &targets, &opts);

        let popover = gtk::Popover::new();
        popover.set_parent(&view.widget);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(req.x as i32, req.y as i32, 1, 1)));
        let stack = gtk::Stack::builder().vhomogeneous(false).hhomogeneous(false).build();
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.set_width_request(250);
        let run = {
            let (shell, view, req, ctx, popover) = (Rc::downgrade(self), Rc::downgrade(view), req.clone(), Rc::new(ctx), popover.downgrade());
            Rc::new(move |action: Action| {
                // Answer CEF first for its own commands; closing the popover cancels the request if it is still unanswered.
                let (Some(shell), Some(view)) = (shell.upgrade(), view.upgrade()) else {
                    req.cancel();
                    return;
                };
                shell.run_menu_action(&view, &req, &ctx, action);
                if let Some(p) = popover.upgrade() {
                    p.popdown();
                }
            })
        };
        let connect = |button: &gtk::Button, action: Action| {
            let run = run.clone();
            button.connect_clicked(move |_| run(action.clone()));
        };
        let mut sub = 0;
        for entry in &plan {
            match entry {
                Entry::Separator => main.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
                Entry::Item(item) => {
                    let b = row_button(item, false);
                    connect(&b, item.action.clone());
                    main.append(&b);
                }
                Entry::Row(items) => {
                    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).homogeneous(true).build();
                    for item in items {
                        let image = gtk::Image::new();
                        image.set_pixel_size(16);
                        if let Some(icon) = item.icon.and_then(crate::icons::by_id) {
                            crate::icons::bind(&image, icon);
                        }
                        let tip = match item.shortcut {
                            Some(k) => format!("{} ({k})", item.label),
                            None => item.label.clone(),
                        };
                        let b = gtk::Button::builder().child(&image).has_frame(false).sensitive(item.enabled).tooltip_text(tip).build();
                        b.update_property(&[gtk::accessible::Property::Label(item.label.as_str())]);
                        connect(&b, item.action.clone());
                        row.append(&b);
                    }
                    main.append(&row);
                }
                Entry::Submenu { label, icon, items, .. } => {
                    sub += 1;
                    let name = format!("sub{sub}");
                    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    page.set_width_request(250);
                    let back_item = Item { id: "back", label: label.clone(), icon: Some("chevron-left"), shortcut: None, enabled: true, checked: false, badge: None, action: Action::Page(PageCmd::Reload) };
                    let back = row_button(&back_item, false);
                    back.connect_clicked({
                        let stack = stack.downgrade();
                        move |_| {
                            if let Some(s) = stack.upgrade() {
                                s.set_visible_child_name("main");
                            }
                        }
                    });
                    page.append(&back);
                    page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                    for item in items {
                        let b = row_button(item, false);
                        connect(&b, item.action.clone());
                        page.append(&b);
                    }
                    stack.add_named(&page, Some(&name));
                    let parent = Item { id: "open-workspace", label: label.clone(), icon: *icon, shortcut: None, enabled: true, checked: false, badge: None, action: Action::Page(PageCmd::Reload) };
                    let b = row_button(&parent, true);
                    b.connect_clicked({
                        let stack = stack.downgrade();
                        move |_| {
                            if let Some(s) = stack.upgrade() {
                                s.set_visible_child_name(&name);
                            }
                        }
                    });
                    main.append(&b);
                }
            }
        }
        stack.add_named(&main, Some("main"));
        stack.set_visible_child_name("main");
        popover.set_child(Some(&stack));
        popover.connect_closed({
            let req = req.clone();
            move |p| {
                req.cancel(); // chosen items have already answered; this covers dismissal
                p.unparent();
            }
        });
        popover.popup();
    }

    /// Carry out a menu item. `ctx` is what the menu was made for.
    fn run_menu_action(self: &Rc<Self>, view: &Rc<BrowserView>, req: &Arc<MenuRequest>, ctx: &Context, action: Action) {
        let clipboard = WidgetExt::display(&view.widget).clipboard();
        let in_tab = !ctx.in_modal;
        let win = |name: &str| {
            let _ = WidgetExt::activate_action(&self.window, name, None);
        };
        match action {
            Action::Cef(id) => req.run(id),
            Action::Copy(text) => clipboard.set_text(&text),
            Action::SaveAs(url) => view.start_download(&url),
            Action::SendTab(id) => {
                *self.menu_page.borrow_mut() = self.tab_view.selected_page();
                self.send_menu_tab(&id);
            }
            Action::BookmarkLink(url) => {
                if let (Some(ws), Some(st)) = (self.bookmark_workspace(), store()) {
                    let _ = st.add_bookmark(ws, &url, &url);
                    self.rebuild_bookmarks();
                    self.toast("Link bookmarked");
                }
            }
            Action::CopyImage(url) => {
                view.fetch_image(&url);
                let (view, weak, started) = (view.clone(), Rc::downgrade(self), std::time::Instant::now());
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    if let Some(png) = view.image_png(&url) {
                        match gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)) {
                            Ok(texture) => WidgetExt::display(&view.widget).clipboard().set_texture(&texture),
                            Err(e) => {
                                if let Some(s) = weak.upgrade() {
                                    s.toast(&format!("Cannot copy the image: {e}"));
                                }
                            }
                        }
                        return glib::ControlFlow::Break;
                    }
                    if started.elapsed() > std::time::Duration::from_secs(6) {
                        if let Some(s) = weak.upgrade() {
                            s.toast("Could not fetch the image to copy it");
                        }
                        return glib::ControlFlow::Break;
                    }
                    glib::ControlFlow::Continue
                });
            }
            Action::Open { url, how } => match how {
                How::Tab { select } => {
                    if self.allow_new_tabs {
                        self.open_tab(&url, select);
                    } else if self.popup_policy == crate::workspace::PopupPolicy::Modal {
                        self.open_modal(&url);
                    } else if let Some(f) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                        f(&url);
                    }
                }
                How::Window => {
                    if let Some(f) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) {
                        f(&url);
                    }
                }
                How::Popup => self.open_modal(&url),
                How::Route(route) => {
                    super::perform_route(&route, &url, Some(self));
                }
            },
            Action::Media(cmd) => {
                let (x, y) = (req.point.0 as f64 / view.user_zoom(), req.point.1 as f64 / view.user_zoom());
                let op = match cmd {
                    MediaCmd::PlayPause => "m.paused ? m.play() : m.pause()",
                    MediaCmd::Mute => "m.muted = !m.muted",
                    MediaCmd::Loop => "m.loop = !m.loop",
                    MediaCmd::Controls => "m.controls = !m.controls",
                };
                view.cdp_eval(&format!("(() => {{ const e = document.elementFromPoint({x}, {y}); const m = (e && e.closest && e.closest('video, audio')) || document.querySelector('video, audio'); if (m) {{ {op}; }} return 'ok'; }})()"));
            }
            Action::Page(cmd) => match cmd {
                PageCmd::Back => {
                    if in_tab {
                        self.history_back();
                    } else {
                        view.go_back();
                    }
                }
                PageCmd::Forward => {
                    if in_tab {
                        self.history_forward();
                    } else {
                        view.go_forward();
                    }
                }
                PageCmd::Reload => view.reload(),
                PageCmd::HardReload => view.reload_ignore_cache(),
                PageCmd::Stop => view.stop(),
                PageCmd::Bookmark => self.toggle_bookmark(),
                PageCmd::Find => win("win.find"),
                PageCmd::SavePdf => win("win.save-pdf"),
                PageCmd::Print => win("win.print"),
                PageCmd::InstallApp => win("win.install-app"),
                PageCmd::Inspect => view.inspect_at(req.point.0, req.point.1),
                PageCmd::SelectAll => req.run(cef::sys::cef_menu_id_t::MENU_ID_SELECT_ALL as i32),
                PageCmd::CopySelection => {
                    let sel = view.selection();
                    if !sel.is_empty() {
                        clipboard.set_text(&sel);
                    }
                }
                PageCmd::Undo => req.run(cef::sys::cef_menu_id_t::MENU_ID_UNDO as i32),
                PageCmd::Redo => req.run(cef::sys::cef_menu_id_t::MENU_ID_REDO as i32),
                PageCmd::Delete => req.run(cef::sys::cef_menu_id_t::MENU_ID_DELETE as i32),
                PageCmd::Cut => {
                    let sel = view.selection();
                    if !sel.is_empty() {
                        clipboard.set_text(&sel);
                    }
                    req.run(cef::sys::cef_menu_id_t::MENU_ID_CUT as i32);
                }
                PageCmd::Paste => view.paste_from_clipboard(),
                PageCmd::CopyLinkText => {
                    let (x, y) = (req.point.0 as f64 / view.user_zoom(), req.point.1 as f64 / view.user_zoom());
                    let js = format!("(() => {{ const e = document.elementFromPoint({x}, {y}); const a = e && e.closest && e.closest('a'); return a ? (a.innerText || a.textContent || a.getAttribute('aria-label') || a.title || '').trim() : ''; }})()");
                    let weak = Rc::downgrade(self);
                    let display = WidgetExt::display(&view.widget);
                    view.eval_string_async(&js, move |text| match (text.filter(|t| !t.is_empty()), weak.upgrade()) {
                        (Some(t), _) => display.clipboard().set_text(&t),
                        (None, Some(s)) => s.toast("That link has no text"),
                        _ => {}
                    });
                }
            },
        }
    }
}
