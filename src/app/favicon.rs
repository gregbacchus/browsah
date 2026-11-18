//! Favicons: the page's icon on its tab. CEF reports the icon addresses (`view.rs`), the first that downloads comes back as a PNG,
//! and it is remembered per site (origin) in the profile database so that restored tabs, suggestions and bookmarks have it before
//! a page has loaded. A grouped tab shows its group's colour as a dot on the icon.

use super::{now_secs, store, Shell, Tab};
use crate::groups::dot_svg;
use gtk::{gdk, gio, glib, graphene, gsk, prelude::*};
use std::rc::Rc;

/// `scheme://host[:port]` of a web address, lower case (`None` for anything that is not http or https).
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?.to_ascii_lowercase();
    (!authority.is_empty()).then(|| format!("{scheme}://{authority}"))
}

/// A texture from PNG bytes.
pub fn texture_from_png(png: &[u8]) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from(png)).ok()
}

thread_local! {
    /// Decoded icons by origin (`None`: none stored), so lists of many rows do not read and decode the database for each.
    static CACHE: std::cell::RefCell<std::collections::HashMap<String, Option<gdk::Texture>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The icon cached for an address's site, if there is one.
pub fn cached_for(url: &str) -> Option<gdk::Texture> {
    let origin = origin_of(url)?;
    CACHE.with(|c| {
        if let Some(hit) = c.borrow().get(&origin) {
            return hit.clone();
        }
        let tex = store().and_then(|s| s.favicon(&origin)).and_then(|png| texture_from_png(&png));
        c.borrow_mut().insert(origin, tex.clone());
        tex
    })
}

/// A 16 px image of the site's icon for a list row or button (`None` when there is none).
pub fn small_image(url: &str) -> Option<gtk::Image> {
    let tex = cached_for(url)?;
    let image = gtk::Image::from_paintable(Some(&tex));
    image.set_pixel_size(16);
    image.set_valign(gtk::Align::Center);
    Some(image)
}

/// `base` with a dot of `color` in its corner (a white ring around it keeps it visible on any icon).
pub fn with_dot(window: &adw::ApplicationWindow, base: &gdk::Texture, color: &str) -> Option<gdk::Texture> {
    let color = gdk::RGBA::parse(color).ok()?;
    let white = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
    let size = 32.0;
    let snapshot = gtk::Snapshot::new();
    let all = graphene::Rect::new(0.0, 0.0, size, size);
    snapshot.append_texture(base, &all);
    for (rect, rgba) in [(graphene::Rect::new(16.0, 16.0, 16.0, 16.0), white), (graphene::Rect::new(19.0, 19.0, 10.0, 10.0), color)] {
        let r = gsk::RoundedRect::from_rect(rect, rect.width() / 2.0);
        snapshot.push_rounded_clip(&r);
        snapshot.append_color(&rgba, &rect);
        snapshot.pop();
    }
    let node = snapshot.to_node()?;
    let renderer = window.native()?.renderer()?;
    Some(renderer.render_texture(&node, Some(&all)))
}

/// `base` without colour.
pub fn grayscale(window: &adw::ApplicationWindow, base: &gdk::Texture) -> Option<gdk::Texture> {
    let size = 32.0;
    let all = graphene::Rect::new(0.0, 0.0, size, size);
    let snapshot = gtk::Snapshot::new();
    // Rec. 709 luma into every channel; alpha is kept.
    let m = graphene::Matrix::from_float([
        0.2126, 0.2126, 0.2126, 0.0, //
        0.7152, 0.7152, 0.7152, 0.0, //
        0.0722, 0.0722, 0.0722, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);
    snapshot.push_color_matrix(&m, &graphene::Vec4::zero());
    snapshot.append_texture(base, &all);
    snapshot.pop();
    let node = snapshot.to_node()?;
    let renderer = window.native()?.renderer()?;
    Some(renderer.render_texture(&node, Some(&all)))
}

impl Shell {
    /// Dim a sleeping tab (and show it, below, with a colourless icon) as `sleeping_tabs` says; an awake tab looks as usual.
    pub(super) fn refresh_sleep_look(&self, tab: &Rc<Tab>) {
        let opacity = if tab.view().is_dormant() { self.settings.borrow().sleeping_tabs.opacity } else { 1.0 };
        let (bar, page) = (self.tab_bar.clone(), tab.page.clone());
        let apply = move || {
            if let Some(w) = super::pinned_tabs::tab_widget(bar.upcast_ref(), &page) {
                w.set_opacity(opacity);
            }
        };
        apply();
        glib::idle_add_local_once(apply); // a tab just added has no widget yet
    }

    /// The page's icon arrived (or, empty, it has none): show it, and remember it for the site.
    pub(super) fn set_favicon(&self, tab: &Rc<Tab>, png: Vec<u8>) {
        if png.is_empty() {
            *tab.favicon.borrow_mut() = None;
        } else {
            let Some(texture) = texture_from_png(&png) else { return };
            *tab.favicon.borrow_mut() = Some(texture.clone());
            if let (Some(origin), Some(st)) = (origin_of(&tab.state.borrow().address), store()) {
                if !self.private.get() {
                    let _ = st.set_favicon(&origin, &png, now_secs());
                    CACHE.with(|c| c.borrow_mut().insert(origin, Some(texture)));
                }
            }
        }
        self.refresh_tab_icon(tab);
    }

    /// The tab's address changed: if it is another site, show that site's remembered icon (or none) until the page gives its own.
    pub(super) fn favicon_for_address(&self, tab: &Rc<Tab>) {
        let address = tab.state.borrow().address.clone();
        let origin = origin_of(&address).unwrap_or_default();
        if *tab.favicon_origin.borrow() == origin {
            return;
        }
        *tab.favicon_origin.borrow_mut() = origin;
        *tab.favicon.borrow_mut() = cached_for(&address);
        self.refresh_tab_icon(tab);
    }

    /// Put the right icon on the tab's page: its favicon (with its group's dot), a plain dot for a grouped tab with no icon, a generic
    /// icon for a pinned tab, else none.
    pub(super) fn refresh_tab_icon(&self, tab: &Rc<Tab>) {
        let mut favicon = tab.favicon.borrow().clone();
        if tab.view().is_dormant() && self.settings.borrow().sleeping_tabs.grayscale_icon {
            favicon = favicon.map(|f| grayscale(&self.window, &f).unwrap_or(f));
        }
        let group = self.group(tab.group.get());
        let icon: Option<gio::Icon> = match (group, favicon) {
            (Some(g), Some(f)) => with_dot(&self.window, &f, &g.color).map(|t| t.upcast()).or_else(|| Some(f.upcast())),
            (Some(g), None) => Some(gio::BytesIcon::new(&glib::Bytes::from_owned(dot_svg(&g.color).into_bytes())).upcast()),
            (None, Some(f)) => Some(f.upcast()),
            (None, None) if tab.page.is_pinned() => Some(gio::ThemedIcon::new("web-browser-symbolic").upcast()),
            (None, None) => None,
        };
        tab.page.set_icon(icon.as_ref());
        self.refresh_sleep_look(tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_of_addresses() {
        assert_eq!(origin_of("https://News.Example.com/a/b?q=1#x").as_deref(), Some("https://news.example.com"));
        assert_eq!(origin_of("http://localhost:8080/x").as_deref(), Some("http://localhost:8080"));
        assert_eq!(origin_of("https://user:pw@host.example:444/").as_deref(), Some("https://host.example:444"));
        assert_eq!(origin_of("about:blank"), None);
        assert_eq!(origin_of("file:///tmp/x.html"), None);
        assert_eq!(origin_of("https://"), None);
        assert_ne!(origin_of("http://a.example"), origin_of("https://a.example"), "a different scheme is a different site");
    }
}
