//! Web apps: one site in a window of its own, with its own icon and its own entry in the desktop's launcher.
//! An app is a directory `apps/<id>/` of the profile with an `app.yaml` (not a workspace: see docs/config/webapp.md).
//! This module has the `app.yaml` schema, the desktop integration and the `browsah app` command line; the
//! window itself is the shell's ordinary window, given the controls and tab rules `window_config` derives.

use crate::profile::{self, ColorPair, Profile};
use crate::workspace::{first_positional, AddressBar, ModalsConfig, NavConfig, PinnedTab, PinnedTabSize, PopupPolicy, Workspace, WorkspaceConfig};
use serde::Deserialize;
use std::path::{Path, PathBuf};

mod update;

/// How much browser a web app shows around its page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppDisplay {
    /// Just the page, with back and reload (the default, as for an installed web app).
    #[default]
    Standalone,
    /// As standalone, plus the address, which cannot be changed.
    Minimal,
    /// The usual controls.
    Browser,
    /// The controls the app's own `nav:` section says (the same options as a workspace's).
    Custom,
}

fn yes() -> bool {
    true
}

/// The controls a web app with `display: custom` chooses (`nav:` in `app.yaml`). Not the whole of a workspace's `nav`: an
/// app has no bookmarks, and its tab bar and new-tab button follow `multiple_tabs`, so those are not options here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct AppNav {
    pub back: bool,
    pub forward: bool,
    pub reload: bool,
    pub address_bar: AddressBar,
}

impl Default for AppNav {
    fn default() -> Self {
        Self { back: true, forward: true, reload: true, address_bar: AddressBar::Editable }
    }
}

/// The contents of an app's `app.yaml` that describe the app itself. (The file can also hold configuration
/// overrides, as `workspace.yaml` can; see `config::load_layers`.)
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct AppConfig {
    /// Display name (window title and launcher entry). Defaults to the id.
    #[serde(default)]
    pub name: Option<String>,
    /// Accent colour for light and dark appearance (also the generated icon's).
    #[serde(default)]
    pub color: Option<ColorPair>,
    /// The page the app opens on (its start URL).
    pub url: String,
    /// Where the app may navigate; anything else is handled by the routing rules. Default: the start URL's site.
    #[serde(default)]
    pub scope: Vec<String>,
    /// How much of the browser's interface the window shows.
    #[serde(default)]
    pub display: AppDisplay,
    /// Which controls the window shows, with `display: custom` (back, forward, reload and the address bar, as a workspace's
    /// `nav:` has them; unset ones are shown). Ignored, with a warning, for the other displays.
    #[serde(default)]
    pub nav: Option<AppNav>,
    /// An icon file inside the app's directory (`icon.png`, `icon.svg`). Default: a generated one.
    #[serde(default)]
    pub icon: Option<String>,
    /// The web manifest's `id`, kept so a later import can tell it is the same app.
    #[serde(default)]
    pub manifest_id: Option<String>,
    /// Whether the app may have more than one window (the launcher's "New Window").
    #[serde(default = "yes")]
    pub multiple_windows: bool,
    /// Whether a window may have more than one tab.
    #[serde(default)]
    pub multiple_tabs: bool,
    /// How modals (popups and links opened to be viewed and closed) are shown.
    #[serde(default)]
    pub modals: ModalsConfig,
    /// Whether the window has a menu button with a few page tools (copy the address, find, save as PDF, print, the app's
    /// configuration).
    #[serde(default = "yes")]
    pub window_menu: bool,
}

/// The keys of `app.yaml` that are not configuration overrides.
pub const APP_IDENTITY_KEYS: &[&str] = &["name", "color", "url", "scope", "display", "nav", "icon", "manifest_id", "multiple_windows", "multiple_tabs", "modals", "window_menu"];

/// Read the app's own settings; overrides are read by the config layers.
pub fn parse_app(text: &str) -> Result<AppConfig, String> {
    let all: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    let mut own = serde_yaml_ng::Mapping::new();
    if let Some(map) = all.as_mapping() {
        for key in APP_IDENTITY_KEYS {
            if let Some(v) = map.get(*key) {
                own.insert(serde_yaml_ng::Value::String((*key).to_owned()), v.clone());
            }
        }
    }
    serde_yaml_ng::from_value(serde_yaml_ng::Value::Mapping(own)).map_err(|e| e.to_string())
}

/// The window an app implies: its start page as the one pinned tab (limited to its scope, which then applies
/// to the app), tabs only if `multiple_tabs`, popups as modals, and controls by `display`.
pub fn window_config(app: AppConfig) -> WorkspaceConfig {
    let mut c = WorkspaceConfig { name: app.name.clone(), color: app.color.clone(), modals: app.modals.clone(), ..WorkspaceConfig::default() };
    let scope = if app.scope.is_empty() { vec![crate::cookies::host_of(&app.url)] } else { app.scope.clone() };
    c.tabs.pinned.push(PinnedTab { url: app.url.clone(), title: None, size: PinnedTabSize::Icon, scope });
    c.tabs.allow_new = app.multiple_tabs;
    c.tabs.popups = PopupPolicy::Modal;
    c.nav = match app.display {
        // The app's own choice of the four controls; the tab bar follows `multiple_tabs`, and there is no new-tab button or bookmarks.
        AppDisplay::Custom => {
            let n = app.nav.clone().unwrap_or_default();
            NavConfig { back: n.back, forward: n.forward, reload: n.reload, address_bar: n.address_bar, new_tab_button: false, tab_bar: app.multiple_tabs, bookmarks: false, split_button: false }
        }
        display => NavConfig {
            address_bar: match display {
                AppDisplay::Minimal => AddressBar::Readonly,
                AppDisplay::Browser => AddressBar::Editable,
                _ => AddressBar::Hidden,
            },
            new_tab_button: false, // an app's tabs come from its links (Ctrl+click, "open in new tab"), not from a button
            tab_bar: display == AppDisplay::Browser || app.multiple_tabs,
            bookmarks: false,
            split_button: false, // an app shows one page
            ..NavConfig::default()
        },
    };
    if app.nav.is_some() && app.display != AppDisplay::Custom {
        eprintln!("browsah: the app's nav: section is only used with display: custom");
    }
    c.app = Some(app);
    c
}

/// The application id of one web app: the profile's id and the app's, which is also the name of its
/// `.desktop` file, so the desktop groups the app's window under its own launcher entry and icon.
pub fn app_id(profile: &Profile, id: &str) -> String {
    format!("{}.{}", profile.app_id(), id)
}

/// The `.desktop` entry that launches a web app.
pub fn desktop_entry(profile: &Profile, w: &Workspace, exe: &Path, libs: &Path) -> String {
    let id = app_id(profile, &w.id);
    let command = format!(
        "env LD_LIBRARY_PATH={libs} {exe} --profile={pid} --app={wid}",
        libs = profile::exec_quote(&libs.to_string_lossy()),
        exe = profile::exec_quote(&exe.to_string_lossy()),
        pid = profile.id,
        wid = w.id,
    );
    // The launcher's right-click "New Window", for an app that may have more than one window.
    let new_window = if w.config.app.as_ref().is_none_or(|a| a.multiple_windows) {
        format!("Actions=new-window;\n\n[Desktop Action new-window]\nName=New Window\nExec={command} --new-window\n")
    } else {
        String::new()
    };
    let mut entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Comment={name} (browsah, profile {profile_name})\n\
         Exec={command}\n\
         Icon={id}\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass={id}\n\
         Categories=Network;\n",
        name = w.display_name(),
        profile_name = profile.display_name(),
    );
    if !new_window.is_empty() {
        entry.push_str(&new_window);
    }
    entry
}

/// Write launcher entries again where they are out of date: the program moved (a package upgrade removes the old
/// versioned directory), an app's `multiple_windows` or name changed, the generated icon changed. Covers every profile's
/// entry and apps and the selector's. Only entries that already exist are touched, never created.
pub fn refresh_launchers(running: &Profile) {
    if running.id == profile::TEST_ID {
        return;
    }
    let Ok((exe, libs)) = profile::launcher() else { return };
    let mut profiles = profile::list();
    if !profiles.iter().any(|p| p.id == running.id) {
        profiles.push(running.clone());
    }
    let mut changed = false;
    // Rewrite a file that exists and is not what would be written now.
    let mut rewrite = |file: PathBuf, now: String| {
        if std::fs::read_to_string(&file).is_ok_and(|current| current != now) {
            changed |= std::fs::write(&file, now).is_ok();
        }
    };
    // An entry is rewritten in full for the running profile; for the others, and the selector (which another build of
    // browsah may own), only when the program it names is gone, so a build run from a source tree leaves them alone.
    let wanted = |file: &Path, own: bool| own || std::fs::read_to_string(file).is_ok_and(|t| profile::entry_program_missing(&t));
    for profile in profiles.iter().filter(|p| p.id != profile::TEST_ID) {
        let own = profile.id == running.id;
        let entry = profile::applications_dir().join(format!("{}.desktop", profile.app_id()));
        if wanted(&entry, own) {
            rewrite(entry, profile::desktop_entry(profile, &exe, &libs));
        }
        rewrite(profile::icons_dir().join(format!("{}.svg", profile.app_id())), profile::icon_svg(profile));
        for w in Workspace::list_apps(profile) {
            let file = profile::applications_dir().join(format!("{}.desktop", app_id(profile, &w.id)));
            let has_own_icon = w.config.app.as_ref().and_then(|a| a.icon.as_ref()).is_some_and(|f| w.dir.join(f).is_file());
            let icon = profile::icons_dir().join(format!("{}.svg", app_id(profile, &w.id)));
            let icon_stale = !has_own_icon && std::fs::read_to_string(&icon).is_ok_and(|current| current != generated_icon(profile, &w));
            let entry_stale = wanted(&file, own) && std::fs::read_to_string(&file).is_ok_and(|current| current != desktop_entry(profile, &w, &exe, &libs));
            if icon_stale || entry_stale {
                let _ = install(profile, &w, &exe, &libs);
            }
        }
    }
    let selector_id = crate::selector::SELECTOR_APP_ID;
    let selector_entry = profile::applications_dir().join(format!("{selector_id}.desktop"));
    if wanted(&selector_entry, false) {
        rewrite(selector_entry, crate::selector::desktop_entry(&exe, &libs));
    }
    rewrite(profile::icons_dir().join(format!("{selector_id}.svg")), crate::selector::icon_svg());
    if changed {
        let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
        profile::refresh_icon_cache();
    }
}

/// Write the app's `.desktop` entry and icon (its own icon file if it names one, else a generated one).
pub fn install(profile: &Profile, w: &Workspace, exe: &Path, libs: &Path) -> std::io::Result<()> {
    if profile.id == profile::TEST_ID {
        return Ok(()); // the self-test's throwaway profile never touches the user's launcher
    }
    let id = app_id(profile, &w.id);
    std::fs::create_dir_all(profile::applications_dir())?;
    std::fs::create_dir_all(profile::icons_dir())?;
    std::fs::write(profile::applications_dir().join(format!("{id}.desktop")), desktop_entry(profile, w, exe, libs))?;
    let own = w.config.app.as_ref().and_then(|a| a.icon.as_ref()).map(|f| w.dir.join(f)).filter(|p| p.is_file());
    remove_icons(&id);
    match own {
        Some(file) if file.extension().is_some_and(|e| e == "svg") => {
            std::fs::copy(&file, profile::icons_dir().join(format!("{id}.svg")))?;
        }
        Some(file) => {
            // A raster icon: the theme looks for it by size; this is the one we have.
            let dir = profile::icons_dir().join("../../256x256/apps");
            std::fs::create_dir_all(&dir)?;
            std::fs::copy(&file, dir.join(format!("{id}.png")))?;
        }
        None => {
            std::fs::write(profile::icons_dir().join(format!("{id}.svg")), generated_icon(profile, w))?;
        }
    }
    let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
    profile::refresh_icon_cache();
    Ok(())
}

/// The generated icon of an app: its own colour (from the manifest's theme colour or `--color`), else its profile's, else the
/// default blue.
pub fn generated_icon(profile: &Profile, w: &Workspace) -> String {
    let color = w.config.color.as_ref().or(profile.config.color.as_ref());
    crate::appicon::svg(color.map(|c| c.light.as_str()))
}

fn remove_icons(id: &str) {
    let _ = std::fs::remove_file(profile::icons_dir().join(format!("{id}.svg")));
    let _ = std::fs::remove_file(profile::icons_dir().join(format!("../../256x256/apps/{id}.png")));
}

pub fn uninstall(profile: &Profile, id: &str) {
    let app = app_id(profile, id);
    let _ = std::fs::remove_file(profile::applications_dir().join(format!("{app}.desktop")));
    remove_icons(&app);
    let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
}

// ---- command line -----------------------------------------------------------------------------

const USAGE: &str = "usage:
  browsah app add <id> --url URL [--name NAME] [--scope PATTERN[,PATTERN...]] [--display standalone|minimal|browser|custom]
                       [--color \"LIGHT [DARK]\"] [--icon FILE] [--profile ID]
  browsah app list [--profile ID]
  browsah app remove <id> [--profile ID] --yes
  browsah app install <id> [--profile ID]      (write the launcher entry and icon again)
  browsah app update <id> [--profile ID] [--apply]   (preview or apply manifest changes)
  browsah app route <id> [--profile ID] [--write | --remove]   (a routing rule that opens the app's site in the app)

`app add` reads the site's web manifest (name, start page, scope, display, theme colour, icon) like installing from a page in the
browser does; the flags override what it finds, and --no-manifest skips reading. Without <id> one is made from the name.
A web app is one site in a window of its own (docs/config/webapp.md); its files are in the profile's apps/<id>/.
The profile defaults to `default_profile` from config.yaml, else `default`. Launch one with: browsah --profile=<profile> --app=<id>, or from the desktop's launcher.";

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

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The text of a new web app's `app.yaml`.
pub fn config_text(name: Option<&str>, color: Option<&profile::ColorPair>, url: &str, scope: &[String], display: &str, icon: Option<&str>, manifest_id: Option<&str>) -> String {
    let mut t = String::from("# Web app. Options: docs/config/webapp.md\n");
    if let Some(n) = name {
        t.push_str(&format!("name: {}\n", quote(n)));
    }
    if let Some(c) = color {
        t.push_str(&format!("color:\n  light: \"{}\"\n  dark: \"{}\"\n", c.light, c.dark));
    }
    t.push_str(&format!("url: {}\n", quote(url)));
    if !scope.is_empty() {
        t.push_str(&format!("scope: [{}]\n", scope.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ")));
    }
    t.push_str(&format!("display: {display}\n"));
    if display == "custom" {
        // The controls to choose from, at the workspace defaults, for editing.
        t.push_str("nav:\n  back: true\n  forward: true\n  reload: true\n  address_bar: editable   # editable | readonly | hidden\n");
    }
    if let Some(i) = icon {
        t.push_str(&format!("icon: {}\n", quote(i)));
    }
    if let Some(i) = manifest_id {
        t.push_str(&format!("manifest_id: {}\n", quote(i)));
    }
    t
}

/// An id made from a name: lower case letters and digits, `-` between words, starting with a letter.
pub fn id_from_name(name: &str) -> String {
    let mut id: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_owned();
    if id.chars().next().is_some_and(|c| c.is_ascii_lowercase()) { id } else { format!("app-{id}").trim_end_matches('-').to_owned() }
}

/// `browsah app ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let profile_id = flag_value(args, "--profile").unwrap_or_else(crate::profile::cli_default_id);
    let profile = match Profile::resolve(&profile_id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let (exe, libs) = match profile::launcher() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot find this program: {e}");
            return 1;
        }
    };
    match args.first().map(String::as_str) {
        Some("add") => add(&profile, &args[1..], &exe, &libs),
        Some("list") => {
            for w in Workspace::list_apps(&profile) {
                let a = w.config.app.as_ref().expect("an app");
                println!("{:<16} {:<20} {}", w.id, w.display_name(), a.url);
            }
            0
        }
        Some("install") => match first_positional(&args[1..]).map(|id| Workspace::resolve_app(&profile, id)) {
            Some(Ok(w)) => match install(&profile, &w, &exe, &libs) {
                Ok(()) => {
                    println!("installed the launcher entry {}.desktop", app_id(&profile, &w.id));
                    0
                }
                Err(e) => {
                    eprintln!("cannot install: {e}");
                    1
                }
            },
            Some(Err(e)) => {
                eprintln!("{e}");
                1
            }
            None => {
                eprintln!("{USAGE}");
                2
            }
        },
        Some("update") => update::cli(&profile, &args[1..], &exe, &libs),
        Some("remove") => remove(&profile, &args[1..]),
        Some("route") => route(&profile, &args[1..]),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

// ---- making an app ------------------------------------------------------------------------------

/// What a new app is made from (by `browsah app add` and by installing from a page).
pub struct NewApp {
    pub name: Option<String>,
    /// `#rrggbb`, or two colours (`LIGHT DARK`).
    pub color: Option<String>,
    /// The start page.
    pub url: String,
    pub scope: Vec<String>,
    pub display: AppDisplay,
    /// The icon file: its extension (`png` or `svg`) and contents.
    pub icon: Option<(&'static str, Vec<u8>)>,
    pub manifest: Option<ManifestInstall>,
}

pub struct ManifestInstall {
    pub info: crate::manifest::AppInfo,
    pub page_url: String,
    pub custom_icon: bool,
}

fn display_name_of(d: AppDisplay) -> &'static str {
    match d {
        AppDisplay::Standalone => "standalone",
        AppDisplay::Minimal => "minimal",
        AppDisplay::Browser => "browser",
        AppDisplay::Custom => "custom",
    }
}

/// An id for an app that is not taken: `base`, else `base-2`, `base-3`...
fn free_id(profile: &Profile, base: &str) -> Option<String> {
    std::iter::once(base.to_owned()).chain((2..100).map(|n| format!("{base}-{n}"))).find(|c| !Workspace::id_taken(profile, c))
}

/// Create the app's directory with its `app.yaml` and icon, then its launcher entry and icon. `id` (or one made from the
/// name) is returned. Used by `browsah app add` and by installing from a page, so both make the same thing.
pub fn create(profile: &Profile, id: Option<&str>, spec: &NewApp, exe: &Path, libs: &Path) -> Result<String, String> {
    let id = match id {
        Some(id) => {
            if !profile::valid_id(id) {
                return Err(format!("{id:?} is not a valid id (lower case letters, digits, - and _, starting with a letter)"));
            }
            if Workspace::id_taken(profile, id) {
                return Err(format!("{id:?} already exists in profile {:?} (as a workspace or a web app)", profile.id));
            }
            id.to_owned()
        }
        None => {
            let name = spec.name.clone().unwrap_or_else(|| crate::cookies::host_of(&spec.url));
            free_id(profile, &id_from_name(&name)).ok_or("no free id")?
        }
    };
    let dir = profile.apps_dir().join(&id);
    let color = spec.color.as_deref().and_then(|c| profile::ColorPair::parse(c).ok());
    let icon_name = spec.icon.as_ref().map(|(ext, _)| format!("icon.{ext}"));
    let text = config_text(spec.name.as_deref(), color.as_ref(), &spec.url, &spec.scope, display_name_of(spec.display), icon_name.as_deref(), spec.manifest.as_ref().and_then(|source| source.info.manifest_id.as_deref()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::write(dir.join("app.yaml"), text).map_err(|e| e.to_string())?;
    if let (Some((_, bytes)), Some(name)) = (&spec.icon, &icon_name) {
        std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
    }
    if let Some(source) = &spec.manifest {
        update::remember_install(&dir, source, icon_name.as_deref().zip(spec.icon.as_ref().map(|(_, data)| data.as_slice())))?;
    }
    let w = Workspace::resolve_app(profile, &id)?;
    install(profile, &w, exe, libs).map_err(|e| format!("created {id:?}, but its launcher entry failed: {e}"))?;
    Ok(id)
}

// ---- reading a site without a browser ------------------------------------------------------------

/// A fetched address (after redirects) and what it said.
pub(crate) struct Fetched {
    pub(crate) url: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) fn fetch(url: &str, limit: u64) -> Result<Fetched, String> {
    use ureq::tls::{RootCerts, TlsConfig};
    use ureq::ResponseExt;
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .user_agent("browsah")
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    let final_url = response.get_uri().to_string();
    let body = response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())?;
    Ok(Fetched { url: final_url, body })
}

/// What the site at `page_url` says about itself, as installing from its page in the browser would: the page's
/// `<link rel="manifest">` is read when it has one (name, start page, scope, display, colour, icons), else only the
/// page's title and address are known.
pub fn info_from_site(page_url: &str) -> Result<crate::manifest::AppInfo, String> {
    use crate::manifest::{page_info, page_links, parse_raw_manifest};
    let page = fetch(page_url, 2_000_000)?;
    let html = String::from_utf8_lossy(&page.body);
    let (link, title) = page_links(&html, &page.url);
    let plain = || page_info(title.as_deref().unwrap_or(""), page_url);
    let Some(link) = link else { return Ok(plain()) };
    let manifest = fetch(&link, 1_000_000).map_err(|e| format!("the manifest {link}: {e}"))?;
    Ok(parse_raw_manifest(&String::from_utf8_lossy(&manifest.body), &manifest.url, &page.url).unwrap_or_else(plain))
}

/// An icon's bytes as a file the launcher can use: PNG and SVG as they are, anything else (ICO, JPEG, WebP...) that GdkPixbuf
/// can read converted to PNG.
fn icon_file(bytes: Vec<u8>) -> Option<(&'static str, Vec<u8>)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(("png", bytes));
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    if head.contains("<svg") || (head.contains("<?xml") && String::from_utf8_lossy(&bytes).to_ascii_lowercase().contains("<svg")) {
        return Some(("svg", bytes));
    }
    if bytes.starts_with(&[0, 0, 1, 0]) {
        // An ICO holds several sizes and GdkPixbuf would pick one on its own: hand it only the largest.
        if let Some(single) = largest_ico_entry(&bytes) {
            if single.starts_with(&[0x89, b'P', b'N', b'G']) {
                return Some(("png", single));
            }
            return convert_to_png(single);
        }
    }
    convert_to_png(bytes)
}

/// Decode with GdkPixbuf (which keeps the alpha channel) and write a PNG.
fn convert_to_png(bytes: Vec<u8>) -> Option<(&'static str, Vec<u8>)> {
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(bytes)).ok()?;
    pixbuf.save_to_bufferv("png", &[]).ok().map(|png| ("png", png))
}

/// The largest image of an ICO file: its PNG data as it is, or a one-image ICO (for a bitmap entry) that GdkPixbuf can read.
/// An entry's width or height byte of 0 means 256.
fn largest_ico_entry(ico: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |i: usize| ico.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |i: usize| ico.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let count = u16_at(4)?;
    let mut best: Option<(usize, usize, usize, usize)> = None; // (pixels, bits per pixel, size, offset)
    for n in 0..count {
        let e = 6 + n * 16;
        let dim = |i: usize| match ico.get(i).copied()? {
            0 => Some(256),
            v => Some(v as usize),
        };
        let (w, h) = (dim(e)?, dim(e + 1)?);
        let (bpp, size, offset) = (u16_at(e + 6)?, u32_at(e + 8)?, u32_at(e + 12)?);
        if offset.checked_add(size)? > ico.len() {
            continue;
        }
        let key = (w * h, bpp);
        if best.map_or(true, |b| key > (b.0, b.1)) {
            best = Some((w * h, bpp, size, offset));
        }
    }
    let (_, _, size, offset) = best?;
    let data = &ico[offset..offset + size];
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(data.to_vec());
    }
    // Keep that entry's directory record, with the data moved to just after a one-entry directory.
    let mut one = ico[..6].to_vec();
    one[4..6].copy_from_slice(&1u16.to_le_bytes());
    let src = ico.get(..6 + count * 16)?;
    let entry = (0..count).map(|n| &src[6 + n * 16..22 + n * 16]).find(|r| u32::from_le_bytes([r[12], r[13], r[14], r[15]]) as usize == offset)?;
    one.extend_from_slice(&entry[..12]);
    one.extend_from_slice(&22u32.to_le_bytes());
    one.extend_from_slice(data);
    Some(one)
}

/// The first of the best three icon candidates that downloads and can be used, with the address it came from.
fn fetch_icon(candidates: &[String]) -> Option<(&'static str, Vec<u8>, String)> {
    candidates.iter().take(3).find_map(|url| fetch(url, 5_000_000).ok().and_then(|f| icon_file(f.body)).map(|(ext, bytes)| (ext, bytes, url.clone())))
}

// ---- command line: add ----------------------------------------------------------------------------

fn add(profile: &Profile, args: &[String], exe: &Path, libs: &Path) -> i32 {
    let id = first_positional(args).map(String::as_str);
    let Some(page) = flag_value(args, "--url") else {
        eprintln!("--url is required\n{USAGE}");
        return 2;
    };
    if !(page.starts_with("http://") || page.starts_with("https://")) {
        eprintln!("--url must be an http or https address");
        return 2;
    }
    let display = match flag_value(args, "--display").as_deref() {
        None => None,
        Some("standalone") => Some(AppDisplay::Standalone),
        Some("minimal") => Some(AppDisplay::Minimal),
        Some("browser") => Some(AppDisplay::Browser),
        Some("custom") => Some(AppDisplay::Custom),
        Some(_) => {
            eprintln!("--display must be standalone, minimal, browser or custom");
            return 2;
        }
    };
    let color = match flag_value(args, "--color").map(|c| profile::ColorPair::parse(&c).map(|_| c)) {
        Some(Err(e)) => {
            eprintln!("--color: {e}");
            return 2;
        }
        Some(Ok(c)) => Some(c),
        None => None,
    };
    let icon_flag = match flag_value(args, "--icon") {
        Some(src) => {
            let ext = Path::new(&src).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
            if !["png", "svg"].contains(&ext.as_str()) {
                eprintln!("--icon must be a png or svg file");
                return 2;
            }
            match std::fs::read(&src) {
                Ok(bytes) => Some((if ext == "svg" { "svg" } else { "png" }, bytes)),
                Err(e) => {
                    eprintln!("--icon: cannot read {src}: {e}");
                    return 2;
                }
            }
        }
        None => None,
    };
    if let Some(id) = id {
        if !profile::valid_id(id) {
            eprintln!("{id:?} is not a valid id (lower case letters, digits, - and _, starting with a letter)");
            return 2;
        }
        if Workspace::id_taken(profile, id) {
            eprintln!("{id:?} already exists in profile {:?} (as a workspace or a web app)", profile.id);
            return 1;
        }
    }
    // What the site says about itself, unless told not to; what the flags give wins.
    let from_site = !args.iter().any(|a| a == "--no-manifest");
    let info = if from_site {
        match info_from_site(&page) {
            Ok(i) => Some(i),
            Err(e) => {
                eprintln!("warning: cannot read {page}: {e}; using only what the flags say");
                None
            }
        }
    } else {
        None
    };
    if let Some(i) = &info {
        println!("{}", if i.from_manifest { format!("read the web manifest of {page}") } else { format!("{page} has no web manifest; using its title and address") });
    }
    let info = info.unwrap_or_else(|| crate::manifest::page_info("", &page));
    let scope_flag: Option<Vec<String>> = flag_value(args, "--scope").map(|s| s.split(',').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect());
    let custom_icon = icon_flag.is_some();
    let icon = match icon_flag {
        Some((ext, bytes)) => Some((ext, bytes, "--icon".to_owned())),
        None if from_site => fetch_icon(&info.icons),
        None => None,
    };
    let name = flag_value(args, "--name").unwrap_or_else(|| info.name.clone());
    let spec = NewApp {
        name: Some(name.clone()),
        color: color.or_else(|| info.color.clone()),
        url: flag_value(args, "--start-url").unwrap_or_else(|| info.start_url.clone()),
        scope: scope_flag.unwrap_or_else(|| info.scope.clone()),
        display: display.unwrap_or(info.display),
        icon: icon.as_ref().map(|(ext, bytes, _)| (*ext, bytes.clone())),
        manifest: info.from_manifest.then(|| ManifestInstall { info: info.clone(), page_url: page, custom_icon }),
    };
    let id = match create(profile, id, &spec, exe, libs) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("cannot create the web app: {e}");
            return 1;
        }
    };
    let dir = profile.apps_dir().join(&id);
    println!("created web app {id:?} in profile {:?} ({})", profile.id, dir.display());
    println!("  name {name:?}, start page {}, scope {}, display {}", spec.url, if spec.scope.is_empty() { "the start page's site".to_owned() } else { spec.scope.join(", ") }, display_name_of(spec.display));
    match &icon {
        Some((_, _, from)) => println!("  icon from {from}"),
        None if info.from_manifest => println!("  no usable icon in the manifest; a generated one is used (add one with --icon)"),
        None => println!("  a generated icon (add one with --icon)"),
    }
    println!("launch it from the desktop's launcher, or with: browsah --profile={} --app={id}", profile.id);
    0
}

/// The addresses that can be sent to an app: its scope, and the start page's site when the scope leaves it out (the app
/// opens there whatever its scope says). Also what `app route` writes as the rule's patterns.
pub fn route_patterns(app: &AppConfig) -> Vec<String> {
    let mut patterns = if app.scope.is_empty() { Vec::new() } else { app.scope.clone() };
    if !patterns.iter().any(|p| crate::workspace::scope_matches(p, &app.url)) {
        patterns.insert(0, crate::cookies::host_of(&app.url));
    }
    patterns
}

/// The name of the routing rule `app route` writes: "Open in <the app's name>".
fn route_rule_name(app_name: &str) -> String {
    format!("Open in {app_name}")
}

/// `profile.yaml`'s text with the routing rule for app `id` (named "Open in <app_name>") set, replacing the one `app route` wrote
/// before, or, with `None`, removed. The earlier rule is found by what it does (`open: app:<id>`) and its "Open in ..." name, so
/// one written under another name (the app was renamed, or an earlier version named it after the id) is replaced, not
/// duplicated; a rule of that app that you wrote with a name of your own is left alone. Comments in the file are not kept, as in
/// every rewrite of a configuration file here.
pub fn with_route_rule(profile_yaml: &str, id: &str, app_name: &str, patterns: Option<&[String]>) -> Result<String, String> {
    use serde_yaml_ng::{Mapping, Value};
    let mut root: Value = if profile_yaml.trim().is_empty() { Value::Mapping(Mapping::new()) } else { serde_yaml_ng::from_str(profile_yaml).map_err(|e| e.to_string())? };
    if root.is_null() {
        root = Value::Mapping(Mapping::new());
    }
    let map = root.as_mapping_mut().ok_or("profile.yaml is not a mapping")?;
    let name = route_rule_name(app_name);
    let key = |k: &str| Value::String(k.to_owned());
    let routing = map.entry(key("routing")).or_insert_with(|| Value::Mapping(Mapping::new()));
    if routing.is_null() {
        *routing = Value::Mapping(Mapping::new());
    }
    let routing = routing.as_mapping_mut().ok_or("`routing` in profile.yaml is not a mapping")?;
    let rules = routing.entry(key("rules")).or_insert_with(|| Value::Sequence(Vec::new()));
    if rules.is_null() {
        *rules = Value::Sequence(Vec::new());
    }
    let rules = rules.as_sequence_mut().ok_or("`routing.rules` in profile.yaml is not a list")?;
    let target = format!("app:{id}");
    let ours = |r: &Value| r.get("open").and_then(Value::as_str) == Some(target.as_str()) && r.get("name").and_then(Value::as_str).is_some_and(|n| n.starts_with("Open in "));
    let at = rules.iter().position(ours);
    match (patterns, at) {
        (Some(p), at) => {
            let mut rule = Mapping::new();
            rule.insert(key("name"), Value::String(name));
            let mut m = Mapping::new();
            m.insert(key("url"), Value::Sequence(p.iter().map(|s| Value::String(s.clone())).collect()));
            rule.insert(key("match"), Value::Mapping(m));
            rule.insert(key("open"), Value::String(format!("app:{id}")));
            match at {
                Some(i) => rules[i] = Value::Mapping(rule),
                None => rules.insert(0, Value::Mapping(rule)), // first, so the app gets its links before broader rules
            }
        }
        (None, Some(i)) => {
            rules.remove(i);
        }
        (None, None) => {}
    }
    // Leave no empty `routing:` behind.
    if let Some(routing) = root.get_mut("routing").and_then(Value::as_mapping_mut) {
        if routing.get("rules").and_then(Value::as_sequence).is_some_and(Vec::is_empty) && routing.len() == 1 {
            routing.remove("rules");
        }
        if routing.is_empty() {
            root.as_mapping_mut().map(|m| m.remove("routing"));
        }
    }
    serde_yaml_ng::to_string(&root).map_err(|e| e.to_string())
}

/// `browsah app route`: show, write (`--write`) or remove (`--remove`) the profile's routing rule that opens the app's
/// site in the app.
fn route(profile: &Profile, args: &[String]) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let w = match Workspace::resolve_app(profile, id) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let app = w.config.app.as_ref().expect("an app");
    let file = profile.dir.join("profile.yaml");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    let (remove, write) = (args.iter().any(|a| a == "--remove"), args.iter().any(|a| a == "--write"));
    let patterns = route_patterns(app);
    let updated = match with_route_rule(&current, id, w.display_name(), if remove { None } else { Some(&patterns) }) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            return 1;
        }
    };
    if !remove && !write {
        println!("# This rule sends links to {} to the {id} app (only addresses inside its scope are delivered to it).", patterns.join(", "));
        println!("# Add it under routing.rules in {}, or have it written with: browsah app route {id} --write --profile={}", file.display(), profile.id);
        println!("routing:\n  rules:\n    - name: {}\n      match: {{ url: [{}] }}\n      open: app:{id}", quote(&route_rule_name(w.display_name())), patterns.iter().map(|p| quote(p)).collect::<Vec<_>>().join(", "));
        return 0;
    }
    if let Err(e) = std::fs::write(&file, updated) {
        eprintln!("cannot write {}: {e}", file.display());
        return 1;
    }
    println!("{} the routing rule for {id:?} in {} (comments in that file are not kept)", if remove { "removed" } else { "wrote" }, file.display());
    0
}

fn remove(profile: &Profile, args: &[String]) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let w = match Workspace::resolve_app(profile, id) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if profile::is_running(profile) {
        eprintln!("profile {:?} is running; close it first", profile.id);
        return 1;
    }
    if !args.iter().any(|a| a == "--yes") {
        eprintln!("this DELETES web app {id:?} ({}) and its launcher entry.\nrun again with --yes to continue", w.dir.display());
        return 1;
    }
    if let Ok(store) = crate::store::Store::open(&profile.dir.join("data.sqlite")) {
        let _ = store.delete_workspace(&w.id);
    }
    if let Err(e) = std::fs::remove_dir_all(&w.dir) {
        eprintln!("cannot delete {}: {e}", w.dir.display());
        return 1;
    }
    uninstall(profile, id);
    println!("removed web app {id:?}");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w`x`w` image whose top-left pixel is fully transparent and the rest opaque red, as PNG.
    fn png_with_hole(w: i32) -> Vec<u8> {
        let pb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, w, w).unwrap();
        pb.fill(0xff0000ff);
        pb.put_pixel(0, 0, 0, 0, 0, 0);
        pb.save_to_bufferv("png", &[]).unwrap()
    }

    /// The same image as a 32-bit bitmap (the data of an ICO entry: a DIB with a doubled height and an empty mask).
    fn dib_with_hole(w: usize) -> Vec<u8> {
        let mut d = Vec::new();
        for v in [40u32, w as u32, 2 * w as u32] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0; 24]);
        for row in (0..w).rev() {
            for col in 0..w {
                d.extend_from_slice(if row == 0 && col == 0 { &[0, 0, 0, 0] } else { &[0, 0, 255, 255] });
            }
        }
        d.extend(std::iter::repeat(0).take(((w + 31) / 32 * 4) * w));
        d
    }

    fn ico(entries: &[(u8, u16, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, 0];
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * entries.len();
        for (w, bpp, data) in entries {
            out.extend_from_slice(&[*w, *w, 0, 0, 1, 0]);
            out.extend_from_slice(&bpp.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, _, data) in entries {
            out.extend_from_slice(data);
        }
        out
    }

    /// Width, whether it has alpha and the alpha of the top-left and bottom-right pixels of a converted icon.
    fn describe(png: &[u8]) -> (i32, bool, u8, u8) {
        let pb = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(png.to_vec())).unwrap();
        let px = pb.read_pixel_bytes();
        let stride = pb.rowstride() as usize;
        let n = pb.n_channels() as usize;
        let last = (pb.height() as usize - 1) * stride + (pb.width() as usize - 1) * n;
        (pb.width(), pb.has_alpha(), px[3], px[last + 3])
    }

    #[test]
    fn icons_keep_their_transparency() {
        // A PNG is used as it is.
        let png = png_with_hole(8);
        let (ext, bytes) = icon_file(png.clone()).unwrap();
        assert_eq!((ext, bytes.clone()), ("png", png));
        assert_eq!(describe(&bytes), (8, true, 0, 255));
    }

    #[test]
    fn an_ico_gives_its_largest_image_with_transparency() {
        // PNG entries: the 32 px one is chosen over the 16 and 8 px ones, wherever they are in the file.
        let file = ico(&[(16, 32, png_with_hole(16)), (32, 32, png_with_hole(32)), (8, 32, png_with_hole(8))]);
        let (ext, bytes) = icon_file(file).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(describe(&bytes), (32, true, 0, 255));
        // Bitmap entries too, and a 0 width byte means 256 (so it wins over 16).
        let file = ico(&[(16, 32, dib_with_hole(16)), (24, 32, dib_with_hole(24)), (4, 32, dib_with_hole(4))]);
        let (ext, bytes) = icon_file(file).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(describe(&bytes), (24, true, 0, 255));
        // Of equal size the one with more bits per pixel wins; a damaged directory is not a panic.
        let file = ico(&[(16, 8, dib_with_hole(16)), (16, 32, dib_with_hole(16))]);
        assert!(largest_ico_entry(&file).is_some());
        assert!(largest_ico_entry(&file[..20]).is_none());
        assert!(icon_file(vec![0, 0, 1, 0, 5, 0]).is_none());
    }

    #[test]
    fn ids_come_from_names() {
        assert_eq!(id_from_name("Google Mail"), "google-mail");
        assert_eq!(id_from_name("  Mail & Calendar!! "), "mail-calendar");
        assert_eq!(id_from_name("365 Office"), "app-365-office");
        assert!(profile::valid_id(&id_from_name("Ünïcode Tool")));
    }

    #[test]
    fn written_config_reads_back() {
        let color = profile::ColorPair::parse("#2a6f97 #61a5c2").unwrap();
        let text = config_text(Some("My \"Mail\""), Some(&color), "https://mail.example/inbox", &["mail.example".into(), "login.example/*".into()], "minimal", Some("icon.png"), Some("https://mail.example/"));
        let a = parse_app(&text).unwrap();
        assert_eq!((a.url.as_str(), a.scope.len(), a.icon.as_deref(), a.manifest_id.as_deref()), ("https://mail.example/inbox", 2, Some("icon.png"), Some("https://mail.example/")));
        assert_eq!(a.name.as_deref(), Some("My \"Mail\""));
        assert!(a.color.is_some() && a.multiple_windows && !a.multiple_tabs);
    }

    #[test]
    fn display_custom_takes_the_apps_own_nav_section() {
        // Only the listed keys change; the rest are shown (editable address).
        let c = window_config(parse_app("url: https://a.example/\ndisplay: custom\nnav:\n  back: false\n  address_bar: readonly\n").unwrap());
        assert!(!c.nav.back && c.nav.forward && c.nav.reload && c.nav.address_bar == AddressBar::Readonly);
        let d = window_config(parse_app("url: https://a.example/\ndisplay: custom\n").unwrap());
        assert!(d.nav.back && d.nav.forward && d.nav.reload && d.nav.address_bar == AddressBar::Editable);
        // The tab bar follows multiple_tabs, and an app has no new-tab button or bookmarks, whatever the nav section is.
        assert!(!c.nav.tab_bar && !c.nav.new_tab_button && !c.nav.bookmarks && !c.tabs.allow_new);
        let t = window_config(parse_app("url: https://a.example/\ndisplay: custom\nmultiple_tabs: true\n").unwrap());
        assert!(t.nav.tab_bar && !t.nav.new_tab_button && !t.nav.bookmarks && t.tabs.allow_new);
        // Those three are not options of an app's nav (a misspelt or removed key is an error in tests).
        for key in ["tab_bar", "new_tab_button", "bookmarks", "nope"] {
            assert!(parse_app(&format!("url: https://a.example/\ndisplay: custom\nnav:\n  {key}: true\n")).is_err(), "{key}");
        }
        // The other displays are unchanged, with or without a nav section (which they ignore).
        let s = window_config(parse_app("url: https://a.example/\nnav:\n  back: false\n").unwrap());
        assert!(s.nav.back && s.nav.address_bar == AddressBar::Hidden && !s.nav.tab_bar);
    }

    #[test]
    fn the_cli_writes_a_nav_section_for_display_custom() {
        let text = config_text(Some("Mail"), None, "https://m.example/", &[], "custom", None, None);
        assert!(text.contains("display: custom\nnav:\n  back: true"), "{text}");
        let app = parse_app(&text).unwrap();
        assert!(app.display == AppDisplay::Custom && app.nav == Some(AppNav::default()), "the written section is the defaults");
        assert!(!config_text(None, None, "https://m.example/", &[], "browser", None, None).contains("nav:"));
    }

    #[test]
    fn a_web_app_implies_its_window() {
        let c = window_config(parse_app("url: https://mail.example.com/inbox\n").unwrap());
        assert!(!c.tabs.allow_new && c.tabs.popups == PopupPolicy::Modal);
        assert_eq!(c.tabs.pinned.len(), 1);
        assert_eq!(c.tabs.pinned[0].scope, ["mail.example.com"], "the site, by default");
        assert!(c.nav.address_bar == AddressBar::Hidden && !c.nav.tab_bar && c.nav.back && c.nav.reload);
        let m = window_config(parse_app("url: https://a.example/\nscope: ['a.example', 'b.example']\ndisplay: minimal\n").unwrap());
        assert_eq!(m.tabs.pinned[0].scope, ["a.example", "b.example"]);
        assert!(m.nav.address_bar == AddressBar::Readonly);
        let b = window_config(parse_app("url: https://a.example/\ndisplay: browser\n").unwrap());
        assert!(b.nav.address_bar == AddressBar::Editable && b.nav.tab_bar && !b.nav.new_tab_button);
        let t = window_config(parse_app("url: https://a.example/\nmultiple_tabs: true\n").unwrap());
        assert!(t.tabs.allow_new && !t.nav.new_tab_button && t.nav.tab_bar && t.nav.address_bar == AddressBar::Hidden, "tabs come from links: no new-tab button");
        assert!(parse_app("scope: [a]").is_err(), "a start URL is required");
        assert!(parse_app("").is_err());
        // Settings that are not the app's own are left to the config layers.
        let o = parse_app("url: https://a.example/\nstorage: isolated\n").unwrap();
        assert!(o.multiple_windows);
    }

    #[test]
    fn docs_cover_every_app_option() {
        let docs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/config/webapp.md")).expect("docs/config/webapp.md exists");
        fn fields<T: serde::de::DeserializeOwned>() -> Vec<String> {
            let err = serde_yaml_ng::from_str::<T>("{ __nope__: 1 }").err().unwrap().to_string();
            err.split("expected").nth(1).unwrap_or("").split('`').skip(1).step_by(2).map(str::to_owned).collect()
        }
        let mut paths: Vec<String> = fields::<AppConfig>();
        paths.extend(fields::<ModalsConfig>().into_iter().map(|f| format!("modals.{f}")));
        paths.extend(fields::<AppNav>().into_iter().map(|f| format!("nav.{f}")));
        assert_eq!(paths.len(), APP_IDENTITY_KEYS.len() + 1 + 4, "{paths:?}");
        for key in APP_IDENTITY_KEYS {
            assert!(paths.iter().any(|p| p == key), "APP_IDENTITY_KEYS has {key}, which AppConfig does not");
        }
        for p in paths {
            assert!(docs.contains(&format!("| `{p}`")), "app option {p} is not documented in docs/config/webapp.md");
        }
    }

    #[test]
    fn the_route_rule_covers_the_start_page_and_the_scope() {
        let app = |t: &str| parse_app(t).unwrap();
        assert_eq!(route_patterns(&app("url: https://mail.example.com/inbox\n")), ["mail.example.com"]);
        assert_eq!(route_patterns(&app("url: https://mail.example.com/inbox\nscope: ['mail.example.com', 'accounts.example.com/signin/*']\n")), ["mail.example.com", "accounts.example.com/signin/*"]);
        assert_eq!(route_patterns(&app("url: https://mail.example.com/inbox\nscope: ['accounts.example.com']\n")), ["mail.example.com", "accounts.example.com"], "the start page's site is added when the scope leaves it out");
    }

    #[test]
    fn the_route_rule_is_written_replaced_and_removed_in_the_profile_file() {
        let p = ["mail.example.com".to_owned()];
        let first = with_route_rule("name: Work\nrouting:\n  rules:\n    - match: { url: [x.com] }\n      open: block\n", "mail", "Mail", Some(&p)).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&first).unwrap();
        let rules = v["routing"]["rules"].as_sequence().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["open"].as_str(), Some("app:mail"), "the app's rule goes first");
        assert_eq!(v["name"].as_str(), Some("Work"));
        // Again: replaced, not added twice.
        let again = with_route_rule(&first, "mail", "Mail", Some(&["a.example".to_owned()])).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&again).unwrap();
        assert_eq!(v["routing"]["rules"].as_sequence().unwrap().len(), 2);
        assert_eq!(v["routing"]["rules"][0]["match"]["url"][0].as_str(), Some("a.example"));
        // Removed: only that rule.
        let gone = with_route_rule(&again, "mail", "Mail", None).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&gone).unwrap();
        assert_eq!(v["routing"]["rules"].as_sequence().unwrap().len(), 1);
        let none = with_route_rule(&with_route_rule("name: W\n", "mail", "Mail", Some(&p)).unwrap(), "mail", "Mail", None).unwrap();
        assert_eq!(none.trim(), "name: W", "no empty routing section is left behind");
        // From nothing, and the result is a profile file the real types accept.
        let fresh = with_route_rule("", "mail", "Mail", Some(&p)).unwrap();
        assert!(crate::profile::parse_config(&fresh).is_ok(), "{fresh}");
        assert!(with_route_rule("- a\n", "mail", "Mail", Some(&p)).is_err());
        // The rule is named after the app: "Open in <name>".
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fresh).unwrap();
        assert_eq!(v["routing"]["rules"][0]["name"].as_str(), Some("Open in Mail"));
        // Renaming the app replaces the rule (not a second one); so does a rule an earlier version named after the id.
        let renamed = with_route_rule(&fresh, "mail", "Work Mail", Some(&p)).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&renamed).unwrap();
        assert_eq!((v["routing"]["rules"].as_sequence().unwrap().len(), v["routing"]["rules"][0]["name"].as_str()), (1, Some("Open in Work Mail")));
        let legacy = "routing:\n  rules:\n    - name: Open in the mail app\n      match: { url: [mail.example.com] }\n      open: app:mail\n";
        let upgraded = with_route_rule(legacy, "mail", "Mail", Some(&p)).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&upgraded).unwrap();
        assert_eq!((v["routing"]["rules"].as_sequence().unwrap().len(), v["routing"]["rules"][0]["name"].as_str()), (1, Some("Open in Mail")));
        // A rule of your own for the app (another name), and rules of other apps, are left alone; removing takes only ours.
        let mixed = "routing:\n  rules:\n    - name: My mail links\n      match: { url: [m.example] }\n      open: app:mail\n    - name: Open in Docs\n      match: { url: [d.example] }\n      open: app:docs\n";
        let added = with_route_rule(mixed, "mail", "Mail", Some(&p)).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&added).unwrap();
        assert_eq!(v["routing"]["rules"].as_sequence().unwrap().len(), 3);
        let removed = with_route_rule(&added, "mail", "Mail", None).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&removed).unwrap();
        let names: Vec<&str> = v["routing"]["rules"].as_sequence().unwrap().iter().filter_map(|r| r["name"].as_str()).collect();
        assert_eq!(names, ["My mail links", "Open in Docs"]);
    }

    #[test]
    fn example_web_app_loads() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/webapp.yaml")).expect("examples/webapp.yaml exists");
        let a = parse_app(&text).expect("the example must parse with deny_unknown_fields");
        assert!(!a.url.is_empty() && a.name.is_some());
    }

    #[test]
    fn desktop_entry_names_the_app_and_launches_it() {
        let p = Profile { id: "work".into(), config: Default::default(), dir: "/tmp/p".into() };
        let w = Workspace { id: "mail".into(), config: window_config(parse_app("name: Mail\nurl: https://m.example/\n").unwrap()), dir: "/tmp/p/apps/mail".into() };
        let e = desktop_entry(&p, &w, Path::new("/opt/browsah/browsah"), Path::new("/opt/browsah"));
        assert!(e.contains("Name=Mail\n") && e.contains("--profile=work --app=mail") && e.contains("Icon=be.geee.browsah.work.mail\n") && e.contains("StartupWMClass=be.geee.browsah.work.mail\n"), "{e}");
        assert_eq!(app_id(&p, "mail"), "be.geee.browsah.work.mail");
        // An app that may have more than one window has "New Window" in its launcher's menu; one that may not has not.
        assert!(e.contains("Actions=new-window;\n") && e.contains("[Desktop Action new-window]\nName=New Window\nExec=env LD_LIBRARY_PATH=/opt/browsah /opt/browsah/browsah --profile=work --app=mail --new-window\n"), "{e}");
        let single = Workspace { id: "mail".into(), config: window_config(parse_app("name: Mail\nurl: https://m.example/\nmultiple_windows: false\n").unwrap()), dir: "/tmp/p/apps/mail".into() };
        let e = desktop_entry(&p, &single, Path::new("/opt/browsah/browsah"), Path::new("/opt/browsah"));
        assert!(!e.contains("new-window") && !e.contains("Actions="), "{e}");
    }
}
