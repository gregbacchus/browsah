//! Workspaces: a named window of tabs that belongs to a profile.
//!
//! A workspace has its own directory, `<profile>/workspaces/<id>/`, holding `workspace.yaml`
//! (hand edited, see docs/config/workspace.md). Its tabs, bookmarks and so on live in the profile's
//! database (later stages), keyed by the workspace id. All workspaces of a profile are windows of
//! the profile's one process and share its application id.

use crate::profile::{valid_id, ColorPair, Profile};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// What a page's script popup (`window.open` with window features) becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PopupPolicy {
    /// A tab of the window (a window of its own if `tabs.allow_new` is false).
    #[default]
    Tab,
    /// A modal over the window.
    Modal,
    /// Not opened.
    Block,
}

/// How a modal is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModalStyle {
    /// Over the window's content (the default).
    #[default]
    Overlay,
    /// A separate window that stays above its workspace's window.
    Window,
}

/// How modals look (`modals:` in `workspace.yaml`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct ModalsConfig {
    pub style: ModalStyle,
}

/// The contents of `workspace.yaml` that describe the workspace itself. (The file can also hold
/// configuration overrides; see `config::load_effective`.)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct WorkspaceConfig {
    /// Display name (window title). Defaults to the id.
    pub name: Option<String>,
    /// Accent colour for light and dark appearance.
    pub color: Option<ColorPair>,
    /// Which navigation controls the window shows.
    pub nav: NavConfig,
    /// Rules for the window's tabs.
    pub tabs: TabsConfig,
    /// How modals (a page opened to be viewed and closed) are shown.
    pub modals: ModalsConfig,
    /// Set when this window context is a web app (read from the app's `app.yaml`, see `webapp.rs`; never from a
    /// `workspace.yaml`).
    #[serde(skip)]
    pub app: Option<crate::webapp::AppConfig>,
}

/// How the address bar behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressBar {
    /// Type to navigate (the default).
    #[default]
    Editable,
    /// Shows the address but cannot be changed.
    Readonly,
    /// Not shown; the page title is shown instead.
    Hidden,
}

/// Which navigation controls the window shows (`nav:` in `workspace.yaml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct NavConfig {
    pub back: bool,
    pub forward: bool,
    pub reload: bool,
    pub address_bar: AddressBar,
    pub new_tab_button: bool,
    pub tab_bar: bool,
    /// Bookmarks: the star button, `Ctrl+D`, and the bar (shown when there are bookmarks).
    pub bookmarks_bar: bool,
}

impl Default for NavConfig {
    fn default() -> Self {
        Self { back: true, forward: true, reload: true, address_bar: AddressBar::Editable, new_tab_button: true, tab_bar: true, bookmarks_bar: true }
    }
}

/// Rules for a workspace's tabs (`tabs:` in `workspace.yaml`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct TabsConfig {
    /// Whether tabs can be opened from the window itself. When false, tabs only arrive by being
    /// sent to the workspace, and pages that open windows get an ephemeral window instead.
    pub allow_new: bool,
    /// What a popup opened by a page's script becomes (`tab`, `modal` or `block`).
    pub popups: PopupPolicy,
    /// Tabs that are always present, first, and cannot be closed.
    pub pinned: Vec<PinnedTab>,
}

impl Default for TabsConfig {
    fn default() -> Self {
        Self { allow_new: true, popups: PopupPolicy::Tab, pinned: Vec::new() }
    }
}

/// A tab that is always present (`tabs.pinned` in `workspace.yaml`).
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct PinnedTab {
    pub url: String,
    /// Label until the page gives its own title.
    #[serde(default)]
    pub title: Option<String>,
    /// Where the tab may navigate (see `scope_matches`). Empty: anywhere.
    #[serde(default)]
    pub scope: Vec<String>,
}

/// Does `url` match a scope pattern? A pattern is `host` or `host/path`. The host is exact, or
/// `*.example.com` for any subdomain (not `example.com` itself), or `example.com` plus its
/// subdomains with `**.example.com`. The path may contain `*` (any characters, including `/`);
/// without a path any path matches, and `host/` matches only the site root. Only http and https
/// URLs match; the scheme and port are ignored.
pub fn scope_matches(pattern: &str, url: &str) -> bool {
    let rest = match url.split_once("://") {
        Some(("http" | "https", rest)) => rest,
        _ => return false,
    };
    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(split);
    // The path ends at a query or fragment; an empty path is the root.
    let path = &tail[..tail.find(['?', '#']).unwrap_or(tail.len())];
    let path = if path.is_empty() { "/" } else { path };
    let host = authority.rsplit('@').next().unwrap_or(authority).split(':').next().unwrap_or("").to_lowercase();
    let (pat_host, pat_path) = match pattern.split_once('/') {
        Some((h, p)) => (h.to_lowercase(), Some(format!("/{p}"))),
        None => (pattern.to_lowercase(), None),
    };
    let host_ok = if let Some(base) = pat_host.strip_prefix("**.") {
        host == base || host.ends_with(&format!(".{base}"))
    } else if let Some(base) = pat_host.strip_prefix("*.") {
        host.ends_with(&format!(".{base}"))
    } else {
        host == pat_host
    };
    host_ok && pat_path.is_none_or(|p| glob(&p, path))
}

/// Match `text` against a pattern where `*` is any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(p) => rest = &rest[p + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// True if `url` is inside at least one of `patterns` (or there are none).
pub fn in_scope(patterns: &[String], url: &str) -> bool {
    patterns.is_empty() || patterns.iter().any(|p| scope_matches(p, url))
}

/// The keys of `workspace.yaml` that are not configuration overrides.
pub const IDENTITY_KEYS: &[&str] = &["name", "color", "nav", "tabs", "modals"];

#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub config: WorkspaceConfig,
    pub dir: PathBuf,
}

impl Workspace {
    /// Is this workspace a web app?
    pub fn is_app(&self) -> bool {
        self.config.app.is_some()
    }

    pub fn display_name(&self) -> &str {
        self.config.name.as_deref().unwrap_or(&self.id)
    }

    /// The file this window context reads: a workspace's `workspace.yaml`, a web app's `app.yaml`.
    pub fn config_file(&self) -> PathBuf {
        self.dir.join(if self.is_app() { "app.yaml" } else { "workspace.yaml" })
    }

    /// The workspace or the web app called `id` (their ids share one namespace: see `id_taken`).
    pub fn resolve_any(profile: &Profile, id: &str) -> Result<Workspace, String> {
        Self::resolve(profile, id).or_else(|e| Self::resolve_app(profile, id).map_err(|a| if a.starts_with("no web app") { e } else { a }))
    }

    /// Is `id` used by a workspace or a web app of the profile? (Remembered tabs, session cookies and windows
    /// are keyed by the id alone, so the two kinds may not share one.)
    pub fn id_taken(profile: &Profile, id: &str) -> bool {
        profile.workspaces_dir().join(id).join("workspace.yaml").exists() || profile.apps_dir().join(id).join("app.yaml").exists()
    }

    /// The web app called `id`: its `app.yaml` is read into the window context the shell uses.
    pub fn resolve_app(profile: &Profile, id: &str) -> Result<Workspace, String> {
        Self::resolve_app_in(&profile.apps_dir(), &profile.id, id)
    }

    fn resolve_app_in(root: &Path, profile_id: &str, id: &str) -> Result<Workspace, String> {
        if !valid_id(id) {
            return Err(format!("{id:?} is not a valid app id (lower case letters, digits, - and _, starting with a letter)"));
        }
        let dir = root.join(id);
        let file = dir.join("app.yaml");
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                let app = crate::webapp::parse_app(&text).map_err(|e| format!("{}: {e}", file.display()))?;
                Ok(Workspace { id: id.to_owned(), config: crate::webapp::window_config(app), dir })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("no web app {id:?} in profile {profile_id:?}; create it with: browsah app add {id} --url URL --profile={profile_id}"))
            }
            Err(e) => Err(format!("{}: {e}", file.display())),
        }
    }

    /// All web apps of a profile, by id.
    pub fn list_apps(profile: &Profile) -> Vec<Workspace> {
        let root = profile.apps_dir();
        let mut out: Vec<Workspace> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Self::resolve_app_in(&root, &profile.id, &e.file_name().to_string_lossy()).ok())
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn resolve(profile: &Profile, id: &str) -> Result<Workspace, String> {
        Self::resolve_in(&profile.workspaces_dir(), &profile.id, id)
    }

    fn resolve_in(root: &Path, profile_id: &str, id: &str) -> Result<Workspace, String> {
        if !valid_id(id) {
            return Err(format!("{id:?} is not a valid workspace id (lower case letters, digits, - and _, starting with a letter)"));
        }
        let dir = root.join(id);
        let file = dir.join("workspace.yaml");
        match std::fs::read_to_string(&file) {
            Ok(text) => Ok(Workspace { id: id.to_owned(), config: parse_config(&text).map_err(|e| format!("{}: {e}", file.display()))?, dir }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("no workspace {id:?} in profile {profile_id:?}; create it with: browsah workspace add {id} --profile={profile_id}"))
            }
            Err(e) => Err(format!("{}: {e}", file.display())),
        }
    }

    /// All workspaces of a profile, by id.
    pub fn list(profile: &Profile) -> Vec<Workspace> {
        Self::list_in(&profile.workspaces_dir(), &profile.id)
    }

    fn list_in(root: &Path, profile_id: &str) -> Vec<Workspace> {
        let mut out: Vec<Workspace> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Self::resolve_in(root, profile_id, &e.file_name().to_string_lossy()).ok())
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

/// Read the workspace's own settings (name, colour); overrides are read by the config layers.
pub fn parse_config(text: &str) -> Result<WorkspaceConfig, String> {
    if text.trim().is_empty() {
        return Ok(WorkspaceConfig::default());
    }
    let all: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    let mut own = serde_yaml_ng::Mapping::new();
    if let Some(map) = all.as_mapping() {
        for key in IDENTITY_KEYS {
            if let Some(v) = map.get(*key) {
                own.insert(serde_yaml_ng::Value::String((*key).to_owned()), v.clone());
            }
        }
    }
    serde_yaml_ng::from_value(serde_yaml_ng::Value::Mapping(own)).map_err(|e| e.to_string())
}

// ---- command line -----------------------------------------------------------------------------

const USAGE: &str = "usage:
  browsah workspace add <id> [--profile ID] [--name NAME] [--color \"LIGHT [DARK]\"]
  browsah workspace list [--profile ID]
  browsah workspace remove <id> [--profile ID] --yes

The profile defaults to `default`. Open a workspace with: browsah --profile=<id> --workspace=<id>";

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            return Some(v.to_owned());
        }
    }
    None
}

/// `browsah workspace ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let profile_id = flag_value(args, "--profile").unwrap_or_else(|| crate::profile::DEFAULT_ID.to_owned());
    let profile = match Profile::resolve(&profile_id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match args.first().map(String::as_str) {
        Some("add") => add(&profile, &args[1..]),
        Some("list") => {
            for w in Workspace::list(&profile) {
                let color = w.config.color.as_ref().map(|c| format!("{} {}", c.light, c.dark)).unwrap_or_else(|| "-".into());
                println!("{:<16} {:<20} {}", w.id, w.display_name(), color);
            }
            0
        }
        Some("remove") => remove(&profile, &args[1..]),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

/// The first argument that is not a flag or a flag's value (`--flag value` or `--flag=value`;
/// `--yes` and `--explain` take no value).
pub fn first_positional(args: &[String]) -> Option<&String> {
    // Skip `--flag value` pairs and `--flag=value`.
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            i += if a.contains('=') || matches!(a.as_str(), "--yes" | "--explain" | "--write" | "--remove" | "--no-manifest" | "--new-window") { 1 } else { 2 };
        } else {
            return Some(a);
        }
    }
    None
}

fn add(profile: &Profile, args: &[String]) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    if !valid_id(id) {
        eprintln!("{id:?} is not a valid workspace id (lower case letters, digits, - and _, starting with a letter)");
        return 2;
    }
    let dir = profile.workspaces_dir().join(id);
    if Workspace::id_taken(profile, id) {
        eprintln!("{id:?} already exists in profile {:?} (as a workspace or a web app)", profile.id);
        return 1;
    }
    let color = match flag_value(args, "--color").map(|c| ColorPair::parse(&c)) {
        Some(Err(e)) => {
            eprintln!("--color: {e}");
            return 2;
        }
        Some(Ok(c)) => Some(c),
        None => None,
    };
    let mut text = String::from("# Workspace configuration. Options: docs/config/workspace.md\n");
    if let Some(n) = flag_value(args, "--name") {
        text.push_str(&format!("name: \"{}\"\n", n.replace('\\', "\\\\").replace('"', "\\\"")));
    }
    if let Some(c) = &color {
        text.push_str(&format!("color:\n  light: \"{}\"\n  dark: \"{}\"\n", c.light, c.dark));
    }
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("workspace.yaml"), text)) {
        eprintln!("cannot create workspace: {e}");
        return 1;
    }
    println!("created workspace {id:?} in profile {:?} ({})", profile.id, dir.display());
    println!("open it with: browsah --profile={} --workspace={id}", profile.id);
    0
}

fn remove(profile: &Profile, args: &[String]) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let w = match Workspace::resolve(profile, id) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if crate::profile::is_running(profile) {
        eprintln!("profile {:?} is running; close it first", profile.id);
        return 1;
    }
    if !args.iter().any(|a| a == "--yes") {
        eprintln!("this DELETES workspace {id:?}: {} (its configuration and, once stored there, its tabs and bookmarks).\nrun again with --yes to continue", w.dir.display());
        return 1;
    }
    if let Ok(store) = crate::store::Store::open(&profile.dir.join("data.sqlite")) {
        let _ = store.delete_workspace(&w.id);
    }
    if let Err(e) = std::fs::remove_dir_all(&w.dir) {
        eprintln!("cannot delete {}: {e}", w.dir.display());
        return 1;
    }
    println!("removed workspace {id:?}");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browsah-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resolve_and_list() {
        let root = temp("resolve");
        assert!(Workspace::resolve_in(&root, "p", "mail").unwrap_err().contains("browsah workspace add mail --profile=p"));
        assert!(Workspace::resolve_in(&root, "p", "Bad Id").is_err());
        std::fs::create_dir_all(root.join("mail")).unwrap();
        std::fs::write(root.join("mail/workspace.yaml"), "name: Mail\ncolor: '#112233'\nsearch_url: 'https://x/?q={query}'\n").unwrap();
        std::fs::create_dir_all(root.join("zeta")).unwrap();
        std::fs::write(root.join("zeta/workspace.yaml"), "").unwrap();
        std::fs::create_dir_all(root.join("stray")).unwrap(); // no workspace.yaml: not a workspace
        let w = Workspace::resolve_in(&root, "p", "mail").unwrap();
        assert_eq!(w.display_name(), "Mail");
        assert!(w.config.color.is_some());
        let ids: Vec<String> = Workspace::list_in(&root, "p").into_iter().map(|w| w.id).collect();
        assert_eq!(ids, ["mail", "zeta"]);
        std::fs::write(root.join("mail/workspace.yaml"), "color: 12\n").unwrap();
        assert!(Workspace::resolve_in(&root, "p", "mail").unwrap_err().contains("workspace.yaml"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scope_patterns() {
        let m = scope_matches;
        // Host patterns.
        assert!(m("mail.example.com", "https://mail.example.com/inbox"));
        assert!(m("mail.example.com", "http://MAIL.example.com:8080/x?y=1#z"));
        assert!(!m("mail.example.com", "https://example.com/"));
        assert!(!m("mail.example.com", "https://evil-mail.example.com/"));
        assert!(!m("mail.example.com", "https://mail.example.com.evil.net/"));
        assert!(m("*.example.com", "https://a.b.example.com/"));
        assert!(!m("*.example.com", "https://example.com/"));
        assert!(m("**.example.com", "https://example.com/"));
        assert!(m("**.example.com", "https://x.example.com/"));
        assert!(!m("**.example.com", "https://notexample.com/"));
        // Paths.
        assert!(m("example.com/app/*", "https://example.com/app/x/y"));
        assert!(m("example.com/app/*", "https://example.com/app/"));
        assert!(!m("example.com/app/*", "https://example.com/app"));
        assert!(!m("example.com/app/*", "https://example.com/other"));
        assert!(m("example.com/app*", "https://example.com/application"));
        assert!(m("example.com/*/edit", "https://example.com/doc/1/edit"));
        assert!(!m("example.com/*/edit", "https://example.com/doc/1/view"));
        assert!(m("example.com/", "https://example.com"));
        assert!(m("example.com/", "https://example.com/"));
        assert!(!m("example.com/", "https://example.com/x"));
        assert!(m("example.com", "https://example.com/anything?q=1"));
        // Query strings do not count as path; userinfo is ignored.
        assert!(m("example.com/search", "https://example.com/search?q=a/b"));
        assert!(m("example.com", "https://user:pw@example.com/"));
        // Only web URLs.
        assert!(!m("example.com", "about:blank"));
        assert!(!m("example.com", "file:///etc/passwd"));
        assert!(!m("example.com", "javascript:alert(1)"));
        // Empty scope allows everything; otherwise any pattern may match.
        assert!(in_scope(&[], "https://anywhere.net/"));
        let s = vec!["a.com".to_string(), "b.com/x/*".to_string()];
        assert!(in_scope(&s, "https://a.com/") && in_scope(&s, "https://b.com/x/1") && !in_scope(&s, "https://b.com/y"));
    }

    #[test]
    fn popups_and_modals_parse() {
        let c = parse_config("tabs: { popups: modal }\nmodals: { style: window }\n").unwrap();
        assert_eq!(c.tabs.popups, PopupPolicy::Modal);
        assert_eq!(c.modals.style, ModalStyle::Window);
        let d = parse_config("").unwrap();
        assert_eq!((d.tabs.popups, d.modals.style), (PopupPolicy::Tab, ModalStyle::Overlay));
        assert_eq!(parse_config("tabs: { popups: block }").unwrap().tabs.popups, PopupPolicy::Block);
        assert!(parse_config("tabs: { popups: sometimes }").is_err());
        assert!(parse_config("modals: { style: sideways }").is_err());
    }

    #[test]
    fn nav_and_tabs_parse() {
        let c = parse_config("nav: { back: false, address_bar: readonly, tab_bar: false }\ntabs:\n  allow_new: false\n  pinned:\n    - url: https://a.example/\n      scope: [a.example]\n    - url: https://b.example/\n").unwrap();
        assert!(!c.nav.back && c.nav.forward && c.nav.reload && c.nav.new_tab_button && !c.nav.tab_bar);
        assert_eq!(c.nav.address_bar, AddressBar::Readonly);
        assert!(!c.tabs.allow_new);
        assert_eq!(c.tabs.pinned.len(), 2);
        assert_eq!(c.tabs.pinned[0].scope, ["a.example"]);
        assert!(c.tabs.pinned[1].scope.is_empty() && c.tabs.pinned[1].title.is_none());
        assert!(parse_config("nav: { address_bar: sideways }").is_err());
        assert!(parse_config("tabs: { pinned: [ { title: no url } ] }").is_err());
        let d = parse_config("").unwrap();
        assert!(d.nav.back && d.tabs.allow_new && d.tabs.pinned.is_empty() && d.nav.address_bar == AddressBar::Editable);
    }

    #[test]
    fn docs_cover_every_workspace_option() {
        let docs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/config/workspace.md")).expect("docs/config/workspace.md exists");
        fn fields<T: serde::de::DeserializeOwned>() -> Vec<String> {
            let err = serde_yaml_ng::from_str::<T>("{ __nope__: 1 }").err().unwrap().to_string();
            err.split("expected").nth(1).unwrap_or("").split('`').skip(1).step_by(2).map(str::to_owned).collect()
        }
        let mut paths: Vec<String> = fields::<WorkspaceConfig>();
        paths.extend(fields::<NavConfig>().into_iter().map(|f| format!("nav.{f}")));
        paths.extend(fields::<TabsConfig>().into_iter().map(|f| format!("tabs.{f}")));
        paths.extend(fields::<ModalsConfig>().into_iter().map(|f| format!("modals.{f}")));
        paths.extend(fields::<PinnedTab>().into_iter().map(|f| format!("tabs.pinned[].{f}")));
        assert!(paths.len() >= 19, "{paths:?}"); // (the `sweep` options are in the layered `Config` now)
        for p in paths {
            assert!(docs.contains(&format!("| `{p}`")), "workspace option {p} is not documented in docs/config/workspace.md");
        }
    }

    #[test]
    fn example_workspace_loads() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/workspace.yaml")).expect("examples/workspace.yaml exists");
        let c = parse_config(&text).expect("the example must parse with deny_unknown_fields");
        assert!(c.name.is_some() && c.color.is_some());
    }

    #[test]
    fn positional_argument_skips_flags() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(first_positional(&a(&["--profile", "work", "mail", "--yes"])).map(String::as_str), Some("mail"));
        assert_eq!(first_positional(&a(&["--profile=work", "mail"])).map(String::as_str), Some("mail"));
        assert_eq!(first_positional(&a(&["--yes"])), None);
    }
}
