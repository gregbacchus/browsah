//! Dialogs a page opens with script (`alert`, `confirm`, `prompt`) and the "Leave site?" question when a
//! page with unsaved changes is left. They are shown over the window; the page waits for the answer.

use super::view::{JsDialogKind, JsDialogQuestion};
use super::Shell;
use crate::permissions::origin_host;
use adw::prelude::*;
use gtk::gio;
use std::rc::Rc;
use std::sync::Arc;

/// Would this page ask "Leave site?": runs its `beforeunload` handlers on a synthetic event and sees whether they object.
const UNLOAD_PROBE: &str = r#"(() => {
  try {
    const e = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(e);
    return JSON.stringify({ unsaved: e.defaultPrevented || typeof window.onbeforeunload === 'function' });
  } catch (_) { return JSON.stringify({ unsaved: false }); }
})()"#;

fn probe_says_unsaved(json: &str) -> bool {
    serde_yaml_ng::from_str::<serde_yaml_ng::Value>(json)
        .ok()
        .and_then(|v| v.get("result")?.get("value")?.as_str().map(str::to_owned))
        .is_some_and(|inner| inner.contains("\"unsaved\":true"))
}

impl Shell {
    /// The window is being closed: ask the pages whether they have unsaved changes (their `beforeunload` handlers)
    /// and, if any do, ask the person. Returns true when the close is on hold (the window closes again on agreement).
    pub(super) fn confirm_window_close(self: &Rc<Self>) -> bool {
        if super::prompts::ui_test() {
            return false;
        }
        let probes: Vec<(Rc<super::Tab>, i32)> = self.tabs.borrow().iter().filter(|t| t.view().is_live()).map(|t| (t.clone(), t.view().cdp_eval(UNLOAD_PROBE))).collect();
        if probes.is_empty() {
            return false;
        }
        let window = self.window.downgrade();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
            let Some(s) = window.upgrade().and_then(|w| super::shell_of(&w)) else { return };
            let unsaved: Vec<String> = probes
                .iter()
                .filter(|(t, id)| t.view().cdp_result(*id).is_some_and(|(ok, json)| ok && probe_says_unsaved(&json)))
                .map(|(t, _)| t.state.borrow().title.clone())
                .collect();
            if unsaved.is_empty() {
                s.close_confirmed.set(true);
                s.window.close();
                return;
            }
            let body = if unsaved.len() == 1 {
                format!("\u{201c}{}\u{201d} may have unsaved changes.", unsaved[0])
            } else {
                format!("{} tabs may have unsaved changes:\n{}", unsaved.len(), unsaved.iter().map(|t| format!("\u{2022} {t}")).collect::<Vec<_>>().join("\n"))
            };
            let dialog = adw::AlertDialog::new(Some("Close this window?"), Some(&body));
            dialog.add_response("cancel", "Stay");
            dialog.add_response("close", "Close window");
            dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let weak = Rc::downgrade(&s);
            dialog.choose(Some(&s.window), gio::Cancellable::NONE, move |response| {
                if response.as_str() == "close" {
                    if let Some(s) = weak.upgrade() {
                        s.close_confirmed.set(true);
                        s.window.close();
                    }
                }
            });
        });
        true
    }

    /// A page opened a dialog: show it now, or after the ones before it.
    pub(super) fn show_js_dialog(self: &Rc<Self>, q: Arc<JsDialogQuestion>) {
        self.js_queue.borrow_mut().push_back(q);
        self.next_js_dialog();
    }

    fn next_js_dialog(self: &Rc<Self>) {
        if self.js_showing.borrow().is_some() || self.js_pending.borrow().is_some() {
            return;
        }
        let Some(q) = self.js_queue.borrow_mut().pop_front() else { return };
        if super::prompts::ui_test() {
            *self.js_pending.borrow_mut() = Some(q);
            return;
        }
        let host = origin_host(&q.origin).to_owned();
        let (heading, ok_label, cancel_label) = match q.kind {
            JsDialogKind::Alert => (format!("{host} says").trim().to_owned(), "OK", None),
            JsDialogKind::Confirm | JsDialogKind::Prompt => (format!("{host} says").trim().to_owned(), "OK", Some("Cancel")),
            JsDialogKind::BeforeUnload { reload } => ((if reload { "Reload this page?" } else { "Leave this page?" }).to_owned(), if reload { "Reload" } else { "Leave" }, Some("Stay")),
        };
        let heading = if heading.is_empty() || heading == "says" { "This page says".to_owned() } else { heading };
        let body = if matches!(q.kind, JsDialogKind::BeforeUnload { .. }) && q.message.is_empty() { "Changes you made may not be saved.".to_owned() } else { q.message.clone() };
        let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
        if let Some(c) = cancel_label {
            dialog.add_response("cancel", c);
        }
        dialog.add_response("ok", ok_label);
        dialog.set_response_appearance("ok", if matches!(q.kind, JsDialogKind::BeforeUnload { .. }) { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested });
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response(if cancel_label.is_some() { "cancel" } else { "ok" });
        let entry = gtk::Entry::builder().text(&q.default_text).activates_default(true).build();
        if q.kind == JsDialogKind::Prompt {
            dialog.set_extra_child(Some(&entry));
            // Type straight away: the entry, not the default button, starts with the focus (its text selected).
            dialog.set_focus(Some(&entry));
        }
        *self.js_showing.borrow_mut() = Some((q.view, dialog.clone()));
        let window = self.window.downgrade();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |response| {
            let Some(shell) = window.upgrade().and_then(|w| super::shell_of(&w)) else {
                q.answer(false, "");
                return;
            };
            *shell.js_showing.borrow_mut() = None;
            let ok = response.as_str() == "ok";
            q.answer(ok, &if ok && q.kind == JsDialogKind::Prompt { entry.text().to_string() } else { String::new() });
            shell.next_js_dialog();
        });
    }

    /// The page's dialogs are over (it navigated or closed): drop what is still waiting from it.
    pub(super) fn dismiss_js_dialog(self: &Rc<Self>, view: u64) {
        self.js_queue.borrow_mut().retain(|q| q.view != view);
        let showing = self.js_showing.borrow().clone();
        if let Some((v, dialog)) = showing {
            if v == view {
                *self.js_showing.borrow_mut() = None;
                dialog.force_close();
                self.next_js_dialog();
            }
        }
        if self.js_pending.borrow().as_ref().is_some_and(|q| q.view == view) {
            *self.js_pending.borrow_mut() = None;
            self.next_js_dialog();
        }
    }

    /// The dialog being held in a test (see `prompts::set_ui_test`).
    pub fn pending_js_dialog(&self) -> Option<Arc<JsDialogQuestion>> {
        self.js_pending.borrow().clone()
    }

    /// Answer the held dialog as the person would.
    pub fn answer_pending_js_dialog(self: &Rc<Self>, ok: bool, text: &str) {
        let q = self.js_pending.borrow_mut().take();
        if let Some(q) = q {
            q.answer(ok, text);
            self.next_js_dialog();
        }
    }
}
