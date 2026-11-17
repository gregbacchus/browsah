//! File pickers a page asks for (`<input type=file>`, `showOpenFilePicker`, directories, save targets), shown
//! as GTK's file dialog over the window. Windowless CEF has no dialog of its own, so without this they do nothing.

use super::view::{FileDialogKind, FileDialogQuestion};
use super::Shell;
use gtk::gio;
use gtk::prelude::*;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

fn paths(files: impl IntoIterator<Item = gio::File>) -> Vec<String> {
    files.into_iter().filter_map(|f| f.path()).map(|p| p.to_string_lossy().into_owned()).collect()
}

/// A filter for what the page accepts, or `None` when it accepts anything.
fn filter_for(accept: &[String]) -> Option<gtk::FileFilter> {
    if accept.is_empty() {
        return None;
    }
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Accepted files"));
    for a in accept {
        let a = a.trim();
        if let Some(ext) = a.strip_prefix('.') {
            filter.add_suffix(ext);
        } else if a.contains('/') {
            filter.add_mime_type(a);
        }
    }
    Some(filter)
}

impl Shell {
    pub(super) fn show_file_dialog(self: &Rc<Self>, q: Arc<FileDialogQuestion>) {
        if super::prompts::ui_test() {
            q.cancel();
            return;
        }
        let default_title = match q.kind {
            FileDialogKind::Open => "Open file",
            FileDialogKind::OpenMultiple => "Open files",
            FileDialogKind::Folder => "Choose folder",
            FileDialogKind::Save => "Save as",
        };
        let mut builder = gtk::FileDialog::builder().title(if q.title.is_empty() { default_title } else { q.title.as_str() });
        if let Some(filter) = filter_for(&q.accept) {
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let all = gtk::FileFilter::new();
            all.set_name(Some("All files"));
            all.add_pattern("*");
            filters.append(&all);
            builder = builder.filters(&filters).default_filter(&filter);
        }
        let dialog = builder.build();
        if !q.default_path.is_empty() {
            let path = Path::new(&q.default_path);
            let file = gio::File::for_path(path);
            if q.kind == FileDialogKind::Save {
                dialog.set_initial_file(Some(&file));
            } else if path.is_dir() {
                dialog.set_initial_folder(Some(&file));
            } else if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
                dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
            }
        }
        let parent = Some(&self.window);
        let cancellable = gio::Cancellable::NONE;
        match q.kind {
            FileDialogKind::Open => dialog.open(parent, cancellable, move |r| match r {
                Ok(f) => q.choose(paths([f])),
                Err(_) => q.cancel(),
            }),
            FileDialogKind::OpenMultiple => dialog.open_multiple(parent, cancellable, move |r| match r {
                Ok(list) => q.choose(paths(list.iter::<gio::File>().flatten())),
                Err(_) => q.cancel(),
            }),
            FileDialogKind::Folder => dialog.select_folder(parent, cancellable, move |r| match r {
                Ok(f) => q.choose(paths([f])),
                Err(_) => q.cancel(),
            }),
            FileDialogKind::Save => dialog.save(parent, cancellable, move |r| match r {
                Ok(f) => q.choose(paths([f])),
                Err(_) => q.cancel(),
            }),
        }
    }
}
