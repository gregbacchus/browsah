//! The "all options" window: a generated copy of a configuration file with every option explained (see
//! `crate::fullconfig`), editable, that can be saved over the real file after a confirmation.

use super::Shell;
use crate::fullconfig::{generate, Level};
use adw::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

thread_local! {
    /// The application file in use (`--config`), set by `run`.
    static APP_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub fn set_app_file(path: PathBuf) {
    APP_FILE.with(|f| *f.borrow_mut() = Some(path));
}

/// The window, for the self-test.
pub struct FullConfigView {
    pub window: adw::Window,
    pub buffer: gtk::TextBuffer,
    /// The real file this is a copy of.
    pub file: PathBuf,
    level: Level,
}

/// Does `text` load as a file of this level? The error says why not.
pub fn check(level: Level, text: &str) -> Result<(), String> {
    match level {
        Level::Application => crate::config::Config::parse(text).map(|_| ()).map_err(|e| e.to_string()),
        Level::Profile => crate::profile::parse_config(text).map(|_| ()),
        Level::Workspace => crate::workspace::parse_config(text).map(|_| ()),
        Level::App => crate::webapp::parse_app(text).map(|_| ()),
    }
}

/// Write `text` as `file`. The file as it was is kept next to it as `<name>.bak` (replacing an earlier backup).
/// Returns the backup's path when one was made.
pub fn save_over(file: &Path, text: &str) -> std::io::Result<Option<PathBuf>> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut backup = None;
    if file.exists() {
        let bak = PathBuf::from(format!("{}.bak", file.display()));
        std::fs::copy(file, &bak)?;
        backup = Some(bak);
    }
    let tmp = PathBuf::from(format!("{}.tmp", file.display()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, file)?;
    Ok(backup)
}

impl Shell {
    /// Show `level`'s file padded out with every option. `None` (with a message) when there is nothing to show.
    pub fn open_full_config(self: &Rc<Self>, mut level: Level) -> Option<Rc<FullConfigView>> {
        let (file, title) = match level {
            Level::Application => (APP_FILE.with(|f| f.borrow().clone()).unwrap_or_else(crate::config::default_path), "browsah".to_owned()),
            Level::Profile => {
                let Some(p) = super::PROFILE.with(|p| p.borrow().clone()) else {
                    self.toast("No profile");
                    return None;
                };
                (p.dir.join("profile.yaml"), p.config.name.clone().unwrap_or_else(|| p.id.clone()))
            }
            Level::Workspace | Level::App => {
                let Some(w) = &self.workspace else {
                    self.toast("This window has no workspace");
                    return None;
                };
                level = if w.is_app() { Level::App } else { Level::Workspace };
                (w.config_file(), w.display_name().to_owned())
            }
        };
        let existing = std::fs::read_to_string(&file).ok();
        let text = match generate(level, &title, existing.as_deref()) {
            Ok(t) => t,
            Err(e) => {
                self.toast(&format!("{}: {e}", file.display()));
                return None;
            }
        };

        let buffer = gtk::TextBuffer::new(None);
        buffer.set_text(&text);
        let view = gtk::TextView::builder().buffer(&buffer).monospace(true).wrap_mode(gtk::WrapMode::WordChar).top_margin(8).bottom_margin(8).left_margin(12).right_margin(12).build();
        let scroll = gtk::ScrolledWindow::builder().child(&view).vexpand(true).hexpand(true).build();
        let note = gtk::Label::builder()
            .label(format!(
                "A generated copy of {}. Edit it, then save it over the real file, or copy what you need.",
                file.display()
            ))
            .xalign(0.0)
            .wrap(true)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .css_classes(["dim-label"])
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&note);
        body.append(&scroll);
        let header = adw::HeaderBar::new();
        let save = gtk::Button::builder().label("Save over the file\u{2026}").css_classes(["suggested-action"]).tooltip_text("Replace the real file with this text").build();
        let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy all").build();
        header.pack_start(&save);
        header.pack_end(&copy);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&body));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));
        let window = adw::Window::builder()
            .title(format!("{} configuration: all options ({title})", level.label()))
            .content(&toolbar)
            .default_width(940)
            .default_height(740)
            .transient_for(&self.window)
            .build();
        window.add_css_class("browsah-fullconfig");
        let fc = Rc::new(FullConfigView { window: window.clone(), buffer: buffer.clone(), file: file.clone(), level });

        copy.connect_clicked({
            let buffer = buffer.clone();
            let toasts = toasts.clone();
            move |b| {
                let (s, e) = buffer.bounds();
                b.clipboard().set_text(buffer.text(&s, &e, false).as_str());
                toasts.add_toast(adw::Toast::new("Copied"));
            }
        });
        save.connect_clicked({
            let fc = fc.clone();
            let toasts = toasts.clone();
            let main = self.window.downgrade();
            move |_| {
                let (s, e) = fc.buffer.bounds();
                let text = fc.buffer.text(&s, &e, false).to_string();
                if let Err(e) = check(fc.level, &text) {
                    let d = adw::AlertDialog::new(Some("Not saved: this text is not a valid configuration"), Some(&e));
                    d.add_response("ok", "OK");
                    d.present(Some(&fc.window));
                    return;
                }
                let d = adw::AlertDialog::new(
                    Some(&format!("Replace {}?", fc.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                    Some(&format!("{}\n\nThe file as it is now is kept as {}.bak. Comments you wrote in it are not in this copy.", fc.file.display(), fc.file.display())),
                );
                d.add_responses(&[("cancel", "Cancel"), ("replace", "Replace")]);
                d.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
                d.set_default_response(Some("cancel"));
                d.set_close_response("cancel");
                let (fc, toasts, main) = (fc.clone(), toasts.clone(), main.clone());
                d.choose(Some(&fc.window.clone()), gtk::gio::Cancellable::NONE, move |r| {
                    if r != "replace" {
                        return;
                    }
                    match save_over(&fc.file, &text) {
                        Ok(_) => {
                            toasts.add_toast(adw::Toast::new("Saved"));
                            if let Some(w) = main.upgrade().and_then(|w| super::shell_of(&w)) {
                                w.toast(&format!("Saved {}", fc.file.display()));
                            }
                        }
                        Err(e) => toasts.add_toast(adw::Toast::new(&format!("Could not save: {e}"))),
                    }
                });
            }
        });
        window.present();
        Some(fc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_keeps_the_old_file_as_a_backup() {
        let dir = std::env::temp_dir().join(format!("browsah-fullconfig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("sub/config.yaml");
        assert_eq!(save_over(&file, "a: 1\n").unwrap(), None, "nothing to back up the first time, and the folder is made");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a: 1\n");
        let bak = save_over(&file, "a: 2\n").unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "a: 1\n");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a: 2\n");
        assert!(!dir.join("sub/config.yaml.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checks_use_the_real_types() {
        assert!(check(Level::Application, "rendering: { frame_rate: 30 }").is_ok());
        assert!(check(Level::Application, "rendering: [").is_err());
        assert!(check(Level::Workspace, "nav: { address_bar: sideways }").is_err());
        assert!(check(Level::Profile, "color: nope").is_err());
        // What is generated for an empty file is acceptable to all three.
        for level in [Level::Application, Level::Profile, Level::Workspace] {
            assert!(check(level, &generate(level, "x", None).unwrap()).is_ok(), "{level:?}");
        }
    }
}
