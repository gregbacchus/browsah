//! Questions put to the person: permission requests from pages, downloads that ask where to save, and
//! the messages about downloads.

use super::view::{DownloadQuestion, DownloadState, PermissionRequest};
use super::{now_secs, store, Shell};
use crate::permissions::{origin_host, Kind};
use adw::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Self-test: do not show dialogs; hold the question so the test can answer it.
static UI_TEST: AtomicBool = AtomicBool::new(false);

/// Whether dialogs are being held for a test.
pub fn ui_test() -> bool {
    UI_TEST.load(Ordering::Relaxed)
}

pub fn set_ui_test(on: bool) {
    UI_TEST.store(on, Ordering::Relaxed);
}

/// A permission request waiting for the person, with the answers already known for some kinds.
pub struct Queued {
    pub req: Arc<PermissionRequest>,
    /// For each kind in `req.ask`: what was remembered (`Some(allowed)`), or `None` to ask.
    pub known: Vec<Option<bool>>,
}

impl Queued {
    /// The kinds the person still has to decide.
    pub fn unresolved(&self) -> Vec<Kind> {
        self.req.ask.iter().zip(&self.known).filter(|(_, k)| k.is_none()).map(|(k, _)| *k).collect()
    }

    /// Finish: `decision` is what the person said for the unresolved kinds (`remember` it).
    fn finish(&self, decision: bool, remember: bool) {
        let answers: Vec<bool> = self.known.iter().map(|k| k.unwrap_or(decision)).collect();
        if remember {
            if let Some(store) = store() {
                for (kind, known) in self.req.ask.iter().zip(&self.known) {
                    if known.is_none() {
                        let _ = store.remember_permission(&self.req.origin, kind.name(), if decision { "allow" } else { "deny" }, now_secs());
                        crate::permissions::note_remembered(&self.req.origin, kind.name(), decision);
                    }
                }
            }
        }
        self.req.answer(&answers);
    }
}

fn phrase(kinds: &[Kind]) -> String {
    let wants: Vec<&str> = kinds.iter().map(|k| k.wants()).collect();
    match wants.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first @ .., last] => format!("{} and {}", first.join(", "), last),
    }
}

impl Shell {
    /// A page needs a decision the configuration leaves to the person (`ask`): use an answer
    /// remembered from before, else show a question.
    pub fn prompt_permission(self: &Rc<Self>, req: Arc<PermissionRequest>) {
        let store = store();
        let known: Vec<Option<bool>> = req
            .ask
            .iter()
            .map(|k| store.as_ref().and_then(|s| s.remembered_permission(&req.origin, k.name())).map(|c| c == "allow"))
            .collect();
        let queued = Rc::new(Queued { req, known });
        if queued.unresolved().is_empty() {
            queued.finish(false, false); // every kind was answered before: nothing is left to decide
            return;
        }
        self.prompt_queue.borrow_mut().push_back(queued);
        self.next_prompt();
    }

    /// Show the next waiting question if none is showing.
    fn next_prompt(self: &Rc<Self>) {
        if self.prompt_dialog.borrow().is_some() || self.pending_prompt.borrow().is_some() {
            return;
        }
        let Some(q) = self.prompt_queue.borrow_mut().pop_front() else { return };
        if UI_TEST.load(Ordering::Relaxed) {
            *self.pending_prompt.borrow_mut() = Some(q);
            return;
        }
        let host = origin_host(&q.req.origin).to_owned();
        let dialog = adw::AlertDialog::new(Some(&format!("{host} wants to {}", phrase(&q.unresolved()))), Some("Your choice is remembered for this site, so you are not asked again."));
        dialog.add_responses(&[("deny", "Block"), ("allow", "Allow")]);
        dialog.set_response_appearance("allow", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("deny"));
        dialog.set_close_response("close");
        let remember = gtk::CheckButton::builder().label("Remember for this site").active(true).build();
        dialog.set_extra_child(Some(&remember));
        *self.prompt_dialog.borrow_mut() = Some((q.req.id, dialog.clone()));
        let window = self.window.downgrade();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |response| {
            let Some(shell) = window.upgrade().and_then(|w| super::shell_of(&w)) else { return };
            *shell.prompt_dialog.borrow_mut() = None;
            match response.as_str() {
                "allow" => q.finish(true, remember.is_active()),
                "deny" => q.finish(false, remember.is_active()),
                _ => q.finish(false, false), // closed with Esc: blocked this time, not remembered
            }
            shell.next_prompt();
        });
    }

    /// The page moved on: drop its question.
    pub fn dismiss_permission(self: &Rc<Self>, id: u64) {
        self.prompt_queue.borrow_mut().retain(|q| q.req.id != id || id == 0);
        let showing = self.prompt_dialog.borrow().clone();
        if let Some((pid, dialog)) = showing {
            if pid == id && id != 0 {
                *self.prompt_dialog.borrow_mut() = None;
                dialog.force_close();
                self.next_prompt();
            }
        }
        let pending = self.pending_prompt.borrow().as_ref().is_some_and(|q| q.req.id == id && id != 0);
        if pending {
            *self.pending_prompt.borrow_mut() = None;
            self.next_prompt();
        }
    }

    /// The question waiting in a test (see `set_ui_test`).
    pub fn pending_permission(&self) -> Option<Rc<Queued>> {
        self.pending_prompt.borrow().clone()
    }

    /// Answer the question waiting in a test as the person would.
    pub fn answer_pending_permission(self: &Rc<Self>, allow: bool, remember: bool) {
        let q = self.pending_prompt.borrow_mut().take();
        if let Some(q) = q {
            q.finish(allow, remember);
            self.next_prompt();
        }
    }

    // ---- downloads ----------------------------------------------------------------------------------

    /// A download started, progressed, finished or was cancelled.
    pub fn download_event(self: &Rc<Self>, id: u32, name: &str, url: &str, state: DownloadState, cancel: Option<Arc<super::view::DownloadCancel>>) {
        self.downloads_update(id, name, url, &state, cancel);
        match state {
            DownloadState::Progress { .. } => {
                if self.downloads_seen.borrow_mut().insert(id) && !name.is_empty() {
                    self.toast(&format!("Downloading {name}"));
                }
            }
            DownloadState::Done { path, .. } => {
                self.downloads_seen.borrow_mut().remove(&id);
                let toast = adw::Toast::builder().title(format!("Downloaded {name}")).button_label("Show in folder").build();
                let folder = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                toast.connect_button_clicked(move |_| {
                    let uri = gio::File::for_path(&folder).uri();
                    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
                });
                self.toasts.add_toast(toast);
                self.completed_downloads.borrow_mut().push(path);
            }
            DownloadState::Canceled | DownloadState::Failed => {
                self.downloads_seen.borrow_mut().remove(&id);
                self.toast(&format!("Download of {} {}", if name.is_empty() { "a file" } else { name }, if matches!(state, DownloadState::Failed) { "failed" } else { "was cancelled" }));
            }
        }
    }

    /// A download is waiting for the person to choose where it goes (`downloads.ask`).
    pub fn ask_download(self: &Rc<Self>, q: Arc<DownloadQuestion>) {
        if UI_TEST.load(Ordering::Relaxed) {
            *self.pending_download.borrow_mut() = Some(q);
            return;
        }
        let dialog = gtk::FileDialog::builder().title("Save as").initial_name(q.suggested.as_str()).initial_folder(&gio::File::for_path(&q.dir)).build();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |result| {
            q.answer(result.ok().and_then(|f| f.path()));
        });
    }

    /// The download question waiting in a test.
    pub fn pending_download_question(&self) -> Option<Arc<DownloadQuestion>> {
        self.pending_download.borrow().clone()
    }

    pub fn take_pending_download(&self) -> Option<Arc<DownloadQuestion>> {
        self.pending_download.borrow_mut().take()
    }

    /// Files that finished downloading in this window (the self-test).
    pub fn completed_downloads(&self) -> Vec<PathBuf> {
        self.completed_downloads.borrow().clone()
    }

    /// Give a view this window's permission rules and download settings (from its configuration).
    pub fn apply_view_settings(&self, view: &super::view::BrowserView) {
        let s = self.settings.borrow();
        view.set_permissions(s.permissions.clone());
        view.set_cookies(s.cookies.clone(), &self.session_owner);
        view.set_color_scheme(match s.theme {
            crate::config::Theme::Light => false,
            crate::config::Theme::Dark => true,
            crate::config::Theme::System => adw::StyleManager::default().is_dark(),
        });
        view.set_downloads(crate::downloads::resolve_directory(&s.downloads), s.downloads.ask);
    }
}

#[allow(dead_code)]
fn _glib_used(_: &glib::GString) {}
