//! The page's context menu: what to offer for a click on a link, an image, media, selected text, an editable field or the page, as
//! a list of entries the window draws (`app/ctxmenu.rs`) and runs. This is the pure part: the model, and the helpers behind a few
//! items (clean links, links to selected text). Item ids are what `context_menu.hide` names.

use crate::profile::ColorPair;

/// How a link or address is opened.
#[derive(Debug, Clone, PartialEq)]
pub enum How {
    /// A tab of this window (in the background, or selected).
    Tab { select: bool },
    /// A window with no workspace.
    Window,
    /// A modal over this window.
    Popup,
    /// What the router's `open:` action says: a workspace, a web app, another profile.
    Route(crate::router::Action),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageCmd {
    Back,
    Forward,
    Reload,
    HardReload,
    Stop,
    Bookmark,
    Find,
    SavePdf,
    Print,
    InstallApp,
    Inspect,
    SelectAll,
    CopySelection,
    CopyLinkText,
    Undo,
    Redo,
    Cut,
    Paste,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaCmd {
    PlayPause,
    Mute,
    Loop,
    Controls,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Page(PageCmd),
    Open { url: String, how: How },
    Copy(String),
    SaveAs(String),
    BookmarkLink(String),
    CopyImage(String),
    Media(MediaCmd),
    /// A CEF menu command (a spelling suggestion), run by CEF itself.
    Cef(i32),
    /// Move the tab to a workspace (empty: a window of its own).
    SendTab(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: &'static str,
    pub label: String,
    /// A Lucide icon id (`icons::by_id`).
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    pub enabled: bool,
    /// A ticked item (loop on).
    pub checked: bool,
    /// A workspace's colour badge instead of an icon (`Some(None)`: a workspace with no colour).
    pub badge: Option<Option<ColorPair>>,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Item(Item),
    Separator,
    /// Icon-only buttons side by side (Back, Forward, Reload, Stop).
    Row(Vec<Item>),
    Submenu { id: &'static str, label: String, icon: Option<&'static str>, items: Vec<Item> },
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Media {
    #[default]
    None,
    Image,
    Video,
    Audio,
    /// A canvas, file or plug-in: nothing to offer.
    Other,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MediaState {
    pub paused: bool,
    pub muted: bool,
    pub looped: bool,
    pub controls: bool,
    pub can_save: bool,
    pub has_audio: bool,
    pub can_toggle_controls: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EditState {
    pub undo: bool,
    pub redo: bool,
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub delete: bool,
    pub select_all: bool,
}

/// What the click was on and what the window knows about it.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub page_url: String,
    pub frame_url: String,
    pub link_url: String,
    /// What the router would do with the link, as `(action, name of the target)` when it is not "stay here".
    pub link_route: Option<(crate::router::Action, String)>,
    pub media: Media,
    pub media_state: MediaState,
    pub source_url: String,
    pub selection: String,
    /// The selection as an address, if it reads like one.
    pub selection_url: Option<String>,
    pub editable: bool,
    pub edit: EditState,
    /// Spelling suggestions (CEF command id, word) and whether a word can be added.
    pub spelling: Vec<(i32, String)>,
    pub add_word: Option<i32>,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub bookmarked: bool,
    pub can_bookmark: bool,
    /// The click was in a modal's page, not a tab: nothing that acts on the tab.
    pub in_modal: bool,
    /// A web app's window: tabs are not sent anywhere.
    pub in_app: bool,
    pub can_send_tab: bool,
}

/// A workspace, web app or profile a link can be sent to.
#[derive(Debug, Clone)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub color: Option<ColorPair>,
}

#[derive(Debug, Clone, Default)]
pub struct Targets {
    /// Workspaces other than this window's.
    pub workspaces: Vec<Target>,
    pub apps: Vec<Target>,
    /// Profiles other than this one.
    pub profiles: Vec<Target>,
    /// Workspaces a tab can be sent to (empty id: a new window is always offered).
    pub send_to: Vec<Target>,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// `{query}` is the selection.
    pub search_url: String,
    /// `{url}` is the image address; none: no image search item.
    pub image_search_url: Option<String>,
    pub developer: bool,
    /// "Open link in new tab" leaves the page where it is.
    pub links_in_background: bool,
    pub hide: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options { search_url: crate::config::DEFAULT_SEARCH_URL.into(), image_search_url: None, developer: true, links_in_background: true, hide: Vec::new() }
    }
}

// ---- helpers ------------------------------------------------------------------------------------------

pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Query parameters that only say where a click came from.
const TRACKING: &[&str] = &["fbclid", "gclid", "gclsrc", "dclid", "msclkid", "mc_cid", "mc_eid", "igshid", "yclid", "twclid", "ttclid", "_hsenc", "_hsmi", "mkt_tok", "vero_id", "wickedid", "oly_enc_id", "oly_anon_id", "ref_src", "ref_url", "spm"];

/// `url` with a redirector (`google.com/url?q=`) replaced by its destination and tracking parameters (`utm_*`, `fbclid`...) removed.
pub fn clean_link(url: &str) -> String {
    let url = crate::router::unwrap_url(url);
    let (base, fragment) = match url.split_once('#') {
        Some((b, f)) => (b.to_owned(), Some(f.to_owned())),
        None => (url.clone(), None),
    };
    let Some((path, query)) = base.split_once('?') else { return url };
    let kept: Vec<&str> = query.split('&').filter(|p| !p.is_empty()).filter(|p| {
        let name = p.split('=').next().unwrap_or("").to_ascii_lowercase();
        !(name.starts_with("utm_") || TRACKING.contains(&name.as_str()))
    }).collect();
    let mut out = path.to_owned();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(&f);
    }
    out
}

/// A link that scrolls to and highlights `selection` on `page_url` (a text fragment, `#:~:text=`), for a web address.
pub fn text_fragment_link(page_url: &str, selection: &str) -> Option<String> {
    if !(page_url.starts_with("http://") || page_url.starts_with("https://")) {
        return None;
    }
    let words: Vec<&str> = selection.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let enc = |s: &str| percent_encode(s).replace('-', "%2D");
    let text = if selection.chars().count() <= 200 || words.len() <= 8 {
        enc(&words.join(" "))
    } else {
        format!("{},{}", enc(&words[..4].join(" ")), enc(&words[words.len() - 4..].join(" ")))
    };
    let base = page_url.split('#').next().unwrap_or(page_url);
    Some(format!("{base}#:~:text={text}"))
}

/// The selection shortened for a menu label.
pub fn shorten(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!("{}\u{2026}", one.chars().take(max).collect::<String>().trim_end())
    }
}

// ---- the plan -----------------------------------------------------------------------------------------

fn item(id: &'static str, label: &str, icon: &'static str, action: Action) -> Item {
    Item { id, label: label.to_owned(), icon: Some(icon), shortcut: None, enabled: true, checked: false, badge: None, action }
}

fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// The entries of the menu for a click, in order (separators between groups, none at the ends).
pub fn plan(ctx: &Context, targets: &Targets, opts: &Options) -> Vec<Entry> {
    let mut groups: Vec<Vec<Entry>> = Vec::new();
    // Chromium marks an image inside a selection with an object-replacement character.
    let selected = ctx.selection.replace('\u{fffc}', " ");
    let link = !ctx.link_url.is_empty() && !ctx.link_url.starts_with("javascript:");
    let media = matches!(ctx.media, Media::Image | Media::Video | Media::Audio);
    let selection = !selected.trim().is_empty();
    let page_context = !ctx.editable && !link && !media && !selection;

    if ctx.editable {
        if !ctx.spelling.is_empty() {
            let mut g: Vec<Entry> = ctx.spelling.iter().map(|(id, word)| Entry::Item(Item { icon: None, ..item("spelling", word, "x", Action::Cef(*id)) })).collect();
            if let Some(id) = ctx.add_word {
                g.push(Entry::Item(item("add-word", "Add to dictionary", "book-plus", Action::Cef(id))));
            }
            groups.push(g);
        }
        let mut undo = item("undo", "Undo", "undo-2", Action::Page(PageCmd::Undo));
        undo.shortcut = Some("Ctrl+Z");
        undo.enabled = ctx.edit.undo;
        let mut redo = item("redo", "Redo", "redo-2", Action::Page(PageCmd::Redo));
        redo.shortcut = Some("Ctrl+Shift+Z");
        redo.enabled = ctx.edit.redo;
        groups.push(vec![Entry::Item(undo), Entry::Item(redo)]);
        let mut cut = item("cut", "Cut", "scissors", Action::Page(PageCmd::Cut));
        cut.shortcut = Some("Ctrl+X");
        cut.enabled = ctx.edit.cut;
        let mut copy = item("copy", "Copy", "copy", Action::Page(PageCmd::CopySelection));
        copy.shortcut = Some("Ctrl+C");
        copy.enabled = ctx.edit.copy || selection;
        let mut paste = item("paste", "Paste", "clipboard-paste", Action::Page(PageCmd::Paste));
        paste.shortcut = Some("Ctrl+V");
        paste.enabled = ctx.edit.paste;
        let mut delete = item("delete", "Delete", "trash-2", Action::Page(PageCmd::Delete));
        delete.enabled = ctx.edit.delete;
        groups.push(vec![Entry::Item(cut), Entry::Item(copy), Entry::Item(paste), Entry::Item(delete)]);
    }

    if link {
        let url = ctx.link_url.clone();
        let mut g = Vec::new();
        if let Some((route, name)) = &ctx.link_route {
            let icon = match route {
                crate::router::Action::App(_) => "app-window",
                crate::router::Action::Profile { .. } => "users",
                crate::router::Action::Workspace(_) => "swatch-book",
                crate::router::Action::Modal => "picture-in-picture-2",
                _ => "external-link",
            };
            g.push(Entry::Item(item("open-routed", &format!("Open in {name}"), icon, Action::Open { url: url.clone(), how: How::Route(route.clone()) })));
        }
        let (label, select) = if opts.links_in_background { ("Open in new tab", false) } else { ("Open in new tab", true) };
        g.push(Entry::Item(item("open-tab", label, "external-link", Action::Open { url: url.clone(), how: How::Tab { select } })));
        g.push(Entry::Item(item("open-window", "Open in new window", "app-window", Action::Open { url: url.clone(), how: How::Window })));
        if !ctx.in_app {
            let mut items: Vec<Item> = targets
                .workspaces
                .iter()
                .map(|t| Item { badge: Some(t.color.clone()), icon: None, ..item("open-workspace", &t.name, "swatch-book", Action::Open { url: url.clone(), how: How::Route(crate::router::Action::Workspace(t.id.clone())) }) })
                .collect();
            items.extend(targets.apps.iter().map(|t| item("open-workspace", &t.name, "app-window", Action::Open { url: url.clone(), how: How::Route(crate::router::Action::App(t.id.clone())) })));
            if !items.is_empty() {
                g.push(Entry::Submenu { id: "open-workspace", label: "Open in workspace".into(), icon: Some("swatch-book"), items });
            }
            if !targets.profiles.is_empty() {
                let items = targets.profiles.iter().map(|t| item("open-profile", &t.name, "users", Action::Open { url: url.clone(), how: How::Route(crate::router::Action::Profile { profile: t.id.clone(), workspace: None }) })).collect();
                g.push(Entry::Submenu { id: "open-profile", label: "Open in other profile".into(), icon: Some("users"), items });
            }
        }
        g.push(Entry::Item(item("open-popup", "Open as popup", "picture-in-picture-2", Action::Open { url: url.clone(), how: How::Popup })));
        groups.push(g);
        let mut g = vec![
            Entry::Item(item("copy-link", "Copy link address", "link", Action::Copy(url.clone()))),
            Entry::Item(item("copy-clean-link", "Copy clean link", "link-2", Action::Copy(clean_link(&url)))),
            Entry::Item(item("copy-link-text", "Copy link text", "text-cursor", Action::Page(PageCmd::CopyLinkText))),
        ];
        if is_web(&url) {
            g.push(Entry::Item(item("save-link", "Save link as\u{2026}", "download", Action::SaveAs(url.clone()))));
        }
        if !ctx.in_modal && ctx.can_bookmark {
            g.push(Entry::Item(item("bookmark-link", "Bookmark link", "star-plus", Action::BookmarkLink(url.clone()))));
        }
        groups.push(g);
    }

    if media {
        let src = ctx.source_url.clone();
        let mut g = Vec::new();
        match ctx.media {
            Media::Image => {
                if !src.is_empty() {
                    g.push(Entry::Item(item("open-image", "Open image in new tab", "image", Action::Open { url: src.clone(), how: How::Tab { select: !opts.links_in_background } })));
                    if is_web(&src) {
                        g.push(Entry::Item(item("save-image", "Save image as\u{2026}", "download", Action::SaveAs(src.clone()))));
                    }
                    g.push(Entry::Item(item("copy-image", "Copy image", "copy", Action::CopyImage(src.clone()))));
                    g.push(Entry::Item(item("copy-image-address", "Copy image address", "link", Action::Copy(src.clone()))));
                    if let (Some(template), true) = (&opts.image_search_url, is_web(&src)) {
                        g.push(Entry::Item(item("search-image", "Search for this image", "search", Action::Open { url: template.replace("{url}", &percent_encode(&src)), how: How::Tab { select: true } })));
                    }
                }
            }
            Media::Video | Media::Audio => {
                let st = &ctx.media_state;
                g.push(Entry::Item(item("media-play", if st.paused { "Play" } else { "Pause" }, if st.paused { "play" } else { "pause" }, Action::Media(MediaCmd::PlayPause))));
                if st.has_audio {
                    g.push(Entry::Item(item("media-mute", if st.muted { "Unmute" } else { "Mute" }, if st.muted { "volume-2" } else { "volume-x" }, Action::Media(MediaCmd::Mute))));
                }
                let mut looped = item("media-loop", "Loop", "repeat", Action::Media(MediaCmd::Loop));
                looped.checked = st.looped;
                g.push(Entry::Item(looped));
                if st.can_toggle_controls {
                    let mut controls = item("media-controls", "Show controls", "sliders-horizontal", Action::Media(MediaCmd::Controls));
                    controls.checked = st.controls;
                    g.push(Entry::Item(controls));
                }
                if !src.is_empty() && is_web(&src) {
                    let what = if ctx.media == Media::Video { "video" } else { "audio" };
                    g.push(Entry::Separator);
                    g.push(Entry::Item(item("open-image", &format!("Open {what} in new tab"), "external-link", Action::Open { url: src.clone(), how: How::Tab { select: !opts.links_in_background } })));
                    if st.can_save {
                        g.push(Entry::Item(item("save-image", &format!("Save {what} as\u{2026}"), "download", Action::SaveAs(src.clone()))));
                    }
                    g.push(Entry::Item(item("copy-image-address", &format!("Copy {what} address"), "link", Action::Copy(src.clone()))));
                }
            }
            _ => {}
        }
        if !g.is_empty() {
            groups.push(g);
        }
    }

    if selection && !ctx.editable {
        let mut copy = item("copy", "Copy", "copy", Action::Page(PageCmd::CopySelection));
        copy.shortcut = Some("Ctrl+C");
        groups.push(vec![Entry::Item(copy)]);
    }
    if selection {
        let mut g = Vec::new();
        let query = selected.split_whitespace().collect::<Vec<_>>().join(" ");
        let query = query.as_str();
        g.push(Entry::Item(item("search-web", &format!("Search for \u{201c}{}\u{201d}", shorten(query, 24)), "search", Action::Open { url: opts.search_url.replace("{query}", &percent_encode(query)), how: How::Tab { select: true } })));
        if let Some(url) = &ctx.selection_url {
            g.push(Entry::Item(item("open-selection", &format!("Open {}", shorten(url, 32)), "external-link", Action::Open { url: url.clone(), how: How::Tab { select: !opts.links_in_background } })));
        }
        if let Some(link) = text_fragment_link(&ctx.page_url, query) {
            g.push(Entry::Item(item("copy-text-link", "Copy link to selected text", "link", Action::Copy(link))));
        }
        let mut all = item("select-all", "Select all", "text-select", Action::Page(PageCmd::SelectAll));
        if ctx.editable {
            all.shortcut = Some("Ctrl+A");
            all.enabled = ctx.edit.select_all;
        }
        g.push(Entry::Item(all));
        groups.push(g);
    } else if ctx.editable {
        let mut all = item("select-all", "Select all", "text-select", Action::Page(PageCmd::SelectAll));
        all.shortcut = Some("Ctrl+A");
        all.enabled = ctx.edit.select_all;
        groups.push(vec![Entry::Item(all)]);
    }

    if page_context {
        let mut back = item("back", "Back", "arrow-left", Action::Page(PageCmd::Back));
        back.enabled = ctx.can_back;
        let mut forward = item("forward", "Forward", "arrow-right", Action::Page(PageCmd::Forward));
        forward.enabled = ctx.can_forward;
        let reload = if ctx.loading { item("stop", "Stop", "x", Action::Page(PageCmd::Stop)) } else { item("reload", "Reload", "rotate-cw", Action::Page(PageCmd::Reload)) };
        groups.push(vec![Entry::Row(vec![back, forward, reload]), Entry::Item(item("hard-reload", "Reload ignoring the cache", "refresh-cw-off", Action::Page(PageCmd::HardReload)))]);
        let mut g = Vec::new();
        if !ctx.in_modal {
            if ctx.can_bookmark {
                let (label, icon) = if ctx.bookmarked { ("Remove bookmark", "star") } else { ("Bookmark this page", "star-plus") };
                let mut b = item("bookmark", label, icon, Action::Page(PageCmd::Bookmark));
                b.shortcut = Some("Ctrl+D");
                g.push(Entry::Item(b));
            }
            let mut find = item("find", "Find in page", "search", Action::Page(PageCmd::Find));
            find.shortcut = Some("Ctrl+F");
            g.push(Entry::Item(find));
            g.push(Entry::Item(item("save-pdf", "Save as PDF\u{2026}", "file-down", Action::Page(PageCmd::SavePdf))));
            let mut print = item("print", "Print\u{2026}", "printer", Action::Page(PageCmd::Print));
            print.shortcut = Some("Ctrl+P");
            g.push(Entry::Item(print));
        }
        g.push(Entry::Item(item("copy-address", "Copy page address", "link", Action::Copy(ctx.page_url.clone()))));
        groups.push(g);
        if !ctx.in_modal {
            let mut g = Vec::new();
            if !ctx.in_app && is_web(&ctx.page_url) {
                g.push(Entry::Item(item("install-app", "Install as web app\u{2026}", "app-window", Action::Page(PageCmd::InstallApp))));
            }
            if ctx.can_send_tab && !ctx.in_app {
                let mut items: Vec<Item> = targets.send_to.iter().map(|t| Item { badge: Some(t.color.clone()), icon: None, ..item("send-tab", &t.name, "swatch-book", Action::SendTab(t.id.clone())) }).collect();
                items.push(item("send-tab", "New window", "app-window", Action::SendTab(String::new())));
                g.push(Entry::Submenu { id: "send-tab", label: "Send to workspace".into(), icon: Some("swatch-book"), items });
            }
            if !g.is_empty() {
                groups.push(g);
            }
        }
        if ctx.frame_url != ctx.page_url && is_web(&ctx.frame_url) {
            groups.push(vec![
                Entry::Item(item("open-frame", "Open frame in new tab", "external-link", Action::Open { url: ctx.frame_url.clone(), how: How::Tab { select: !opts.links_in_background } })),
                Entry::Item(item("copy-frame-address", "Copy frame address", "link", Action::Copy(ctx.frame_url.clone()))),
            ]);
        }
    }

    if opts.developer {
        let mut g = Vec::new();
        if page_context && !ctx.page_url.is_empty() && !ctx.page_url.starts_with("view-source:") {
            g.push(Entry::Item(item("view-source", "View page source", "code-xml", Action::Open { url: format!("view-source:{}", ctx.page_url), how: How::Tab { select: true } })));
        }
        g.push(Entry::Item(item("inspect", "Inspect", "bug", Action::Page(PageCmd::Inspect))));
        groups.push(g);
    }

    // Hidden items, empty submenus and groups, then separators between what is left.
    let hidden = |id: &str| opts.hide.iter().any(|h| h == id);
    let mut out: Vec<Entry> = Vec::new();
    for group in groups {
        let kept: Vec<Entry> = group
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item(i) if hidden(i.id) => None,
                Entry::Row(items) => {
                    let items: Vec<Item> = items.into_iter().filter(|i| !hidden(i.id)).collect();
                    (!items.is_empty()).then_some(Entry::Row(items))
                }
                Entry::Submenu { id, .. } if hidden(id) => None,
                Entry::Separator => Some(Entry::Separator),
                other => Some(other),
            })
            .collect();
        if kept.iter().any(|e| !matches!(e, Entry::Separator)) {
            if !out.is_empty() {
                out.push(Entry::Separator);
            }
            out.extend(kept);
        }
    }
    out
}

/// The ids of every item a menu can have (`context_menu.hide` takes these).
pub const ITEM_IDS: &[&str] = &[
    "back", "forward", "reload", "stop", "hard-reload", "bookmark", "find", "save-pdf", "print", "copy-address", "install-app", "send-tab", "open-frame", "copy-frame-address", "view-source", "inspect",
    "open-routed", "open-tab", "open-window", "open-workspace", "open-profile", "open-popup", "copy-link", "copy-clean-link", "copy-link-text", "save-link", "bookmark-link",
    "open-image", "save-image", "copy-image", "copy-image-address", "search-image", "media-play", "media-mute", "media-loop", "media-controls",
    "copy", "search-web", "open-selection", "copy-text-link", "select-all", "undo", "redo", "cut", "paste", "delete", "spelling", "add-word",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(plan: &[Entry]) -> Vec<&'static str> {
        plan.iter()
            .flat_map(|e| match e {
                Entry::Item(i) => vec![i.id],
                Entry::Row(items) => items.iter().map(|i| i.id).collect(),
                Entry::Submenu { id, .. } => vec![*id],
                Entry::Separator => vec!["-"],
            })
            .collect()
    }

    fn page() -> Context {
        Context { page_url: "https://site.example/a".into(), frame_url: "https://site.example/a".into(), can_bookmark: true, can_send_tab: true, can_back: true, ..Context::default() }
    }

    fn well_formed(plan: &[Entry]) {
        assert!(!matches!(plan.first(), Some(Entry::Separator)) && !matches!(plan.last(), Some(Entry::Separator)), "separators at the ends: {:?}", ids(plan));
        assert!(!plan.windows(2).any(|w| matches!(w, [Entry::Separator, Entry::Separator])), "doubled separators: {:?}", ids(plan));
    }

    #[test]
    fn a_page_has_navigation_page_actions_and_developer_items() {
        let p = plan(&page(), &Targets::default(), &Options::default());
        well_formed(&p);
        let got = ids(&p);
        assert_eq!(&got[..4], &["back", "forward", "reload", "hard-reload"]);
        for id in ["bookmark", "find", "save-pdf", "print", "copy-address", "install-app", "send-tab", "view-source", "inspect"] {
            assert!(got.contains(&id), "{id} in {got:?}");
        }
        let mut loading = page();
        loading.loading = true;
        assert!(ids(&plan(&loading, &Targets::default(), &Options::default())).contains(&"stop"));
        let mut marked = page();
        marked.bookmarked = true;
        let p = plan(&marked, &Targets::default(), &Options::default());
        assert!(p.iter().any(|e| matches!(e, Entry::Item(i) if i.id == "bookmark" && i.label == "Remove bookmark" && i.icon == Some("star"))));
    }

    #[test]
    fn a_link_offers_the_open_and_copy_items() {
        let mut ctx = page();
        ctx.link_url = "https://other.example/x?utm_source=a&id=1".into();
        let targets = Targets {
            workspaces: vec![Target { id: "w".into(), name: "Work".into(), color: None }],
            apps: vec![Target { id: "figma".into(), name: "Figma".into(), color: None }],
            profiles: vec![Target { id: "home".into(), name: "Home".into(), color: None }],
            send_to: vec![],
        };
        let p = plan(&ctx, &targets, &Options::default());
        well_formed(&p);
        let got = ids(&p);
        for id in ["open-tab", "open-window", "open-workspace", "open-profile", "open-popup", "copy-link", "copy-clean-link", "copy-link-text", "save-link", "bookmark-link", "inspect"] {
            assert!(got.contains(&id), "{id} in {got:?}");
        }
        assert!(!got.contains(&"back"), "a link click is not the page menu: {got:?}");
        let clean = p.iter().find_map(|e| match e { Entry::Item(i) if i.id == "copy-clean-link" => Some(i.action.clone()), _ => None }).unwrap();
        assert_eq!(clean, Action::Copy("https://other.example/x?id=1".into()));
        let tab = p.iter().find_map(|e| match e { Entry::Item(i) if i.id == "open-tab" => Some(i.action.clone()), _ => None }).unwrap();
        assert_eq!(tab, Action::Open { url: ctx.link_url.clone(), how: How::Tab { select: false } }, "background by default");
    }

    #[test]
    fn a_routed_link_leads_with_where_it_would_go() {
        let mut ctx = page();
        ctx.link_url = "https://www.figma.com/design/x".into();
        ctx.link_route = Some((crate::router::Action::App("figma".into()), "Figma".into()));
        let p = plan(&ctx, &Targets::default(), &Options::default());
        assert!(matches!(&p[0], Entry::Item(i) if i.id == "open-routed" && i.label == "Open in Figma" && i.icon == Some("app-window")), "{:?}", p[0]);
        assert_eq!(ids(&p)[1], "open-tab");
    }

    #[test]
    fn an_image_offers_image_items_and_search_only_when_configured() {
        let mut ctx = page();
        ctx.media = Media::Image;
        ctx.source_url = "https://img.example/a b.png".into();
        let off = ids(&plan(&ctx, &Targets::default(), &Options::default()));
        for id in ["open-image", "save-image", "copy-image", "copy-image-address"] {
            assert!(off.contains(&id), "{id} in {off:?}");
        }
        assert!(!off.contains(&"search-image"), "{off:?}");
        let opts = Options { image_search_url: Some("https://search.example/?u={url}".into()), ..Options::default() };
        let p = plan(&ctx, &Targets::default(), &opts);
        let search = p.iter().find_map(|e| match e { Entry::Item(i) if i.id == "search-image" => Some(i.action.clone()), _ => None }).expect("search item");
        assert_eq!(search, Action::Open { url: "https://search.example/?u=https%3A%2F%2Fimg.example%2Fa%20b.png".into(), how: How::Tab { select: true } });
    }

    #[test]
    fn an_image_that_is_a_link_has_both_groups() {
        let mut ctx = page();
        ctx.media = Media::Image;
        ctx.source_url = "https://img.example/a.png".into();
        ctx.link_url = "https://x.example/".into();
        let got = ids(&plan(&ctx, &Targets::default(), &Options::default()));
        let (link, image) = (got.iter().position(|i| *i == "open-tab").unwrap(), got.iter().position(|i| *i == "open-image").unwrap());
        assert!(link < image && got.contains(&"copy-link") && got.contains(&"copy-image"), "{got:?}");
    }

    #[test]
    fn selected_text_offers_search_and_a_link_to_it() {
        let mut ctx = page();
        ctx.selection = "  some   selected text  ".into();
        let p = plan(&ctx, &Targets::default(), &Options::default());
        well_formed(&p);
        let got = ids(&p);
        for id in ["copy", "search-web", "copy-text-link", "select-all"] {
            assert!(got.contains(&id), "{id} in {got:?}");
        }
        assert!(!got.contains(&"open-selection"));
        let search = p.iter().find_map(|e| match e { Entry::Item(i) if i.id == "search-web" => Some(i.clone()), _ => None }).unwrap();
        assert_eq!(search.label, "Search for \u{201c}some selected text\u{201d}");
        assert_eq!(search.action, Action::Open { url: "https://duckduckgo.com/?q=some%20selected%20text".into(), how: How::Tab { select: true } });
        ctx.selection = "example.com/path".into();
        ctx.selection_url = Some("https://example.com/path".into());
        assert!(ids(&plan(&ctx, &Targets::default(), &Options::default())).contains(&"open-selection"));
    }

    #[test]
    fn an_editable_field_has_the_edit_items_with_their_state() {
        let mut ctx = page();
        ctx.editable = true;
        ctx.edit = EditState { paste: true, select_all: true, ..EditState::default() };
        ctx.spelling = vec![(200, "hello".into())];
        ctx.add_word = Some(206);
        let p = plan(&ctx, &Targets::default(), &Options::default());
        well_formed(&p);
        let got = ids(&p);
        assert_eq!(&got[..2], &["spelling", "add-word"]);
        for id in ["undo", "redo", "cut", "copy", "paste", "delete", "select-all"] {
            assert!(got.contains(&id), "{id} in {got:?}");
        }
        let enabled = |id: &str| p.iter().find_map(|e| match e { Entry::Item(i) if i.id == id => Some(i.enabled), _ => None }).unwrap();
        assert!(enabled("paste") && !enabled("cut") && !enabled("undo"));
    }

    #[test]
    fn media_has_playback_items_that_follow_its_state() {
        let mut ctx = page();
        ctx.media = Media::Video;
        ctx.source_url = "https://v.example/a.mp4".into();
        ctx.media_state = MediaState { paused: true, has_audio: true, can_save: true, can_toggle_controls: true, ..MediaState::default() };
        let p = plan(&ctx, &Targets::default(), &Options::default());
        let label = |id: &str| p.iter().find_map(|e| match e { Entry::Item(i) if i.id == id => Some(i.label.clone()), _ => None });
        assert_eq!(label("media-play").as_deref(), Some("Play"));
        assert_eq!(label("media-mute").as_deref(), Some("Mute"));
        assert_eq!(label("save-image").as_deref(), Some("Save video as\u{2026}"));
        ctx.media_state.paused = false;
        ctx.media_state.muted = true;
        let p = plan(&ctx, &Targets::default(), &Options::default());
        let label = |id: &str| p.iter().find_map(|e| match e { Entry::Item(i) if i.id == id => Some(i.label.clone()), _ => None });
        assert_eq!((label("media-play").as_deref(), label("media-mute").as_deref()), (Some("Pause"), Some("Unmute")));
    }

    #[test]
    fn hidden_items_and_developer_items_can_be_removed() {
        let opts = Options { developer: false, hide: vec!["print".into(), "open-popup".into()], ..Options::default() };
        let got = ids(&plan(&page(), &Targets::default(), &opts));
        assert!(!got.contains(&"inspect") && !got.contains(&"view-source") && !got.contains(&"print"), "{got:?}");
        let mut ctx = page();
        ctx.link_url = "https://x.example/".into();
        let got = ids(&plan(&ctx, &Targets::default(), &opts));
        assert!(!got.contains(&"open-popup") && got.contains(&"open-tab"), "{got:?}");
        well_formed(&plan(&ctx, &Targets::default(), &opts));
    }

    #[test]
    fn a_web_app_page_cannot_be_installed_again_from_its_menu() {
        let ctx = Context { in_app: true, ..page() };
        let entries = plan(&ctx, &Targets::default(), &Options::default());
        well_formed(&entries);
        let got = ids(&entries);
        assert!(!got.contains(&"install-app") && got.contains(&"print"), "{got:?}");
    }

    #[test]
    fn a_modals_page_has_no_tab_items() {
        let mut ctx = page();
        ctx.in_modal = true;
        let got = ids(&plan(&ctx, &Targets::default(), &Options::default()));
        for id in ["bookmark", "find", "print", "save-pdf", "install-app", "send-tab"] {
            assert!(!got.contains(&id), "{id} in {got:?}");
        }
        assert!(got.contains(&"reload") && got.contains(&"copy-address"));
    }

    #[test]
    fn every_item_id_in_a_plan_is_listed() {
        let mut ctx = page();
        ctx.link_url = "https://x.example/".into();
        ctx.media = Media::Image;
        ctx.source_url = "https://i.example/a.png".into();
        ctx.selection = "text".into();
        ctx.editable = true;
        ctx.spelling = vec![(200, "w".into())];
        ctx.add_word = Some(206);
        let opts = Options { image_search_url: Some("{url}".into()), ..Options::default() };
        let targets = Targets { workspaces: vec![Target { id: "w".into(), name: "W".into(), color: None }], profiles: vec![Target { id: "p".into(), name: "P".into(), color: None }], ..Targets::default() };
        for ctx in [ctx, page()] {
            for id in ids(&plan(&ctx, &targets, &opts)) {
                assert!(id == "-" || ITEM_IDS.contains(&id), "{id} is not in ITEM_IDS");
            }
        }
    }

    #[test]
    fn every_icon_a_plan_uses_exists() {
        let mut ctx = page();
        ctx.link_url = "https://x.example/".into();
        ctx.link_route = Some((crate::router::Action::App("a".into()), "A".into()));
        ctx.selection = "text".into();
        ctx.selection_url = Some("https://s.example/".into());
        ctx.editable = true;
        ctx.spelling = vec![(200, "w".into())];
        ctx.add_word = Some(206);
        let opts = Options { image_search_url: Some("{url}".into()), ..Options::default() };
        let targets = Targets { workspaces: vec![Target { id: "w".into(), name: "W".into(), color: None }], apps: vec![Target { id: "a".into(), name: "A".into(), color: None }], profiles: vec![Target { id: "p".into(), name: "P".into(), color: None }], send_to: vec![Target { id: "w".into(), name: "W".into(), color: None }] };
        let mut plans = Vec::new();
        for (media, source) in [(Media::None, ""), (Media::Image, "https://i.example/a.png"), (Media::Video, "https://v.example/a.mp4")] {
            let mut c = ctx.clone();
            c.media = media;
            c.source_url = source.into();
            c.media_state = MediaState { has_audio: true, can_save: true, can_toggle_controls: true, ..MediaState::default() };
            plans.push(plan(&c, &targets, &opts));
            c.editable = false;
            c.selection.clear();
            c.link_url.clear();
            c.frame_url = "https://frame.example/".into();
            plans.push(plan(&c, &targets, &opts));
        }
        let mut seen = 0;
        let mut check = |icon: Option<&'static str>, label: &str| {
            if let Some(id) = icon {
                seen += 1;
                assert!(crate::icons::by_id(id).is_some(), "no icon {id} (for {label})");
            }
        };
        for p in &plans {
            for e in p {
                match e {
                    Entry::Item(i) => check(i.icon, &i.label),
                    Entry::Row(items) => items.iter().for_each(|i| check(i.icon, &i.label)),
                    Entry::Submenu { icon, label, items, .. } => {
                        check(*icon, label);
                        items.iter().for_each(|i| check(i.icon, &i.label));
                    }
                    Entry::Separator => {}
                }
            }
        }
        assert!(seen > 50, "{seen}");
    }

    #[test]
    fn clean_links_lose_tracking_and_keep_the_rest() {
        assert_eq!(clean_link("https://e.example/p?a=1&utm_source=x&fbclid=abc&b=2#frag"), "https://e.example/p?a=1&b=2#frag");
        assert_eq!(clean_link("https://e.example/p?utm_medium=x"), "https://e.example/p");
        assert_eq!(clean_link("https://e.example/p"), "https://e.example/p");
        assert_eq!(clean_link("https://www.google.com/url?q=https%3A%2F%2Fdest.example%2Fp%3Futm_campaign%3Dz%26k%3D1&sa=D"), "https://dest.example/p?k=1");
    }

    #[test]
    fn a_text_fragment_link_marks_the_selection() {
        assert_eq!(text_fragment_link("https://e.example/a#old", "hello  world").as_deref(), Some("https://e.example/a#:~:text=hello%20world"));
        assert_eq!(text_fragment_link("https://e.example/a", "a-b, c&d").as_deref(), Some("https://e.example/a#:~:text=a%2Db%2C%20c%26d"));
        let long = (1..=60).map(|n| format!("w{n}")).collect::<Vec<_>>().join(" ");
        let link = text_fragment_link("https://e.example/a", &long).unwrap();
        assert!(link.ends_with("text=w1%20w2%20w3%20w4,w57%20w58%20w59%20w60"), "{link}");
        assert_eq!(text_fragment_link("file:///tmp/a.html", "x"), None);
        assert_eq!(text_fragment_link("https://e.example/", "   "), None);
    }

    #[test]
    fn labels_are_shortened_on_word_boundaries_of_whitespace() {
        assert_eq!(shorten("short", 24), "short");
        assert_eq!(shorten("a very long selection that goes on and on", 12), "a very long\u{2026}");
    }
}
