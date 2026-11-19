//! Profiles: the root of a person's browsing data (cookies, storage, cache, later history).
//!
//! Each profile has its own directory, its own GNOME application id and its own `.desktop`
//! entry and icon, and runs as its own process (Chromium locks its cache directory, so one
//! profile means one process; see docs/research/profiles-workspaces-design.md).
//!
//! Layout of `$XDG_DATA_HOME/browsah/profiles/<id>/`:
//! - `profile.yaml`: the profile's configuration (hand edited, see docs/config/profile.md)
//! - `cef/`: Chromium's cache directory (cookies, local storage, caches)
//!
//! This module has no GTK or CEF dependency apart from locating directories, so the `profile`
//! command line works without starting either.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Application id of every profile is this prefix, a dot and the profile id.
pub const APP_ID_PREFIX: &str = "be.geee.browsah";

/// Profiles that exist without being created: the one used when none is named, and the one the
/// self-test and benchmarks use so that they never touch (or collide with) a real profile.
pub const DEFAULT_ID: &str = "default";
pub const TEST_ID: &str = "selftest";

/// A colour for light and dark appearance. In YAML either a single colour (used for both), two
/// colours in one string separated by a space (`"#4a2f80 #7a56c2"`, light then dark) or a map
/// with `light` and `dark`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorPair {
    pub light: String,
    pub dark: String,
}

impl ColorPair {
    /// Parse `"#rrggbb"` or `"#rrggbb #rrggbb"` (also `#rgb`).
    pub fn parse(text: &str) -> Result<ColorPair, String> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let (light, dark) = match parts.as_slice() {
            [one] => (*one, *one),
            [light, dark] => (*light, *dark),
            _ => return Err(format!("expected one colour or two (light dark), got {text:?}")),
        };
        Ok(ColorPair { light: normalise_hex(light)?, dark: normalise_hex(dark)? })
    }
}

fn normalise_hex(c: &str) -> Result<String, String> {
    let h = c.strip_prefix('#').ok_or_else(|| format!("colour {c:?} must start with #"))?;
    if !h.chars().all(|x| x.is_ascii_hexdigit()) {
        return Err(format!("colour {c:?} is not hexadecimal"));
    }
    match h.len() {
        3 => Ok(format!("#{}", h.chars().flat_map(|x| [x, x]).collect::<String>().to_lowercase())),
        6 => Ok(format!("#{}", h.to_lowercase())),
        _ => Err(format!("colour {c:?} must be #rgb or #rrggbb")),
    }
}

impl<'de> Deserialize<'de> for ColorPair {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Map { light: String, dark: String },
        }
        match Raw::deserialize(d)? {
            Raw::Text(t) => ColorPair::parse(&t).map_err(serde::de::Error::custom),
            Raw::Map { light, dark } => Ok(ColorPair {
                light: normalise_hex(&light).map_err(serde::de::Error::custom)?,
                dark: normalise_hex(&dark).map_err(serde::de::Error::custom)?,
            }),
        }
    }
}

/// The contents of `profile.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct ProfileConfig {
    /// Display name (window titles, the `.desktop` entry). Defaults to the id.
    pub name: Option<String>,
    /// Icon and accent colour, for light and dark appearance.
    pub color: Option<ColorPair>,
    /// The workspace opened when the profile starts with no workspace remembered or named.
    pub default_workspace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub id: String,
    pub config: ProfileConfig,
    pub dir: PathBuf,
}

/// Whether `id` is usable: lower case, starts with a letter, then letters, digits, `-` or `_`.
/// (It becomes the last element of a GApplication id, so it must also be a valid one.)
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 31
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn profiles_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("browsah").join("profiles")
}

pub fn app_id(id: &str) -> String {
    format!("{APP_ID_PREFIX}.{id}")
}

impl Profile {
    pub fn app_id(&self) -> String {
        app_id(&self.id)
    }

    pub fn display_name(&self) -> &str {
        self.config.name.as_deref().unwrap_or(&self.id)
    }

    /// Where the profile's workspaces live (one directory each).
    pub fn workspaces_dir(&self) -> PathBuf {
        self.dir.join("workspaces")
    }

    /// Where the profile's web apps live (one directory each, with an `app.yaml`).
    pub fn apps_dir(&self) -> PathBuf {
        self.dir.join("apps")
    }

    fn state_file(&self) -> PathBuf {
        self.dir.join("state.yaml")
    }

    /// What the profile remembers between runs (written by the program, not hand edited).
    pub fn load_state(&self) -> ProfileState {
        std::fs::read_to_string(self.state_file()).ok().and_then(|t| serde_yaml_ng::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save_state(&self, state: &ProfileState) {
        if let Ok(text) = serde_yaml_ng::to_string(state) {
            let _ = std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(self.state_file(), text));
        }
    }

    /// Chromium's cache directory for this profile.
    pub fn cef_dir(&self) -> PathBuf {
        self.dir.join("cef")
    }

    /// Find a profile. `default` and `selftest` exist implicitly; others must have been created
    /// with `browsah profile add`.
    pub fn resolve(id: &str) -> Result<Profile, String> {
        Self::resolve_in(&profiles_dir(), id)
    }

    fn resolve_in(root: &Path, id: &str) -> Result<Profile, String> {
        if !valid_id(id) {
            return Err(format!("{id:?} is not a valid profile id (lower case letters, digits, - and _, starting with a letter)"));
        }
        let dir = root.join(id);
        let file = dir.join("profile.yaml");
        let config = match std::fs::read_to_string(&file) {
            Ok(text) => parse_config(&text).map_err(|e| format!("{}: {e}", file.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && (id == DEFAULT_ID || id == TEST_ID) => ProfileConfig::default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!("no profile {id:?}; create it with: browsah profile add {id}"));
            }
            Err(e) => return Err(format!("{}: {e}", file.display())),
        };
        Ok(Profile { id: id.to_owned(), config, dir })
    }

    /// Create the directories a running profile needs.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.cef_dir())
    }
}

/// Read the profile's own settings (name, colour) from `profile.yaml`. The file also holds
/// configuration overrides (see `config::load_effective`); those are not read here.
/// Remembered between runs, in `state.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileState {
    /// Workspaces whose windows were open when the profile last ran.
    pub open_workspaces: Vec<String>,
}

pub fn parse_config(text: &str) -> Result<ProfileConfig, String> {
    if text.trim().is_empty() {
        return Ok(ProfileConfig::default());
    }
    let all: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    let mut own = serde_yaml_ng::Mapping::new();
    if let Some(map) = all.as_mapping() {
        for key in crate::config::PROFILE_IDENTITY_KEYS {
            if let Some(v) = map.get(*key) {
                own.insert(serde_yaml_ng::Value::String((*key).to_owned()), v.clone());
            }
        }
    }
    serde_yaml_ng::from_value(serde_yaml_ng::Value::Mapping(own)).map_err(|e| e.to_string())
}

/// Every profile that has a directory with a `profile.yaml`, plus the implicit default.
pub fn list() -> Vec<Profile> {
    list_in(&profiles_dir())
}

fn list_in(root: &Path) -> Vec<Profile> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if id != TEST_ID && e.path().join("profile.yaml").is_file() {
                if let Ok(p) = Profile::resolve_in(root, &id) {
                    out.push(p);
                }
            }
        }
    }
    // The built-in `default` profile (what a plain `browsah` starts) is there once it has been used or created, and when it
    // is the only profile (so there is always one to start). Otherwise a profile you removed would keep coming back.
    let default_dir = root.join(DEFAULT_ID);
    if !out.iter().any(|p| p.id == DEFAULT_ID) && (default_dir.is_dir() || out.is_empty()) {
        out.push(Profile { id: DEFAULT_ID.into(), config: ProfileConfig::default(), dir: default_dir });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ---- desktop integration ----------------------------------------------------------------------

pub fn applications_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("applications")
}

pub fn icons_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("icons").join("hicolor").join("scalable").join("apps")
}

/// Quote a path for a `.desktop` Exec line.
pub fn exec_quote(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c)) {
        format!("\"{}\"", s.replace('\\', "\\\\\\\\").replace('"', "\\\"").replace('$', "\\\\$").replace('`', "\\\\`"))
    } else {
        s.to_owned()
    }
}

/// The `.desktop` entry for a profile. Each profile declares the web handlers, so GNOME's
/// default-browser setting can name one of them and external links reach that profile.
/// `exe` is the browsah binary; `libs` is the directory holding `libcef.so` (no rpath yet).
pub fn desktop_entry(p: &Profile, exe: &Path, libs: &Path) -> String {
    let id = app_id(&p.id);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Browsah ({name})\n\
         GenericName=Web Browser\n\
         Comment=Browsah profile {name}\n\
         Exec=env LD_LIBRARY_PATH={libs} {exe} --profile={pid} %U\n\
         Icon={id}\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass={id}\n\
         Categories=Network;WebBrowser;\n\
         MimeType=text/html;application/xhtml+xml;x-scheme-handler/http;x-scheme-handler/https;\n\
         Actions=new-window;\n\
         \n\
         [Desktop Action new-window]\n\
         Name=New Window\n\
         Exec=env LD_LIBRARY_PATH={libs} {exe} --profile={pid} --new-window\n",
        name = p.display_name(),
        libs = exec_quote(&libs.to_string_lossy()),
        exe = exec_quote(&exe.to_string_lossy()),
        pid = p.id,
    )
}

/// The profile's icon: a circle in its colour (the light one; the pair is for the windows) with four swishes, see
/// [`crate::appicon`]. Without a colour it is the profile picker's blue.
pub fn icon_svg(p: &Profile) -> String {
    crate::appicon::svg(p.config.color.as_ref().map(|c| c.light.as_str()))
}

/// Rebuild the user's `hicolor` icon cache. GTK trusts that cache: while it is older than an icon file, a new icon is not found
/// (the launcher shows the generic one). The theme folder is `icons/hicolor` (three levels up from `scalable/apps` is `icons`, which
/// is not a theme), it has no `index.theme` of its own (`-t`), and `-f` rebuilds even when the cache looks current.
pub fn refresh_icon_cache() {
    let theme = icons_dir().join("../..");
    for tool in ["gtk-update-icon-cache", "gtk4-update-icon-cache"] {
        if std::process::Command::new(tool).args(["-q", "-t", "-f"]).arg(&theme).status().is_ok_and(|s| s.success()) {
            break;
        }
    }
}

/// Write the profile's `profile.yaml` (if new), `.desktop` entry and icon.
fn install(p: &Profile, exe: &Path, libs: &Path) -> std::io::Result<()> {
    p.ensure_dirs()?;
    std::fs::create_dir_all(applications_dir())?;
    std::fs::create_dir_all(icons_dir())?;
    std::fs::write(applications_dir().join(format!("{}.desktop", p.app_id())), desktop_entry(p, exe, libs))?;
    std::fs::write(icons_dir().join(format!("{}.svg", p.app_id())), icon_svg(p))?;
    // Best effort: let desktops notice the new entry and icon without a re-login.
    let _ = std::process::Command::new("update-desktop-database").arg(applications_dir()).status();
    refresh_icon_cache();
    Ok(())
}

fn uninstall(p: &Profile) {
    let _ = std::fs::remove_file(applications_dir().join(format!("{}.desktop", p.app_id())));
    let _ = std::fs::remove_file(icons_dir().join(format!("{}.svg", p.app_id())));
    let _ = std::process::Command::new("update-desktop-database").arg(applications_dir()).status();
}

/// Is a process for this profile running (does anyone own its application id on the bus)?
pub fn is_running(p: &Profile) -> bool {
    use gtk::gio;
    use gtk::prelude::*;
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return false };
    conn.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(p.app_id(),).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .ok()
    .and_then(|v| v.child_value(0).get::<bool>())
    .unwrap_or(false)
}

// ---- command line -----------------------------------------------------------------------------

const USAGE: &str = "usage:
  browsah profile add <id> [--name NAME] [--color \"LIGHT [DARK]\"]
  browsah profile list
  browsah profile install <id>      (write the launcher entry and icon again, e.g. after an update)
  browsah profile remove <id> --yes [--keep-data]

An id is lower case letters, digits, - and _, starting with a letter. Colours are #rrggbb.
Launch a profile with: browsah --profile=<id> [URL]";

/// The program the launcher entries run and the directory with `libcef.so`. A package can give stable paths in
/// `BROWSAH_LAUNCHER` and `BROWSAH_LIBS` (Homebrew's launcher script does: its versioned install directory is deleted by
/// the next upgrade, which would leave every entry pointing at nothing); otherwise this program's own path.
pub fn launcher() -> std::io::Result<(PathBuf, PathBuf)> {
    launcher_from(std::env::var_os("BROWSAH_LAUNCHER"), std::env::var_os("BROWSAH_LIBS"), || std::env::current_exe().and_then(|p| p.canonicalize()))
}

fn launcher_from(exe: Option<std::ffi::OsString>, libs: Option<std::ffi::OsString>, own: impl FnOnce() -> std::io::Result<PathBuf>) -> std::io::Result<(PathBuf, PathBuf)> {
    if let (Some(e), Some(l)) = (exe, libs) {
        let (e, l) = (PathBuf::from(e), PathBuf::from(l));
        if e.is_absolute() && l.is_absolute() && e.is_file() {
            return Ok((e, l));
        }
    }
    let exe = own()?;
    let libs = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((exe, libs))
}

/// Whether a `.desktop` file's `Exec=` program no longer exists (an upgrade removed the version it was written for).
/// The program is the first absolute path on the line after any `env NAME=value` words; false when none can be found.
pub fn entry_program_missing(entry: &str) -> bool {
    let Some(line) = entry.lines().find_map(|l| l.strip_prefix("Exec=")) else { return false };
    let mut rest = line.trim_start();
    loop {
        let (word, after) = if let Some(quoted) = rest.strip_prefix('"') {
            match quoted.find('"') {
                Some(i) => (&quoted[..i], quoted[i + 1..].trim_start()),
                None => return false,
            }
        } else {
            match rest.split_once(char::is_whitespace) {
                Some((w, a)) => (w, a.trim_start()),
                None => (rest, ""),
            }
        };
        if word.starts_with('/') {
            return !Path::new(word).exists();
        }
        if after.is_empty() {
            return false;
        }
        rest = after;
    }
}

/// `browsah profile ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let (exe, libs) = match launcher() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot find this program: {e}");
            return 1;
        }
    };
    match args.first().map(String::as_str) {
        Some("add") => add(&args[1..], &exe, &libs),
        Some("list") => {
            for p in list() {
                let color = p.config.color.as_ref().map(|c| format!("{} {}", c.light, c.dark)).unwrap_or_else(|| "-".into());
                println!("{:<16} {:<20} {:<12} {}", p.id, p.display_name(), color, p.app_id());
            }
            0
        }
        Some("install") => match args.get(1).map(|id| Profile::resolve(id)) {
            Some(Ok(p)) => match install(&p, &exe, &libs) {
                Ok(()) => {
                    println!("wrote the launcher entry {}.desktop and its icon", p.app_id());
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
        Some("remove") => remove(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

/// Value of `--flag VALUE` or `--flag=VALUE`.
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

fn add(args: &[String], exe: &Path, libs: &Path) -> i32 {
    let Some(id) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("{USAGE}");
        return 2;
    };
    if !valid_id(id) {
        eprintln!("{id:?} is not a valid profile id (lower case letters, digits, - and _, starting with a letter)");
        return 2;
    }
    let dir = profiles_dir().join(id);
    if dir.join("profile.yaml").exists() {
        eprintln!("profile {id:?} already exists ({})", dir.display());
        return 1;
    }
    let color = match flag_value(args, "--color").map(|c| ColorPair::parse(&c)) {
        Some(Err(e)) => {
            eprintln!("--color: {e}");
            return 2;
        }
        Some(Ok(c)) => Some(c),
        // No colour given: pick one, so the new profile's icon and windows are told apart from the others'.
        None => Some(auto_color(id, &list().iter().filter_map(|p| p.config.color.as_ref().map(|c| c.light.clone())).collect::<Vec<_>>())),
    };
    let name = flag_value(args, "--name");
    let p = Profile { id: id.clone(), config: ProfileConfig { name, color, ..Default::default() }, dir: dir.clone() };
    if let Err(e) = write_config(&p).and_then(|_| install(&p, exe, libs)) {
        eprintln!("cannot create profile: {e}");
        return 1;
    }
    println!("created profile {id:?} in {}", dir.display());
    println!("launch it with: browsah --profile={id}   (application id {})", p.app_id());
    0
}

/// Light and dark colours a new profile can take (`browsah profile add` without `--color`).
const AUTO_COLORS: &[(&str, &str)] = &[
    ("#3584e4", "#62a0ea"),
    ("#33a06f", "#57e389"),
    ("#e5731c", "#ffa348"),
    ("#9141ac", "#c061cb"),
    ("#d6416a", "#f66151"),
    ("#2aa1b3", "#5bc8d4"),
    ("#c4a000", "#f5c211"),
    ("#6c7a89", "#9aa8b6"),
];

/// A colour for a new profile: the first of `AUTO_COLORS` no other profile uses (`used`: their light colours), else one
/// chosen from the id.
fn auto_color(id: &str, used: &[String]) -> ColorPair {
    let free = AUTO_COLORS.iter().find(|(l, _)| !used.iter().any(|u| u.eq_ignore_ascii_case(l)));
    let (light, dark) = free.copied().unwrap_or_else(|| {
        let h = id.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
        AUTO_COLORS[h % AUTO_COLORS.len()]
    });
    ColorPair { light: light.to_owned(), dark: dark.to_owned() }
}

/// Write `profile.yaml` with the options that were given and a comment pointing at the docs.
fn write_config(p: &Profile) -> std::io::Result<()> {
    std::fs::create_dir_all(&p.dir)?;
    let mut text = String::from("# Profile configuration. Options: docs/config/profile.md\n");
    if let Some(n) = &p.config.name {
        text.push_str(&format!("name: {}\n", serde_json_string(n)));
    }
    if let Some(c) = &p.config.color {
        text.push_str(&format!("color:\n  light: \"{}\"\n  dark: \"{}\"\n", c.light, c.dark));
    }
    std::fs::write(p.dir.join("profile.yaml"), text)
}

/// A YAML double-quoted string.
fn serde_json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Why a profile cannot be removed (`None`: it can). `others` is how many other profiles there are. The self-test's profile is
/// built in; `default` can go once there is another profile (it comes back, empty, if a plain `browsah` is started).
fn removal_refusal(id: &str, others: usize) -> Option<String> {
    if id == TEST_ID {
        return Some(format!("the {id:?} profile is built in and cannot be removed"));
    }
    if id == DEFAULT_ID && others == 0 {
        return Some("the \"default\" profile is the only one: create another (browsah profile add NAME) before removing it".to_owned());
    }
    None
}

fn remove(args: &[String]) -> i32 {
    let Some(id) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("{USAGE}");
        return 2;
    };
    if let Some(why) = removal_refusal(id, list().iter().filter(|p| p.id != *id).count()) {
        eprintln!("{why}");
        return 2;
    }
    let p = match Profile::resolve(id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if is_running(&p) {
        eprintln!("profile {id:?} is running; close it first");
        return 1;
    }
    let keep = args.iter().any(|a| a == "--keep-data");
    if !args.iter().any(|a| a == "--yes") {
        eprintln!(
            "this removes the profile's desktop entry and icon{}.\nrun again with --yes to continue{}",
            if keep { "" } else { ", and DELETES its directory including cookies and stored data" },
            if keep { "" } else { " (or add --keep-data to keep the directory)" },
        );
        return 1;
    }
    uninstall(&p);
    if keep {
        let _ = std::fs::remove_file(p.dir.join("profile.yaml"));
        println!("removed profile {id:?}; its data stays in {}", p.dir.display());
    } else if let Err(e) = std::fs::remove_dir_all(&p.dir) {
        eprintln!("cannot delete {}: {e}", p.dir.display());
        return 1;
    } else {
        println!("removed profile {id:?} and its data");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browsah-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ids() {
        for ok in ["work", "home2", "a", "my-profile", "my_profile"] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["", "Work", "1home", "-x", "a b", "a.b", "a/b", &"x".repeat(32)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn ids_make_valid_application_ids() {
        for id in ["work", "my-profile", "my_profile", "a1"] {
            assert!(gtk::gio::Application::id_is_valid(&app_id(id)), "{}", app_id(id));
        }
    }

    #[test]
    fn colours() {
        assert_eq!(ColorPair::parse("#4a2f80 #7a56c2").unwrap(), ColorPair { light: "#4a2f80".into(), dark: "#7a56c2".into() });
        assert_eq!(ColorPair::parse("#ABC").unwrap(), ColorPair { light: "#aabbcc".into(), dark: "#aabbcc".into() });
        assert!(ColorPair::parse("red").is_err());
        assert!(ColorPair::parse("#12345").is_err());
        assert!(ColorPair::parse("#111111 #222222 #333333").is_err());
        assert!(ColorPair::parse("").is_err());
    }

    #[test]
    fn config_accepts_all_colour_forms() {
        for text in ["color: \"#112233\"", "color: \"#112233 #445566\"", "color:\n  light: \"#112233\"\n  dark: \"#445566\"\n"] {
            let c = parse_config(text).unwrap();
            assert!(c.color.is_some(), "{text}");
        }
        assert!(parse_config("color: nope").is_err());
        assert!(parse_config("").unwrap().color.is_none());
    }

    #[test]
    fn docs_cover_every_profile_option() {
        let docs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/config/profile.md")).expect("docs/config/profile.md exists");
        let err = serde_yaml_ng::from_str::<ProfileConfig>("{ __nope__: 1 }").err().unwrap().to_string();
        let fields: Vec<&str> = err.split("expected").nth(1).unwrap_or("").split('`').skip(1).step_by(2).collect();
        assert!(!fields.is_empty());
        for f in fields {
            assert!(docs.contains(&format!("| `{f}`")), "profile option {f} is not documented in docs/config/profile.md");
        }
    }

    #[test]
    fn example_profile_loads_with_unknown_keys_rejected() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/profile.yaml")).expect("examples/profile.yaml exists");
        let c = parse_config(&text).expect("the example must parse with deny_unknown_fields");
        assert!(c.name.is_some() && c.color.is_some());
    }

    #[test]
    fn resolve_and_list() {
        let root = temp("resolve");
        // Implicit profiles exist without a directory; others do not.
        assert_eq!(Profile::resolve_in(&root, DEFAULT_ID).unwrap().app_id(), "be.geee.browsah.default");
        assert!(Profile::resolve_in(&root, TEST_ID).is_ok());
        assert!(Profile::resolve_in(&root, "work").unwrap_err().contains("browsah profile add work"));
        assert!(Profile::resolve_in(&root, "Bad Id").is_err());

        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::write(root.join("work/profile.yaml"), "name: Work\ncolor: \"#4a2f80 #7a56c2\"\n").unwrap();
        let w = Profile::resolve_in(&root, "work").unwrap();
        assert_eq!(w.display_name(), "Work");
        assert_eq!(w.cef_dir(), root.join("work/cef"));
        let ids: Vec<String> = list_in(&root).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["work"], "`default` was never created, so with another profile it is not listed");
        // Once it exists (it has been used) it is listed, in order; with a file of its own too.
        std::fs::create_dir_all(root.join("default/cef")).unwrap();
        let ids: Vec<String> = list_in(&root).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["default", "work"]);
        std::fs::remove_dir_all(root.join("default")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::write(root.join("alpha/profile.yaml"), "name: Alpha\n").unwrap();
        let ids: Vec<String> = list_in(&root).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["alpha", "work"], "removed or never made: not listed");

        // A broken file is an error naming the file, not a silent default.
        std::fs::write(root.join("work/profile.yaml"), "color: 12\n").unwrap();
        assert!(Profile::resolve_in(&root, "work").unwrap_err().contains("profile.yaml"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_default_profile_is_listed_when_it_exists_or_is_alone() {
        let root = temp("default-listing");
        // Nothing at all: the built-in default, so there is always a profile to start.
        assert_eq!(list_in(&root).into_iter().map(|p| p.id).collect::<Vec<_>>(), ["default"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_profiles() {
        assert!(removal_refusal("selftest", 3).is_some(), "the test profile is built in");
        assert!(removal_refusal("default", 0).is_some(), "not the only profile");
        assert!(removal_refusal("default", 1).is_none(), "once there is another");
        assert!(removal_refusal("work", 0).is_none());
    }

    #[test]
    fn desktop_entry_and_icon() {
        let p = Profile {
            id: "work".into(),
            config: ProfileConfig { name: Some("Work".into()), color: ColorPair::parse("#4a2f80 #7a56c2").ok(), ..Default::default() },
            dir: PathBuf::from("/x/work"),
        };
        let d = desktop_entry(&p, Path::new("/opt/my apps/browsah"), Path::new("/opt/my apps"));
        assert!(d.contains("Name=Browsah (Work)"));
        assert!(d.contains("StartupWMClass=be.geee.browsah.work"));
        assert!(d.contains("--profile=work %U"));
        assert!(d.contains("x-scheme-handler/https;"));
        assert!(d.contains("\"/opt/my apps/browsah\""), "paths with spaces are quoted: {d}");
        // The circle is shaded from the profile's (light) colour; without a colour it is the picker's blue.
        let svg = icon_svg(&p);
        assert!(svg.contains("<circle") && svg.contains(&crate::appicon::palette(Some("#4a2f80")).0));
        let q = Profile { config: ProfileConfig::default(), ..p };
        assert_eq!(icon_svg(&q), crate::appicon::svg(None));
        assert_ne!(icon_svg(&q), svg);
    }

    #[test]
    fn a_missing_program_in_an_entry_is_found() {
        let line = |exe: &str| format!("[Desktop Entry]\nName=x\nExec=env LD_LIBRARY_PATH=/gone {exe} --profile=work %U\n");
        assert!(entry_program_missing(&line("/no/such/0.1.2/libexec/browsah")));
        assert!(!entry_program_missing(&line("/usr/bin/env")));
        assert!(entry_program_missing("Exec=\"/no such dir/browsah\" --profile=w\n"));
        assert!(!entry_program_missing("Exec=/usr/bin/env A=b\n"));
        assert!(!entry_program_missing("Name=no exec line\n"));
        assert!(!entry_program_missing("Exec=browsah --profile=w\n"), "a name looked up on PATH is not judged");
    }

    #[test]
    fn launcher_paths_prefer_the_packages_stable_ones() {
        let own = || Ok(PathBuf::from("/opt/cellar/0.1.2/libexec/browsah"));
        // No package: this program and its directory.
        assert_eq!(launcher_from(None, None, own).unwrap(), (PathBuf::from("/opt/cellar/0.1.2/libexec/browsah"), PathBuf::from("/opt/cellar/0.1.2/libexec")));
        // A package's stable paths win when the launcher exists (here: this test program stands in for it).
        let me = std::env::current_exe().unwrap();
        let got = launcher_from(Some(me.clone().into()), Some("/opt/stable/libexec".into()), own).unwrap();
        assert_eq!(got, (me, PathBuf::from("/opt/stable/libexec")));
        // Missing launcher, one variable only, or relative paths: not trusted.
        for (e, l) in [(Some("/nope/browsah"), Some("/opt/stable/libexec")), (Some("/usr/bin/env"), None), (None, Some("/opt/x")), (Some("browsah"), Some("libexec"))] {
            let got = launcher_from(e.map(Into::into), l.map(Into::into), own).unwrap();
            assert_eq!(got.0, PathBuf::from("/opt/cellar/0.1.2/libexec/browsah"), "{e:?} {l:?}");
        }
    }

    #[test]
    fn auto_colors_avoid_the_ones_in_use() {
        let first = auto_color("a", &[]);
        assert_eq!(first.light, AUTO_COLORS[0].0);
        let second = auto_color("b", &[first.light.clone()]);
        assert_eq!(second.light, AUTO_COLORS[1].0);
        let all: Vec<String> = AUTO_COLORS.iter().map(|(l, _)| l.to_string()).collect();
        assert!(AUTO_COLORS.iter().any(|(l, _)| *l == auto_color("c", &all).light));
        for (l, d) in AUTO_COLORS {
            assert!(ColorPair::parse(&format!("{l} {d}")).is_ok());
        }
    }

    #[test]
    fn flag_values() {
        let a: Vec<String> = ["work", "--name", "My Work", "--color=#111111"].iter().map(|s| s.to_string()).collect();
        assert_eq!(flag_value(&a, "--name").as_deref(), Some("My Work"));
        assert_eq!(flag_value(&a, "--color").as_deref(), Some("#111111"));
        assert_eq!(flag_value(&a, "--nope"), None);
    }
}
