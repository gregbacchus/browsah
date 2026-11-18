//! The downloads list: a header button with a popover of recent downloads (in progress, finished, stopped),
//! with Open, Show in folder, Cancel and Remove. The list is the profile's: every window shows the same rows.
//! Finished and stopped downloads are remembered in the profile database; the files are never touched by
//! removing a row.

use super::view::{DownloadCancel, DownloadState};
use super::{store, Shell};
use gtk::{gio, prelude::*};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum RowState {
    Active { percent: i32, received: i64 },
    Done,
    Cancelled,
    Failed,
}

#[derive(Clone)]
pub struct Row {
    /// CEF's id while the download runs (0 for one remembered from an earlier run).
    pub key: u32,
    pub name: String,
    pub url: String,
    pub path: Option<PathBuf>,
    pub state: RowState,
    pub bytes: i64,
    pub at: i64,
    pub workspace: Option<String>,
    /// The remembered record, once there is one.
    pub db_id: Option<i64>,
    pub cancel: Option<Arc<DownloadCancel>>,
}

thread_local! {
    static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    static UIS: RefCell<Vec<Weak<Ui>>> = const { RefCell::new(Vec::new()) };
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

/// One window's button and popover.
pub struct Ui {
    pub button: gtk::MenuButton,
    list: gtk::Box,
}

/// Human-readable size: `1.4 MB`.
pub fn size_text(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b < 1024.0 {
        format!("{} B", bytes.max(0))
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    } else {
        format!("{:.1} GB", b / 1024.0 / 1024.0 / 1024.0)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Ui {
    pub fn new() -> Rc<Ui> {
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        let clear = gtk::Button::builder().label("Clear list").has_frame(false).halign(gtk::Align::End).build();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
        let scroller = gtk::ScrolledWindow::builder().child(&list).propagate_natural_height(true).max_content_height(380).min_content_width(360).hscrollbar_policy(gtk::PolicyType::Never).build();
        content.set_size_request(400, -1);
        content.append(&scroller);
        content.append(&clear);
        let popover = gtk::Popover::builder().child(&content).build();
        let button = gtk::MenuButton::builder().tooltip_text("Downloads").popover(&popover).visible(false).build();
        button.add_css_class("flat");
        crate::icons::bind(&button, crate::icons::ARROW_DOWN_TO_LINE);
        let ui = Rc::new(Ui { button, list });
        clear.connect_clicked(|_| clear_all());
        UIS.with(|u| u.borrow_mut().push(Rc::downgrade(&ui)));
        // The remembered downloads of earlier runs, once.
        if !LOADED.replace(true) {
            if let Some(st) = store() {
                let rows: Vec<Row> = st
                    .downloads(50)
                    .into_iter()
                    .map(|d| Row {
                        key: 0,
                        name: d.name,
                        url: d.url,
                        path: Some(PathBuf::from(d.path)),
                        state: match d.state.as_str() {
                            "done" => RowState::Done,
                            "cancelled" => RowState::Cancelled,
                            _ => RowState::Failed,
                        },
                        bytes: d.bytes,
                        at: d.at,
                        workspace: d.workspace,
                        db_id: Some(d.id),
                        cancel: None,
                    })
                    .collect();
                ROWS.with(|r| *r.borrow_mut() = rows);
            }
        }
        refresh();
        ui
    }
}

/// Rebuild every window's list from the rows.
pub fn refresh() {
    let rows: Vec<Row> = ROWS.with(|r| r.borrow().clone());
    UIS.with(|u| u.borrow_mut().retain(|w| w.upgrade().is_some()));
    let uis: Vec<Rc<Ui>> = UIS.with(|u| u.borrow().iter().filter_map(Weak::upgrade).collect());
    let active = rows.iter().any(|r| matches!(r.state, RowState::Active { .. }));
    for ui in uis {
        while let Some(child) = ui.list.first_child() {
            ui.list.remove(&child);
        }
        for row in &rows {
            ui.list.append(&row_widget(row));
        }
        ui.button.set_visible(!rows.is_empty());
        if active {
            ui.button.add_css_class("accent");
        } else {
            ui.button.remove_css_class("accent");
        }
    }
}

fn status_text(row: &Row) -> String {
    match &row.state {
        RowState::Active { percent, received } if *percent >= 0 => format!("{percent}% \u{2014} {}", size_text(*received)),
        RowState::Active { received, .. } => format!("{} so far", size_text(*received)),
        RowState::Done if row.path.as_ref().is_some_and(|p| !p.exists()) => "The file was moved or deleted".to_owned(),
        RowState::Done => size_text(row.bytes),
        RowState::Cancelled => "Cancelled".to_owned(),
        RowState::Failed => "Failed".to_owned(),
    }
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(icon).tooltip_text(tip).has_frame(false).valign(gtk::Align::Center).build();
    b.add_css_class("flat");
    b
}

fn row_widget(row: &Row) -> gtk::Box {
    let outer = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).margin_top(4).margin_bottom(4).build();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
    let name = gtk::Label::builder().label(if row.name.is_empty() { "download" } else { &row.name }).halign(gtk::Align::Start).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).width_chars(24).max_width_chars(40).build();
    let status = gtk::Label::builder().label(status_text(row)).halign(gtk::Align::Start).xalign(0.0).build();
    status.add_css_class("dim-label");
    status.add_css_class("caption");
    text.append(&name);
    if let RowState::Active { percent, .. } = &row.state {
        let bar = gtk::ProgressBar::new();
        if *percent >= 0 {
            bar.set_fraction(*percent as f64 / 100.0);
        } else {
            bar.pulse();
        }
        text.append(&bar);
    }
    text.append(&status);
    outer.append(&text);
    let key = row.key;
    let db_id = row.db_id;
    let path = row.path.clone();
    match &row.state {
        RowState::Active { .. } => {
            let cancel = icon_button("process-stop-symbolic", "Cancel the download");
            let handle = row.cancel.clone();
            cancel.connect_clicked(move |_| {
                if let Some(h) = &handle {
                    h.cancel();
                }
            });
            outer.append(&cancel);
        }
        _ => {
            if row.state == RowState::Done && path.as_ref().is_some_and(|p| p.exists()) {
                let open = icon_button("document-open-symbolic", "Open the file");
                let p = path.clone();
                open.connect_clicked(move |_| {
                    if let Some(p) = &p {
                        open_path(p);
                    }
                });
                outer.append(&open);
                let show = icon_button("folder-symbolic", "Show in folder");
                let p = path.clone();
                show.connect_clicked(move |_| {
                    if let Some(p) = &p {
                        open_path(p.parent().unwrap_or(Path::new("/")));
                    }
                });
                outer.append(&show);
            }
            let remove = icon_button("edit-delete-symbolic", "Remove from the list (the file is kept)");
            remove.connect_clicked(move |_| remove_row(key, db_id));
            outer.append(&remove);
        }
    }
    outer
}

/// Hand a file or folder to the desktop.
fn open_path(p: &Path) {
    if std::env::var_os("BROWSAH_NO_OS_LAUNCH").is_some() {
        return;
    }
    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(p).uri(), gio::AppLaunchContext::NONE);
}

fn remove_row(key: u32, db_id: Option<i64>) {
    if let (Some(id), Some(st)) = (db_id, store()) {
        let _ = st.remove_downloads(Some(id));
    }
    ROWS.with(|r| {
        r.borrow_mut().retain(|row| !(row.key == key && row.db_id == db_id && !matches!(row.state, RowState::Active { .. })));
    });
    refresh();
}

/// Forget every finished download (the ones still running stay).
pub fn clear_all() {
    if let Some(st) = store() {
        let _ = st.remove_downloads(None);
    }
    ROWS.with(|r| r.borrow_mut().retain(|row| matches!(row.state, RowState::Active { .. })));
    refresh();
}

/// The rows, newest first (for the self-test): (name, state).
pub fn rows() -> Vec<(String, RowState)> {
    ROWS.with(|r| r.borrow().iter().map(|r| (r.name.clone(), r.state.clone())).collect())
}

/// Cancel every download that is running (for the self-test; the buttons do the same one at a time).
pub fn cancel_active() {
    let handles: Vec<Arc<DownloadCancel>> = ROWS.with(|r| r.borrow().iter().filter(|r| matches!(r.state, RowState::Active { .. })).filter_map(|r| r.cancel.clone()).collect());
    for h in handles {
        h.cancel();
    }
}

/// The number of row widgets this window's list shows (for the self-test).
impl Ui {
    pub fn shown_rows(&self) -> usize {
        let mut n = 0;
        let mut child = self.list.first_child();
        while let Some(c) = child {
            n += 1;
            child = c.next_sibling();
        }
        n
    }
}

impl Shell {
    /// A download began, progressed or ended: update the list (and remember it when it ends).
    pub(super) fn downloads_update(&self, id: u32, name: &str, url: &str, state: &DownloadState, cancel: Option<Arc<DownloadCancel>>) {
        let workspace = self.workspace.as_ref().map(|w| w.id.clone());
        ROWS.with(|rows| {
            let mut rows = rows.borrow_mut();
            let pos = rows.iter().position(|r| r.key == id && id != 0 && matches!(r.state, RowState::Active { .. }));
            let idx = match pos {
                Some(i) => i,
                None => {
                    rows.insert(0, Row { key: id, name: name.to_owned(), url: url.to_owned(), path: None, state: RowState::Active { percent: -1, received: 0 }, bytes: 0, at: now(), workspace, db_id: None, cancel: None });
                    0
                }
            };
            let row = &mut rows[idx];
            if !name.is_empty() {
                row.name = name.to_owned();
            }
            if cancel.is_some() {
                row.cancel = cancel;
            }
            match state {
                DownloadState::Progress { percent, received } => row.state = RowState::Active { percent: *percent, received: *received },
                DownloadState::Done { path, bytes } => {
                    row.state = RowState::Done;
                    row.path = Some(path.clone());
                    row.bytes = *bytes;
                }
                DownloadState::Canceled => row.state = RowState::Cancelled,
                DownloadState::Failed => row.state = RowState::Failed,
            }
            if !matches!(row.state, RowState::Active { .. }) {
                row.cancel = None;
                row.at = now();
                if row.db_id.is_none() && !row.name.is_empty() {
                    let word = match row.state {
                        RowState::Done => "done",
                        RowState::Cancelled => "cancelled",
                        _ => "failed",
                    };
                    row.db_id = store().and_then(|st| st.record_download(row.workspace.as_deref(), &row.name, &row.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), &row.url, row.at, word, row.bytes).ok());
                }
            }
        });
        refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_readable() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(1023), "1023 B");
        assert_eq!(size_text(2048), "2 KB");
        assert_eq!(size_text(1_500_000), "1.4 MB");
        assert_eq!(size_text(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
