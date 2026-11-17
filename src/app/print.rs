//! Printing and saving as PDF. Chromium writes the page to a PDF (`print_to_pdf`); *Save as PDF* asks where, and
//! *Print* writes a temporary PDF and opens it in the desktop's PDF viewer, which has the printer dialog.

use super::Shell;
use adw::prelude::*;
use gtk::{gio, glib};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// What to do when a PDF has been written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfJob {
    /// Tell the person where it is.
    Save,
    /// Open it in the PDF viewer (to print).
    Open,
}

/// A file name for a page's PDF: its title (or address), made safe.
pub fn pdf_name(title: &str, address: &str) -> String {
    let base = if title.trim().is_empty() { crate::cookies::host_of(address) } else { title.trim().to_owned() };
    let name = crate::downloads::sanitise_name(&base);
    let name: String = name.chars().take(80).collect();
    if name.to_lowercase().ends_with(".pdf") { name } else { format!("{name}.pdf") }
}

fn launch(path: &Path) {
    if std::env::var_os("BROWSAH_NO_OS_LAUNCH").is_some() {
        return;
    }
    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), gio::AppLaunchContext::NONE);
}

impl Shell {
    /// Write the selected page to `path` and do `job` when it is done.
    pub fn write_pdf(self: &Rc<Self>, path: &Path, job: PdfJob) {
        let Some(tab) = self.current() else { return };
        let print = self.settings.borrow().print.clone();
        self.pdf_jobs.borrow_mut().insert(path.to_string_lossy().into_owned(), job);
        tab.view().print_to_pdf(path, print.paper.inches(), print.backgrounds);
    }

    /// How many PDFs are still being written (for the self-test).
    pub fn pdf_jobs_pending(&self) -> usize {
        self.pdf_jobs.borrow().len()
    }

    /// *Save as PDF*: ask where, then write it.
    pub(super) fn save_pdf(self: &Rc<Self>) {
        let Some(tab) = self.current() else { return };
        let (title, address) = {
            let s = tab.state.borrow();
            (s.title.clone(), s.address.clone())
        };
        let dir = crate::downloads::resolve_directory(&self.settings.borrow().downloads);
        let dialog = gtk::FileDialog::builder().title("Save as PDF").initial_name(pdf_name(&title, &address)).initial_folder(&gio::File::for_path(&dir)).build();
        let weak = self.window.downgrade();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let Ok(file) = res else { return };
            let Some(path) = file.path() else { return };
            if let Some(shell) = weak.upgrade().and_then(|w| super::shell_of(&w)) {
                shell.write_pdf(&path, PdfJob::Save);
            }
        });
    }

    /// *Print*: write a temporary PDF and open it in the viewer.
    pub(super) fn print_page(self: &Rc<Self>) {
        let Some(tab) = self.current() else { return };
        let (title, address) = {
            let s = tab.state.borrow();
            (s.title.clone(), s.address.clone())
        };
        let dir: PathBuf = glib::user_cache_dir().join("browsah").join("print");
        let _ = std::fs::create_dir_all(&dir);
        // Old print files are not needed once the viewer has them.
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 24 * 3600);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let path = dir.join(format!("{}-{}", std::process::id(), pdf_name(&title, &address)));
        self.write_pdf(&path, PdfJob::Open);
    }

    /// A PDF was written (or not).
    pub(super) fn pdf_done(self: &Rc<Self>, path: &str, ok: bool) {
        let job = self.pdf_jobs.borrow_mut().remove(path);
        let p = PathBuf::from(path);
        if !ok {
            self.toast("Could not write the PDF");
            return;
        }
        match job {
            Some(PdfJob::Open) => launch(&p),
            _ => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let toast = adw::Toast::builder().title(format!("Saved {name}")).button_label("Open").build();
                toast.connect_button_clicked(move |_| launch(&p));
                self.toasts.add_toast(toast);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_title_or_the_host() {
        assert_eq!(pdf_name("Quarterly report", "https://x.example/r"), "Quarterly report.pdf");
        assert_eq!(pdf_name("", "https://docs.example.com/a/b"), "docs.example.com.pdf");
        assert_eq!(pdf_name("a/b:c", "https://x/"), "b_c.pdf");
        assert_eq!(pdf_name("Already.PDF", "https://x/"), "Already.PDF");
        assert!(pdf_name(&"long ".repeat(100), "https://x/").chars().count() <= 84);
    }
}
