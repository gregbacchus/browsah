//! Lucide icons (docs/research/icons-lucide.md): the toolbar and list icons are Lucide's, drawn from a 24 px grid with a 1.5 stroke
//! (the proportions), and shown at the size the interface already uses (GTK's 16 px icon size).
//!
//! GTK repaints a *symbolic* icon as filled shapes, which fills outline icons, so these are installed as ordinary icons, one file per
//! colour (dark and light style) in the cache folder, and every widget bound with `bind` follows the style. The SVGs themselves are
//! in `assets/icons/lucide/`, unchanged; the stroke width and colour are put in when the file is written.

use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;

/// The grid every icon is drawn on (its `viewBox`). It is shown at whatever size the widget asks for, 16 px for buttons.
pub const SIZE: i32 = 24;
/// The stroke width on that grid.
pub const STROKE: f32 = 1.5;

/// One icon: its file, what to draw as a fallback if the files cannot be written, and whether the shape is filled.
#[derive(Clone, Copy, Debug)]
pub struct Icon {
    pub id: &'static str,
    svg: &'static str,
    fallback: &'static str,
    filled: bool,
}

macro_rules! icon {
    ($name:ident, $id:literal, $fallback:literal) => {
        pub const $name: Icon = Icon { id: $id, svg: include_str!(concat!("../assets/icons/lucide/", $id, ".svg")), fallback: $fallback, filled: false };
    };
}

icon!(ROTATE_CW, "rotate-cw", "view-refresh-symbolic");
icon!(ARROW_LEFT, "arrow-left", "go-previous-symbolic");
icon!(ARROW_RIGHT, "arrow-right", "go-next-symbolic");
icon!(STAR_PLUS, "star-plus", "non-starred-symbolic");
icon!(STAR, "star", "starred-symbolic");
icon!(ARROW_DOWN_TO_LINE, "arrow-down-to-line", "folder-download-symbolic");
icon!(MENU, "menu", "open-menu-symbolic");
icon!(SWATCH_BOOK, "swatch-book", "view-app-grid-symbolic");
icon!(ROTATE_CCW_CLOCK, "rotate-ccw-clock", "document-open-recent-symbolic");
icon!(TRASH, "trash", "user-trash-symbolic");
icon!(SWEPT_BROOM, "swept-broom", "edit-clear-all-symbolic");
/// The bookmarked star: Lucide's star painted in.
pub const STAR_FILLED: Icon = Icon { id: "star-filled", filled: true, ..STAR };

pub const ALL: &[Icon] = &[ROTATE_CW, ARROW_LEFT, ARROW_RIGHT, STAR_PLUS, STAR_FILLED, ARROW_DOWN_TO_LINE, MENU, SWATCH_BOOK, ROTATE_CCW_CLOCK, TRASH, SWEPT_BROOM];

/// The SVG for `icon` in `color`: Lucide's file with its stroke width and colour set (and the first shape painted for a filled icon).
pub fn render(icon: &Icon, color: &str) -> String {
    let mut svg = icon
        .svg
        .lines()
        .filter(|l| !l.trim_start().starts_with("<!--") && !l.trim_start().starts_with("class=") && !l.trim_start().starts_with("width=") && !l.trim_start().starts_with("height="))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("stroke=\"currentColor\"", &format!("stroke=\"{color}\""))
        .replace("stroke-width=\"2\"", &format!("stroke-width=\"{STROKE}\""))
        .replacen("<svg", &format!("<svg width=\"{SIZE}\" height=\"{SIZE}\""), 1);
    if icon.filled {
        svg = svg.replacen("<path ", &format!("<path fill=\"{color}\" "), 1);
    }
    svg
}

const DARK: &str = "#ebebeb";
const LIGHT: &str = "#363636";

thread_local! {
    static INSTALLED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    /// Widgets (or tab pages) with an `icon-name` that follows the style.
    static BOUND: RefCell<Vec<(glib::WeakRef<glib::Object>, Icon)>> = const { RefCell::new(Vec::new()) };
}

/// Write the icon files (once) and let the icon theme find them. False if the files cannot be written.
fn install() -> bool {
    INSTALLED.with(|i| {
        *i.get_or_init(|| {
            let base = glib::user_cache_dir().join("browsah").join("icons");
            let dir = base.join("hicolor").join("scalable").join("actions");
            let wrote = std::fs::create_dir_all(&dir).is_ok() && ALL.iter().all(|i| [("dark", DARK), ("light", LIGHT)].iter().all(|(style, color)| std::fs::write(dir.join(format!("browsah-{}-{style}.svg", i.id)), render(i, color)).is_ok()));
            let Some(display) = gtk::gdk::Display::default() else { return false };
            if wrote {
                gtk::IconTheme::for_display(&display).add_search_path(&base);
                adw::StyleManager::default().connect_dark_notify(|_| refresh());
            }
            wrote
        })
    })
}

/// The icon name for the current style (the stock fallback if the files could not be written).
pub fn name(icon: &Icon) -> String {
    if install() {
        format!("browsah-{}-{}", icon.id, if adw::StyleManager::default().is_dark() { "dark" } else { "light" })
    } else {
        icon.fallback.to_owned()
    }
}

fn refresh() {
    BOUND.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(w, _)| w.upgrade().is_some());
        for (w, icon) in b.iter() {
            if let Some(w) = w.upgrade() {
                w.set_property("icon-name", name(icon));
            }
        }
    });
}

/// Give `widget` (a button, menu button, image or view-stack page) the icon, and keep it right when the style changes. Calling it
/// again for the same widget replaces the icon (the bookmark star).
pub fn bind(widget: &impl IsA<glib::Object>, icon: Icon) {
    let object: glib::Object = widget.clone().upcast();
    object.set_property("icon-name", name(&icon));
    BOUND.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(w, _)| w.upgrade().is_some_and(|o| o != object));
        b.push((object.downgrade(), icon));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_is_drawn_on_the_24px_grid_with_the_1_5_stroke_in_the_given_colour() {
        let svg = render(&ROTATE_CW, "#abcdef");
        assert!(svg.contains("stroke=\"#abcdef\"") && svg.contains("stroke-width=\"1.5\"") && svg.contains("width=\"24\" height=\"24\""), "{svg}");
        assert!(!svg.contains("currentColor") && !svg.contains("<!--") && !svg.contains("stroke-width=\"2\""), "{svg}");
        assert_eq!(svg.matches("<svg").count(), 1);
    }

    #[test]
    fn the_filled_star_is_the_star_painted_in() {
        let plain = render(&STAR, "#fff");
        let filled = render(&STAR_FILLED, "#fff");
        assert!(!plain.contains("fill=\"#fff\""));
        assert_eq!(filled.matches("fill=\"#fff\"").count(), 1, "{filled}");
        assert!(filled.contains("stroke=\"#fff\"") && filled.contains("<path fill=\"#fff\""), "{filled}");
    }

    #[test]
    fn every_icon_is_valid_svg_text_with_paths() {
        for i in ALL {
            let svg = render(i, "#000");
            assert!(svg.trim_start().starts_with("<svg") && svg.trim_end().ends_with("</svg>") && svg.contains("<path") || svg.contains("<circle") || svg.contains("<rect"), "{}", i.id);
        }
    }
}
