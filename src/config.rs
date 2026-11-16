//! User configuration: one optional YAML file.
//!
//! **Every option is documented in `docs/config/` and shown in `examples/config.yaml`; keep them
//! in step with this file (see AGENTS.md).** Tests check the docs against the types in both
//! directions, and load the example with unknown keys rejected.

use crate::cli::Mode;
use serde::Deserialize;
use std::path::PathBuf;

pub const DEFAULT_HOME_PAGE: &str = "https://duckduckgo.com/";
pub const DEFAULT_SEARCH_URL: &str = "https://duckduckgo.com/?q={query}";

/// Which rendering path to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct Config {
    pub home_page: String,
    pub search_url: String,
    pub window: WindowConfig,
    pub rendering: RenderingConfig,
    pub chromium: ChromiumConfig,
    pub gpu: GpuConfig,
    pub input: InputConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct RenderingConfig {
    pub mode: RenderMode,
    pub frame_rate: i32,
    pub device_resolution: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct ChromiumConfig {
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct GpuConfig {
    pub workarounds: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct InputConfig {
    pub wheel_notch: f64,
    pub touchpad_speed: f64,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self { wheel_notch: 100.0, touchpad_speed: 3.0 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            home_page: DEFAULT_HOME_PAGE.into(),
            search_url: DEFAULT_SEARCH_URL.into(),
            window: WindowConfig::default(),
            rendering: RenderingConfig::default(),
            chromium: ChromiumConfig::default(),
            gpu: GpuConfig::default(),
            input: InputConfig::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 1280, height: 800 }
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
        let mut cfg: Config = serde_yaml_ng::from_str(text)?;
        cfg.sanitize();
        Ok(cfg)
    }

    /// Replace values that cannot work with usable ones, so a typo never leaves a broken window.
    fn sanitize(&mut self) {
        self.window.width = self.window.width.clamp(400, 16384);
        self.window.height = self.window.height.clamp(300, 16384);
        self.rendering.frame_rate = self.rendering.frame_rate.clamp(1, 120);
        self.input.wheel_notch = self.input.wheel_notch.clamp(10.0, 1000.0);
        self.input.touchpad_speed = self.input.touchpad_speed.clamp(0.1, 20.0);
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

/// Load the config. A missing file gives the defaults; an unreadable or invalid file is reported
/// on stderr and also gives the defaults, so the browser still starts.
pub fn load(path: Option<&str>) -> Config {
    let path = path.map(PathBuf::from).unwrap_or_else(default_path);
    match std::fs::read_to_string(&path) {
        Ok(text) => match Config::parse(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("config: {}: {e}; using defaults", path.display());
                Config::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => {
            eprintln!("config: cannot read {}: {e}; using defaults", path.display());
            Config::default()
        }
    }
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
        add("rendering", field_names::<RenderingConfig>());
        add("chromium", field_names::<ChromiumConfig>());
        add("gpu", field_names::<GpuConfig>());
        add("input", field_names::<InputConfig>());
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
        let roots = ["home_page", "search_url", "window", "rendering", "chromium", "gpu", "input"];
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
}
