//! `BrowserView`: one CEF browser rendered into a `GtkPicture`.
//!
//! CEF runs on its own thread, so its handlers only touch `Shared` (behind mutexes and atomics)
//! and hand frames and events to the GTK thread with `MainContext::invoke`. GTK objects live in a
//! thread-local registry keyed by the view's id and are only used on the GTK thread.

use super::input;
use crate::cef_rt::{on_ui, BrowserHandle, HostHandle};
use crate::cli::Mode;
use crate::frames::{import_frame, DmabufFrame, View};
use cef::*;
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// One entry of a context menu.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
}

/// CEF's callback for completing a context menu. Resolved (or cancelled) exactly once.
struct MenuCallback(RunContextMenuCallback);
unsafe impl Send for MenuCallback {}

/// A context menu CEF wants shown, with the callback to answer it.
pub struct MenuRequest {
    /// Position in logical widget coordinates.
    pub x: f64,
    pub y: f64,
    pub items: Vec<MenuItem>,
    callback: Mutex<Option<MenuCallback>>,
}

impl std::fmt::Debug for MenuRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MenuRequest({} items at {},{})", self.items.len(), self.x, self.y)
    }
}

impl MenuRequest {
    /// Answer with a command for CEF to run. Posted to CEF's UI thread.
    fn run(&self, id: i32) {
        if let Some(cb) = self.callback.lock().unwrap().take() {
            on_ui(move || {
                let cb = cb; // whole wrapper, not its field, so the closure stays Send
                cb.0.cont(id, EventFlags::from(sys::cef_event_flags_t::EVENTFLAG_NONE));
            });
        }
    }

    /// Dismiss the menu without choosing anything (a no-op if already answered).
    fn cancel(&self) {
        if let Some(cb) = self.callback.lock().unwrap().take() {
            on_ui(move || {
                let cb = cb;
                cb.0.cancel();
            });
        }
    }
}

/// Context-menu commands this browser can carry out (CEF's `MENU_ID_*`).
fn supported(id: i32) -> bool {
    use sys::cef_menu_id_t as M;
    [
        M::MENU_ID_BACK,
        M::MENU_ID_FORWARD,
        M::MENU_ID_RELOAD,
        M::MENU_ID_RELOAD_NOCACHE,
        M::MENU_ID_STOPLOAD,
        M::MENU_ID_UNDO,
        M::MENU_ID_REDO,
        M::MENU_ID_CUT,
        M::MENU_ID_COPY,
        M::MENU_ID_PASTE,
        M::MENU_ID_DELETE,
        M::MENU_ID_SELECT_ALL,
    ]
    .iter()
    .any(|m| *m as i32 == id)
}

/// What a page tells the shell about itself.
#[derive(Debug, Clone)]
pub enum ViewEvent {
    /// Show this context menu.
    ContextMenu(Arc<MenuRequest>),
    /// The browser exists; navigation calls now work.
    Ready,
    /// A page's own copy button put this text on its clipboard (the shell puts it on the system clipboard, if the person has just used the page).
    ClipboardWrite(String),
    /// The main frame's address changed.
    Address(String),
    Title(String),
    Loading { loading: bool, can_back: bool, can_forward: bool },
    /// A page asks for permissions the configuration leaves to the person.
    Permission(Arc<PermissionRequest>),
    /// The page moved on: a question with this id (from `Permission`) is no longer wanted.
    PermissionDismissed(u64),
    /// A download changed.
    Download { id: u32, name: String, url: String, state: DownloadState, cancel: Option<Arc<DownloadCancel>> },
    /// A download is waiting for the person to say where to save it.
    DownloadAsk(Arc<DownloadQuestion>),
    /// A page called `alert`, `confirm` or `prompt`, or is being left with unsaved changes.
    JsDialog(Arc<JsDialogQuestion>),
    /// The page's dialog is no longer wanted (it navigated away or was closed).
    JsDialogDismissed(u64),
    /// The browser has closed (after `ask_close`, once the page agreed to be left).
    Closed,
    /// The person chose to stay on a page that was asked to close.
    CloseCancelled,
    /// A page asks for files (`<input type=file>`, a directory, a save target).
    FileDialog(Arc<FileDialogQuestion>),
    /// Ctrl and the wheel: zoom in (1) or out (-1).
    Zoom(i32),
    /// The page's icon, as a PNG (empty: the page has none).
    Favicon(Vec<u8>),
    /// A page was written to a PDF (or that failed).
    PdfSaved { path: String, ok: bool },
    /// A page entered (true) or left (false) element fullscreen.
    Fullscreen(bool),
    /// Find in page: how many matches and which one is active (1-based; 0 for none).
    FindResult { count: i32, active: i32 },
    /// The page navigated to an expected non-web scheme and it was handed to the desktop.
    ExternalLaunch { scheme: String, url: String, signin: bool },
    /// The router sent this link elsewhere; the navigation was cancelled and this is what to do.
    Route { action: crate::router::Action, url: String },
    /// A main-frame navigation left the view's scope and was cancelled (see `set_scope`): open it elsewhere.
    OutOfScope(String),
    /// A link asked for a new tab (Ctrl+click, middle click): open it; `select` for a foreground tab.
    OpenTab { url: String, select: bool },
    /// A popup that keeps its opener (`window.open` with a user gesture): adopt it as a tab.
    Popup(PopupSpec),
    /// The page's renderer process died (crash, out of memory, killed).
    Crashed(String),
}

/// Keeps a DevTools observer registration alive (dropping it unregisters the observer).
struct RegHandle(#[allow(dead_code)] Registration);
unsafe impl Send for RegHandle {}

/// The prefix of the console message a page's copy hook sends (`COPY_HOOK`); `on_console_message` takes it and the rest is the text.
const COPY_PREFIX: &str = "\u{1}browsah-copy\u{1}";

/// Pages copy with `navigator.clipboard.writeText`/`write`, `document.execCommand('copy')` or a `copy` event handler. Windowless
/// Chromium cannot reach the Wayland clipboard, so these fail in the page (`NotAllowedError`, `false`), and a copy that did work
/// would stay inside the browser. This script, run in every frame as it starts loading (`on_load_start`), makes them succeed and
/// reports the text as a console message; the shell uses it only if the person has just used the page. Paste (`readText`) is left alone.
const COPY_HOOK: &str = r#"(() => {
  if (window.__browsahCopyHook) return;
  window.__browsahCopyHook = true;
  const send = t => { try { if (typeof t === 'string' && t.length <= 1000000) console.debug('\u0001browsah-copy\u0001' + t); } catch (e) {} };
  const selected = () => {
    const a = document.activeElement;
    if (a && (a.tagName === 'TEXTAREA' || a.tagName === 'INPUT') && typeof a.selectionStart === 'number' && a.selectionStart !== a.selectionEnd)
      return a.value.substring(a.selectionStart, a.selectionEnd);
    return String(getSelection());
  };
  let inCopy = false, inExec = false, got = null;
  const setData = DataTransfer.prototype.setData;
  DataTransfer.prototype.setData = function (type, data) {
    if (inCopy && String(type).toLowerCase() === 'text/plain') got = String(data);
    return setData.apply(this, arguments);
  };
  for (const name of ['copy', 'cut']) {
    addEventListener(name, () => { inCopy = true; got = null; setTimeout(() => { inCopy = false; }, 0); }, true);
    addEventListener(name, () => { if (!inExec && got !== null) send(got); });
  }
  const exec = Document.prototype.execCommand;
  Document.prototype.execCommand = function (cmd) {
    const c = String(cmd).toLowerCase();
    if (c !== 'copy' && c !== 'cut') return exec.apply(this, arguments);
    const before = selected();
    inExec = true; got = null;
    let ok = false;
    try { ok = exec.apply(this, arguments); } finally { inExec = false; }
    const text = got !== null ? got : before;
    if (text) { send(text); return true; }
    return ok;
  };
  const C = window.Clipboard && Clipboard.prototype;
  if (C) {
    C.writeText = function (t) { send(String(t)); return Promise.resolve(); };
    C.write = async function (items) {
      for (const item of items || []) {
        if (item.types.includes('text/plain')) send(await (await item.getType('text/plain')).text());
      }
    };
  }
})()"#;

wrap_dev_tools_message_observer! {
    struct ViewDevTools { shared: Arc<Shared> }

    impl DevToolsMessageObserver {
        // Results of `execute_dev_tools_method` calls, kept by message id for the caller to read.
        fn on_dev_tools_method_result(
            &self,
            _b: Option<&mut Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            let text = result.map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
            self.shared.cdp_results.lock().unwrap().insert(message_id, (success != 0, text));
        }
    }
}

wrap_download_image_callback! {
    struct ViewImage { shared: Arc<Shared> }

    impl DownloadImageCallback {
        fn on_download_image_finished(&self, url: Option<&CefString>, status: ::std::os::raw::c_int, image: Option<&mut Image>) {
            let url = url.map(|u| u.to_string()).unwrap_or_default();
            let image = image.map(|i| &*i);
            let (w, h) = image.map(|i| (i.width(), i.height())).unwrap_or((0, 0));
            // `as_png` needs the pixel size outputs, or it gives nothing.
            let (mut pw, mut ph) = (0, 0);
            if let Some(png) = image.and_then(|i| i.as_png(1.0, 1, Some(&mut pw), Some(&mut ph))) {
                let mut buf = vec![0u8; png.size()];
                let n = png.data(Some(&mut buf), 0);
                buf.truncate(n);
                self.shared.image_data.lock().unwrap().insert(url.clone(), buf);
            }
            self.shared.images.lock().unwrap().insert(url, (status, w, h));
        }
    }
}

/// State shared with CEF's thread.
struct Shared {
    id: u64,
    view: Mutex<View>,
    browser: Mutex<Option<BrowserHandle>>,
    host: Mutex<Option<HostHandle>>,
    /// Frames CEF has delivered (any path).
    frames: AtomicU64,
    /// The view is not on screen (a background tab): CEF is told to stop rendering it. Starts
    /// false (a new view is assumed visible); set when the widget is unmapped, cleared when mapped.
    hidden: AtomicBool,
    /// A navigation requested before the browser existed; run when it is created.
    pending_url: Mutex<Option<String>>,
    /// DevTools method results by message id: (success, JSON text).
    cdp_results: Mutex<HashMap<i32, (bool, String)>>,
    cdp_registration: Mutex<Option<RegHandle>>,
    /// A drag that started in the page (a `draggable` element): windowless Chromium has no system drag loop, so the pointer
    /// drives it (see `BrowserView::page_drag_move`). Set on CEF's thread, read on the GTK thread.
    page_drag: Mutex<Option<PageDrag>>,
    /// Downloaded images by URL: (HTTP status, width, height).
    images: Mutex<HashMap<String, (i32, usize, usize)>>,
    /// The same images as PNG bytes (for an app's icon).
    image_data: Mutex<HashMap<String, Vec<u8>>>,
    next_cdp_id: std::sync::atomic::AtomicI32,
    /// Held while a DevTools message id is taken and the call posted, so ids run in order (see `Shared::post_cdp`).
    cdp_order: Mutex<()>,
    /// Set while the browser has not been created yet (a restored tab): its address.
    deferred_url: Mutex<Option<String>>,
    /// Where the view may navigate (patterns of `workspace::scope_matches`); empty: anywhere.
    scope: Mutex<Vec<String>>,
    /// The workspace this view's tab is in (`None`: an ephemeral window); the router's source.
    workspace: Mutex<Option<String>>,
    /// URL schemes this view may hand to the desktop (see `on_protocol_execution`).
    allowed_schemes: Mutex<Vec<String>>,
    /// What a script popup from this view becomes.
    popup_policy: Mutex<crate::workspace::PopupPolicy>,
    /// What the configuration says about permissions, and where downloads go (set by the shell).
    permissions: Mutex<crate::permissions::PermissionsConfig>,
    cookies: Mutex<crate::cookies::CookiesConfig>,
    /// The person's zoom for this page (1.0 = 100 %), on top of the scale correction for device resolution.
    user_zoom: Mutex<f64>,
    /// The last find-in-page result: (matches, active one).
    find_result: Mutex<(i32, i32)>,
    /// The colour scheme the page is to see: `Some(true)` dark, `Some(false)` light, `None` not set yet.
    scheme: Mutex<Option<bool>>,
    /// The main-frame navigation under way began as a clicked link (so its redirects may be routed too).
    link_nav: AtomicBool,
    /// Whether developer tools were open after the last toggle (read on CEF's UI thread, kept here).
    dev_tools: AtomicBool,
    /// Who the session cookies of this view belong to (its window), and the sites whose cookies the rules
    /// blocked since the page started loading.
    session_owner: Mutex<String>,
    blocked_cookies: Mutex<std::collections::BTreeSet<String>>,
    /// Where this view keeps its site data when apart from the profile's (a workspace's `storage: isolated`).
    storage: Option<std::path::PathBuf>,
    downloads: Mutex<(std::path::PathBuf, bool)>,
    /// Rendering settings, so a popup browser can be created like its opener.
    accel: bool,
    frame_rate: i32,
}

thread_local! {
    /// Set by the shell just before it creates a view: the folder for that view's own site data.
    static NEXT_STORAGE: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
    /// The request contexts made so far, by folder (GTK thread).
    static CONTEXTS: RefCell<HashMap<std::path::PathBuf, RequestContext>> = RefCell::new(HashMap::new());
}

/// The next view created keeps its cookies and site data in `dir` (`None`: with the profile's).
pub fn set_next_storage(dir: Option<std::path::PathBuf>) {
    NEXT_STORAGE.with(|s| *s.borrow_mut() = dir);
}

wrap_cookie_visitor! {
    struct SessionCookies { rules: crate::cookies::CookiesConfig, store: String, mine: std::collections::HashSet<String>, ending: bool }

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&Cookie>, _count: ::std::os::raw::c_int, _total: ::std::os::raw::c_int, delete_cookie: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            if let (Some(c), Some(del)) = (cookie, delete_cookie) {
                let domain = c.domain.to_string();
                let key = crate::cookies::session_key(&self.store, "", &domain, &c.name.to_string(), &c.path.to_string());
                let url = format!("https://{}/", domain.trim_start_matches('.'));
                // Ending a window: its own session cookies. Otherwise (a window opening): session cookies that no
                // window owns, left by an earlier run.
                let doomed = if self.ending { self.mine.contains(&key) } else { self.rules.decide(&url) == crate::cookies::Action::Session && !self.mine.contains(&key) };
                if doomed {
                    *del = 1;
                }
            }
            1
        }
    }
}

/// Clean up session cookies in the folder's store (`None`: the profile's own), then write the store to disk.
/// `ending`: the window `owner` is closing, so delete the ones it saved. Otherwise a window is opening: delete
/// the ones `rules` call session that no window owns (an earlier run left them).
pub fn end_session_cookies(storage: Option<&std::path::Path>, rules: crate::cookies::CookiesConfig, owner: &str, ending: bool) {
    let store = storage.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mine: std::collections::HashSet<String> = {
        let mut all = SESSION_COOKIES.lock().unwrap();
        let map = all.get_or_insert_with(Default::default);
        if ending {
            map.remove(owner).unwrap_or_default()
        } else {
            map.values().flatten().cloned().collect()
        }
    };
    if (ending && mine.is_empty()) || (!ending && !rules.has_session_rules()) {
        return;
    }
    let rc = match storage {
        Some(dir) => CONTEXTS.with(|c| c.borrow().get(dir).cloned()),
        None => request_context_get_global_context(),
    };
    let Some(rc) = rc else { return };
    on_ui(move || {
        if let Some(manager) = rc.cookie_manager(None) {
            let mut visitor = SessionCookies::new(rules, store, mine, ending);
            manager.visit_all_cookies(Some(&mut visitor));
            manager.flush_store(None);
        }
    });
}

/// The CEF request context for a folder, made on first use. Its folder must lie inside the profile's CEF folder.
fn context_for(dir: &std::path::Path) -> Option<RequestContext> {
    CONTEXTS.with(|c| {
        if let Some(rc) = c.borrow().get(dir) {
            return Some(rc.clone());
        }
        let _ = std::fs::create_dir_all(dir);
        let settings = RequestContextSettings { cache_path: CefString::from(dir.to_string_lossy().as_ref()), persist_session_cookies: 1, ..Default::default() };
        let rc = request_context_create_context(Some(&settings), None)?;
        c.borrow_mut().insert(dir.to_path_buf(), rc.clone());
        Some(rc)
    })
}

impl Shared {
    /// Take the next DevTools message id and post `run` (which makes the call) to CEF's UI thread. CEF numbers calls by their ids
    /// and wants them to increase in the order they run, but calls are made from both threads (a sweep's probe from the GTK thread,
    /// the page hooks from CEF's): taking the id and posting under one lock keeps the two orders the same. (An id that goes
    /// backwards is renumbered by CEF, and the caller then reads another call's result.)
    fn post_cdp(&self, run: impl FnOnce(i32) + Send + 'static) -> i32 {
        let _order = self.cdp_order.lock().unwrap();
        let id = self.next_cdp_id.fetch_add(1, Ordering::Relaxed);
        on_ui(move || run(id));
        id
    }

    fn new(id: u64, device_resolution: bool, accel: bool, frame_rate: i32) -> Arc<Shared> {
        Arc::new(Shared {
            id,
            view: Mutex::new(View { size: (1280, 800), scale: std::env::var("BROWSAH_FORCE_SCALE").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0), text: crate::frames::text_scale(), backing: device_resolution }),
            browser: Mutex::new(None),
            host: Mutex::new(None),
            frames: AtomicU64::new(0),
            hidden: AtomicBool::new(false),
            pending_url: Mutex::new(None),
            cdp_results: Mutex::new(HashMap::new()),
            cdp_registration: Mutex::new(None),
            page_drag: Mutex::new(None),
            images: Mutex::new(HashMap::new()),
            image_data: Mutex::new(HashMap::new()),
            next_cdp_id: std::sync::atomic::AtomicI32::new(1),
            cdp_order: Mutex::new(()),
            deferred_url: Mutex::new(None),
            scope: Mutex::new(Vec::new()),
            workspace: Mutex::new(None),
            allowed_schemes: Mutex::new(Vec::new()),
            popup_policy: Mutex::new(crate::workspace::PopupPolicy::Tab),
            permissions: Mutex::new(crate::permissions::PermissionsConfig::default()),
            cookies: Mutex::new(crate::cookies::CookiesConfig::default()),
            user_zoom: Mutex::new(1.0),
            find_result: Mutex::new((0, 0)),
            scheme: Mutex::new(None),
            link_nav: AtomicBool::new(false),
            dev_tools: AtomicBool::new(false),
            session_owner: Mutex::new(String::new()),
            blocked_cookies: Mutex::new(Default::default()),
            storage: NEXT_STORAGE.with(|s| s.borrow().clone()),
            downloads: Mutex::new((gtk::glib::home_dir(), false)),
            accel,
            frame_rate,
        })
    }
}

/// A popup browser CEF is creating for a page (`window.open`) that keeps its opener. It is
/// built on CEF's thread; the shell turns it into a tab with `BrowserView::adopt`.
#[derive(Clone)]
pub struct PopupSpec {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for PopupSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PopupSpec(view {})", self.shared.id)
    }
}

/// GTK-thread-only state, looked up by view id from the closures sent across threads.
struct GtkSide {
    overlay: gtk::Overlay,
    picture: gtk::Picture,
    /// The `<select>` dropdown and similar popup, drawn above the page. It sits in a `GtkFixed`
    /// layer so it gets exactly the size and position CEF asks for (a bare picture would take
    /// its natural size: the texture's pixel size).
    popup: gtk::Picture,
    popup_layer: gtk::Fixed,
    display: gdk::Display,
    on_event: Rc<dyn Fn(ViewEvent)>,
    /// Called with every finished page frame (used by the latency measurement).
    frame_hook: Option<Rc<dyn Fn(&gdk::Texture)>>,
    cursor: String,
    /// Every cursor the page has asked for, in order (the self-test looks for one it expects,
    /// since the real pointer resting over the window can move the cursor on again).
    cursor_log: Vec<String>,
    tooltip: String,
    selection: String,
    popup_visible: bool,
}

thread_local! {
    static VIEWS: RefCell<HashMap<u64, GtkSide>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Self-test only: ignore pointer events from the real mouse. A pointer resting over the window
/// is re-delivered whenever the layout shifts (tab titles, spinners) and would race the signals
/// the test emits.
static SYNTHETIC_POINTER_ONLY: AtomicBool = AtomicBool::new(false);

pub fn set_synthetic_pointer_only(on: bool) {
    SYNTHETIC_POINTER_ONLY.store(on, Ordering::Relaxed);
}

fn ignore_pointer(current_event: Option<gdk::Event>) -> bool {
    current_event.is_some() && SYNTHETIC_POINTER_ONLY.load(Ordering::Relaxed)
}

/// GTK thread: show a finished frame.
fn present(id: u64, tex: gdk::Texture) {
    let hook = VIEWS.with(|v| {
        v.borrow().get(&id).and_then(|side| {
            side.picture.set_paintable(Some(&tex));
            side.picture.remove_css_class("dormant");
            side.frame_hook.clone()
        })
    });
    if let Some(hook) = hook {
        hook(&tex);
    }
}

/// GTK thread: show a finished popup frame.
fn present_popup(id: u64, tex: gdk::Texture) {
    VIEWS.with(|v| {
        if let Some(side) = v.borrow().get(&id) {
            side.popup.set_paintable(Some(&tex));
        }
    });
}

/// GTK thread: run `f` on the view's GTK-side state.
fn with_side<R>(id: u64, f: impl FnOnce(&mut GtkSide) -> R) -> Option<R> {
    VIEWS.with(|v| v.borrow_mut().get_mut(&id).map(f))
}

/// GTK thread: deliver an event to the shell. The callback runs without the registry borrowed,
/// so it may call back into the view.
fn deliver(id: u64, event: ViewEvent) {
    let cb = VIEWS.with(|v| v.borrow().get(&id).map(|s| s.on_event.clone()));
    if let Some(cb) = cb {
        cb(event);
    }
}

/// Any thread: queue an event for the GTK thread.
fn send(id: u64, event: ViewEvent) {
    glib::MainContext::default().invoke(move || deliver(id, event));
}

wrap_render_handler! {
    struct ViewRender { shared: Arc<Shared> }

    impl RenderHandler {
        fn view_rect(&self, _b: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (w, h) = self.shared.view.lock().unwrap().device_size();
            if let Some(rect) = rect {
                *rect = Rect { x: 0, y: 0, width: w, height: h };
            }
        }

        fn screen_info(&self, _b: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            let Some(info) = info else { return 0 };
            let v = *self.shared.view.lock().unwrap();
            let (w, h) = v.device_size();
            let r = Rect { x: 0, y: 0, width: w, height: h };
            // The real device scale factor (1 when rendering at 1x): CEF paints the view, which is logical, at it.
            info.device_scale_factor = v.dsf();
            info.rect = r.clone();
            info.available_rect = r;
            info.depth = 24;
            info.depth_per_component = 8;
            info.is_monochrome = 0;
            1
        }

        // A drag that starts in the page (a `draggable` element: a sortable list, a reorderable panel). Returning true takes the
        // drag: the pointer then drives CEF's drag calls until the button is released (`page_drag_move`, `page_drag_end`).
        fn start_dragging(&self, _b: Option<&mut Browser>, drag_data: Option<&mut DragData>, allowed_ops: DragOperationsMask, _x: ::std::os::raw::c_int, _y: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            let Some(data) = drag_data.and_then(|d| ImplDragData::clone(&*d)) else { return 0 };
            *self.shared.page_drag.lock().unwrap() = Some(PageDrag { data: SendBox(data), ops: SendBox(allowed_ops), entered: false, op: SendBox(drag_ops(0)), ending: false });
            1
        }

        // What the page would do with the drop where the pointer is (none, copy, move): remembered for the end of the drag,
        // and shown as the pointer.
        fn update_drag_cursor(&self, _b: Option<&mut Browser>, operation: DragOperationsMask) {
            let bits = operation.as_ref().0;
            if let Some(d) = self.shared.page_drag.lock().unwrap().as_mut() {
                d.op = SendBox(operation);
            }
            let name = match bits {
                0 => "no-drop",
                1 => "copy",
                _ => "grabbing",
            };
            let id = self.shared.id;
            glib::MainContext::default().invoke(move || {
                with_side(id, |side| side.overlay.set_cursor_from_name(Some(name)));
            });
        }

        // Readback path: the buffer is only valid during the call, so copy it.
        fn on_paint(
            &self,
            _b: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let popup = type_ == PaintElementType::POPUP;
            if (type_ != PaintElementType::VIEW && !popup) || buffer.is_null() {
                return;
            }
            if !popup {
                self.shared.frames.fetch_add(1, Ordering::Relaxed);
            }
            let id = self.shared.id;
            let len = width as usize * height as usize * 4;
            let bytes = glib::Bytes::from(unsafe { std::slice::from_raw_parts(buffer, len) });
            glib::MainContext::default().invoke(move || {
                let tex = gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, width as usize * 4);
                if popup {
                    present_popup(id, tex.upcast());
                } else {
                    present(id, tex.upcast());
                }
            });
        }

        // Accelerated path: only the dmabuf plane descriptors cross to the GTK thread.
        fn on_accelerated_paint(
            &self,
            _b: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else { return };
            let popup = type_ == PaintElementType::POPUP;
            if type_ != PaintElementType::VIEW && !popup {
                return;
            }
            if !popup {
                self.shared.frames.fetch_add(1, Ordering::Relaxed);
            }
            let id = self.shared.id;
            if let Some(frame) = DmabufFrame::from_info(info) {
                glib::MainContext::default().invoke(move || {
                    let display = VIEWS.with(|v| v.borrow().get(&id).map(|s| s.display.clone()));
                    if let Some(tex) = display.and_then(|d| import_frame(&d, frame)) {
                        if popup {
                            present_popup(id, tex);
                        } else {
                            present(id, tex);
                        }
                    }
                });
            }
        }

        // `<select>` dropdowns and similar are painted as a separate popup element.
        fn on_popup_show(&self, _b: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            let id = self.shared.id;
            let show = show != 0;
            glib::MainContext::default().invoke(move || {
                with_side(id, |side| {
                    side.popup_visible = show;
                    side.popup.set_visible(show);
                    if !show {
                        side.popup.set_paintable(None::<&gdk::Paintable>);
                    }
                });
            });
        }

        fn on_popup_size(&self, _b: Option<&mut Browser>, rect: Option<&Rect>) {
            let Some(rect) = rect else { return };
            let k = self.shared.view.lock().unwrap().k();
            // View pixels to logical pixels.
            let (x, y, w, h) = (
                (rect.x as f32 / k).round() as i32,
                (rect.y as f32 / k).round() as i32,
                (rect.width as f32 / k).round() as i32,
                (rect.height as f32 / k).round() as i32,
            );
            let id = self.shared.id;
            glib::MainContext::default().invoke(move || {
                with_side(id, |side| {
                    side.popup.set_size_request(w.max(1), h.max(1));
                    side.popup_layer.move_(&side.popup, x.max(0) as f64, y.max(0) as f64);
                });
            });
        }

        // Kept so the shell can offer the text to the clipboard (see `BrowserView::selection`).
        fn on_text_selection_changed(&self, _b: Option<&mut Browser>, selected_text: Option<&CefString>, _range: Option<&Range>) {
            let text = selected_text.map(|t| t.to_string()).unwrap_or_default();
            let id = self.shared.id;
            glib::MainContext::default().invoke(move || {
                with_side(id, |side| side.selection = text);
            });
        }
    }
}

wrap_life_span_handler! {
    struct ViewLife { shared: Arc<Shared> }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return };
            let Some(host) = browser.host() else { return };
            crate::cef_rt::browser_created();
            host.was_hidden(self.shared.hidden.load(Ordering::Relaxed) as _);
            // The browser's view first asks for the screen info before this browser exists (and so gets scale 1):
            // ask again now, on CEF's UI thread, so the scale we report takes effect.
            host.notify_screen_info_changed();
            host.was_resized();
            let mut observer = ViewDevTools::new(self.shared.clone());
            if let Some(reg) = host.add_dev_tools_message_observer(Some(&mut observer)) {
                *self.shared.cdp_registration.lock().unwrap() = Some(RegHandle(reg));
            }
            *self.shared.host.lock().unwrap() = Some(HostHandle(host));
            apply_scheme(&self.shared);
            *self.shared.browser.lock().unwrap() = Some(BrowserHandle(browser.clone()));
            if let Some(url) = self.shared.pending_url.lock().unwrap().take() {
                if let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&CefString::from(url.as_str())));
                }
            }
            send(self.shared.id, ViewEvent::Ready);
        }

        // A page wants a new window. A windowless browser has no window to give it, so create the
        // popup as another windowless browser (keeping `window.opener`, which login and payment
        // flows rely on) and let the shell show it as a tab. Only for a user gesture: pages that
        // open windows on their own (timers, load) are blocked.
        fn on_before_popup(
            &self,
            _b: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_js_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let (Some(window_info), Some(client), Some(settings)) = (window_info, client, settings) else { return 1 };
            if user_gesture == 0 {
                return 1;
            }
            // `target=_blank` links, and `window.open(url)` without window features, ask for a tab:
            // those are routed. A sized popup (`NEW_POPUP`, `NEW_WINDOW`) is the kind that sign-in and
            // payment pages rely on: it is never re-routed, so it keeps its opener.
            if matches!(disposition, WindowOpenDisposition::NEW_FOREGROUND_TAB | WindowOpenDisposition::NEW_BACKGROUND_TAB) {
                if let Some(url) = target_url.map(|u| u.to_string()).filter(|u| !u.is_empty()) {
                    if routed_elsewhere(&self.shared, &url, crate::router::Kind::Link, crate::router::Target::NewTab) {
                        return 1;
                    }
                }
            }
            if *self.shared.popup_policy.lock().unwrap() == crate::workspace::PopupPolicy::Block {
                return 1; // this workspace does not open popups
            }
            let opener = &self.shared;
            let backing = opener.view.lock().unwrap().backing;
            let shared = Shared::new(NEXT_ID.fetch_add(1, Ordering::Relaxed), backing, opener.accel, opener.frame_rate);
            *window_info = WindowInfo {
                windowless_rendering_enabled: 1,
                shared_texture_enabled: opener.accel as _,
                runtime_style: RuntimeStyle::from(sys::cef_runtime_style_t::CEF_RUNTIME_STYLE_ALLOY),
                ..Default::default()
            };
            settings.windowless_frame_rate = opener.frame_rate;
            *client = Some(make_client(shared.clone()));
            send(opener.id, ViewEvent::Popup(PopupSpec { shared }));
            0
        }

        // The browser is gone: drop its GTK-side state so late events find nothing.
        fn on_before_close(&self, _b: Option<&mut Browser>) {
            crate::cef_rt::browser_closed();
            let id = self.shared.id;
            glib::MainContext::default().invoke(move || {
                deliver(id, ViewEvent::Closed);
                VIEWS.with(|v| v.borrow_mut().remove(&id));
            });
        }
    }
}

wrap_load_handler! {
    struct ViewLoad { shared: Arc<Shared> }

    impl LoadHandler {
        // A page starts loading: forget what Chromium kept of earlier permission answers for its origin
        // (see `forget_in_chromium`), so this page's requests reach the permission handler.
        fn on_load_start(&self, b: Option<&mut Browser>, frame: Option<&mut Frame>, _transition: TransitionType) {
            if let (Some(b), Some(frame)) = (b, frame) {
                if frame.is_main() != 0 {
                    self.shared.link_nav.store(false, Ordering::Relaxed); // the navigation committed: its redirects are over
                    forget_in_chromium(b, &CefString::from(&frame.url()).to_string());
                }
                // Every frame, each time it starts loading: the page's copy buttons (see `COPY_HOOK`).
                frame.execute_java_script(Some(&CefString::from(COPY_HOOK)), Some(&CefString::from("browsah://copy-hook")), 0);
            }
        }

        fn on_loading_state_change(
            &self,
            _b: Option<&mut Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            send(
                self.shared.id,
                ViewEvent::Loading { loading: is_loading != 0, can_back: can_go_back != 0, can_forward: can_go_forward != 0 },
            );
        }
    }
}

wrap_display_handler! {
    struct ViewDisplay { shared: Arc<Shared> }

    impl DisplayHandler {
        // The copy hook's report (`COPY_HOOK`): the page's copy button wrote this text. Taken here, not logged.
        fn on_console_message(&self, _b: Option<&mut Browser>, _level: LogSeverity, message: Option<&CefString>, _source: Option<&CefString>, _line: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            let Some(text) = message.map(|m| m.to_string()) else { return 0 };
            match text.strip_prefix(COPY_PREFIX) {
                Some(copied) => {
                    send(self.shared.id, ViewEvent::ClipboardWrite(copied.to_owned()));
                    1
                }
                None => 0,
            }
        }

        // The page's icon addresses changed: fetch the first that downloads.
        fn on_favicon_urlchange(&self, b: Option<&mut Browser>, icon_urls: Option<&mut CefStringList>) {
            let Some(host) = b.and_then(|b| b.host()) else { return };
            let urls = icon_urls.map(string_list).unwrap_or_default();
            fetch_favicon(&self.shared, host, urls);
        }

        // A page asks to fill the screen (a video's fullscreen button, `requestFullscreen`) or is leaving it.
        fn on_fullscreen_mode_change(&self, _b: Option<&mut Browser>, fullscreen: ::std::os::raw::c_int) {
            send(self.shared.id, ViewEvent::Fullscreen(fullscreen != 0));
        }

        fn on_address_change(&self, _b: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            // Only the main frame's address is the page's address.
            if frame.is_some_and(|f| f.is_main() != 0) {
                if let Some(url) = url {
                    send(self.shared.id, ViewEvent::Address(url.to_string()));
                }
            }
        }

        fn on_title_change(&self, _b: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(title) = title {
                send(self.shared.id, ViewEvent::Title(title.to_string()));
            }
        }

        // The page's cursor (pointer, text, resize...): show the matching GTK cursor.
        fn on_cursor_change(
            &self,
            _b: Option<&mut Browser>,
            _cursor: ::std::os::raw::c_ulong,
            type_: CursorType,
            _custom: Option<&CursorInfo>,
        ) -> ::std::os::raw::c_int {
            let name = input::cursor_name(type_);
            let id = self.shared.id;
            glib::MainContext::default().invoke(move || {
                with_side(id, |side| {
                    side.cursor = name.to_owned();
                    side.cursor_log.push(name.to_owned());
                    side.overlay.set_cursor_from_name(Some(name));
                });
            });
            1
        }

        // Page tooltips (the `title` attribute): show them as GTK tooltips. Empty text hides.
        fn on_tooltip(&self, _b: Option<&mut Browser>, text: Option<&mut CefString>) -> ::std::os::raw::c_int {
            let text = text.map(|t| t.to_string()).unwrap_or_default();
            let id = self.shared.id;
            glib::MainContext::default().invoke(move || {
                with_side(id, |side| {
                    side.tooltip = text.clone();
                    side.overlay.set_tooltip_text(if text.is_empty() { None } else { Some(&text) });
                });
            });
            1
        }
    }
}

wrap_context_menu_handler! {
    struct ViewMenu { shared: Arc<Shared> }

    impl ContextMenuHandler {
        // Replace CEF's own (windowless: invisible) menu with ours, limited to the commands that
        // work here.
        fn run_context_menu(
            &self,
            _b: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(params), Some(model), Some(callback)) = (params, model, callback) else { return 0 };
            let mut items: Vec<MenuItem> = Vec::new();
            for i in 0..model.count() {
                let id = model.command_id_at(i);
                if model.type_at(i) == MenuItemType::SEPARATOR {
                    // No leading or doubled separators.
                    if items.last().is_some_and(|l| !l.separator) {
                        items.push(MenuItem { id: 0, label: String::new(), enabled: false, separator: true });
                    }
                } else if model.is_visible_at(i) != 0 && supported(id) {
                    let label = CefString::from(&model.label_at(i)).to_string().replace('&', "");
                    items.push(MenuItem { id, label, enabled: model.is_enabled_at(i) != 0, separator: false });
                }
            }
            while items.last().is_some_and(|l| l.separator) {
                items.pop();
            }
            if items.is_empty() {
                callback.cancel();
                return 1;
            }
            let k = self.shared.view.lock().unwrap().k() as f64;
            let req = Arc::new(MenuRequest {
                x: params.xcoord() as f64 / k,
                y: params.ycoord() as f64 / k,
                items,
                callback: Mutex::new(Some(MenuCallback(callback.clone()))),
            });
            send(self.shared.id, ViewEvent::ContextMenu(req));
            1
        }
    }
}

/// The session cookies each window has saved (by owner, then store/domain/name/path). A cookie a `session` rule
/// covers is sent only to the window that saved it, so one left behind by anything else (an earlier run, another
/// workspace) is never returned. They are deleted when the window closes.
static SESSION_COOKIES: Mutex<Option<HashMap<String, std::collections::HashSet<String>>>> = Mutex::new(None);

wrap_cookie_access_filter! {
    struct CookieRules { shared: Arc<Shared> }

    impl CookieAccessFilter {
        // Cookies sent with a request, and cookies a response asks to save: allowed or denied by the site's
        // address (`cookies:` in the configuration). `session` cookies belong to the window that saved them.
        fn can_send_cookie(&self, _b: Option<&mut Browser>, _f: Option<&mut Frame>, request: Option<&mut Request>, cookie: Option<&Cookie>) -> ::std::os::raw::c_int {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            match self.shared.cookies.lock().unwrap().decide(&url) {
                crate::cookies::Action::Allow => 1,
                crate::cookies::Action::Deny => {
                    self.shared.blocked_cookies.lock().unwrap().insert(crate::cookies::host_of(&url));
                    0
                }
                crate::cookies::Action::Session => {
                    let Some(c) = cookie else { return 0 };
                    let (owner, key) = (self.shared.session_owner.lock().unwrap().clone(), self.session_key(&url, c));
                    SESSION_COOKIES.lock().unwrap().as_ref().and_then(|m| m.get(&owner)).is_some_and(|set| set.contains(&key)) as _
                }
            }
        }

        fn can_save_cookie(&self, _b: Option<&mut Browser>, _f: Option<&mut Frame>, request: Option<&mut Request>, _resp: Option<&mut Response>, cookie: Option<&Cookie>) -> ::std::os::raw::c_int {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            match self.shared.cookies.lock().unwrap().decide(&url) {
                crate::cookies::Action::Allow => 1,
                crate::cookies::Action::Deny => {
                    self.shared.blocked_cookies.lock().unwrap().insert(crate::cookies::host_of(&url));
                    0
                }
                crate::cookies::Action::Session => {
                    if let Some(c) = cookie {
                        let (owner, key) = (self.shared.session_owner.lock().unwrap().clone(), self.session_key(&url, c));
                        SESSION_COOKIES.lock().unwrap().get_or_insert_with(Default::default).entry(owner).or_default().insert(key);
                    }
                    1
                }
            }
        }
    }
}

impl CookieRules {
    fn session_key(&self, url: &str, c: &Cookie) -> String {
        let store = self.shared.storage.as_deref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        crate::cookies::session_key(&store, url, &c.domain.to_string(), &c.name.to_string(), &c.path.to_string())
    }
}

wrap_resource_request_handler! {
    struct ViewResources { shared: Arc<Shared> }

    impl ResourceRequestHandler {
        fn cookie_access_filter(&self, _b: Option<&mut Browser>, _f: Option<&mut Frame>, _r: Option<&mut Request>) -> Option<CookieAccessFilter> {
            // Nothing to filter unless the configuration has cookie rules.
            if self.shared.cookies.lock().unwrap().is_open() {
                return None;
            }
            Some(CookieRules::new(self.shared.clone()))
        }

        // A page (a redirect at the end of a sign-in, say) navigates to an address with a scheme
        // browsah does not handle. Hand it to the desktop only if this view was told that scheme is
        // expected (`BrowserView::allow_scheme`, set for the callback of a sign-in another application
        // started); anything else stays blocked.
        fn on_protocol_execution(
            &self,
            _b: Option<&mut Browser>,
            _f: Option<&mut Frame>,
            request: Option<&mut Request>,
            allow_os_execution: Option<&mut ::std::os::raw::c_int>,
        ) {
            let (Some(request), Some(allow)) = (request, allow_os_execution) else { return };
            let url = CefString::from(&request.url()).to_string();
            let scheme = url.split_once(':').map(|(s, _)| s.to_lowercase()).unwrap_or_default();
            let from_env = std::env::var("BROWSAH_ALLOW_SCHEMES").unwrap_or_default();
            let signin = self.shared.allowed_schemes.lock().unwrap().iter().any(|s| *s == scheme);
            let expected = signin || from_env.split(',').any(|s| !s.is_empty() && s == scheme);
            // Chromium's own launching needs a prompt that does not exist here, so the shell does it.
            *allow = 0;
            if expected {
                send(self.shared.id, ViewEvent::ExternalLaunch { scheme, url, signin });
            }
        }

        // The default would be "cancel" (a zero return value), which fails every request.
        fn on_before_resource_load(&self, _b: Option<&mut Browser>, _f: Option<&mut Frame>, _r: Option<&mut Request>, _cb: Option<&mut Callback>) -> ReturnValue {
            ReturnValue::from(sys::cef_return_value_t::RV_CONTINUE)
        }
    }
}

/// Ask the router about a link from a view. If it says anything but "carry on", report the
/// decision to the shell and return true: the navigation must be cancelled.
fn routed_elsewhere(shared: &Shared, url: &str, kind: crate::router::Kind, target: crate::router::Target) -> bool {
    use crate::router::{route, Action, Request, Source};
    let source = Source::Tab(shared.workspace.lock().unwrap().clone());
    let ws = if let Source::Tab(w) = &source { w.clone() } else { None };
    // A redirector (`google.com/url?q=...`) is judged, and opened, as the address it leads to.
    let url = crate::router::unwrap_url(url);
    let decision = route(&Request { url: url.clone(), source, kind, target });
    match decision.action {
        Action::Allow => false,
        // Already where it is told to go.
        Action::Workspace(ref w) if ws.as_deref() == Some(w.as_str()) => false,
        action => {
            send(shared.id, ViewEvent::Route { action, url });
            true
        }
    }
}

wrap_request_handler! {
    struct ViewRequest { shared: Arc<Shared> }

    impl RequestHandler {
        fn on_render_process_terminated(
            &self,
            _b: Option<&mut Browser>,
            status: TerminationStatus,
            _error_code: ::std::os::raw::c_int,
            error_string: Option<&CefString>,
        ) {
            let why = error_string.map(|e| e.to_string()).filter(|e| !e.is_empty()).unwrap_or_else(|| format!("{status:?}"));
            send(self.shared.id, ViewEvent::Crashed(why));
        }

        // A link opened in a new tab (`target=_blank`, middle click, Ctrl+click): ask the router
        // before CEF creates the tab. Script popups (`window.open`) do not come here.
        fn on_open_urlfrom_tab(
            &self,
            _b: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            target_url: Option<&CefString>,
            disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(url) = target_url.map(|u| u.to_string()).filter(|_| user_gesture != 0) else { return 0 };
            if routed_elsewhere(&self.shared, &url, crate::router::Kind::Link, crate::router::Target::NewTab) {
                return 1;
            }
            // Returning false would navigate the clicked tab itself, so a link that asks for a new
            // tab (Ctrl+click, middle click) is opened as one here.
            let select = match disposition {
                WindowOpenDisposition::NEW_FOREGROUND_TAB => true,
                WindowOpenDisposition::NEW_BACKGROUND_TAB => false,
                _ => return 0,
            };
            if *self.shared.popup_policy.lock().unwrap() == crate::workspace::PopupPolicy::Block {
                return 1;
            }
            send(self.shared.id, ViewEvent::OpenTab { url: crate::router::unwrap_url(&url), select });
            1
        }

        // A pinned tab limited to a scope: a navigation from inside the scope to somewhere outside
        // it is cancelled and reported, to be opened as another tab. Not restricted: redirects, and
        // pages that are already outside the scope (reached by a redirect, as in a sign-in flow).
        fn on_before_browse(
            &self,
            _b: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            user_gesture: ::std::os::raw::c_int,
            is_redirect: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let (Some(frame), Some(request)) = (frame, request) else { return 0 };
            if frame.is_main() == 0 {
                return 0;
            }
            if is_redirect != 0 {
                // A hop of the redirects of a clicked link: with `routing.follow_redirects`, judge where it leads (a tracking or
                // shortening address that sends you to a site a rule claims).
                if self.shared.link_nav.load(Ordering::Relaxed) && crate::router::follow_redirects() {
                    let hop = CefString::from(&request.url()).to_string();
                    if routed_elsewhere(&self.shared, &hop, crate::router::Kind::Link, crate::router::Target::SameTab) {
                        return 1;
                    }
                }
                return 0;
            }
            self.shared.link_nav.store(false, Ordering::Relaxed);
            self.shared.blocked_cookies.lock().unwrap().clear(); // a new page: its blocked cookies start afresh
            let target = CefString::from(&request.url()).to_string();
            // An extension's own pages (its popup, an options page) are never routed: they belong to the extension.
            if target.starts_with("chrome-extension://") {
                return 0;
            }
            let current = CefString::from(&frame.url()).to_string();
            // The router: a clicked link (a user gesture on a GET) or any other navigation of a
            // page that is already showing something. A tab's own first load is never routed.
            if !current.is_empty() && current != "about:blank" && target != "about:blank" {
                let method = CefString::from(&request.method()).to_string();
                let kind = if user_gesture != 0 && method.eq_ignore_ascii_case("GET") {
                    Some(crate::router::Kind::Link)
                } else if user_gesture == 0 {
                    Some(crate::router::Kind::Typed)
                } else {
                    None // a form post: not routed
                };
                if let Some(kind) = kind {
                    if routed_elsewhere(&self.shared, &target, kind, crate::router::Target::SameTab) {
                        return 1;
                    }
                    // Only a clicked link has its redirects followed (not typed addresses, scripts or forms).
                    self.shared.link_nav.store(kind == crate::router::Kind::Link, Ordering::Relaxed);
                }
            }
            let scope = self.shared.scope.lock().unwrap().clone();
            if scope.is_empty() {
                return 0;
            }
            let outside = |u: &str| !crate::workspace::in_scope(&scope, u);
            if target == "about:blank" || !outside(&target) {
                return 0;
            }
            if current.is_empty() || current == "about:blank" || outside(&current) {
                return 0; // the first load, or a detour already under way
            }
            send(self.shared.id, ViewEvent::OutOfScope(target));
            1
        }

        fn resource_request_handler(
            &self,
            _b: Option<&mut Browser>,
            _f: Option<&mut Frame>,
            _r: Option<&mut Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            _initiator: Option<&CefString>,
            _disable_default: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            Some(ViewResources::new(self.shared.clone()))
        }
    }
}

// ---- permissions ------------------------------------------------------------------------------------

struct MediaCb(MediaAccessCallback);
unsafe impl Send for MediaCb {}
struct PromptCb(PermissionPromptCallback);
unsafe impl Send for PromptCb {}

enum Responder {
    Media(MediaCb),
    Prompt(PromptCb),
}

/// A page's request for permissions, with what the configuration already decided and the kinds that need
/// asking. Answer it exactly once (`answer`); dropping it unanswered leaves the page waiting.
pub struct PermissionRequest {
    /// Chromium's prompt id (0 for media requests).
    pub id: u64,
    pub origin: String,
    /// Kinds the configuration says to ask about.
    pub ask: Vec<crate::permissions::Kind>,
    /// Kinds already decided: allowed or not.
    pub decided: Vec<(crate::permissions::Kind, bool)>,
    responder: Mutex<Option<Responder>>,
}

impl std::fmt::Debug for PermissionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PermissionRequest({} for {:?}, asking {:?})", self.origin, self.decided, self.ask)
    }
}

/// How many permission requests the handler has seen (the self-test).
pub static PERMISSION_REQUESTS: AtomicU64 = AtomicU64::new(0);

/// The Chromium content setting a kind is stored under (for permissions asked through a prompt).
fn content_setting_type(kind: crate::permissions::Kind) -> Option<ContentSettingTypes> {
    use sys::cef_content_setting_types_t as T;
    let t = match kind {
        crate::permissions::Kind::Location => T::CEF_CONTENT_SETTING_TYPE_GEOLOCATION,
        crate::permissions::Kind::Notifications => T::CEF_CONTENT_SETTING_TYPE_NOTIFICATIONS,
        crate::permissions::Kind::Microphone => T::CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_MIC,
        crate::permissions::Kind::Camera => T::CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_CAMERA,
        crate::permissions::Kind::Midi => T::CEF_CONTENT_SETTING_TYPE_MIDI_SYSEX,
        crate::permissions::Kind::Clipboard => T::CEF_CONTENT_SETTING_TYPE_CLIPBOARD_READ_WRITE,
        // Screen capture goes through the media-access path and is not stored; asking Chromium to change
        // its content setting crashes (measured), so it is left out.
        crate::permissions::Kind::Screen | crate::permissions::Kind::Other => return None,
    };
    Some(ContentSettingTypes::from(t))
}

/// Chromium keeps the answer it is given to a permission prompt and then answers the same origin itself,
/// without asking the handler again. That would make a later `deny` in the configuration, a forgotten
/// answer, or "allow but do not remember" ineffective. So what it kept is cleared (back to its default)
/// whenever a page of the origin starts loading: a permission lasts as long as the page it was given to
/// (clearing it at once instead makes Chromium refuse a request it is still working on), and the next
/// load asks the handler again.
fn forget_in_chromium(browser: &Browser, url: &str) {
    let origin = match url.split_once("://") {
        Some((scheme @ ("http" | "https"), rest)) => format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or("")),
        _ => return,
    };
    let Some(rc) = browser.host().and_then(|h| h.request_context()) else { return };
    let url = CefString::from(origin.as_str());
    for k in crate::permissions::Kind::ALL {
        if let Some(t) = content_setting_type(k) {
            rc.set_content_setting(Some(&url), Some(&url), t, ContentSettingValues::from(sys::cef_content_setting_values_t::CEF_CONTENT_SETTING_VALUE_DEFAULT));
        }
    }
}

/// Reply to Chromium: `allowed` is, for each kind, whether it may be used.
fn respond(responder: Responder, allowed: Vec<(crate::permissions::Kind, bool)>) {
    on_ui(move || match responder {
        Responder::Media(cb) => {
            let cb = cb;
            let mask: u32 = allowed.iter().filter(|(_, ok)| *ok).map(|(k, _)| k.media_bits()).fold(0, |a, b| a | b);
            if mask == 0 {
                cb.0.cancel();
            } else {
                cb.0.cont(mask);
            }
        }
        Responder::Prompt(cb) => {
            let cb = cb;
            let all = allowed.iter().all(|(_, ok)| *ok);
            cb.0.cont(PermissionRequestResult::from(if all { sys::cef_permission_request_result_t::CEF_PERMISSION_RESULT_ACCEPT } else { sys::cef_permission_request_result_t::CEF_PERMISSION_RESULT_DENY }));
        }
    });
}

impl PermissionRequest {
    /// Answer for the kinds that were asked: `ask_allowed` for each (in the order of `ask`).
    pub fn answer(&self, ask_allowed: &[bool]) {
        let Some(responder) = self.responder.lock().unwrap().take() else { return };
        let mut all = self.decided.clone();
        all.extend(self.ask.iter().copied().zip(ask_allowed.iter().copied()));
        respond(responder, all);
    }
}

/// Judge a request by the configuration. Anything that needs asking is passed to the shell.
fn handle_permission(shared: &Shared, browser: Option<&mut Browser>, id: u64, origin: String, kinds: Vec<crate::permissions::Kind>, responder: Responder) {
    use crate::permissions::Choice;
    // Chromium gives an origin with a trailing slash; the configuration and the remembered answers do not use one.
    let origin = origin.trim_end_matches('/').to_owned();
    PERMISSION_REQUESTS.fetch_add(1, Ordering::Relaxed);
    let cfg = shared.permissions.lock().unwrap().clone();
    if std::env::var_os("BROWSAH_DEBUG").is_some() {
        eprintln!("browsah: permission request from {origin} for {kinds:?}");
    }
    let (mut decided, mut ask) = (Vec::new(), Vec::new());
    for k in kinds {
        match cfg.decide(&origin, k) {
            Choice::Allow => decided.push((k, true)),
            Choice::Deny => decided.push((k, false)),
            Choice::Ask => ask.push(k),
        }
    }
    let _ = browser;
    if ask.is_empty() {
        respond(responder, decided);
    } else {
        let req = Arc::new(PermissionRequest { id, origin, ask, decided, responder: Mutex::new(Some(responder)) });
        send(shared.id, ViewEvent::Permission(req));
    }
}

wrap_permission_handler! {
    struct ViewPermissions { shared: Arc<Shared> }

    impl PermissionHandler {
        // Camera, microphone and screen capture.
        fn on_request_media_access_permission(
            &self,
            b: Option<&mut Browser>,
            _f: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(origin), Some(callback)) = (requesting_origin, callback) else { return 0 };
            let mut kinds = crate::permissions::Kind::from_media_mask(requested_permissions);
            if kinds.is_empty() {
                kinds.push(crate::permissions::Kind::Other);
            }
            handle_permission(&self.shared, None, 0, origin.to_string(), kinds, Responder::Media(MediaCb(callback.clone()))); // (media access is not stored by Chromium)
            let _ = b;
            1
        }

        // Everything else that asks: location, notifications, clipboard, MIDI...
        fn on_show_permission_prompt(
            &self,
            b: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(origin), Some(callback)) = (requesting_origin, callback) else { return 0 };
            let kinds = crate::permissions::Kind::from_prompt_mask(requested_permissions);
            handle_permission(&self.shared, b, prompt_id, origin.to_string(), kinds, Responder::Prompt(PromptCb(callback.clone())));
            1
        }

        fn on_dismiss_permission_prompt(&self, _b: Option<&mut Browser>, prompt_id: u64, _result: PermissionRequestResult) {
            send(self.shared.id, ViewEvent::PermissionDismissed(prompt_id));
        }
    }
}

// ---- colour scheme -------------------------------------------------------------------------------

/// Tell the page's browser which colour scheme to report (`Emulation.setEmulatedMedia`, per browser). Chromium's own idea of
/// the desktop's scheme does not follow the desktop here (it reports light whatever the portal says), so browsah always says.
fn apply_scheme(shared: &Arc<Shared>) {
    let Some(dark) = *shared.scheme.lock().unwrap() else { return };
    let Some(h) = shared.host.lock().unwrap().as_ref().map(|h| HostHandle(h.0.clone())) else { return };
    shared.post_cdp(move |id| {
        let h = h;
        let json = format!(r#"{{"features":[{{"name":"prefers-color-scheme","value":"{}"}}]}}"#, if dark { "dark" } else { "light" });
        let Some(mut params) = parse_json(Some(&CefString::from(json.as_str())), JsonParserOptions::RFC).and_then(|v| v.dictionary()) else { return };
        h.0.execute_dev_tools_method(id, Some(&CefString::from("Emulation.setEmulatedMedia")), Some(&mut params));
    });
}

// ---- PDF -----------------------------------------------------------------------------------------

wrap_pdf_print_callback! {
    struct ViewPdf { shared: Arc<Shared> }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(&self, path: Option<&CefString>, ok: ::std::os::raw::c_int) {
            send(self.shared.id, ViewEvent::PdfSaved { path: path.map(|p| p.to_string()).unwrap_or_default(), ok: ok != 0 });
        }
    }
}

// ---- favicons ------------------------------------------------------------------------------------

struct SendBox<T>(T);
unsafe impl<T> Send for SendBox<T> {}

/// A drag that began in the page and is still running.
struct PageDrag {
    data: SendBox<DragData>,
    ops: SendBox<DragOperationsMask>,
    /// CEF has been told the drag entered the page (on the first pointer move).
    entered: bool,
    /// The operation the page last answered with (copy, move, none), from `update_drag_cursor`.
    op: SendBox<DragOperationsMask>,
    /// The button was released or the drag cancelled: the end is queued on CEF's thread and the pointer is ignored meanwhile.
    ending: bool,
}

fn drag_ops(bits: u32) -> DragOperationsMask {
    DragOperationsMask::from(sys::cef_drag_operations_mask_t(bits))
}

/// Run CEF's drag-target calls on its UI thread (they are ignored elsewhere).
fn drag_on_ui(shared: &Shared, f: impl FnOnce(&BrowserHost) + 'static) {
    if let Some(h) = shared.host.lock().unwrap().as_ref() {
        let h = SendBox((HostHandle(h.0.clone()), f));
        on_ui(move || {
            let h = h;
            let (host, f) = h.0;
            f(&host.0);
        });
    }
}

/// What a GTK drop carries, as CEF's drag data: files by path, text as a fragment (and a link when it is one address).
fn drag_data_from(value: &glib::Value) -> Option<DragData> {
    let data = drag_data_create()?;
    if let Ok(list) = value.get::<gdk::FileList>() {
        let mut any = false;
        for f in list.files() {
            if let Some(path) = f.path() {
                let path = path.to_string_lossy().into_owned();
                let name = f.basename().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                data.add_file(Some(&CefString::from(path.as_str())), Some(&CefString::from(name.as_str())));
                any = true;
            }
        }
        return any.then_some(data);
    }
    let text = value.get::<String>().ok()?;
    let text = text.trim_end_matches(['\r', '\n']);
    if (text.starts_with("http://") || text.starts_with("https://")) && !text.contains(char::is_whitespace) {
        data.set_link_url(Some(&CefString::from(text)));
    }
    data.set_fragment_text(Some(&CefString::from(text)));
    Some(data)
}

/// The strings of a CEF string list.
fn string_list(list: &mut CefStringList) -> Vec<String> {
    let raw: *mut sys::_cef_string_list_t = list.into();
    let mut out = Vec::new();
    unsafe {
        for i in 0..sys::cef_string_list_size(raw) {
            let mut value: sys::cef_string_t = std::mem::zeroed();
            if sys::cef_string_list_value(raw, i, &mut value) != 0 {
                out.push(CefString::from(&value as *const sys::cef_string_t).to_string());
                sys::cef_string_utf16_clear(&mut value);
            }
        }
    }
    out
}

wrap_download_image_callback! {
    struct ViewFavicon { shared: Arc<Shared>, rest: Vec<String> }

    impl DownloadImageCallback {
        fn on_download_image_finished(&self, _url: Option<&CefString>, status: ::std::os::raw::c_int, image: Option<&mut Image>) {
            let (mut pw, mut ph) = (0, 0);
            let png = (status == 200).then_some(()).and(image.map(|i| &*i)).and_then(|i| i.as_png(1.0, 1, Some(&mut pw), Some(&mut ph)));
            let bytes = png.map(|png| {
                let mut buf = vec![0u8; png.size()];
                let n = png.data(Some(&mut buf), 0);
                buf.truncate(n);
                buf
            });
            match bytes {
                Some(b) if !b.is_empty() => send(self.shared.id, ViewEvent::Favicon(b)),
                // That one did not work: try the next address, or say there is no icon.
                _ if !self.rest.is_empty() => {
                    let host = self.shared.host.lock().unwrap().as_ref().map(|h| HostHandle(h.0.clone()));
                    if let Some(h) = host {
                        fetch_favicon(&self.shared, h.0.clone(), self.rest.clone());
                    }
                }
                _ => send(self.shared.id, ViewEvent::Favicon(Vec::new())),
            }
        }
    }
}

/// Download the first icon of `urls` (on CEF's UI thread); the rest are tried if it fails.
fn fetch_favicon(shared: &Arc<Shared>, host: BrowserHost, mut urls: Vec<String>) {
    urls.retain(|u| u.starts_with("http://") || u.starts_with("https://") || u.starts_with("data:"));
    if urls.is_empty() {
        send(shared.id, ViewEvent::Favicon(Vec::new()));
        return;
    }
    let first = urls.remove(0);
    let mut cb = ViewFavicon::new(shared.clone(), urls);
    // 64 px: a tab's icon is 16 logical pixels, up to 4x on a scale-4 display.
    host.download_image(Some(&CefString::from(first.as_str())), 1, 64, 0, Some(&mut cb));
}

// ---- find in page ---------------------------------------------------------------------------------

wrap_find_handler! {
    struct ViewFind { shared: Arc<Shared> }

    impl FindHandler {
        fn on_find_result(&self, _b: Option<&mut Browser>, _id: ::std::os::raw::c_int, count: ::std::os::raw::c_int, _rect: Option<&Rect>, active: ::std::os::raw::c_int, _final: ::std::os::raw::c_int) {
            *self.shared.find_result.lock().unwrap() = (count, active);
            send(self.shared.id, ViewEvent::FindResult { count, active });
        }
    }
}

// ---- JavaScript dialogs ---------------------------------------------------------------------------

struct JsCb(JsdialogCallback);
unsafe impl Send for JsCb {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsDialogKind {
    Alert,
    Confirm,
    Prompt,
    /// "Leave site?": the page asks before it is left (`reload`: by reloading).
    BeforeUnload { reload: bool },
}

/// A dialog a page opened with script. Answer it exactly once (`answer`); the page waits for it.
pub struct JsDialogQuestion {
    /// The view it came from.
    pub view: u64,
    pub kind: JsDialogKind,
    pub origin: String,
    pub message: String,
    pub default_text: String,
    responder: Mutex<Option<JsCb>>,
}

impl std::fmt::Debug for JsDialogQuestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JsDialogQuestion({:?} from {}: {:?})", self.kind, self.origin, self.message)
    }
}

impl JsDialogQuestion {
    /// `ok`: OK / Leave was chosen; `text` is what was typed into a prompt.
    pub fn answer(&self, ok: bool, text: &str) {
        let Some(cb) = self.responder.lock().unwrap().take() else { return };
        if !ok && matches!(self.kind, JsDialogKind::BeforeUnload { .. }) {
            send(self.view, ViewEvent::CloseCancelled);
        }
        let text = text.to_owned();
        on_ui(move || {
            let cb = cb;
            cb.0.cont(ok as _, Some(&CefString::from(text.as_str())));
        });
    }
}

wrap_jsdialog_handler! {
    struct ViewJsDialog { shared: Arc<Shared> }

    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            _b: Option<&mut Browser>,
            origin_url: Option<&CefString>,
            dialog_type: JsdialogType,
            message_text: Option<&CefString>,
            default_prompt_text: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>,
            _suppress: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let kind = if dialog_type == JsdialogType::CONFIRM {
                JsDialogKind::Confirm
            } else if dialog_type == JsdialogType::PROMPT {
                JsDialogKind::Prompt
            } else {
                JsDialogKind::Alert
            };
            let q = Arc::new(JsDialogQuestion {
                view: self.shared.id,
                kind,
                origin: origin_url.map(|u| u.to_string()).unwrap_or_default(),
                message: message_text.map(|m| m.to_string()).unwrap_or_default(),
                default_text: default_prompt_text.map(|m| m.to_string()).unwrap_or_default(),
                responder: Mutex::new(Some(JsCb(callback.clone()))),
            });
            send(self.shared.id, ViewEvent::JsDialog(q));
            1
        }

        fn on_before_unload_dialog(&self, _b: Option<&mut Browser>, message_text: Option<&CefString>, is_reload: ::std::os::raw::c_int, callback: Option<&mut JsdialogCallback>) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let q = Arc::new(JsDialogQuestion {
                view: self.shared.id,
                kind: JsDialogKind::BeforeUnload { reload: is_reload != 0 },
                origin: String::new(),
                message: message_text.map(|m| m.to_string()).unwrap_or_default(),
                default_text: String::new(),
                responder: Mutex::new(Some(JsCb(callback.clone()))),
            });
            send(self.shared.id, ViewEvent::JsDialog(q));
            1
        }

        fn on_reset_dialog_state(&self, _b: Option<&mut Browser>) {
            send(self.shared.id, ViewEvent::JsDialogDismissed(self.shared.id));
        }
    }
}

// ---- file pickers -----------------------------------------------------------------------------------
struct FileCb(FileDialogCallback);
unsafe impl Send for FileCb {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogKind {
    Open,
    OpenMultiple,
    Folder,
    Save,
}

/// A page's request for files. Answer it exactly once (`choose` or `cancel`); the page waits for it.
pub struct FileDialogQuestion {
    pub kind: FileDialogKind,
    pub title: String,
    /// A suggested file or folder.
    pub default_path: String,
    /// What the page accepts: MIME types (`image/*`) and extensions (`.png`).
    pub accept: Vec<String>,
    responder: Mutex<Option<FileCb>>,
}

impl std::fmt::Debug for FileDialogQuestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileDialogQuestion({:?}: {:?})", self.kind, self.title)
    }
}

impl FileDialogQuestion {
    pub fn choose(&self, paths: Vec<String>) {
        let Some(cb) = self.responder.lock().unwrap().take() else { return };
        on_ui(move || {
            let cb = cb;
            let mut list = CefStringList::new();
            for p in &paths {
                list.append(p);
            }
            cb.0.cont(Some(&mut list));
        });
    }

    pub fn cancel(&self) {
        let Some(cb) = self.responder.lock().unwrap().take() else { return };
        on_ui(move || {
            let cb = cb;
            cb.0.cancel();
        });
    }
}

impl Drop for FileDialogQuestion {
    fn drop(&mut self) {
        // A question nobody answered must not leave the page waiting.
        self.cancel();
    }
}

wrap_dialog_handler! {
    struct ViewFileDialog { shared: Arc<Shared> }

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            _b: Option<&mut Browser>,
            mode: FileDialogMode,
            title: Option<&CefString>,
            default_file_path: Option<&CefString>,
            accept_filters: Option<&mut CefStringList>,
            accept_extensions: Option<&mut CefStringList>,
            _accept_descriptions: Option<&mut CefStringList>,
            callback: Option<&mut FileDialogCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let mode: sys::cef_file_dialog_mode_t = mode.into();
            let kind = match mode {
                sys::cef_file_dialog_mode_t::FILE_DIALOG_OPEN_MULTIPLE => FileDialogKind::OpenMultiple,
                sys::cef_file_dialog_mode_t::FILE_DIALOG_OPEN_FOLDER => FileDialogKind::Folder,
                sys::cef_file_dialog_mode_t::FILE_DIALOG_SAVE => FileDialogKind::Save,
                _ => FileDialogKind::Open,
            };
            let mut accept = accept_filters.map(string_list).unwrap_or_default();
            accept.extend(accept_extensions.map(string_list).unwrap_or_default());
            accept.retain(|a| !a.is_empty());
            let q = Arc::new(FileDialogQuestion {
                kind,
                title: title.map(|t| t.to_string()).unwrap_or_default(),
                default_path: default_file_path.map(|t| t.to_string()).unwrap_or_default(),
                accept,
                responder: Mutex::new(Some(FileCb(callback.clone()))),
            });
            send(self.shared.id, ViewEvent::FileDialog(q));
            1
        }
    }
}

// ---- downloads --------------------------------------------------------------------------------------

struct DownloadCb(BeforeDownloadCallback);
unsafe impl Send for DownloadCb {}

#[derive(Debug, Clone)]
pub enum DownloadState {
    /// Percent complete (-1 when the size is not known) and bytes received so far.
    Progress { percent: i32, received: i64 },
    Done { path: std::path::PathBuf, bytes: i64 },
    Canceled,
    /// Stopped by an error (the connection dropped, the disk is full...).
    Failed,
}

/// What stops a download (the item's callback, used from any thread: the call is posted to CEF's UI thread).
pub struct DownloadCancel(Mutex<DownloadItemCb>);

struct DownloadItemCb(DownloadItemCallback);
unsafe impl Send for DownloadItemCb {}

impl std::fmt::Debug for DownloadCancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DownloadCancel")
    }
}

impl DownloadCancel {
    pub fn cancel(&self) {
        let cb = DownloadItemCb(self.0.lock().unwrap().0.clone());
        on_ui(move || {
            let cb = cb;
            cb.0.cancel();
        });
    }
}

/// A file about to be downloaded that is waiting for the person to choose where it goes.
pub struct DownloadQuestion {
    pub suggested: String,
    pub dir: std::path::PathBuf,
    responder: Mutex<Option<DownloadCb>>,
}

impl std::fmt::Debug for DownloadQuestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DownloadQuestion({:?})", self.suggested)
    }
}

impl DownloadQuestion {
    /// Save to `path`, or (None) cancel the download.
    pub fn answer(&self, path: Option<std::path::PathBuf>) {
        let Some(cb) = self.responder.lock().unwrap().take() else { return };
        match path {
            Some(p) => {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                on_ui(move || {
                    let cb = cb;
                    cb.0.cont(Some(&CefString::from(p.to_string_lossy().as_ref())), 0);
                });
            }
            // A download whose callback is dropped without continuing is cancelled.
            None => drop(cb),
        }
    }
}

wrap_download_handler! {
    struct ViewDownloads { shared: Arc<Shared> }

    impl DownloadHandler {
        fn can_download(&self, _b: Option<&mut Browser>, _url: Option<&CefString>, _method: Option<&CefString>) -> ::std::os::raw::c_int {
            1
        }

        fn on_before_download(
            &self,
            _b: Option<&mut Browser>,
            _item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let name = suggested_name.map(|n| n.to_string()).unwrap_or_default();
            let (dir, ask) = self.shared.downloads.lock().unwrap().clone();
            if ask {
                let q = Arc::new(DownloadQuestion { suggested: crate::downloads::sanitise_name(&name), dir, responder: Mutex::new(Some(DownloadCb(callback.clone()))) });
                send(self.shared.id, ViewEvent::DownloadAsk(q));
            } else {
                let _ = std::fs::create_dir_all(&dir);
                let path = crate::downloads::unique_path(&dir, &name, |p| p.exists());
                callback.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            }
            1
        }

        fn on_download_updated(&self, _b: Option<&mut Browser>, item: Option<&mut DownloadItem>, cb: Option<&mut DownloadItemCallback>) {
            let Some(item) = item else { return };
            let cancel = cb.map(|c| Arc::new(DownloadCancel(Mutex::new(DownloadItemCb(c.clone())))));
            let url = CefString::from(&item.url()).to_string();
            let path = std::path::PathBuf::from(CefString::from(&item.full_path()).to_string());
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let name = if name.is_empty() { CefString::from(&item.suggested_file_name()).to_string().rsplit(['/', '\\']).next().unwrap_or_default().to_owned() } else { name };
            let state = if item.is_canceled() != 0 {
                DownloadState::Canceled
            } else if item.is_interrupted() != 0 {
                DownloadState::Failed
            } else if item.is_complete() != 0 {
                DownloadState::Done { path, bytes: item.received_bytes() }
            } else {
                DownloadState::Progress { percent: item.percent_complete(), received: item.received_bytes() }
            };
            send(self.shared.id, ViewEvent::Download { id: item.id(), name, url, state, cancel });
        }
    }
}

wrap_client! {
    struct ViewClient {
        render: RenderHandler,
        life: LifeSpanHandler,
        load: LoadHandler,
        display: DisplayHandler,
        menu: ContextMenuHandler,
        request: RequestHandler,
        permission: PermissionHandler,
        download: DownloadHandler,
        jsdialog: JsdialogHandler,
        file_dialog: DialogHandler,
        find: FindHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(self.render.clone()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(self.life.clone()) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(self.load.clone()) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(self.display.clone()) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(self.menu.clone()) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(self.request.clone()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(self.permission.clone()) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(self.download.clone()) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(self.jsdialog.clone()) }
        fn dialog_handler(&self) -> Option<DialogHandler> { Some(self.file_dialog.clone()) }
        fn find_handler(&self) -> Option<FindHandler> { Some(self.find.clone()) }
    }
}

fn make_client(shared: Arc<Shared>) -> Client {
    ViewClient::new(
        ViewRender::new(shared.clone()),
        ViewLife::new(shared.clone()),
        ViewLoad::new(shared.clone()),
        ViewDisplay::new(shared.clone()),
        ViewMenu::new(shared.clone()),
        ViewRequest::new(shared.clone()),
        ViewPermissions::new(shared.clone()),
        ViewDownloads::new(shared.clone()),
        ViewJsDialog::new(shared.clone()),
        ViewFileDialog::new(shared.clone()),
        ViewFind::new(shared),
    )
}

/// The GTK input controllers, exposed so the self-test can emit their signals as a user would.
pub struct Controllers {
    pub motion: gtk::EventControllerMotion,
    pub click: gtk::GestureClick,
    pub scroll: gtk::EventControllerScroll,
    pub key: gtk::EventControllerKey,
    pub focus: gtk::EventControllerFocus,
    pub im: gtk::IMMulticontext,
}

/// Input-method state while a key press is being filtered.
#[derive(Default)]
struct ImState {
    /// Text committed by the input method during the current `filter_keypress` call.
    committed: Vec<String>,
    filtering: bool,
    preediting: bool,
}

pub struct BrowserView {
    /// The widget showing the page (and popups). Put it in the window.
    pub widget: gtk::Overlay,
    picture: gtk::Picture,
    pub controllers: Controllers,
    shared: Arc<Shared>,
    im_state: RefCell<ImState>,
    /// Last pointer position in widget (logical) coordinates, for wheel events.
    pointer: Cell<(f64, f64)>,
    /// Keys, clicks and scrolls sent to the page so far (tab sweeping reads this as activity).
    input_events: Cell<u64>,
    /// When the person last used the page (a key, click or scroll): a copy button only counts then.
    last_input: Cell<Option<std::time::Instant>>,
    /// Scroll distance not yet sent (sub-pixel remainders), and the kinetic-scroll generation.
    scroll_acc: Cell<(f64, f64)>,
    kinetic_gen: Cell<u64>,
}

impl BrowserView {
    /// Create the browser and start loading `url`. `on_event` is called on the GTK thread.
    pub fn new(
        display: gdk::Display,
        mode: Mode,
        device_resolution: bool,
        frame_rate: i32,
        url: &str,
        on_event: impl Fn(ViewEvent) + 'static,
    ) -> Rc<BrowserView> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let shared = Shared::new(id, device_resolution, mode == Mode::Accel, frame_rate);
        let view = Self::build(display, shared.clone(), on_event);
        Self::create_browser(&shared, url);
        view
    }

    /// A view whose browser is not created until `wake` (a restored tab). `screenshot`, if it
    /// exists, is shown (blurred) in the meantime.
    #[allow(clippy::too_many_arguments)]
    pub fn new_dormant(
        display: gdk::Display,
        mode: Mode,
        device_resolution: bool,
        frame_rate: i32,
        url: &str,
        screenshot: Option<&std::path::Path>,
        on_event: impl Fn(ViewEvent) + 'static,
    ) -> Rc<BrowserView> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let shared = Shared::new(id, device_resolution, mode == Mode::Accel, frame_rate);
        *shared.deferred_url.lock().unwrap() = Some(url.to_owned());
        let view = Self::build(display, shared, on_event);
        if let Some(tex) = screenshot.filter(|p| p.is_file()).and_then(|p| gdk::Texture::from_filename(p).ok()) {
            view.picture.set_paintable(Some(&tex));
            view.picture.add_css_class("dormant");
        }
        view
    }

    /// Create the CEF browser for a view.
    fn create_browser(shared: &Arc<Shared>, url: &str) {
        let window_info = WindowInfo {
            windowless_rendering_enabled: 1,
            shared_texture_enabled: shared.accel as _,
            runtime_style: RuntimeStyle::from(sys::cef_runtime_style_t::CEF_RUNTIME_STYLE_ALLOY),
            ..Default::default()
        };
        let settings = BrowserSettings { windowless_frame_rate: shared.frame_rate, ..Default::default() };
        let mut client = make_client(shared.clone());
        let mut context = shared.storage.as_deref().and_then(context_for);
        browser_host_create_browser(Some(&window_info), Some(&mut client), Some(&CefString::from(url)), Some(&settings), None, context.as_mut());
    }

    /// What a script popup from this view becomes (`block` cancels it before it is created).
    pub fn set_popup_policy(&self, policy: crate::workspace::PopupPolicy) {
        *self.shared.popup_policy.lock().unwrap() = policy;
    }

    /// Replace the function that receives this view's events (a view moving to another owner).
    pub fn set_event_handler(&self, on_event: Rc<dyn Fn(ViewEvent)>) {
        with_side(self.shared.id, |s| s.on_event = on_event);
    }

    /// What the configuration says about permissions for this view's page.
    pub fn set_permissions(&self, cfg: crate::permissions::PermissionsConfig) {
        *self.shared.permissions.lock().unwrap() = cfg;
    }

    /// Which sites' cookies this view may send and save.
    pub fn set_cookies(&self, cfg: crate::cookies::CookiesConfig, owner: &str) {
        *self.shared.cookies.lock().unwrap() = cfg;
        *self.shared.session_owner.lock().unwrap() = owner.to_owned();
    }

    /// Sites whose cookies the rules blocked since this page started loading.
    pub fn blocked_cookie_sites(&self) -> Vec<String> {
        self.shared.blocked_cookies.lock().unwrap().iter().cloned().collect()
    }

    /// Where this view's downloads go, and whether to ask.
    pub fn set_downloads(&self, dir: std::path::PathBuf, ask: bool) {
        *self.shared.downloads.lock().unwrap() = (dir, ask);
    }

    /// Allow this view to hand addresses with `scheme` to the desktop (a sign-in's callback).
    pub fn allow_scheme(&self, scheme: &str) {
        self.shared.allowed_schemes.lock().unwrap().push(scheme.to_lowercase());
    }

    /// Tell the view which workspace it is in (the router's idea of where a link came from).
    pub fn set_workspace(&self, workspace: Option<String>) {
        *self.shared.workspace.lock().unwrap() = workspace;
    }

    /// Limit where this view may navigate (see `workspace::scope_matches`); empty means anywhere.
    pub fn scope(&self) -> Vec<String> {
        self.shared.scope.lock().unwrap().clone()
    }

    pub fn set_scope(&self, patterns: Vec<String>) {
        *self.shared.scope.lock().unwrap() = patterns;
    }

    /// Whether a (blurred) screenshot is standing in for the page.
    pub fn showing_placeholder(&self) -> bool {
        self.picture.has_css_class("dormant")
    }

    /// Whether the browser has not been created yet.
    pub fn is_dormant(&self) -> bool {
        self.shared.deferred_url.lock().unwrap().is_some()
    }

    /// Create the browser of a dormant view (a no-op otherwise). A navigation requested while it
    /// was dormant wins over the remembered address.
    pub fn wake(&self) {
        let url = self.shared.deferred_url.lock().unwrap().take();
        if let Some(url) = url {
            let pending = self.shared.pending_url.lock().unwrap().take();
            Self::create_browser(&self.shared, pending.as_deref().unwrap_or(&url));
        }
    }

    /// Copy the page's last frame into memory. Do this promptly after the tab is left: the frame
    /// is a GPU buffer that CEF may release once the view is hidden (a later download failed
    /// intermittently), whereas encoding the PNG can wait (`save_png`).
    pub fn capture(&self) -> Option<gdk::MemoryTexture> {
        let p = self.picture.paintable();
        let tex = p?.downcast::<gdk::Texture>().ok()?;
        let (w, h) = (tex.width() as usize, tex.height() as usize);
        let stride = w * 4;
        let mut buf = vec![0u8; stride * h];
        tex.download(&mut buf, stride);
        let bytes = glib::Bytes::from_owned(buf);
        Some(gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, stride))
    }

    /// The view for a popup browser CEF is already creating (see `PopupSpec`).
    pub fn adopt(display: gdk::Display, spec: PopupSpec, on_event: impl Fn(ViewEvent) + 'static) -> Rc<BrowserView> {
        Self::build(display, spec.shared, on_event)
    }

    /// The GTK side of a view: widgets, input controllers, size tracking.
    fn build(display: gdk::Display, shared: Arc<Shared>, on_event: impl Fn(ViewEvent) + 'static) -> Rc<BrowserView> {
        let id = shared.id;
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        let popup = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .can_target(false)
            .visible(false)
            .build();
        let popup_layer = gtk::Fixed::new();
        popup_layer.set_can_target(false);
        popup_layer.put(&popup, 0.0, 0.0);
        let widget = gtk::Overlay::new();
        widget.set_child(Some(&picture));
        widget.add_overlay(&popup_layer);
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_focusable(true);
        widget.set_focus_on_click(true);

        VIEWS.with(|v| {
            v.borrow_mut().insert(
                id,
                GtkSide {
                    overlay: widget.clone(),
                    picture: picture.clone(),
                    popup,
                    popup_layer,
                    display,
                    on_event: Rc::new(on_event),
                    frame_hook: None,
                    cursor: "default".into(),
                    cursor_log: Vec::new(),
                    tooltip: String::new(),
                    selection: String::new(),
                    popup_visible: false,
                },
            );
        });

        let controllers = Controllers {
            motion: gtk::EventControllerMotion::new(),
            click: gtk::GestureClick::builder().button(0).build(),
            scroll: gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES),
            key: gtk::EventControllerKey::new(),
            focus: gtk::EventControllerFocus::new(),
            im: gtk::IMMulticontext::new(),
        };
        let view = Rc::new(BrowserView {
            widget: widget.clone(),
            picture: picture.clone(),
            controllers,
            shared: shared.clone(),
            im_state: RefCell::new(ImState::default()),
            pointer: Cell::new((0.0, 0.0)),
            input_events: Cell::new(0),
            last_input: Cell::new(None),
            scroll_acc: Cell::new((0.0, 0.0)),
            kinetic_gen: Cell::new(0),
        });
        view.connect_input();

        // A view that is not on screen (a background tab) stops rendering; showing it again
        // wakes it (the first fresh frame arrives within about 10 ms: docs/research).
        widget.connect_map({
            let w = Rc::downgrade(&view);
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.set_hidden(false);
                }
            }
        });
        widget.connect_unmap({
            let w = Rc::downgrade(&view);
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.set_hidden(true);
                }
            }
        });

        // Track the widget's size and scale; CEF renders at the matching device resolution.
        {
            let view = view.clone();
            picture.add_tick_callback(move |w, _| {
                let (pw, ph) = (w.width(), w.height());
                // BROWSAH_FORCE_SCALE=2 pretends the display is scale 2 (to test HiDPI on a 1x screen).
                let scale = std::env::var("BROWSAH_FORCE_SCALE").ok().and_then(|s| s.parse::<f32>().ok()).or_else(|| w.native().and_then(|n| n.surface()).map(|s| s.scale() as f32)).unwrap_or(1.0);
                // The desktop's text scaling ("Large Text"): an extra scale factor for the page, followed live.
                let text = crate::frames::text_scale();
                let changed = {
                    let mut v = view.shared.view.lock().unwrap();
                    let changed = pw > 0 && ph > 0 && (v.size != (pw, ph) || v.scale != scale || v.text != text);
                    if changed {
                        v.size = (pw, ph);
                        v.scale = scale;
                        v.text = text;
                    }
                    changed
                };
                if changed {
                    if std::env::var_os("BROWSAH_DEBUG").is_some() {
                        let k = view.shared.view.lock().unwrap().k();
                        eprintln!("browsah: view {pw}x{ph} logical, surface scale {scale}, render scale {k}, page zoom level {:.3}", if k > 1.0 { (k as f64).ln() / 1.2f64.ln() } else { 0.0 });
                    }
                    if let Some(h) = view.shared.host.lock().unwrap().as_ref() {
                        let h2 = HostHandle(h.0.clone());
                        on_ui(move || {
                            let h2 = h2;
                            h2.0.notify_screen_info_changed(); // the scale may have changed: ask for the screen info again
                        });
                        h.0.was_resized();
                    }
                    view.apply_zoom();
                }
                glib::ControlFlow::Continue
            });
        }

        view
    }

    fn with_browser(&self, f: impl FnOnce(&Browser)) {
        if let Some(b) = self.shared.browser.lock().unwrap().as_ref() {
            f(&b.0);
        }
    }

    fn with_host(&self, f: impl FnOnce(&BrowserHost)) {
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            f(&h.0);
        }
    }

    /// Load `url`. Before the browser exists (a tab just opened) it is remembered and loaded
    /// as soon as the browser is created.
    pub fn navigate(&self, url: &str) {
        // Decide under the lock (so it cannot race with the browser being created), but call
        // into CEF outside it.
        let browser = {
            let guard = self.shared.browser.lock().unwrap();
            match guard.as_ref() {
                Some(b) => Some(b.0.clone()),
                None => {
                    *self.shared.pending_url.lock().unwrap() = Some(url.to_owned());
                    None
                }
            }
        };
        match browser {
            Some(b) => {
                if let Some(frame) = b.main_frame() {
                    frame.load_url(Some(&CefString::from(url)));
                }
            }
            None => self.wake(), // a restored tab being navigated: bring its browser up on this address
        }
    }

    pub fn go_back(&self) {
        self.with_browser(|b| b.go_back());
    }

    pub fn go_forward(&self) {
        self.with_browser(|b| b.go_forward());
    }

    pub fn reload(&self) {
        self.with_browser(|b| b.reload());
    }

    pub fn stop(&self) {
        self.with_browser(|b| b.stop_load());
    }

    /// Run a DevTools protocol method with no parameters; returns the id to read the result with
    /// `cdp_result`. Posted to CEF's UI thread.
    /// How many keys, clicks and scrolls the user has sent to this page.
    fn note_input(&self) {
        self.input_events.set(self.input_events.get() + 1);
        self.last_input.set(Some(std::time::Instant::now()));
    }

    /// Whether the person used the page within `within` (like a browser's transient user activation).
    pub fn used_within(&self, within: std::time::Duration) -> bool {
        self.last_input.get().is_some_and(|t| t.elapsed() <= within)
    }

    pub fn input_events(&self) -> u64 {
        self.input_events.get()
    }

    /// Open Chromium's developer tools for this page in a window of their own (or close them if open).
    pub fn toggle_dev_tools(&self) {
        let shared = self.shared.clone();
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            on_ui(move || {
                let h = h;
                if h.0.has_dev_tools() != 0 {
                    h.0.close_dev_tools();
                    shared.dev_tools.store(false, Ordering::Relaxed);
                    return;
                }
                // A plain window and no client of ours: the tools must not feed their title or frames into this view.
                let window_info = WindowInfo::default();
                h.0.show_dev_tools(Some(&window_info), None, Some(&BrowserSettings::default()), None);
                shared.dev_tools.store(h.0.has_dev_tools() != 0, Ordering::Relaxed);
            });
        }
    }

    /// Whether developer tools were open after the last toggle.
    pub fn has_dev_tools(&self) -> bool {
        self.shared.dev_tools.load(Ordering::Relaxed)
    }

    /// Ask the page to leave fullscreen (it was the browser's key, not the page's, that was pressed).
    pub fn exit_page_fullscreen(&self) {
        self.cdp_eval("document.fullscreenElement ? document.exitFullscreen() : 0");
    }

    /// Run a JavaScript expression in the page (DevTools `Runtime.evaluate`, by value); returns the id to
    /// read the result with `cdp_result`. Posted to CEF's UI thread.
    pub fn cdp_eval(&self, expression: &str) -> i32 {
        self.cdp_eval_as(expression, false)
    }

    /// As `cdp_eval`; `gesture` runs it as if the person had just clicked (for what needs a user gesture).
    pub fn cdp_eval_as(&self, expression: &str, gesture: bool) -> i32 {
        let h = self.shared.host.lock().unwrap().as_ref().map(|h| HostHandle(h.0.clone()));
        let expression = expression.to_owned();
        self.shared.post_cdp(move |id| {
            let Some(h) = h else { return };
            let Some(mut params) = dictionary_value_create() else { return };
            params.set_string(Some(&CefString::from("expression")), Some(&CefString::from(expression.as_str())));
            params.set_bool(Some(&CefString::from("returnByValue")), 1);
            params.set_bool(Some(&CefString::from("awaitPromise")), 1);
            if gesture {
                params.set_bool(Some(&CefString::from("userGesture")), 1);
            }
            h.0.execute_dev_tools_method(id, Some(&CefString::from("Runtime.evaluate")), Some(&mut params));
        })
    }

    pub fn cdp(&self, method: &'static str) -> i32 {
        let h = self.shared.host.lock().unwrap().as_ref().map(|h| HostHandle(h.0.clone()));
        self.shared.post_cdp(move |id| {
            if let Some(h) = h {
                h.0.execute_dev_tools_method(id, Some(&CefString::from(method)), None);
            }
        })
    }

    /// Download an image (an app icon) through the page's network stack. Posted to CEF's UI thread.
    pub fn download_image(&self, url: &str) {
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            let url = url.to_owned();
            let shared = self.shared.clone();
            on_ui(move || {
                let h = h;
                let mut cb = ViewImage::new(shared);
                h.0.download_image(Some(&CefString::from(url.as_str())), 0, 512, 0, Some(&mut cb));
            });
        }
    }

    /// A downloaded image as PNG bytes.
    pub fn image_png(&self, url: &str) -> Option<Vec<u8>> {
        self.shared.image_data.lock().unwrap().get(url).cloned().filter(|b| !b.is_empty())
    }

    pub fn image_result(&self, url: &str) -> Option<(i32, usize, usize)> {
        self.shared.images.lock().unwrap().get(url).copied()
    }

    /// The result of a `cdp` call once it has arrived: (success, JSON).
    pub fn cdp_result(&self, id: i32) -> Option<(bool, String)> {
        self.shared.cdp_results.lock().unwrap().get(&id).cloned()
    }

    /// Tell CEF whether the widget has the keyboard focus. Needed once the browser exists: a new
    /// tab is focused before its browser is created, so the focus event had nowhere to go.
    pub fn sync_focus(&self) {
        let focused = self.widget.has_focus();
        self.with_host(|h| h.set_focus(focused as _));
    }

    /// Change the frame-rate cap. Posted to CEF's UI thread.
    pub fn set_frame_rate(&self, fps: i32) {
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            on_ui(move || {
                let h = h;
                h.0.set_windowless_frame_rate(fps);
            });
        }
    }

    /// Tell CEF whether the view is on screen. Posted to CEF's UI thread.
    fn set_hidden(&self, hidden: bool) {
        self.shared.hidden.store(hidden, Ordering::Relaxed);
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            on_ui(move || {
                let h = h;
                h.0.was_hidden(hidden as _);
                if !hidden {
                    h.0.invalidate(PaintElementType::VIEW);
                }
            });
        }
    }

    /// Whether CEF has been told the view is hidden (a background tab).
    pub fn is_hidden(&self) -> bool {
        self.shared.hidden.load(Ordering::Relaxed)
    }

    /// Frames CEF has delivered so far (used by the self-test).
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }

    /// Whether a page frame has been shown.
    pub fn has_frame(&self) -> bool {
        self.picture.paintable().is_some()
    }

    /// The cursor the page asked for (GTK cursor name).
    pub fn cursor(&self) -> String {
        with_side(self.shared.id, |s| s.cursor.clone()).unwrap_or_default()
    }

    /// How many cursor changes have happened so far (a mark for `cursor_seen`).
    pub fn cursor_mark(&self) -> usize {
        with_side(self.shared.id, |s| s.cursor_log.len()).unwrap_or(0)
    }

    /// Whether the page asked for cursor `name` after `mark`.
    pub fn cursor_seen(&self, mark: usize, name: &str) -> bool {
        with_side(self.shared.id, |s| s.cursor_log.iter().skip(mark).any(|c| c == name)).unwrap_or(false)
    }

    /// The page's current tooltip text (empty when none).
    pub fn tooltip(&self) -> String {
        with_side(self.shared.id, |s| s.tooltip.clone()).unwrap_or_default()
    }

    /// The text currently selected in the page.
    pub fn selection(&self) -> String {
        with_side(self.shared.id, |s| s.selection.clone()).unwrap_or_default()
    }

    /// Whether a popup (a `<select>` dropdown) is showing.
    pub fn popup_visible(&self) -> bool {
        with_side(self.shared.id, |s| s.popup_visible).unwrap_or(false)
    }

    /// Whether the popup has received a frame.
    pub fn popup_has_frame(&self) -> bool {
        with_side(self.shared.id, |s| s.popup.paintable().is_some()).unwrap_or(false)
    }

    /// Call `hook` with every finished page frame, on the GTK thread.
    pub fn set_frame_hook(&self, hook: Option<Rc<dyn Fn(&gdk::Texture)>>) {
        with_side(self.shared.id, |s| s.frame_hook = hook);
    }

    /// Apply the person's zoom for this page (CEF resets zoom when the host changes, so call this again after
    /// address changes and when loading ends). The display's scale is not part of it: CEF paints at the device
    /// scale factor itself.
    pub fn apply_zoom(&self) {
        let user = self.user_zoom().ln() / 1.2f64.ln();
        self.with_host(|h| h.set_zoom_level(user));
    }

    /// Set the colour scheme the page reports (`prefers-color-scheme`): dark or light. Applied now if the browser exists, else
    /// when it is created.
    pub fn set_color_scheme(&self, dark: bool) {
        let changed = self.shared.scheme.lock().unwrap().replace(dark) != Some(dark);
        if changed {
            apply_scheme(&self.shared);
        }
    }

    /// Write the page to a PDF file. The result arrives as a `PdfSaved` event. Posted to CEF's UI thread.
    pub fn print_to_pdf(&self, path: &std::path::Path, paper_inches: (f64, f64), backgrounds: bool) {
        let shared = self.shared.clone();
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            let path = path.to_string_lossy().into_owned();
            on_ui(move || {
                let h = h;
                let settings = PdfPrintSettings { print_background: backgrounds as _, paper_width: paper_inches.0, paper_height: paper_inches.1, prefer_css_page_size: 1, ..Default::default() };
                let mut cb = ViewPdf::new(shared);
                h.0.print_to_pdf(Some(&CefString::from(path.as_str())), Some(&settings), Some(&mut cb));
            });
        }
    }

    /// Find `text` in the page (`next`: the same search again, moving on). Posted to CEF's UI thread.
    pub fn find(&self, text: &str, forward: bool, match_case: bool, next: bool) {
        *self.shared.find_result.lock().unwrap() = (0, 0);
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            let text = text.to_owned();
            on_ui(move || {
                let h = h;
                h.0.find(Some(&CefString::from(text.as_str())), forward as _, match_case as _, next as _);
            });
        }
    }

    /// End the search and remove its highlights. Posted to CEF's UI thread.
    pub fn stop_find(&self) {
        *self.shared.find_result.lock().unwrap() = (0, 0);
        if let Some(h) = self.shared.host.lock().unwrap().as_ref() {
            let h = HostHandle(h.0.clone());
            on_ui(move || {
                let h = h;
                h.0.stop_finding(1);
            });
        }
    }

    /// The last find-in-page result: (matches, active match).
    pub fn find_result(&self) -> (i32, i32) {
        *self.shared.find_result.lock().unwrap()
    }

    /// The view's logical size.
    pub fn widget_size(&self) -> (i32, i32) {
        self.shared.view.lock().unwrap().size
    }

    /// The frame being shown, as a texture (for the self-test's pictures).
    pub fn picture_texture(&self) -> Option<gdk::Texture> {
        self.picture.paintable().and_then(|p| p.downcast::<gdk::Texture>().ok())
    }

    /// The pixel size of the frame being shown, if there is one.
    pub fn frame_size(&self) -> Option<(i32, i32)> {
        self.picture.paintable().map(|p| (p.intrinsic_width(), p.intrinsic_height())).filter(|(w, _)| *w > 0)
    }

    /// The person's zoom for this page (1.0 = 100 %).
    pub fn user_zoom(&self) -> f64 {
        *self.shared.user_zoom.lock().unwrap()
    }

    /// Set the person's zoom and apply it.
    pub fn set_user_zoom(&self, factor: f64) {
        *self.shared.user_zoom.lock().unwrap() = factor;
        self.apply_zoom();
    }

    /// Whether the browser exists (a dormant view has none).
    pub fn is_live(&self) -> bool {
        self.shared.host.lock().unwrap().is_some()
    }

    /// Close the browser the polite way: the page may ask "Leave site?" first (`ViewEvent::Closed` follows when it
    /// agrees, or `CloseCancelled` when the person stays).
    pub fn ask_close(&self) {
        self.with_host(|h| h.close_browser(0));
    }

    /// Close a view that another has replaced (a tab put to sleep, a rendering fall-back). Its events no longer reach the tab:
    /// a closing browser still reports an address, a title and loading changes, and these would overwrite the tab's own
    /// (a slept tab's address turning into `about:blank` is what it would wake and be saved as).
    pub fn retire(&self) {
        self.set_event_handler(Rc::new(|_| {}));
        self.close();
    }

    /// Ask CEF to close the browser. Call before shutting CEF down.
    pub fn close(&self) {
        self.with_host(|h| h.close_browser(1));
    }

    // ---- input ------------------------------------------------------------------------------

    /// Render scale: view pixels per logical pixel.
    fn k(&self) -> f32 {
        self.shared.view.lock().unwrap().k()
    }

    /// Files, text and links dropped onto the page: GTK's drop target feeds CEF's drag-target calls (windowless
    /// Chromium receives no drops of its own). Dragging *out of* the page is not supported.
    fn connect_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK);
        target.set_types(&[gdk::FileList::static_type(), String::static_type()]);
        target.set_preload(true);
        self.widget.add_css_class("browsah-view"); // no drop highlight (the page draws its own)
        let ops = || DragOperationsMask::from(sys::cef_drag_operations_mask_t(1 | 2 | 16));
        // Whether CEF has been told about this drag (the value of a file drag may load after the pointer enters).
        let entered = Rc::new(Cell::new(false));
        let weak = Rc::downgrade(self);
        // Tell CEF a drag arrived, once the data is there. Returns whether it has been told.
        let enter = {
            let entered = entered.clone();
            move |v: &Rc<BrowserView>, t: &gtk::DropTarget, x: f64, y: f64| -> bool {
                if entered.get() {
                    return true;
                }
                let Some(mut data) = t.value().and_then(|v| drag_data_from(&v)) else { return false };
                entered.set(true);
                let ev = v.mouse_event(x, y, gdk::ModifierType::empty());
                drag_on_ui(&v.shared, move |h| {
                    h.drag_target_drag_enter(Some(&mut data), Some(&ev), ops());
                    h.drag_target_drag_over(Some(&ev), ops());
                });
                true
            }
        };
        target.connect_enter({
            let w = weak.clone();
            let enter = enter.clone();
            move |t, x, y| {
                if let Some(v) = w.upgrade() {
                    enter(&v, t, x, y);
                }
                gdk::DragAction::COPY
            }
        });
        target.connect_motion({
            let w = weak.clone();
            let enter = enter.clone();
            let entered = entered.clone();
            move |t, x, y| {
                let Some(v) = w.upgrade() else { return gdk::DragAction::empty() };
                if enter(&v, t, x, y) && entered.get() {
                    let ev = v.mouse_event(x, y, gdk::ModifierType::empty());
                    drag_on_ui(&v.shared, move |h| h.drag_target_drag_over(Some(&ev), ops()));
                }
                gdk::DragAction::COPY
            }
        });
        target.connect_leave({
            let w = weak.clone();
            let entered = entered.clone();
            move |_| {
                if let Some(v) = w.upgrade() {
                    if entered.replace(false) {
                        drag_on_ui(&v.shared, |h| h.drag_target_drag_leave());
                    }
                }
            }
        });
        target.connect_drop({
            let w = weak;
            move |t, value, x, y| {
                let Some(v) = w.upgrade() else { return false };
                let ev = v.mouse_event(x, y, gdk::ModifierType::empty());
                let late = !entered.replace(false);
                if late {
                    // The data arrived only now: start the drag, and let the page answer before releasing.
                    let _ = t;
                    let Some(mut data) = drag_data_from(value) else { return false };
                    drag_on_ui(&v.shared, move |h| {
                        h.drag_target_drag_enter(Some(&mut data), Some(&ev), ops());
                        h.drag_target_drag_over(Some(&ev), ops());
                    });
                    let ev = v.mouse_event(x, y, gdk::ModifierType::empty());
                    let weak = Rc::downgrade(&v);
                    glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                        if let Some(v) = weak.upgrade() {
                            drag_on_ui(&v.shared, move |h| h.drag_target_drop(Some(&ev)));
                        }
                    });
                } else {
                    drag_on_ui(&v.shared, move |h| {
                        h.drag_target_drag_over(Some(&ev), ops());
                        h.drag_target_drop(Some(&ev));
                    });
                }
                true
            }
        });
        self.widget.add_controller(target);
    }

    /// Start dropping `text` at (x, y) as the drop target would (for the self-test).
    pub fn test_drop_text(&self, text: &str, x: f64, y: f64) {
        self.test_drop_value(&glib::Value::from(text), x, y);
    }

    /// As `test_drop_text`, for a file.
    pub fn test_drop_file(&self, path: &str, x: f64, y: f64) {
        let list = gdk::FileList::from_array(&[gtk::gio::File::for_path(path)]);
        self.test_drop_value(&list.to_value(), x, y);
    }

    fn test_drop_value(&self, value: &glib::Value, x: f64, y: f64) {
        let Some(mut data) = drag_data_from(value) else { return };
        let ev = self.mouse_event(x, y, gdk::ModifierType::empty());
        let ops = DragOperationsMask::from(sys::cef_drag_operations_mask_t(1 | 2 | 16));
        drag_on_ui(&self.shared, move |h| {
            h.drag_target_drag_enter(Some(&mut data), Some(&ev), ops.clone());
            h.drag_target_drag_over(Some(&ev), ops.clone());
        });
    }

    /// Release the drop started by `test_drop_text`.
    pub fn test_drop_release(&self, x: f64, y: f64) {
        let ev = self.mouse_event(x, y, gdk::ModifierType::empty());
        drag_on_ui(&self.shared, move |h| h.drag_target_drop(Some(&ev)));
    }

    fn mouse_event(&self, x: f64, y: f64, state: gdk::ModifierType) -> MouseEvent {
        let k = self.k() as f64;
        MouseEvent { x: (x * k).round() as i32, y: (y * k).round() as i32, modifiers: input::modifiers(state) }
    }

    fn connect_input(self: &Rc<Self>) {
        let c = &self.controllers;
        let weak = Rc::downgrade(self);

        // Mouse movement, entering and leaving.
        c.motion.connect_motion({
            let w = weak.clone();
            move |ctrl, x, y| {
                if ignore_pointer(ctrl.current_event()) {
                    return;
                }
                if let Some(v) = w.upgrade() {
                    v.pointer_moved(x, y, ctrl.current_event_state());
                }
            }
        });
        c.motion.connect_enter({
            let w = weak.clone();
            move |ctrl, x, y| {
                if ignore_pointer(ctrl.current_event()) {
                    return;
                }
                if let Some(v) = w.upgrade() {
                    v.pointer.set((x, y));
                    let ev = v.mouse_event(x, y, ctrl.current_event_state());
                    v.with_host(|h| h.send_mouse_move_event(Some(&ev), 0));
                }
            }
        });
        c.motion.connect_leave({
            let w = weak.clone();
            move |ctrl| {
                if ignore_pointer(ctrl.current_event()) {
                    return;
                }
                if let Some(v) = w.upgrade() {
                    if v.page_dragging() {
                        return;
                    }
                    let (x, y) = v.pointer.get();
                    let ev = v.mouse_event(x, y, gdk::ModifierType::empty());
                    v.with_host(|h| h.send_mouse_move_event(Some(&ev), 1));
                }
            }
        });

        // Buttons. Pressing focuses the page, like clicking in any browser.
        c.click.connect_pressed({
            let w = weak.clone();
            move |g, n_press, x, y| {
                if let Some(v) = w.upgrade() {
                    v.widget.grab_focus();
                    v.click(g.current_button(), n_press, x, y, g.current_event_state(), false);
                }
            }
        });
        c.click.connect_released({
            let w = weak.clone();
            move |g, n_press, x, y| {
                if let Some(v) = w.upgrade() {
                    v.click(g.current_button(), n_press, x, y, g.current_event_state(), true);
                }
            }
        });

        // Wheel and touchpad scrolling. The kinetic flag makes GTK report the finger's velocity
        // when it lifts, and we keep the page coasting (the page itself only sees wheel events).
        c.scroll.set_flags(gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::KINETIC);
        c.scroll.connect_scroll({
            let w = weak.clone();
            move |ctrl, dx, dy| {
                if ignore_pointer(ctrl.current_event()) {
                    return glib::Propagation::Stop;
                }
                let Some(v) = w.upgrade() else { return glib::Propagation::Proceed };
                v.note_input();
                v.kinetic_gen.set(v.kinetic_gen.get() + 1); // new input stops any coasting
                let k = v.k();
                let unit = ctrl.unit();
                if std::env::var_os("BROWSAH_DEBUG_SCROLL").is_some() {
                    eprintln!("browsah: scroll dx={dx:.2} dy={dy:.2} unit={unit:?}");
                }
                // Touchpad events (not wheel notches) are accelerated by their size.
                if unit == gdk::ScrollUnit::Wheel && ctrl.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    if dy != 0.0 {
                        send(v.shared.id, ViewEvent::Zoom(if dy < 0.0 { 1 } else { -1 }));
                    }
                    return glib::Propagation::Stop;
                }
                let accel = |d: f64| if unit == gdk::ScrollUnit::Wheel { d } else { d * input::touchpad_acceleration(d) };
                v.scroll_by(input::wheel_delta_f(accel(dx), unit, k), input::wheel_delta_f(accel(dy), unit, k), ctrl.current_event_state());
                glib::Propagation::Stop
            }
        });
        c.scroll.connect_decelerate({
            let w = weak.clone();
            move |_, vx, vy| {
                if let Some(v) = w.upgrade() {
                    v.coast(vx, vy);
                }
            }
        });

        // Keyboard.
        c.key.connect_key_pressed({
            let w = weak.clone();
            move |ctrl, key, keycode, state| match w.upgrade() {
                Some(v) if v.key(true, key, keycode, state, ctrl.current_event()) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        c.key.connect_key_released({
            let w = weak.clone();
            move |ctrl, key, keycode, state| {
                if let Some(v) = w.upgrade() {
                    v.key(false, key, keycode, state, ctrl.current_event());
                }
            }
        });

        // Focus: tell CEF and the input method.
        c.focus.connect_enter({
            let w = weak.clone();
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.controllers.im.focus_in();
                    v.with_host(|h| h.set_focus(1));
                }
            }
        });
        c.focus.connect_leave({
            let w = weak.clone();
            move |_| {
                if let Some(v) = w.upgrade() {
                    v.controllers.im.focus_out();
                    v.with_host(|h| h.set_focus(0));
                }
            }
        });

        // Input method: composed or IME text arrives through `commit`; the in-progress
        // composition through `preedit-changed`.
        c.im.set_client_widget(Some(&self.widget));
        c.im.set_use_preedit(true);
        c.im.connect_commit({
            let w = weak.clone();
            move |_, text| {
                if let Some(v) = w.upgrade() {
                    v.im_commit(text);
                }
            }
        });
        c.im.connect_preedit_changed({
            let w = weak.clone();
            move |im| {
                if let Some(v) = w.upgrade() {
                    let (text, _attrs, cursor) = im.preedit_string();
                    v.im_state.borrow_mut().preediting = !text.is_empty();
                    v.with_host(|h| {
                        let range = Range { from: u32::MAX, to: u32::MAX };
                        let sel = Range { from: cursor as u32, to: cursor as u32 };
                        h.ime_set_composition(Some(&CefString::from(text.as_str())), None, Some(&range), Some(&sel));
                    });
                }
            }
        });

        self.widget.add_controller(c.motion.clone());
        self.widget.add_controller(c.click.clone());
        self.widget.add_controller(c.scroll.clone());
        self.widget.add_controller(c.key.clone());
        self.widget.add_controller(c.focus.clone());
        self.connect_drop();
    }

    /// Send a scroll of `dx`, `dy` view pixels, carrying sub-pixel remainders over.
    fn scroll_by(&self, dx: f64, dy: f64, state: gdk::ModifierType) {
        let (mut ax, mut ay) = self.scroll_acc.get();
        ax += dx;
        ay += dy;
        let (px, py) = (input::take_whole(&mut ax), input::take_whole(&mut ay));
        self.scroll_acc.set((ax, ay));
        if px != 0 || py != 0 {
            let (x, y) = self.pointer.get();
            let ev = self.mouse_event(x, y, state);
            self.with_host(|h| h.send_mouse_wheel_event(Some(&ev), px, py));
        }
    }

    /// Keep scrolling after a touchpad flick: `vx`, `vy` are the release velocity in logical px/s,
    /// decaying exponentially until too slow to see.
    fn coast(self: &Rc<Self>, vx: f64, vy: f64) {
        const TAU: f64 = 0.35; // seconds for the speed to fall by a factor e
        let generation = self.kinetic_gen.get() + 1;
        self.kinetic_gen.set(generation);
        let (mut vx, mut vy) = (vx, vy);
        let weak = Rc::downgrade(self);
        let mut last = std::time::Instant::now();
        glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
            let Some(v) = weak.upgrade() else { return glib::ControlFlow::Break };
            if v.kinetic_gen.get() != generation {
                return glib::ControlFlow::Break;
            }
            let now = std::time::Instant::now();
            let dt = (now - last).as_secs_f64();
            last = now;
            let k = v.k();
            v.scroll_by(
                input::wheel_delta_f(vx * dt, gdk::ScrollUnit::Surface, k),
                input::wheel_delta_f(vy * dt, gdk::ScrollUnit::Surface, k),
                gdk::ModifierType::empty(),
            );
            let decay = (-dt / TAU).exp();
            vx *= decay;
            vy *= decay;
            if vx.hypot(vy) < 20.0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// The pointer moved to (x, y) with these buttons and keys held.
    pub(super) fn pointer_moved(&self, x: f64, y: f64, state: gdk::ModifierType) {
        self.pointer.set((x, y));
        if self.page_drag_move(x, y) {
            return;
        }
        let ev = self.mouse_event(x, y, state);
        self.with_host(|h| h.send_mouse_move_event(Some(&ev), 0));
    }

    pub(super) fn click(&self, button: u32, n_press: i32, x: f64, y: f64, state: gdk::ModifierType, up: bool) {
        self.note_input();
        let kind = match button {
            1 => MouseButtonType::LEFT,
            2 => MouseButtonType::MIDDLE,
            3 => MouseButtonType::RIGHT,
            // 0 = no current event (synthetic signal emission in the self-test): treat as left.
            0 => MouseButtonType::LEFT,
            _ => return,
        };
        self.pointer.set((x, y));
        // A drag that started in the page ends with the left button: the drop, not a click.
        if up && kind == MouseButtonType::LEFT && self.page_drag_end(x, y, true) {
            return;
        }
        if self.page_dragging() {
            return;
        }
        let ev = self.mouse_event(x, y, state);
        self.with_host(|h| h.send_mouse_click_event(Some(&ev), kind, up as _, n_press.max(1)));
    }

    /// Whether a drag that started in the page is running.
    fn page_dragging(&self) -> bool {
        self.shared.page_drag.lock().unwrap().is_some()
    }

    /// While a drag that started in the page runs, the pointer is the drag: tell CEF it entered (once) and where it is. Returns
    /// whether a drag is running (the move is then not a mouse move).
    fn page_drag_move(&self, x: f64, y: f64) -> bool {
        let ev = self.mouse_event(x, y, gdk::ModifierType::BUTTON1_MASK);
        let (first, data, ops) = {
            let mut g = self.shared.page_drag.lock().unwrap();
            let Some(d) = g.as_mut() else { return false };
            if d.ending {
                return true;
            }
            let first = !d.entered;
            d.entered = true;
            (first, ImplDragData::clone(&d.data.0), d.ops.0.clone())
        };
        drag_on_ui(&self.shared, move |h| {
            if let (true, Some(mut data)) = (first, data) {
                h.drag_target_drag_enter(Some(&mut data), Some(&ev), ops.clone());
            }
            h.drag_target_drag_over(Some(&ev), ops);
        });
        true
    }

    /// End the drag that started in the page: drop it where the pointer is (`drop`) or cancel it (`Esc`). Returns whether there
    /// was one. The calls run on CEF's thread after the moves already queued, so the last answer of the page is known by then.
    fn page_drag_end(&self, x: f64, y: f64, drop: bool) -> bool {
        {
            let mut g = self.shared.page_drag.lock().unwrap();
            match g.as_mut() {
                Some(d) if !d.ending => d.ending = true,
                Some(_) => return true,
                None => return false,
            }
        }
        let ev = self.mouse_event(x, y, gdk::ModifierType::BUTTON1_MASK);
        let shared = self.shared.clone();
        drag_on_ui(&self.shared, move |h| {
            let Some(d) = shared.page_drag.lock().unwrap().take() else { return };
            let ops = d.ops.0;
            let accepted = d.op.0.as_ref().0 != 0;
            if d.entered {
                if drop && accepted {
                    h.drag_target_drag_over(Some(&ev), ops);
                    h.drag_target_drop(Some(&ev));
                } else {
                    h.drag_target_drag_leave();
                }
            }
            h.drag_source_ended_at(ev.x, ev.y, if drop && accepted { d.op.0 } else { drag_ops(0) });
            h.drag_source_system_drag_ended();
        });
        true
    }

    /// Text from the input method. Either an ordinary character that the input method handled on
    /// the way through (handled as a normal key press by `key`), or composed text for the page.
    fn im_commit(&self, text: &str) {
        let mut st = self.im_state.borrow_mut();
        if st.filtering {
            st.committed.push(text.to_owned());
            return;
        }
        drop(st);
        self.commit_text(text);
    }

    fn commit_text(&self, text: &str) {
        let range = Range { from: u32::MAX, to: u32::MAX };
        self.with_host(|h| h.ime_commit_text(Some(&CefString::from(text)), Some(&range), 0));
    }

    /// Copy, cut and paste go through GTK's clipboard. Windowless Chromium cannot reach the
    /// Wayland clipboard itself (it has no surface of its own to take clipboard focus), so its
    /// own copy stays inside the browser. Copy and cut also send the key so the page deletes (cut)
    /// as usual; paste is handled here and not sent, or the page would paste its own copy too.
    /// Returns whether the key was fully handled.
    fn clipboard_shortcut(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        use gdk::ModifierType as M;
        if !state.contains(M::CONTROL_MASK) || state.intersects(M::ALT_MASK | M::SUPER_MASK) {
            return false;
        }
        let clipboard = self.widget.display().clipboard();
        match key {
            gdk::Key::c | gdk::Key::C | gdk::Key::x | gdk::Key::X => {
                let sel = self.selection();
                if !sel.is_empty() {
                    clipboard.set_text(&sel);
                }
                false // still sent to the page (cut removes the selection)
            }
            gdk::Key::v | gdk::Key::V => {
                if !clipboard.formats().contains_type(glib::GString::static_type()) {
                    return false; // not text: let the page handle it
                }
                self.paste_from_clipboard();
                true
            }
            _ => false,
        }
    }

    /// Show a context menu as a popover at the pointer. Copy, cut and paste use the clipboard
    /// bridge (see `clipboard_shortcut`) instead of Chromium's own clipboard.
    pub fn show_context_menu(self: &Rc<Self>, req: Arc<MenuRequest>) {
        use sys::cef_menu_id_t as M;
        let popover = gtk::Popover::new();
        popover.set_parent(&self.widget);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(req.x as i32, req.y as i32, 1, 1)));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for item in &req.items {
            if item.separator {
                list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                continue;
            }
            let label = gtk::Label::builder().label(item.label.as_str()).xalign(0.0).build();
            let button = gtk::Button::builder().child(&label).has_frame(false).sensitive(item.enabled).build();
            let (view, req, id) = (Rc::downgrade(self), req.clone(), item.id);
            let popover_weak = popover.downgrade();
            button.connect_clicked(move |_| {
                // Answer CEF first: closing the popover cancels the request if still unanswered.
                let close = || {
                    if let Some(p) = popover_weak.upgrade() {
                        p.popdown();
                    }
                };
                let Some(view) = view.upgrade() else {
                    req.cancel();
                    return close();
                };
                if id == M::MENU_ID_COPY as i32 {
                    let sel = view.selection();
                    if !sel.is_empty() {
                        view.widget.display().clipboard().set_text(&sel);
                    }
                    req.cancel();
                } else if id == M::MENU_ID_PASTE as i32 {
                    view.paste_from_clipboard();
                    req.cancel();
                } else if id == M::MENU_ID_CUT as i32 {
                    let sel = view.selection();
                    if !sel.is_empty() {
                        view.widget.display().clipboard().set_text(&sel);
                    }
                    req.run(id);
                } else {
                    req.run(id);
                }
                close();
            });
            list.append(&button);
        }
        popover.set_child(Some(&list));
        popover.connect_closed({
            let req = req.clone();
            move |p| {
                req.cancel(); // chosen items have already answered; this covers dismissal
                p.unparent();
            }
        });
        popover.popup();
    }

    fn paste_from_clipboard(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.widget.display().clipboard().read_text_async(None::<&gtk::gio::Cancellable>, move |res| {
            if let (Ok(Some(text)), Some(v)) = (res, weak.upgrade()) {
                v.insert_text(text.as_str());
            }
        });
    }

    /// Insert text into the focused field (paste, composed text).
    pub fn insert_text(&self, text: &str) {
        self.commit_text(text);
    }

    /// Handle a key press or release. Returns whether the event was consumed.
    fn key(self: &Rc<Self>, press: bool, key: gdk::Key, keycode: u32, state: gdk::ModifierType, event: Option<gdk::Event>) -> bool {
        if press {
            self.note_input();
        }
        if press && key == gdk::Key::Escape && self.page_dragging() {
            let (x, y) = self.pointer.get();
            self.page_drag_end(x, y, false); // Esc cancels a drag that started in the page
            return true;
        }
        if press && input::is_shell_shortcut(key, state) {
            return false; // the window's accelerators handle it
        }
        if press && self.clipboard_shortcut(key, state) {
            return true;
        }
        // Give the input method first refusal (compose sequences, IME). Ordinary characters
        // pass through it too, and come back as a commit of exactly the key's own character:
        // those are sent to the page as normal key events (a page sees keydown/keypress), while
        // anything else the input method produced goes in as composed text.
        let mut typed: Option<char> = input::typed_char(key, state);
        if let Some(ev) = event.as_ref() {
            {
                let mut st = self.im_state.borrow_mut();
                st.committed.clear();
                st.filtering = true;
            }
            let handled = self.controllers.im.filter_keypress(ev);
            let (committed, preediting) = {
                let mut st = self.im_state.borrow_mut();
                st.filtering = false;
                (std::mem::take(&mut st.committed), st.preediting)
            };
            if handled {
                let plain = !preediting
                    && committed.len() == 1
                    && committed[0].chars().count() == 1
                    && committed[0].chars().next() == key.to_unicode();
                if !plain {
                    for t in committed {
                        self.commit_text(&t);
                    }
                    return true;
                }
                typed = committed[0].chars().next();
            }
        }

        let code = input::windows_key_code(key);
        let ch = typed.map(|c| c as u32).filter(|c| *c <= 0xFFFF).unwrap_or(0) as u16;
        let mut ev = KeyEvent {
            size: std::mem::size_of::<KeyEvent>(),
            type_: if press { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP },
            modifiers: input::modifiers(state),
            windows_key_code: code,
            native_key_code: keycode as i32,
            is_system_key: 0,
            character: ch,
            unmodified_character: ch,
            focus_on_editable_field: 0,
        };
        self.with_host(|h| {
            h.send_key_event(Some(&ev));
            if press && ch != 0 {
                // The character event is what inserts text into a field.
                ev.type_ = KeyEventType::CHAR;
                ev.windows_key_code = ch as i32;
                h.send_key_event(Some(&ev));
            }
        });
        // Only keys with a code are consumed; others (media keys...) are left to the system.
        code != 0
    }
}
