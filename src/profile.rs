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

use serde::Deserialize;
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

pub fn parse_config(text: &str) -> Result<ProfileConfig, String> {
    if text.trim().is_empty() {
        return Ok(ProfileConfig::default());
    }
    serde_yaml_ng::from_str(text).map_err(|e| e.to_string())
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
    if !out.iter().any(|p| p.id == DEFAULT_ID) {
        out.push(Profile { id: DEFAULT_ID.into(), config: ProfileConfig::default(), dir: root.join(DEFAULT_ID) });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ---- desktop integration ----------------------------------------------------------------------

fn applications_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("applications")
}

fn icons_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("icons").join("hicolor").join("scalable").join("apps")
}

/// Quote a path for a `.desktop` Exec line.
fn exec_quote(s: &str) -> String {
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
         MimeType=text/html;application/xhtml+xml;x-scheme-handler/http;x-scheme-handler/https;\n",
        name = p.display_name(),
        libs = exec_quote(&libs.to_string_lossy()),
        exe = exec_quote(&exe.to_string_lossy()),
        pid = p.id,
    )
}

/// A simple scalable icon: a rounded square shading from the light to the dark colour, with the
/// profile's initial.
pub fn icon_svg(p: &Profile) -> String {
    let (light, dark) = match &p.config.color {
        Some(c) => (c.light.clone(), c.dark.clone()),
        None => {
            // No colour chosen: derive one from the id so profiles still look different.
            let h = p.id.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360;
            (format!("hsl({h} 55% 55%)"), format!("hsl({h} 55% 32%)"))
        }
    };
    let initial = p.display_name().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "B".into());
    let initial = initial.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"128\" viewBox=\"0 0 128 128\">\n\
         <defs><linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{light}\"/><stop offset=\"1\" stop-color=\"{dark}\"/></linearGradient></defs>\n\
         <rect x=\"8\" y=\"8\" width=\"112\" height=\"112\" rx=\"26\" fill=\"url(#g)\"/>\n\
         <text x=\"64\" y=\"86\" font-family=\"sans-serif\" font-size=\"68\" font-weight=\"700\" text-anchor=\"middle\" fill=\"#ffffff\">{initial}</text>\n\
         </svg>\n"
    )
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
    let _ = std::process::Command::new("gtk-update-icon-cache").args(["-q", "-t"]).arg(icons_dir().join("../../..")).status();
    Ok(())
}

fn uninstall(p: &Profile) {
    let _ = std::fs::remove_file(applications_dir().join(format!("{}.desktop", p.app_id())));
    let _ = std::fs::remove_file(icons_dir().join(format!("{}.svg", p.app_id())));
    let _ = std::process::Command::new("update-desktop-database").arg(applications_dir()).status();
}

/// Is a process for this profile running (does anyone own its application id on the bus)?
fn running(p: &Profile) -> bool {
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
  browsah profile remove <id> --yes [--keep-data]

An id is lower case letters, digits, - and _, starting with a letter. Colours are #rrggbb.
Launch a profile with: browsah --profile=<id> [URL]";

/// `browsah profile ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find this program: {e}");
            return 1;
        }
    };
    let libs = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("add") => add(&args[1..], &exe, &libs),
        Some("list") => {
            for p in list() {
                let color = p.config.color.as_ref().map(|c| format!("{} {}", c.light, c.dark)).unwrap_or_else(|| "-".into());
                println!("{:<16} {:<20} {:<12} {}", p.id, p.display_name(), color, p.app_id());
            }
            0
        }
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
        None => None,
    };
    let name = flag_value(args, "--name");
    let p = Profile { id: id.clone(), config: ProfileConfig { name, color }, dir: dir.clone() };
    if let Err(e) = write_config(&p).and_then(|_| install(&p, exe, libs)) {
        eprintln!("cannot create profile: {e}");
        return 1;
    }
    println!("created profile {id:?} in {}", dir.display());
    println!("launch it with: browsah --profile={id}   (application id {})", p.app_id());
    0
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

fn remove(args: &[String]) -> i32 {
    let Some(id) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("{USAGE}");
        return 2;
    };
    if id == DEFAULT_ID || id == TEST_ID {
        eprintln!("the {id:?} profile is built in and cannot be removed");
        return 2;
    }
    let p = match Profile::resolve(id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if running(&p) {
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
        assert_eq!(ids, ["default", "work"]);

        // A broken file is an error naming the file, not a silent default.
        std::fs::write(root.join("work/profile.yaml"), "color: 12\n").unwrap();
        assert!(Profile::resolve_in(&root, "work").unwrap_err().contains("profile.yaml"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_entry_and_icon() {
        let p = Profile {
            id: "work".into(),
            config: ProfileConfig { name: Some("Work".into()), color: ColorPair::parse("#4a2f80 #7a56c2").ok() },
            dir: PathBuf::from("/x/work"),
        };
        let d = desktop_entry(&p, Path::new("/opt/my apps/browsah"), Path::new("/opt/my apps"));
        assert!(d.contains("Name=Browsah (Work)"));
        assert!(d.contains("StartupWMClass=be.geee.browsah.work"));
        assert!(d.contains("--profile=work %U"));
        assert!(d.contains("x-scheme-handler/https;"));
        assert!(d.contains("\"/opt/my apps/browsah\""), "paths with spaces are quoted: {d}");
        let svg = icon_svg(&p);
        assert!(svg.contains("#4a2f80") && svg.contains("#7a56c2") && svg.contains(">W<"));
        // Without a colour the icon is still coloured, deterministically.
        let q = Profile { config: ProfileConfig::default(), ..p };
        assert_eq!(icon_svg(&q), icon_svg(&q));
        assert!(icon_svg(&q).contains("hsl("));
    }

    #[test]
    fn flag_values() {
        let a: Vec<String> = ["work", "--name", "My Work", "--color=#111111"].iter().map(|s| s.to_string()).collect();
        assert_eq!(flag_value(&a, "--name").as_deref(), Some("My Work"));
        assert_eq!(flag_value(&a, "--color").as_deref(), Some("#111111"));
        assert_eq!(flag_value(&a, "--nope"), None);
    }
}
