//! Applying tab sweeping rules to a workspace's tabs: measuring use, checking for unsaved form input and
//! playing media, then closing or discarding the tabs the rules fire for. The model is in
//! `crate::sweep`.

use super::{gio, glib, now_secs, shell_of, tab_title, Shell, Tab};
use crate::sweep::Action;
use adw::prelude::*;
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    /// When each closed workspace was last swept by this process (unix seconds).
    static LAST_BACKGROUND: std::cell::RefCell<std::collections::HashMap<String, i64>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Sweep the remembered tabs of every workspace that has sweeping rules and no open window, if its
/// interval has passed (`force`: regardless). Returns what was swept. The same rules as for an open
/// window; there is nothing to protect because the pages are not running.
pub(super) fn background_sweep(profile: &crate::profile::Profile, app_file: &std::path::Path, force: bool) -> Vec<String> {
    let Some(store) = super::store() else { return Vec::new() };
    let now = now_secs();
    let mut all = Vec::new();
    for ws in crate::sweep::workspaces_with_sweeping(profile, app_file) {
        if super::windows().iter().any(|s| s.workspace.as_ref().is_some_and(|w| w.id == ws.id)) {
            continue; // an open window sweeps its own tabs
        }
        let interval = crate::sweep::Sweeper::from_config(&crate::sweep::effective_sweep(profile, &ws, app_file), None).map(|s| s.interval_secs).unwrap_or(300);
        let due = LAST_BACKGROUND.with(|l| l.borrow().get(&ws.id).is_none_or(|last| now - last >= interval));
        if !(force || due) {
            continue;
        }
        LAST_BACKGROUND.with(|l| l.borrow_mut().insert(ws.id.clone(), now));
        let notes = crate::sweep::sweep_stored(profile, &ws, &store, app_file, now, true);
        if !notes.is_empty() {
            if let Some(s) = super::windows().first() {
                s.toast(&format!("Swept {} unused tab{} in workspace {}", notes.len(), if notes.len() == 1 { "" } else { "s" }, ws.display_name()));
            }
            all.extend(notes.into_iter().map(|n| format!("{}: {n}", ws.id)));
        }
    }
    if !all.is_empty() {
        crate::log_info!("background sweep of closed workspaces: {all:?}");
    }
    all
}

/// Sweep closed workspaces soon after start-up (tabs that went idle while the browser was closed) and
/// then every minute (each workspace on its own interval).
pub(super) fn start_background_sweeping(profile: crate::profile::Profile, app_file: std::path::PathBuf) {
    let (p1, f1) = (profile.clone(), app_file.clone());
    glib::timeout_add_seconds_local_once(8, move || {
        background_sweep(&p1, &f1, false);
    });
    glib::timeout_add_seconds_local(60, move || {
        background_sweep(&profile, &app_file, false);
        glib::ControlFlow::Continue
    });
}

/// What the probe asks a page: is anything typed in a form or editor that has not been saved, and is
/// media playing. Conservative on purpose: a box a script filled in looks unsaved.
const PROBE: &str = r#"(() => {
  const dirtyField = (e) => {
    if (e.tagName === 'TEXTAREA') return e.value !== e.defaultValue;
    if (e.tagName === 'SELECT') return [...e.options].some(o => o.selected !== o.defaultSelected);
    if (e.tagName === 'INPUT') {
      const t = (e.type || 'text').toLowerCase();
      if (t === 'checkbox' || t === 'radio') return e.checked !== e.defaultChecked;
      if (['hidden', 'submit', 'button', 'reset', 'image', 'file', 'range', 'color'].includes(t)) return false;
      return e.value !== e.defaultValue;
    }
    return e.isContentEditable && (e.textContent || '').trim().length > 0;
  };
  const fields = document.querySelectorAll('input, textarea, select, [contenteditable=""], [contenteditable="true"]');
  const dirty = [...fields].some(dirtyField);
  const playing = [...document.querySelectorAll('video, audio')].some(m => !m.paused && !m.ended && !m.muted && m.readyState > 2);
  return JSON.stringify({ dirty, playing });
})()"#;

/// How long a probe has to answer before the tab is treated as unknown (and so left alone).
const PROBE_WAIT: Duration = Duration::from_millis(1500);

/// Read a probe's answer out of the DevTools result.
fn parse_probe(json: &str) -> Option<(bool, bool)> {
    let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(json).ok()?;
    let inner = v.get("result")?.get("value")?.as_str()?;
    let p: serde_yaml_ng::Value = serde_yaml_ng::from_str(inner).ok()?;
    Some((p.get("dirty")?.as_bool()?, p.get("playing")?.as_bool()?))
}

impl Shell {
    /// Start the periodic check. It ticks every 30 seconds and runs the rules when their interval has passed, so a change of
    /// rules, interval or `sleep_tabs_after` (a configuration reload) takes effect without reopening the window.
    pub(super) fn start_sweeping(self: &Rc<Self>) {
        let window = self.window.downgrade();
        glib::timeout_add_seconds_local(30, move || match window.upgrade().and_then(|w| shell_of(&w)) {
            Some(s) => {
                let interval = s.sweeper.borrow().as_ref().map(|r| r.interval_secs);
                if interval.is_some_and(|i| now_secs() - s.last_pass.get() >= i) {
                    s.sweep_now();
                }
                s.refresh_swept(); // tabs age out of the list
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    /// Note the use of the tab being looked at: the selected tab of the active window, and more if
    /// something was typed or clicked since last time.
    fn note_use(&self, now: i64) {
        if !self.window.is_active() {
            return;
        }
        let Some(tab) = self.current() else { return };
        let mut u = tab.usage.borrow_mut();
        u.record(now, 0.2);
        let input = tab.view().input_events();
        if input != tab.input_seen.get() {
            tab.input_seen.set(input);
            u.record(now, 0.3);
        }
    }

    /// Check the rules now: ask each live tab whether it holds unsaved input or is playing media, then
    /// act (`sweeps_done` counts finished passes). Tabs whose probe does not answer are left alone.
    pub fn sweep_now(self: &Rc<Self>) {
        if self.sweeper.borrow().is_none() {
            return;
        }
        self.last_pass.set(now_secs());
        self.note_use(now_secs());
        let probes: Vec<(Rc<Tab>, i32)> = self.tabs.borrow().iter().filter(|t| !t.view().is_dormant()).map(|t| (t.clone(), t.view().cdp_eval(PROBE))).collect();
        let window = self.window.downgrade();
        glib::timeout_add_local_once(PROBE_WAIT, move || {
            if let Some(s) = window.upgrade().and_then(|w| shell_of(&w)) {
                s.finish_sweep(probes);
            }
        });
    }

    fn finish_sweep(self: &Rc<Self>, probes: Vec<(Rc<Tab>, i32)>) {
        let Some(sweeper) = self.sweeper.borrow().clone() else { return };
        // What the probes found. A tab that did not answer is "unknown": not swept.
        crate::log_debug!("sweep pass in {:?}: {} tab(s), {} rule(s), {} probed", self.workspace_id(), self.tabs.borrow().len(), sweeper.rules.len(), probes.len());
        let mut unknown = std::collections::HashSet::new();
        for (tab, id) in &probes {
            match tab.view().cdp_result(*id).and_then(|(ok, json)| ok.then_some(json)).and_then(|j| parse_probe(&j)) {
                Some((dirty, playing)) => {
                    tab.dirty.set(dirty);
                    tab.playing.set(playing);
                }
                None => {
                    unknown.insert(tab.uid);
                }
            }
            self.mark_tab(tab);
        }
        let now = now_secs();
        let selected = self.current();
        let mut notes = Vec::new();
        let mut closed = 0;
        let mut discarded = 0;
        for tab in self.tabs().into_iter().filter(|t| !t.parked.get()) {
            // The tab being looked at in any window is never swept or put to sleep (a sleeping tab wakes when it is
            // selected, so the selected tab of a window that is out of sight stays awake too). Pinned tabs are safe from
            // the sweep rules but may sleep: `decide` knows which rule may take them.
            let protected = selected.as_ref().is_some_and(|s| Rc::ptr_eq(s, &tab))
                || tab.dirty.get()
                || tab.playing.get()
                || tab.state.borrow().loading
                || unknown.contains(&tab.uid) && !tab.view().is_dormant();
            let url = tab.state.borrow().address.clone();
            if protected {
                if crate::log::enabled(crate::log::Level::Debug) {
                    let why = if selected.as_ref().is_some_and(|s| Rc::ptr_eq(s, &tab)) {
                        "selected"
                    } else if tab.dirty.get() {
                        "unsaved form input"
                    } else if tab.playing.get() {
                        "playing media"
                    } else if tab.state.borrow().loading {
                        "still loading"
                    } else {
                        "probe gave no answer"
                    };
                    crate::log_debug!("sweep keeps {url}: {why}");
                }
                continue;
            }
            // The last tab of a workspace is never closed (the selected tab is safe anyway; this makes it explicit).
            if self.tabs.borrow().len() <= 1 {
                crate::log_debug!("sweep keeps {url}: the last tab of the window");
                continue;
            }
            let usage = tab.usage.borrow().clone();
            let Some(rule) = sweeper.decide(&url, &usage, now, tab.created.get(), tab.is_pinned()) else {
                if crate::log::enabled(crate::log::Level::Debug) {
                    crate::log_debug!("sweep keeps {url}: no rule fires ({})", sweeper.explain(&url, &usage, now, tab.created.get(), tab.is_pinned()));
                }
                continue;
            };
            let idle = crate::sweep::short_duration(now - usage.last_used().unwrap_or(tab.created.get()));
            crate::log_info!(
                "sweep {:?}: rule {:?} {:?} {url} (title {:?}, idle {idle}, pinned {}, {})",
                self.workspace_id(),
                rule.name,
                rule.action,
                tab.state.borrow().title,
                tab.is_pinned(),
                if tab.view().is_dormant() { "already asleep" } else { "awake" }
            );
            match rule.action {
                Action::Close => {
                    *self.pending_close_reason.borrow_mut() = Some(format!("sweep:{}", rule.name));
                    self.tab_view.close_page(&tab.page);
                    *self.pending_close_reason.borrow_mut() = None;
                    closed += 1;
                    notes.push(format!("closed {url} ({})", rule.name));
                }
                Action::Discard => {
                    if !tab.view().is_dormant() {
                        self.discard_tab(&tab);
                        // Automatic sleeping (`sleep_tabs_after`) is quiet: only the rules the person wrote announce themselves.
                        if rule.name != crate::sweep::SLEEP_RULE {
                            discarded += 1;
                        }
                        notes.push(format!("discarded {url} ({})", rule.name));
                    }
                }
            }
        }
        if closed + discarded > 0 {
            let mut parts = Vec::new();
            if closed > 0 {
                parts.push(format!("closed {closed} unused tab{}", if closed == 1 { "" } else { "s" }));
            }
            if discarded > 0 {
                parts.push(format!("put {discarded} to sleep"));
            }
            self.toast(&format!("Swept: {} (Ctrl+Shift+T brings closed tabs back)", parts.join(", ")));
        }
        self.refresh_swept();
        crate::log_info!("sweep pass in {:?} done: {} closed, {} put to sleep, notes {notes:?}", self.workspace_id(), closed, discarded);
        *self.last_sweep.borrow_mut() = notes;
        self.sweeps_done.set(self.sweeps_done.get() + 1);
        self.schedule_save();
    }

    /// Show on the tab why it is safe from sweeping: unsaved input or playing media.
    fn mark_tab(&self, tab: &Rc<Tab>) {
        if tab.dirty.get() {
            tab.page.set_indicator_icon(Some(&gio::ThemedIcon::new("document-edit-symbolic")));
            tab.page.set_indicator_tooltip("Unsaved changes in a form: this tab will not be swept");
        } else if tab.playing.get() {
            tab.page.set_indicator_icon(Some(&gio::ThemedIcon::new("audio-volume-high-symbolic")));
            tab.page.set_indicator_tooltip("Playing: this tab will not be swept");
        } else {
            tab.page.set_indicator_icon(None::<&gio::Icon>);
            tab.page.set_indicator_tooltip("");
        }
    }

    /// Free a tab's page but keep the tab: its last frame is saved as a screenshot, its browser is
    /// closed, and a sleeping view takes its place until it is shown again.
    pub(super) fn discard_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        use super::view::BrowserView;
        let mut url = tab.state.borrow().address.clone();
        if url.is_empty() {
            url = tab.first_url.borrow().clone(); // the page had not reported an address yet
        }
        if url.is_empty() || url == "about:blank" {
            // A sleeping tab comes back from its address: with none it would wake blank, so a blank tab stays as it is.
            crate::log_info!("tab not put to sleep: it has no address (title {:?}, workspace {:?})", tab.state.borrow().title, self.workspace_id());
            return;
        }
        crate::log_info!("tab put to sleep: {url:?} (title {:?}, workspace {:?})", tab.state.borrow().title, self.workspace_id());
        self.snapshot_with(tab, true);
        let cell: Rc<std::cell::OnceCell<std::rc::Weak<Tab>>> = Rc::new(std::cell::OnceCell::new());
        let _ = cell.set(Rc::downgrade(tab));
        let shot = self.screenshot_path(tab.uid);
        self.prepare_storage();
        let new = BrowserView::new_dormant(gtk::prelude::WidgetExt::display(&self.window), self.mode.get(), self.backing, self.frame_rate.get(), &url, shot.as_deref(), self.event_closure(&cell));
        new.set_workspace(self.workspace_id().map(str::to_owned));
        new.set_popup_policy(self.popup_policy);
        self.apply_view_settings(&new);
        let old = tab.view();
        tab.holder.prepend(&new.widget);
        tab.holder.remove(&old.widget);
        *tab.view.borrow_mut() = new;
        old.retire();
        {
            let mut s = tab.state.borrow_mut();
            s.ready = false;
            s.loading = false;
        }
        tab.page.set_title(&tab_title(&tab.state.borrow()));
        tab.dirty.set(false);
        tab.playing.set(false);
        self.mark_tab(tab);
        self.refresh_tab_icon(tab);
    }

    /// What the last sweep did (the self-test).
    pub fn last_sweep_notes(&self) -> Vec<String> {
        self.last_sweep.borrow().clone()
    }

    pub fn sweeps_done(&self) -> u32 {
        self.sweeps_done.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_answers_are_read_from_the_devtools_result() {
        let ok = r#"{"result":{"type":"string","value":"{\"dirty\":true,\"playing\":false}"}}"#;
        assert_eq!(parse_probe(ok), Some((true, false)));
        let playing = r#"{"result":{"type":"string","value":"{\"dirty\":false,\"playing\":true}"}}"#;
        assert_eq!(parse_probe(playing), Some((false, true)));
        // An exception in the page or anything unexpected: no answer, so the tab is left alone.
        assert_eq!(parse_probe(r#"{"result":{"type":"object","subtype":"error"},"exceptionDetails":{}}"#), None);
        assert_eq!(parse_probe("not json at all {"), None);
        assert_eq!(parse_probe(r#"{"result":{"type":"string","value":"nonsense"}}"#), None);
    }
}
