//! The task manager (`Shift+Esc`, main menu): which tabs keep the page busy or hold memory, and what each of the browser's
//! processes costs. A tab's numbers come from the page itself (`Performance.getMetrics` through DevTools: main-thread CPU, script
//! memory, elements); the processes come from `/proc` (`crate::taskman`). It samples only while the window is open, on the GTK
//! thread's timer (the process scan runs on a worker thread), and never waits for an answer: calls are posted on one tick and read
//! on the next. docs/research/2026-11-21-task-manager.md.

use super::view::BrowserView;
use super::{favicon, windows, Shell, Tab};
use crate::taskman::{self, TabMetrics};
use adw::prelude::*;
use gtk::glib;
use gtk::glib::object::ObjectType;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

const EVERY: Duration = Duration::from_millis(1500);
/// A tab (its unique number) and which pane of it (0 left, 1 right).
type TabKey = (u64, usize);

struct Pending {
    key: TabKey,
    view: Rc<BrowserView>,
    id: i32,
    tries: u8,
}

#[derive(Default, Clone, Copy)]
struct TabData {
    metrics: Option<TabMetrics>,
    cpu: Option<f64>,
}

/// The open task manager; its lists are public to the self-test.
pub struct TaskManager {
    pub window: adw::Window,
    /// Everything under the header bar (the self-test checks its text weight).
    pub content: gtk::Box,
    pub tab_list: gtk::ListBox,
    pub proc_list: gtk::ListBox,
    pub summary: adw::ActionRow,
    pub cpu_sort: gtk::ToggleButton,
    pub memory_sort: gtk::ToggleButton,
    tab_rows: RefCell<HashMap<TabKey, adw::ActionRow>>,
    proc_rows: RefCell<HashMap<u32, adw::ActionRow>>,
    /// What each row sorts by (largest first), by the row's address: the rows keep their identity while they move.
    order: Rc<RefCell<HashMap<usize, f64>>>,
    data: RefCell<HashMap<TabKey, TabData>>,
    pending: RefCell<Vec<Pending>>,
    /// Views the page counters were switched on for, to switch them off again when the window closes.
    enabled: RefCell<HashMap<u64, Weak<BrowserView>>>,
    prev_procs: RefCell<HashMap<u32, u64>>,
    prev_at: Cell<Option<Instant>>,
    scanning: Cell<bool>,
    ticks_per_second: f64,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<TaskManager>>> = const { RefCell::new(None) };
}

/// The open task manager, if any.
pub fn task_manager() -> Option<Rc<TaskManager>> {
    OPEN.with(|o| o.borrow().clone())
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).css_classes(["dim-label"]).margin_top(12).margin_bottom(6).build()
}

fn list() -> gtk::ListBox {
    gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build()
}

fn icon_button(icon: crate::icons::Icon, tip: &str) -> gtk::Button {
    let b = gtk::Button::builder().valign(gtk::Align::Center).has_frame(false).tooltip_text(tip).build();
    let image = gtk::Image::new();
    crate::icons::bind(&image, icon);
    b.set_child(Some(&image));
    b
}

impl Shell {
    /// Show the task manager (the one window of the profile; a second request raises it).
    pub fn open_task_manager(self: &Rc<Self>) -> Rc<TaskManager> {
        if let Some(open) = task_manager() {
            open.window.present();
            return open;
        }
        let tab_list = list();
        let proc_list = list();
        let summary = adw::ActionRow::builder().title("Everything this browser runs").subtitle("Measuring\u{2026}").build();
        let summary_list = list();
        summary_list.append(&summary);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_start(12).margin_end(12).margin_bottom(12).build();
        body.append(&summary_list);
        body.append(&heading("Tabs (CPU is the page's main thread: script, layout and styles)"));
        body.append(&tab_list);
        body.append(&heading("Processes (a page renderer can serve several tabs of a site)"));
        body.append(&proc_list);
        let scroll = gtk::ScrolledWindow::builder().child(&body).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        let cpu_sort = gtk::ToggleButton::builder().label("CPU").active(true).build();
        let memory_sort = gtk::ToggleButton::builder().label("Memory").group(&cpu_sort).build();
        let sorts = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).css_classes(["linked", "browsah-sort"]).build();
        sorts.append(&cpu_sort);
        sorts.append(&memory_sort);
        let sort_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).margin_top(12).build();
        sort_row.append(&gtk::Label::builder().label("Sort by").css_classes(["dim-label"]).build());
        sort_row.append(&sorts);
        body.prepend(&sort_row);
        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scroll));
        let window = adw::Window::builder().title("Task manager").content(&toolbar).default_width(720).default_height(640).transient_for(&self.window).build();

        let order: Rc<RefCell<HashMap<usize, f64>>> = Rc::new(RefCell::new(HashMap::new()));
        for l in [&tab_list, &proc_list] {
            let order = order.clone();
            l.set_sort_func(move |a, b| {
                let o = order.borrow();
                let key = |r: &gtk::ListBoxRow| o.get(&(r.as_ptr() as usize)).copied().unwrap_or(-1.0);
                key(b).partial_cmp(&key(a)).unwrap_or(std::cmp::Ordering::Equal).into()
            });
        }
        let tm = Rc::new(TaskManager {
            window: window.clone(),
            content: body.clone(),
            tab_list,
            proc_list,
            summary,
            cpu_sort: cpu_sort.clone(),
            memory_sort: memory_sort.clone(),
            tab_rows: RefCell::new(HashMap::new()),
            proc_rows: RefCell::new(HashMap::new()),
            order,
            data: RefCell::new(HashMap::new()),
            pending: RefCell::new(Vec::new()),
            enabled: RefCell::new(HashMap::new()),
            prev_procs: RefCell::new(HashMap::new()),
            prev_at: Cell::new(None),
            scanning: Cell::new(false),
            ticks_per_second: taskman::ticks_per_second(),
        });
        OPEN.with(|o| *o.borrow_mut() = Some(tm.clone()));
        memory_sort.connect_toggled({
            let tm = Rc::downgrade(&tm);
            move |_| {
                if let Some(tm) = tm.upgrade() {
                    tm.resort();
                }
            }
        });
        window.connect_close_request({
            let tm = tm.clone();
            move |_| {
                tm.stop();
                glib::Propagation::Proceed
            }
        });
        window.present();
        // The first reading soon, then on the usual beat; the loop ends with the window.
        tm.sample();
        let weak = Rc::downgrade(&tm);
        glib::timeout_add_local(EVERY, move || match weak.upgrade().filter(|tm| tm.window.is_visible()) {
            Some(tm) => {
                tm.sample();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        tm
    }
}

impl TaskManager {
    fn by_memory(&self) -> bool {
        self.memory_sort.is_active()
    }

    /// The window is closing: switch the page counters off and forget the window.
    fn stop(&self) {
        for (_, view) in self.enabled.borrow_mut().drain() {
            if let Some(v) = view.upgrade().filter(|v| v.is_live()) {
                v.cdp("Performance.disable");
            }
        }
        self.pending.borrow_mut().clear();
        OPEN.with(|o| *o.borrow_mut() = None);
    }

    fn resort(&self) {
        let memory = self.by_memory();
        {
            let data = self.data.borrow();
            let mut order = self.order.borrow_mut();
            for (key, row) in self.tab_rows.borrow().iter() {
                let d = data.get(key).copied().unwrap_or_default();
                let value = d.metrics.map(|m| if memory { m.js_used } else { d.cpu.unwrap_or(0.0) }).unwrap_or(-1.0);
                order.insert(row.as_ptr() as usize, value);
            }
        }
        self.tab_list.invalidate_sort();
        self.proc_list.invalidate_sort();
    }

    /// One beat: read what the last beat asked for, ask again, and scan the processes.
    pub fn sample(self: &Rc<Self>) {
        // 1. Answers to the last beat's calls.
        let mut still = Vec::new();
        for mut p in self.pending.borrow_mut().drain(..) {
            match p.view.take_cdp_result(p.id) {
                Some((true, json)) => {
                    if let Some(m) = taskman::parse_metrics(&json) {
                        let mut data = self.data.borrow_mut();
                        let d = data.entry(p.key).or_default();
                        d.cpu = d.metrics.as_ref().and_then(|prev| taskman::tab_cpu_percent(prev, &m));
                        d.metrics = Some(m);
                    }
                }
                Some((false, _)) => {}
                None if p.tries < 3 => {
                    p.tries += 1;
                    still.push(p);
                }
                None => {}
            }
        }
        *self.pending.borrow_mut() = still;

        // 2. The tabs of every window, their rows, and new calls for the ones that are awake.
        let mut seen = HashSet::new();
        for shell in windows() {
            let workspace = shell.workspace.as_ref().map(|w| w.display_name().to_owned()).unwrap_or_else(|| "Window".to_owned());
            let tabs: Vec<Rc<Tab>> = shell.tabs.borrow().clone();
            for tab in tabs {
                for (pane, view) in tab.views().into_iter().enumerate() {
                    let key = (tab.uid, pane);
                    seen.insert(key);
                    let (title, address) = {
                        let state = if pane == 0 { tab.state.clone() } else { tab.split_pane().map(|p| p.state.clone()).unwrap_or_else(|| tab.state.clone()) };
                        let s = state.borrow();
                        (s.title.clone(), s.address.clone())
                    };
                    let name = if title.is_empty() { address.clone() } else { title };
                    let name = if pane == 1 { format!("{name} (right pane)") } else { name };
                    let row = self.tab_row(key, &shell, &tab);
                    row.set_title(&glib::markup_escape_text(if name.is_empty() { "New tab" } else { &name }));
                    let asleep = view.is_dormant() || !view.is_live();
                    let d = self.data.borrow().get(&key).copied().unwrap_or_default();
                    let detail = if asleep {
                        "Asleep: no page is loaded, nothing is used".to_owned()
                    } else if let Some(m) = d.metrics {
                        format!(
                            "CPU {} \u{b7} script memory {} \u{b7} {} elements",
                            d.cpu.map(taskman::percent_text).unwrap_or_else(|| "measuring\u{2026}".into()),
                            taskman::bytes_text(m.js_used),
                            m.nodes
                        )
                    } else {
                        "Measuring\u{2026}".to_owned()
                    };
                    row.set_subtitle(&glib::markup_escape_text(&format!("{workspace} \u{b7} {detail}")));
                    if !asleep {
                        let uid = view.uid();
                        if !self.enabled.borrow().contains_key(&uid) {
                            view.cdp("Performance.enable");
                            self.enabled.borrow_mut().insert(uid, Rc::downgrade(&view));
                        }
                        let id = view.cdp("Performance.getMetrics");
                        self.pending.borrow_mut().push(Pending { key, view: view.clone(), id, tries: 0 });
                    } else {
                        self.data.borrow_mut().remove(&key);
                        self.enabled.borrow_mut().remove(&view.uid());
                    }
                }
            }
        }
        // Rows of tabs that are gone.
        self.tab_rows.borrow_mut().retain(|key, row| {
            let keep = seen.contains(key);
            if !keep {
                self.order.borrow_mut().remove(&(row.as_ptr() as usize));
                self.tab_list.remove(row);
            }
            keep
        });
        self.data.borrow_mut().retain(|k, _| seen.contains(k));
        self.resort();

        // 3. The processes, read on a worker thread.
        if !self.scanning.replace(true) {
            let tm = self.clone();
            glib::spawn_future_local(async move {
                let procs = gtk::gio::spawn_blocking(taskman::scan).await.unwrap_or_default();
                tm.scanning.set(false);
                if tm.window.is_visible() {
                    tm.show_processes(procs);
                }
            });
        }
    }

    /// The row of a tab, made on first sight: its icon and the two buttons.
    fn tab_row(&self, key: TabKey, shell: &Rc<Shell>, tab: &Rc<Tab>) -> adw::ActionRow {
        if let Some(r) = self.tab_rows.borrow().get(&key) {
            return r.clone();
        }
        let row = adw::ActionRow::builder().title("").build();
        row.set_title_lines(1);
        row.set_subtitle_lines(1);
        if let Some(icon) = favicon::small_image(&tab.state.borrow().address) {
            row.add_prefix(&icon);
        }
        let (weak_shell, weak_tab) = (Rc::downgrade(shell), Rc::downgrade(tab));
        let sleep = icon_button(crate::icons::PAUSE, "Put the tab to sleep (frees its page; it reloads when shown)");
        sleep.connect_clicked({
            let (weak_shell, weak_tab) = (weak_shell.clone(), weak_tab.clone());
            move |_| {
                if let (Some(s), Some(t)) = (weak_shell.upgrade(), weak_tab.upgrade()) {
                    if !t.view().is_dormant() {
                        s.discard_tab(&t);
                    }
                }
            }
        });
        let close = icon_button(crate::icons::X, "Close the tab");
        close.connect_clicked(move |_| {
            if let (Some(s), Some(t)) = (weak_shell.upgrade(), weak_tab.upgrade()) {
                s.tab_view.close_page(&t.page);
            }
        });
        row.add_suffix(&sleep);
        row.add_suffix(&close);
        self.tab_list.append(&row);
        self.tab_rows.borrow_mut().insert(key, row.clone());
        row
    }

    /// Show a fresh scan: each process's CPU since the last one, the total, and the rows.
    fn show_processes(&self, procs: Vec<taskman::Proc>) {
        let now = Instant::now();
        let wall = self.prev_at.replace(Some(now)).map(|t| now.duration_since(t).as_secs_f64());
        let memory = self.by_memory();
        let (mut total_cpu, mut total_bytes) = (0.0, 0u64);
        let mut seen = HashSet::new();
        let mut counts: HashMap<taskman::Kind, usize> = HashMap::new();
        for p in &procs {
            seen.insert(p.pid);
            *counts.entry(p.kind).or_default() += 1;
            let cpu = match (wall, self.prev_procs.borrow().get(&p.pid)) {
                (Some(w), Some(prev)) => Some(taskman::proc_cpu_percent(*prev, p.ticks, w, self.ticks_per_second)),
                _ => None,
            };
            total_cpu += cpu.unwrap_or(0.0);
            total_bytes += p.bytes;
            let existing = self.proc_rows.borrow().get(&p.pid).cloned();
            let row = existing.unwrap_or_else(|| {
                let row = adw::ActionRow::builder().title(p.kind.label()).build();
                row.set_title_lines(1);
                self.proc_list.append(&row);
                self.proc_rows.borrow_mut().insert(p.pid, row.clone());
                row
            });
            row.set_subtitle(&format!("CPU {} \u{b7} memory {} \u{b7} process {}", cpu.map(taskman::percent_text).unwrap_or_else(|| "measuring\u{2026}".into()), taskman::bytes_text(p.bytes as f64), p.pid));
            self.order.borrow_mut().insert(row.as_ptr() as usize, if memory { p.bytes as f64 } else { cpu.unwrap_or(0.0) });
        }
        self.proc_rows.borrow_mut().retain(|pid, row| {
            let keep = seen.contains(pid);
            if !keep {
                self.order.borrow_mut().remove(&(row.as_ptr() as usize));
                self.proc_list.remove(row);
            }
            keep
        });
        *self.prev_procs.borrow_mut() = procs.iter().map(|p| (p.pid, p.ticks)).collect();
        let renderers = counts.get(&taskman::Kind::Renderer).copied().unwrap_or(0);
        self.summary.set_subtitle(&format!(
            "CPU {} of one core \u{b7} memory {} \u{b7} {} processes, {} page renderers",
            if wall.is_some() { taskman::percent_text(total_cpu) } else { "measuring\u{2026}".into() },
            taskman::bytes_text(total_bytes as f64),
            procs.len(),
            renderers
        ));
        self.proc_list.invalidate_sort();
    }

    /// The text of a tab's row, for the self-test: (title, subtitle).
    pub fn tab_texts(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(f64, String, String)> = self.tab_rows.borrow().values().map(|r| (self.order.borrow().get(&(r.as_ptr() as usize)).copied().unwrap_or(-1.0), r.title().to_string(), r.subtitle().map(|s| s.to_string()).unwrap_or_default())).collect();
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        rows.into_iter().map(|(_, t, s)| (t, s)).collect()
    }
}
