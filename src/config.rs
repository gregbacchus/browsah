//! User configuration: one optional YAML file.
//!
//! **Every option is documented in `docs/config/` and shown in `examples/config.yaml`; keep them
//! in step with this file (see AGENTS.md).** Tests check the docs against the types in both
//! directions, and load the example with unknown keys rejected.

use crate::cli::Mode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_HOME_PAGE: &str = "https://duckduckgo.com/";
pub const DEFAULT_SEARCH_URL: &str = "https://duckduckgo.com/?q={query}";

/// Which rendering path to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
    /// dmabuf frames, no copy (`OnAcceleratedPaint`).
    Accelerated,
    /// GPU readback into CPU memory (`OnPaint`). Slower; the fallback.
    Readback,
}

impl RenderMode {
    pub fn mode(self) -> Mode {
        match self {
            RenderMode::Accelerated => Mode::Accel,
            RenderMode::Readback => Mode::Paint,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct Config {
    /// Reload the config files when they change (application-only: a profile cannot set it).
    pub watch_config: bool,
    /// The profile the selector opens links in without asking (application-only).
    pub default_profile: Option<String>,
    pub home_page: String,
    /// What a new tab opens: `home` (the home page), an address, or nothing (null: a blank tab).
    pub new_tab_page: Option<String>,
    /// Where a workspace keeps cookies and site data: `shared` with the profile's other windows or `isolated`.
    pub storage: Storage,
    /// The colour scheme pages see (`prefers-color-scheme`): `system` follows the desktop, like the rest of the window.
    pub theme: Theme,
    pub search_url: String,
    pub window: WindowConfig,
    pub rendering: RenderingConfig,
    pub chromium: ChromiumConfig,
    pub gpu: GpuConfig,
    pub input: InputConfig,
    pub history: HistoryConfig,
    pub permissions: crate::permissions::PermissionsConfig,
    pub downloads: crate::downloads::DownloadsConfig,
    pub cookies: crate::cookies::CookiesConfig,
    pub print: PrintConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    /// The colour border around the page (see `FrameConfig`).
    pub frame: FrameConfig,
}

/// The border of the window's colour around the page (`window.frame`): a window with a colour (its workspace's, else its
/// profile's) shows it on the sides and bottom of the page, with rounded page corners.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct FrameConfig {
    pub enabled: bool,
    /// Thickness in logical pixels (0 is the same as off).
    pub thickness: i32,
    /// Radius of the page's corners, in logical pixels.
    pub radius: i32,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self { enabled: true, thickness: 4, radius: 8 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct RenderingConfig {
    pub mode: RenderMode,
    pub frame_rate: i32,
    pub device_resolution: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct ChromiumConfig {
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct GpuConfig {
    pub workarounds: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct InputConfig {
    pub wheel_notch: f64,
    pub touchpad_speed: f64,
    pub touchpad_acceleration: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct HistoryConfig {
    pub enabled: bool,
    pub retention_days: u32,
    pub exclude: Vec<String>,
    /// Offer bookmarks, open tabs and history while an address is typed.
    pub suggest: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { enabled: true, retention_days: 365, exclude: Vec::new(), suggest: true }
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self { wheel_notch: 100.0, touchpad_speed: 6.0, touchpad_acceleration: 0.025 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_config: true,
            default_profile: None,
            home_page: DEFAULT_HOME_PAGE.into(),
            new_tab_page: None,
            storage: Storage::Shared,
            theme: Theme::System,
            search_url: DEFAULT_SEARCH_URL.into(),
            window: WindowConfig::default(),
            rendering: RenderingConfig::default(),
            chromium: ChromiumConfig::default(),
            gpu: GpuConfig::default(),
            input: InputConfig::default(),
            history: HistoryConfig::default(),
            permissions: Default::default(),
            downloads: Default::default(),
            cookies: Default::default(),
            print: Default::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 1280, height: 800, frame: FrameConfig::default() }
    }
}

impl Default for RenderingConfig {
    fn default() -> Self {
        Self { mode: RenderMode::Accelerated, frame_rate: 60, device_resolution: true }
    }
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self { workarounds: true }
    }
}

impl Config {
    /// Parse YAML text. Unknown keys are ignored in release builds so old and new files keep
    /// loading; test builds reject them.
    pub fn parse(text: &str) -> Result<Config, serde_yaml_ng::Error> {
        if text.trim().is_empty() {
            return Ok(Config::default());
        }
        let mut value: Value = serde_yaml_ng::from_str(text)?;
        strip_routing(&mut value); // read by the router, not part of the options
        let mut cfg: Config = serde_yaml_ng::from_value(value)?;
        cfg.sanitize();
        Ok(cfg)
    }

    /// The address a new tab opens, or `None` for a blank tab. `home` means the home page; an empty
    /// value means blank.
    pub fn new_tab_url(&self) -> Option<String> {
        match self.new_tab_page.as_deref().map(str::trim) {
            None | Some("") => None,
            Some("home") => Some(self.home_page.clone()),
            Some(other) => Some(other.to_owned()),
        }
    }

    /// Replace values that cannot work with usable ones, so a typo never leaves a broken window.
    fn sanitize(&mut self) {
        self.window.width = self.window.width.clamp(400, 16384);
        self.window.height = self.window.height.clamp(300, 16384);
        self.window.frame.thickness = self.window.frame.thickness.clamp(0, 40);
        self.window.frame.radius = self.window.frame.radius.clamp(0, 40);
        self.rendering.frame_rate = self.rendering.frame_rate.clamp(1, 120);
        self.input.wheel_notch = self.input.wheel_notch.clamp(10.0, 1000.0);
        self.input.touchpad_speed = self.input.touchpad_speed.clamp(0.1, 20.0);
        self.input.touchpad_acceleration = self.input.touchpad_acceleration.clamp(0.0, 1.0);
        if !self.search_url.contains("{query}") {
            eprintln!("config: search_url has no {{query}} placeholder; using the default");
            self.search_url = DEFAULT_SEARCH_URL.into();
        }
        if self.home_page.trim().is_empty() {
            self.home_page = DEFAULT_HOME_PAGE.into();
        }
    }
}

/// `$XDG_CONFIG_HOME/browsah/config.yaml` (`~/.config/browsah/config.yaml` by default).
pub fn default_path() -> PathBuf {
    gtk::glib::user_config_dir().join("browsah").join("config.yaml")
}

// ---- layers -----------------------------------------------------------------------------------
//
// The effective configuration is the application file (`config.yaml`) with the profile's file
// (`profile.yaml`) merged over it. Rules, the same at every level:
// - maps merge key by key, recursively;
// - a scalar or a list in the upper layer replaces the lower one;
// - a key ending in `+` (`flags+:`) appends the upper list to the lower one instead;
// - application-only options (`APP_ONLY`) are ignored, with a warning, in a profile.
//
// Process-wide options (`chromium`, `gpu`, rendering mode) can be set per profile because a
// profile is its own process.

use serde_yaml_ng::Value;

/// Keys of `profile.yaml` that describe the profile itself and are not configuration.
pub const PROFILE_IDENTITY_KEYS: &[&str] = &["name", "color", "default_workspace"];

/// Merge `over` onto `base` using the rules above.
pub fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Mapping(b), Value::Mapping(o)) => {
            for (k, v) in o {
                let key = k.as_str().unwrap_or("");
                if let Some(plain) = key.strip_suffix('+') {
                    let plain = Value::String(plain.to_owned());
                    match (b.get_mut(&plain), v) {
                        (Some(Value::Sequence(bs)), Value::Sequence(vs)) => bs.extend(vs.iter().cloned()),
                        _ => {
                            b.insert(plain, v.clone());
                        }
                    }
                } else if let Some(existing) = b.get_mut(k) {
                    merge(existing, v);
                } else {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// The paths (`rendering.frame_rate`) of every leaf in a mapping; a list is one leaf.
pub fn leaf_paths(v: &Value) -> Vec<String> {
    fn walk(prefix: &str, v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Mapping(m) => {
                for (k, v) in m {
                    let k = k.as_str().unwrap_or("?").trim_end_matches('+');
                    walk(&if prefix.is_empty() { k.to_owned() } else { format!("{prefix}.{k}") }, v, out);
                }
            }
            _ => out.push(prefix.to_owned()),
        }
    }
    let mut out = Vec::new();
    walk("", v, &mut out);
    out.sort();
    out
}

/// A configuration together with where it came from.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    /// Options each layer above the application file set: (layer name, dotted option paths).
    pub overrides: Vec<(String, Vec<String>)>,
    /// Things worth telling the user (ignored options).
    pub warnings: Vec<String>,
}

impl Loaded {
    pub fn defaults() -> Loaded {
        Loaded { config: Config::default(), overrides: Vec::new(), warnings: Vec::new() }
    }

    /// The options set by the layer called `layer` (`"profile"`, `"workspace"`).
    pub fn overrides_of(&self, layer: &str) -> &[String] {
        self.overrides.iter().find(|(l, _)| l == layer).map(|(_, p)| p.as_slice()).unwrap_or(&[])
    }
}

/// Options that only the application file may set.
/// The colour scheme web pages are shown in (what `prefers-color-scheme` says).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the desktop's light or dark setting, as the rest of the window does (also while running).
    #[default]
    System,
    Light,
    Dark,
}

/// Paper size for printing and saving as PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    #[default]
    A4,
    Letter,
}

impl Paper {
    /// Width and height in inches.
    pub fn inches(self) -> (f64, f64) {
        match self {
            Paper::A4 => (8.27, 11.69),
            Paper::Letter => (8.5, 11.0),
        }
    }
}

/// The `print:` section: printing and saving a page as a PDF.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct PrintConfig {
    pub paper: Paper,
    /// Print background colours and images.
    pub backgrounds: bool,
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self { paper: Paper::A4, backgrounds: true }
    }
}

/// Whether a workspace's cookies and site data are kept apart from the rest of the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Storage {
    #[default]
    Shared,
    Isolated,
}

pub const APP_ONLY: &[&str] = &["watch_config", "default_profile"];

/// Options fixed per process, so a workspace (a window of the process) cannot set them; a profile
/// (which is a process) can.
#[cfg_attr(not(test), allow(dead_code))] // documentation of what `WORKSPACE_IGNORED` contains
pub const PROCESS_ONLY: &[&str] = &["chromium", "gpu", "rendering.mode", "rendering.device_resolution"];

/// What a workspace file may not set: the application-only and the process-only options.
pub(crate) const WORKSPACE_IGNORED: &[&str] = &["watch_config", "default_profile", "chromium", "gpu", "rendering.mode", "rendering.device_resolution"];

/// One layer merged over the application file.
pub struct Layer<'a> {
    pub name: &'static str,
    pub file: &'a std::path::Path,
    /// Keys that describe the thing itself (`name`, `color`), not configuration.
    pub identity: &'a [&'a str],
    /// Dotted option paths this layer may not set; ignored with a warning.
    pub ignored: &'a [&'a str],
}

/// Remove a dotted path from a mapping; true if something was removed.
fn remove_path(v: &mut Value, path: &str) -> bool {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let Value::Mapping(m) = v else { return false };
    match rest {
        None => m.remove(head).is_some(),
        Some(r) => m.get_mut(head).is_some_and(|child| remove_path(child, r)),
    }
}

fn read_yaml(path: &std::path::Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Value::Mapping(Default::default())),
        Ok(text) => {
            let v: Value = serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            if v.is_mapping() || v.is_null() {
                Ok(if v.is_null() { Value::Mapping(Default::default()) } else { v })
            } else {
                Err(format!("{}: expected a mapping of options", path.display()))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Mapping(Default::default())),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// `routing:` is read by the router (`router.rs`), not merged like options, so it is not part of
/// the configuration.
fn strip_routing(v: &mut Value) {
    if let Value::Mapping(m) = v {
        m.remove("routing");
    }
}

/// Build the effective configuration from the application file with `layers` merged over it, in
/// order. A missing file is empty. An invalid file is an error naming it (the caller decides
/// whether to fall back to defaults or keep what it has).
pub fn load_layers(app_file: &std::path::Path, layers: &[Layer]) -> Result<Loaded, String> {
    let mut merged = read_yaml(app_file)?;
    strip_routing(&mut merged);
    let mut overrides = Vec::new();
    let mut warnings = Vec::new();
    let mut last_set = app_file;
    for layer in layers {
        let mut over = read_yaml(layer.file)?;
        strip_routing(&mut over);
        for k in layer.identity {
            if let Value::Mapping(m) = &mut over {
                m.remove(*k);
            }
        }
        for path in layer.ignored {
            if remove_path(&mut over, path) {
                warnings.push(format!("{}: `{path}` cannot be set in a {} file; ignored", layer.file.display(), layer.name));
            }
        }
        let paths = leaf_paths(&over);
        if !paths.is_empty() {
            last_set = layer.file;
            overrides.push((layer.name.to_owned(), paths));
        }
        merge(&mut merged, &over);
    }
    let mut config: Config = serde_yaml_ng::from_value(merged).map_err(|e| format!("{}: {e}", last_set.display()))?;
    config.sanitize();
    Ok(Loaded { config, overrides, warnings })
}

/// The profile layer, as a `Layer`.
pub fn profile_layer(file: &std::path::Path) -> Layer<'_> {
    Layer { name: "profile", file, identity: PROFILE_IDENTITY_KEYS, ignored: APP_ONLY }
}

/// The workspace layer, as a `Layer`.
pub fn workspace_layer(file: &std::path::Path) -> Layer<'_> {
    Layer { name: "workspace", file, identity: crate::workspace::IDENTITY_KEYS, ignored: WORKSPACE_IGNORED }
}

/// The application file with the profile's file merged over it (the configuration of the process).
pub fn load_effective(app_file: &std::path::Path, profile_file: Option<&std::path::Path>) -> Result<Loaded, String> {
    match profile_file {
        Some(pf) => load_layers(app_file, &[profile_layer(pf)]),
        None => load_layers(app_file, &[]),
    }
}

/// The configuration of one window: application, profile, then the workspace's file if it has one.
pub fn load_window(app_file: &std::path::Path, profile_file: &std::path::Path, workspace_file: Option<&std::path::Path>) -> Result<Loaded, String> {
    match workspace_file {
        Some(wf) => load_layers(app_file, &[profile_layer(profile_file), workspace_layer(wf)]),
        None => load_layers(app_file, &[profile_layer(profile_file)]),
    }
}

/// `load_effective` for start-up: problems are reported and defaults used, so a typo never
/// leaves the browser unable to start.
pub fn load_for_startup(app_file: &std::path::Path, profile_file: Option<&std::path::Path>) -> Loaded {
    match load_effective(app_file, profile_file) {
        Ok(l) => {
            for w in &l.warnings {
                eprintln!("config: {w}");
            }
            l
        }
        Err(e) => {
            eprintln!("config: {e}; using defaults");
            Loaded::defaults()
        }
    }
}

/// What a reload changes, by dotted option path.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Options that take effect immediately.
    pub live: Vec<String>,
    /// Options that only take effect at the next start.
    pub restart: Vec<String>,
}

/// Options that are fixed when the process or a window starts.
const RESTART_REQUIRED: &[&str] = &["chromium.", "gpu.", "rendering.mode", "rendering.device_resolution", "window.width", "window.height"];

pub fn changes(old: &Config, new: &Config) -> Changes {
    let flat = |c: &Config| -> std::collections::BTreeMap<String, Value> {
        let v = serde_yaml_ng::to_value(c).unwrap_or(Value::Null);
        let mut out = std::collections::BTreeMap::new();
        for path in leaf_paths(&v) {
            let mut cur = &v;
            for part in path.split('.') {
                cur = cur.get(part).unwrap_or(&Value::Null);
            }
            out.insert(path, cur.clone());
        }
        out
    };
    let (a, b) = (flat(old), flat(new));
    let mut ch = Changes::default();
    for (k, v) in &b {
        if a.get(k) != Some(v) {
            if RESTART_REQUIRED.iter().any(|r| k == r || (r.ends_with('.') && k.starts_with(r))) {
                ch.restart.push(k.clone());
            } else {
                ch.live.push(k.clone());
            }
        }
    }
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field names of a config struct, read from serde's "unknown field" error (the structs deny
    /// unknown fields in test builds).
    fn field_names<T: serde::de::DeserializeOwned>() -> Vec<String> {
        let err = serde_yaml_ng::from_str::<T>("{ __nope__: 1 }")
            .err()
            .expect("config structs deny unknown fields in test builds")
            .to_string();
        let after = err.split("expected").nth(1).unwrap_or("");
        after.split('`').skip(1).step_by(2).map(str::to_owned).collect()
    }

    /// Every config option as a dotted path (`rendering.frame_rate`). When you add a config
    /// struct, add it here (see AGENTS.md).
    fn all_option_paths() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |prefix: &str, fields: Vec<String>| {
            for f in fields {
                out.push(if prefix.is_empty() { f } else { format!("{prefix}.{f}") });
            }
        };
        add("", field_names::<Config>());
        add("window", field_names::<WindowConfig>());
        add("window.frame", field_names::<FrameConfig>());
        add("rendering", field_names::<RenderingConfig>());
        add("chromium", field_names::<ChromiumConfig>());
        add("gpu", field_names::<GpuConfig>());
        add("input", field_names::<InputConfig>());
        add("history", field_names::<HistoryConfig>());
        add("permissions", field_names::<crate::permissions::PermissionsConfig>());
        add("downloads", field_names::<crate::downloads::DownloadsConfig>());
        add("cookies", field_names::<crate::cookies::CookiesConfig>());
        add("print", field_names::<PrintConfig>());
        add("cookies.rules[]", field_names::<crate::cookies::CookieRuleConfig>());
        add("permissions.sites[]", field_names::<crate::permissions::SiteConfig>());
        out
    }

    fn docs_text() -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/config");
        let mut files = Vec::new();
        for e in std::fs::read_dir(&root).expect("docs/config exists") {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "md") {
                files.push(p);
            }
        }
        files.sort();
        files.iter().map(|f| std::fs::read_to_string(f).unwrap()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn docs_cover_every_config_option() {
        let docs = docs_text();
        let missing: Vec<String> =
            all_option_paths().into_iter().filter(|p| !docs.contains(&format!("`{p}`"))).collect();
        assert!(
            missing.is_empty(),
            "options missing from docs/config/*.md (document each as `path` in a table row): {missing:#?}"
        );
    }

    #[test]
    fn docs_do_not_describe_options_that_do_not_exist() {
        let known: std::collections::HashSet<String> = all_option_paths().into_iter().collect();
        let roots = ["watch_config", "default_profile", "home_page", "new_tab_page", "storage", "theme", "search_url", "window", "rendering", "chromium", "gpu", "input", "history", "permissions", "downloads", "cookies", "print"];
        let mut unknown = Vec::new();
        for line in docs_text().lines().filter(|l| l.starts_with("| `")) {
            let Some(path) = line[3..].split('`').next() else { continue };
            let root = path.split(['.', '[']).next().unwrap_or("");
            if roots.contains(&root) && !known.contains(path) {
                unknown.push(path.to_owned());
            }
        }
        assert!(unknown.is_empty(), "documented options that don't exist (renamed or removed?): {unknown:#?}");
    }

    #[test]
    fn example_config_loads_with_unknown_keys_rejected() {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/config.yaml"))
            .expect("examples/config.yaml exists");
        let cfg = Config::parse(&text).expect("the example must parse with deny_unknown_fields");
        assert!(cfg.search_url.contains("{query}"));
    }

    /// Every key path in a YAML value (`window.frame.enabled`, `cookies.rules[].action`); list entries
    /// share one `[]` path, and a trailing `+` (append) is dropped.
    fn yaml_paths(v: &serde_yaml_ng::Value, prefix: &str, out: &mut std::collections::HashSet<String>) {
        use serde_yaml_ng::Value;
        match v {
            Value::Mapping(m) => {
                for (k, v) in m {
                    let k = k.as_str().unwrap_or("").trim_end_matches('+');
                    let path = if prefix.is_empty() { k.to_owned() } else { format!("{prefix}.{k}") };
                    out.insert(path.clone());
                    yaml_paths(v, &path, out);
                }
            }
            Value::Sequence(items) => {
                for i in items {
                    yaml_paths(i, &format!("{prefix}[]"), out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn example_config_shows_every_option() {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/config.yaml")).unwrap();
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
        let mut shown = std::collections::HashSet::new();
        yaml_paths(&value, "", &mut shown);
        let missing: Vec<String> = all_option_paths().into_iter().filter(|p| !shown.contains(p)).collect();
        assert!(missing.is_empty(), "examples/config.yaml does not show these options (add each, with a comment): {missing:#?}");
    }

    #[test]
    fn examples_show_every_profile_workspace_app_routing_and_sweep_option() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let mut shown = std::collections::HashSet::new();
        for f in ["config.yaml", "profile.yaml", "workspace.yaml", "webapp.yaml"] {
            let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&std::fs::read_to_string(root.join(f)).unwrap()).unwrap();
            yaml_paths(&v, "", &mut shown);
        }
        let mut want: Vec<String> = Vec::new();
        let mut add = |prefix: &str, fields: Vec<String>| want.extend(fields.into_iter().map(|f| if prefix.is_empty() { f } else { format!("{prefix}.{f}") }));
        add("", field_names::<crate::profile::ProfileConfig>());
        add("", field_names::<crate::workspace::WorkspaceConfig>());
        add("nav", field_names::<crate::workspace::NavConfig>());
        add("tabs", field_names::<crate::workspace::TabsConfig>());
        add("tabs.pinned[]", field_names::<crate::workspace::PinnedTab>());
        add("modals", field_names::<crate::workspace::ModalsConfig>());
        add("app", field_names::<crate::workspace::AppConfig>());
        add("sweep", field_names::<crate::sweep::SweepConfig>());
        add("sweep.rules[]", field_names::<crate::sweep::RuleConfig>());
        add("sweep.rules[].match", field_names::<crate::sweep::MatchConfig>());
        add("routing", field_names::<crate::router::RoutingConfig>());
        add("routing.rules[]", field_names::<crate::router::RuleConfig>());
        add("routing.rules[].match", field_names::<crate::router::MatchConfig>());
        add("routing.unwrap[]", field_names::<crate::router::UnwrapConfig>());
        let missing: Vec<String> = want.into_iter().filter(|p| !shown.contains(p)).collect();
        assert!(missing.is_empty(), "no file in examples/ shows these options (add each, with a comment): {missing:#?}");
    }

    #[test]
    fn empty_and_missing_keys_give_defaults() {
        let d = Config::default();
        for text in ["", "   \n", "{}", "window: {}", "rendering:\n  frame_rate: 30\n"] {
            let c = Config::parse(text).unwrap();
            assert_eq!(c.home_page, d.home_page, "{text:?}");
            assert_eq!(c.window.width, d.window.width, "{text:?}");
            assert_eq!(c.gpu.workarounds, d.gpu.workarounds, "{text:?}");
        }
        assert_eq!(Config::parse("rendering:\n  frame_rate: 30\n").unwrap().rendering.frame_rate, 30);
    }

    #[test]
    fn unusable_values_are_replaced() {
        let c = Config::parse("window: { width: 5, height: 99999 }\nrendering: { frame_rate: 0 }\nsearch_url: 'https://x'\nhome_page: ''\n").unwrap();
        assert_eq!(c.window.width, 400);
        assert_eq!(c.window.height, 16384);
        assert_eq!(c.rendering.frame_rate, 1);
        assert_eq!(c.search_url, DEFAULT_SEARCH_URL);
        assert_eq!(c.home_page, DEFAULT_HOME_PAGE);
    }

    #[test]
    fn render_mode_names() {
        let c = Config::parse("rendering: { mode: readback }").unwrap();
        assert_eq!(c.rendering.mode.mode(), Mode::Paint);
        assert_eq!(Config::default().rendering.mode.mode(), Mode::Accel);
        assert!(Config::parse("rendering: { mode: bogus }").is_err());
    }

    // ---- layers ----

    fn yaml(t: &str) -> Value {
        serde_yaml_ng::from_str(t).unwrap()
    }

    #[test]
    fn merge_rules() {
        let mut base = yaml("a: 1\nm: { x: 1, y: 2 }\nl: [1, 2]\nk: [1]\n");
        merge(&mut base, &yaml("a: 5\nm: { y: 9, z: 3 }\nl: [7]\nk+: [2, 3]\nnew+: [4]\n"));
        assert_eq!(base, yaml("a: 5\nm: { x: 1, y: 9, z: 3 }\nl: [7]\nk: [1, 2, 3]\nnew: [4]\n"));
        // A scalar replaces a map and vice versa.
        let mut b = yaml("a: { x: 1 }");
        merge(&mut b, &yaml("a: 2"));
        assert_eq!(b, yaml("a: 2"));
    }

    #[test]
    fn leaf_paths_of_overrides() {
        assert_eq!(leaf_paths(&yaml("a: 1\nm: { x: 1, y: { z: 2 } }\nl+: [1]\n")), ["a", "l", "m.x", "m.y.z"]);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browsah-cfg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn profile_overrides_application_config() {
        let d = temp_dir("layers");
        let (app, prof) = (d.join("config.yaml"), d.join("profile.yaml"));
        std::fs::write(&app, "search_url: 'https://a/?q={query}'\ninput: { touchpad_speed: 4.0 }\nchromium: { flags: ['--one'] }\nwatch_config: true\n").unwrap();
        std::fs::write(&prof, "name: Work\ncolor: '#123456'\nsearch_url: 'https://b/?q={query}'\ninput: { wheel_notch: 80 }\nchromium: { 'flags+': ['--two'] }\nwatch_config: false\n").unwrap();
        let l = load_effective(&app, Some(&prof)).unwrap();
        assert_eq!(l.config.search_url, "https://b/?q={query}"); // profile wins
        assert_eq!(l.config.input.touchpad_speed, 4.0); // application value kept
        assert_eq!(l.config.input.wheel_notch, 80.0); // profile adds
        assert_eq!(l.config.chromium.flags, ["--one", "--two"]); // appended
        assert!(l.config.watch_config, "application-only option cannot be set by a profile");
        assert!(l.warnings.iter().any(|w| w.contains("watch_config")));
        assert_eq!(l.overrides_of("profile"), ["chromium.flags", "input.wheel_notch", "search_url"]);
        // Identity keys are not configuration (they would be unknown options).
        assert!(!l.overrides_of("profile").iter().any(|p| p == "name" || p == "color"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn missing_files_are_empty_and_invalid_files_are_errors() {
        let d = temp_dir("invalid");
        let l = load_effective(&d.join("nope.yaml"), Some(&d.join("nope2.yaml"))).unwrap();
        assert_eq!(l.config.home_page, DEFAULT_HOME_PAGE);
        std::fs::write(d.join("bad.yaml"), "rendering: { frame_rate: fast }\n").unwrap();
        let e = load_effective(&d.join("bad.yaml"), None).unwrap_err();
        assert!(e.contains("bad.yaml"), "{e}");
        std::fs::write(d.join("p.yaml"), "input: { wheel_notch: [] }\n").unwrap();
        let e = load_effective(&d.join("nope.yaml"), Some(&d.join("p.yaml"))).unwrap_err();
        assert!(e.contains("p.yaml"), "error names the profile file: {e}");
        std::fs::write(d.join("list.yaml"), "- a\n- b\n").unwrap();
        assert!(load_effective(&d.join("list.yaml"), None).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn reload_changes_are_classified() {
        let a = Config::default();
        assert_eq!(changes(&a, &a.clone()), Changes::default());
        let mut b = a.clone();
        b.search_url = "https://x/?q={query}".into();
        b.input.touchpad_speed = 9.0;
        b.rendering.frame_rate = 30;
        b.chromium.flags.push("--x".into());
        b.rendering.mode = RenderMode::Readback;
        b.window.width = 900;
        b.gpu.workarounds = false;
        let c = changes(&a, &b);
        assert_eq!(c.live, ["input.touchpad_speed", "rendering.frame_rate", "search_url"]);
        assert_eq!(c.restart, ["chromium.flags", "gpu.workarounds", "rendering.mode", "window.width"]);
    }

    #[test]
    fn example_profile_overrides_apply() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let l = load_effective(&root.join("examples/config.yaml"), Some(&root.join("examples/profile.yaml"))).unwrap();
        assert!(!l.overrides_of("profile").is_empty(), "examples/profile.yaml should show overrides");
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }

    #[test]
    fn workspace_layer_and_ignored_options() {
        let d = temp_dir("ws-layer");
        let (app, prof, ws) = (d.join("config.yaml"), d.join("profile.yaml"), d.join("workspace.yaml"));
        std::fs::write(&app, "search_url: 'https://a/?q={query}'\ninput: { touchpad_speed: 4.0, wheel_notch: 60 }\n").unwrap();
        std::fs::write(&prof, "search_url: 'https://b/?q={query}'\nchromium: { flags: ['--p'] }\ninput: { wheel_notch: 80 }\n").unwrap();
        std::fs::write(&ws, "name: Mail\ncolor: '#123456'\nsearch_url: 'https://c/?q={query}'\ninput: { touchpad_speed: 9.0 }\nchromium: { flags: ['--w'] }\nrendering: { mode: readback, frame_rate: 20 }\nwatch_config: false\n").unwrap();
        let l = load_window(&app, &prof, Some(&ws)).unwrap();
        assert_eq!(l.config.search_url, "https://c/?q={query}"); // workspace over profile over application
        assert_eq!(l.config.input.touchpad_speed, 9.0); // workspace over application
        assert_eq!(l.config.input.wheel_notch, 80.0); // profile over application, untouched by workspace
        assert_eq!(l.config.rendering.frame_rate, 20); // a per-window option the workspace may set
        assert_eq!(l.config.chromium.flags, ["--p"], "process-only options cannot be set by a workspace");
        assert_eq!(l.config.rendering.mode, RenderMode::Accelerated);
        assert!(l.config.watch_config);
        assert_eq!(l.warnings.len(), 3, "{:?}", l.warnings);
        assert_eq!(l.overrides_of("workspace"), ["input.touchpad_speed", "rendering.frame_rate", "search_url"]);
        assert_eq!(l.overrides_of("profile"), ["chromium.flags", "input.wheel_notch", "search_url"]);
        // Without a workspace file the window is the profile's.
        assert_eq!(load_window(&app, &prof, None).unwrap().config.search_url, "https://b/?q={query}");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_frame_options_default_and_clamp() {
        let d = Config::default().window.frame;
        assert!(d.enabled && d.thickness == 4 && d.radius == 8, "thin by default");
        let mut c: Config = serde_yaml_ng::from_str("window: { frame: { thickness: 500, radius: -3, enabled: false } }").unwrap();
        c.sanitize();
        assert!(!c.window.frame.enabled && c.window.frame.thickness == 40 && c.window.frame.radius == 0);
        // A reload that changes only the frame is applied live, not at the next start.
        let mut new = Config::default();
        new.window.frame.thickness = 12;
        let ch = changes(&Config::default(), &new);
        assert!(ch.live.iter().any(|k| k == "window.frame.thickness") && ch.restart.is_empty(), "{ch:?}");
    }

    #[test]
    fn the_theme_defaults_to_system_and_layers() {
        assert_eq!(Config::default().theme, Theme::System);
        assert_eq!(Config::parse("theme: dark\n").unwrap().theme, Theme::Dark);
        assert!(Config::parse("theme: sepia\n").is_err());
        // A workspace's file overrides the profile's, which overrides the application's.
        let dir = std::env::temp_dir().join(format!("browsah-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (app, prof, ws) = (dir.join("config.yaml"), dir.join("profile.yaml"), dir.join("workspace.yaml"));
        std::fs::write(&app, "theme: light\n").unwrap();
        std::fs::write(&prof, "theme: dark\n").unwrap();
        std::fs::write(&ws, "name: W\n").unwrap();
        assert_eq!(load_window(&app, &prof, Some(&ws)).unwrap().config.theme, Theme::Dark, "the profile's");
        std::fs::write(&ws, "theme: system\n").unwrap();
        assert_eq!(load_window(&app, &prof, Some(&ws)).unwrap().config.theme, Theme::System, "the workspace's");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paper_sizes_in_inches() {
        assert_eq!(Paper::A4.inches(), (8.27, 11.69));
        assert_eq!(Paper::Letter.inches(), (8.5, 11.0));
        let c: Config = serde_yaml_ng::from_str("print: { paper: letter, backgrounds: false }").unwrap();
        assert!(c.print.paper == Paper::Letter && !c.print.backgrounds);
        assert!(Config::default().print.paper == Paper::A4);
    }

    #[test]
    fn what_a_new_tab_opens() {
        let url = |text: &str| Config::parse(text).unwrap().new_tab_url();
        assert_eq!(url(""), None, "blank by default");
        assert_eq!(url("new_tab_page: null"), None);
        assert_eq!(url("new_tab_page: ''"), None);
        assert_eq!(url("new_tab_page: home"), Some(DEFAULT_HOME_PAGE.into()));
        assert_eq!(url("home_page: 'https://h.example/'\nnew_tab_page: home"), Some("https://h.example/".into()));
        assert_eq!(url("new_tab_page: 'https://n.example/'"), Some("https://n.example/".into()));
        // A workspace can undo what the profile set (a null in the upper layer replaces the lower value).
        let d = temp_dir("newtab");
        let (prof, ws) = (d.join("profile.yaml"), d.join("workspace.yaml"));
        std::fs::write(&prof, "new_tab_page: home\n").unwrap();
        std::fs::write(&ws, "new_tab_page: null\n").unwrap();
        assert!(load_window(&d.join("none.yaml"), &prof, None).unwrap().config.new_tab_url().is_some());
        assert!(load_window(&d.join("none.yaml"), &prof, Some(&ws)).unwrap().config.new_tab_url().is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn workspace_ignored_list_is_application_only_plus_process_only() {
        let mut expected: Vec<&str> = APP_ONLY.to_vec();
        expected.extend(PROCESS_ONLY);
        assert_eq!(WORKSPACE_IGNORED, expected.as_slice());
    }

    #[test]
    fn remove_nested_paths() {
        let mut v = yaml("rendering: { mode: readback, frame_rate: 20 }\ngpu: { workarounds: false }\n");
        assert!(remove_path(&mut v, "rendering.mode"));
        assert!(!remove_path(&mut v, "rendering.mode"));
        assert!(remove_path(&mut v, "gpu"));
        assert_eq!(v, yaml("rendering: { frame_rate: 20 }"));
    }

    #[test]
    fn example_workspace_overrides_apply() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let l = load_window(&root.join("examples/config.yaml"), &root.join("examples/profile.yaml"), Some(&root.join("examples/workspace.yaml"))).unwrap();
        assert!(!l.overrides_of("workspace").is_empty(), "examples/workspace.yaml should show overrides");
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }
}
