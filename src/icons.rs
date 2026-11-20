//! Lucide icons (docs/research/icons-lucide.md): the toolbar and list icons are Lucide's, drawn from a 24 px grid with a 1.5 stroke
//! (the proportions), and shown at the size the interface already uses (GTK's 16 px icon size).
//!
//! GTK repaints a *symbolic* icon as filled shapes, which fills outline icons, so these are installed as ordinary icons, one file per
//! colour (dark and light style) in the cache folder, and every widget bound with `bind` follows the style. The SVGs themselves are
//! in `assets/icons/lucide/`, unchanged; the stroke width and colour are put in when the file is written.

use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

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
icon!(LAYOUT_GRID_CIRCLES, "layout-grid-circles", "view-app-grid-symbolic");
icon!(ROTATE_CCW_CLOCK, "rotate-ccw-clock", "document-open-recent-symbolic");
icon!(TRASH, "trash", "user-trash-symbolic");
icon!(PEN_LINE, "pen-line", "document-edit-symbolic");
icon!(SWEPT_BROOM, "swept-broom", "edit-clear-all-symbolic");
icon!(EXTERNAL_LINK, "external-link", "document-open-symbolic");
icon!(APP_WINDOW, "app-window", "window-new-symbolic");
icon!(USERS, "users", "system-users-symbolic");
icon!(PICTURE_IN_PICTURE_2, "picture-in-picture-2", "window-new-symbolic");
icon!(LINK, "link", "insert-link-symbolic");
icon!(LINK_2, "link-2", "insert-link-symbolic");
icon!(TEXT_CURSOR, "text-cursor", "edit-select-all-symbolic");
icon!(DOWNLOAD, "download", "document-save-symbolic");
icon!(IMAGE, "image", "image-x-generic-symbolic");
icon!(COPY, "copy", "edit-copy-symbolic");
icon!(SEARCH, "search", "edit-find-symbolic");
icon!(TEXT_SELECT, "text-select", "edit-select-all-symbolic");
icon!(BOOK_PLUS, "book-plus", "list-add-symbolic");
icon!(UNDO_2, "undo-2", "edit-undo-symbolic");
icon!(REDO_2, "redo-2", "edit-redo-symbolic");
icon!(SCISSORS, "scissors", "edit-cut-symbolic");
icon!(CLIPBOARD_PASTE, "clipboard-paste", "edit-paste-symbolic");
icon!(TRASH_2, "trash-2", "edit-delete-symbolic");
icon!(X, "x", "process-stop-symbolic");
icon!(REFRESH_CW_OFF, "refresh-cw-off", "view-refresh-symbolic");
icon!(FILE_DOWN, "file-down", "document-save-symbolic");
icon!(PRINTER, "printer", "document-print-symbolic");
icon!(CODE_XML, "code-xml", "text-x-generic-symbolic");
icon!(BUG, "bug", "dialog-information-symbolic");
icon!(PLAY, "play", "media-playback-start-symbolic");
icon!(PAUSE, "pause", "media-playback-pause-symbolic");
icon!(VOLUME_X, "volume-x", "audio-volume-muted-symbolic");
icon!(VOLUME_2, "volume-2", "audio-volume-high-symbolic");
icon!(REPEAT, "repeat", "media-playlist-repeat-symbolic");
icon!(SLIDERS_HORIZONTAL, "sliders-horizontal", "preferences-system-symbolic");
icon!(CHEVRON_RIGHT, "chevron-right", "go-next-symbolic");
icon!(CHEVRON_LEFT, "chevron-left", "go-previous-symbolic");
/// The bookmarked star: Lucide's star painted in.
pub const STAR_FILLED: Icon = Icon { id: "star-filled", filled: true, ..STAR };

pub const ALL: &[Icon] = &[ROTATE_CW, ARROW_LEFT, ARROW_RIGHT, STAR_PLUS, STAR_FILLED, ARROW_DOWN_TO_LINE, MENU, LAYOUT_GRID_CIRCLES, ROTATE_CCW_CLOCK, TRASH, PEN_LINE, SWEPT_BROOM, EXTERNAL_LINK, APP_WINDOW, USERS, PICTURE_IN_PICTURE_2, LINK, LINK_2, TEXT_CURSOR, DOWNLOAD, IMAGE, COPY, SEARCH, TEXT_SELECT, BOOK_PLUS, UNDO_2, REDO_2, SCISSORS, CLIPBOARD_PASTE, TRASH_2, X, REFRESH_CW_OFF, FILE_DOWN, PRINTER, CODE_XML, BUG, PLAY, PAUSE, VOLUME_X, VOLUME_2, REPEAT, SLIDERS_HORIZONTAL, CHEVRON_RIGHT, CHEVRON_LEFT];

/// The icon with this Lucide id.
pub fn by_id(id: &str) -> Option<Icon> {
    ALL.iter().find(|i| i.id == id).copied()
}

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

fn relative_luminance(hex: &str) -> f64 {
    let channel = |i| {
        let srgb = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).expect("normalized colour")) / 255.0;
        if srgb <= 0.04045 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
}

/// True when the light stroke has greater contrast against the supplied tint.
pub fn dark_background(hex: &str) -> bool {
    let crossover = ((relative_luminance(DARK) + 0.05) * (relative_luminance(LIGHT) + 0.05)).sqrt() - 0.05;
    relative_luminance(hex) < crossover
}

pub fn foreground(dark_background: bool) -> &'static str {
    if dark_background { DARK } else { LIGHT }
}

thread_local! {
    static INSTALLED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    /// Widget, icon, and the window tint override when it sits directly on window chrome.
    static BOUND: RefCell<Vec<(glib::WeakRef<glib::Object>, Icon, Option<bool>)>> = const { RefCell::new(Vec::new()) };
    /// Called when the style changes, for icons that are not an `icon-name` of a widget (a tab's status icon).
    static ON_STYLE: RefCell<Vec<Rc<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
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
    name_for_background(icon, adw::StyleManager::default().is_dark())
}

/// The icon variant for a known background, independent of the desktop appearance.
pub fn name_for_background(icon: &Icon, dark_background: bool) -> String {
    if install() {
        format!("browsah-{}-{}", icon.id, if dark_background { "dark" } else { "light" })
    } else {
        icon.fallback.to_owned()
    }
}

/// Run `f` whenever the style (dark or light) changes, to set an icon that `bind` cannot (it only sets a widget's `icon-name`).
pub fn on_style_change(f: impl Fn() + 'static) {
    install();
    ON_STYLE.with(|h| h.borrow_mut().push(Rc::new(f)));
}

fn refresh() {
    let hooks: Vec<Rc<dyn Fn()>> = ON_STYLE.with(|h| h.borrow().clone());
    for h in hooks {
        h();
    }
    BOUND.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(w, _, _)| w.upgrade().is_some());
        for (w, icon, background) in b.iter() {
            if let Some(w) = w.upgrade() {
                w.set_property("icon-name", bound_name(icon, *background));
            }
        }
    });
}

fn bound_name(icon: &Icon, background: Option<bool>) -> String {
    background.map_or_else(|| name(icon), |dark| name_for_background(icon, dark))
}

/// Set a tint override for icons on the window chrome. Popovers use their own surface colours.
pub fn set_chrome_background(widgets: &[&glib::Object], dark: bool) {
    BOUND.with(|bindings| {
        for (weak, icon, background) in bindings.borrow_mut().iter_mut() {
            if let Some(widget) = weak.upgrade().filter(|widget| widgets.contains(&widget)) {
                *background = Some(dark);
                widget.set_property("icon-name", name_for_background(icon, dark));
            }
        }
    });
}

/// Bind an icon to a widget, preserving its background when the icon changes.
pub fn bind(widget: &impl IsA<glib::Object>, icon: Icon) {
    let object: glib::Object = widget.clone().upcast();
    BOUND.with(|bindings| {
        let mut bindings = bindings.borrow_mut();
        bindings.retain(|(widget, _, _)| widget.upgrade().is_some());
        if let Some((_, current, background)) = bindings.iter_mut().find(|(widget, _, _)| widget.upgrade().as_ref() == Some(&object)) {
            *current = icon;
            object.set_property("icon-name", bound_name(&icon, *background));
        } else {
            object.set_property("icon-name", name(&icon));
            bindings.push((object.downgrade(), icon, None));
        }
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
    fn every_icon_has_its_file_and_a_unique_id() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons/lucide");
        let mut seen = std::collections::HashSet::new();
        for i in ALL {
            assert!(seen.insert(i.id), "{} twice", i.id);
            let file = i.id.strip_suffix("-filled").unwrap_or(i.id);
            assert!(dir.join(format!("{file}.svg")).is_file(), "{file}.svg");
            assert_eq!(by_id(i.id).map(|x| x.id), Some(i.id));
        }
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
