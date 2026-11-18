//! Chrome extensions (docs/config/options.md `extensions`, docs/research/chrome-extensions.md).
//!
//! CEF 156 has no extension API, but Chromium's own `--load-extension=DIR,DIR` switch works (measured: content scripts, MV3
//! service workers, native messaging, popups). This module decides which folders to load, builds the switch, finds the
//! native messaging hosts an extension needs, and (in the second half of the file) installs and updates extensions from the
//! Chrome Web Store. Nothing here needs CEF or GTK, so it is all unit tested.

use crate::config::ExtensionsConfig;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The id Chromium gives an extension: the first 16 bytes of the SHA-256 of `bytes`, each nibble written as `a`..`p`.
/// With the extension's public key (DER) it is the Web Store id; with an unpacked folder's absolute path it is the id Chromium
/// gives an extension that has no `key` in its manifest.
pub fn id_from_bytes(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest.as_ref()[..16].iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (what a manifest's `key` uses).
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes of standard base64 text; `None` if it is not valid.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.trim_end_matches('=').bytes() {
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits & 0xff) as u8);
        }
    }
    Some(out)
}

/// One extension to load.
#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    pub id: String,
    /// The name from the manifest, with a `__MSG_x__` placeholder resolved from `_locales`.
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    /// The page of the toolbar button's popup (`action.default_popup`, or `browser_action` for manifest v2), relative to `dir`.
    pub popup: Option<String>,
    /// The toolbar button's icon (the largest PNG the manifest names), absolute.
    pub icon: Option<PathBuf>,
    /// Installed in the profile's `extensions/` folder (as opposed to named by `extensions.paths`).
    pub installed: bool,
    /// The manifest asks for the `nativeMessaging` permission: the extension talks to a program on the computer (1Password's app).
    pub native_messaging: bool,
}

/// A manifest `__MSG_key__` placeholder resolved from `_locales/<default_locale>/messages.json` (else `en`); the text itself
/// if it is not a placeholder or cannot be resolved.
fn localized(dir: &Path, manifest: &serde_json::Value, text: &str) -> String {
    let Some(key) = text.strip_prefix("__MSG_").and_then(|t| t.strip_suffix("__")) else { return text.to_owned() };
    let default = manifest.get("default_locale").and_then(|v| v.as_str()).unwrap_or("en");
    for locale in [default, "en"] {
        let Ok(raw) = std::fs::read_to_string(dir.join("_locales").join(locale).join("messages.json")) else { continue };
        let Ok(messages) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
        // Message names are case-insensitive.
        let found = messages.as_object().and_then(|o| o.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)));
        if let Some(m) = found.and_then(|(_, v)| v.get("message")).and_then(|m| m.as_str()) {
            return m.to_owned();
        }
    }
    text.to_owned()
}

/// The icon file of the largest size a manifest names (`action.default_icon` as a map, else `icons`).
fn best_icon(dir: &Path, manifest: &serde_json::Value) -> Option<PathBuf> {
    let action = manifest.get("action").or_else(|| manifest.get("browser_action"));
    let maps = [action.and_then(|a| a.get("default_icon")), manifest.get("icons")];
    for map in maps.into_iter().flatten() {
        if let Some(path) = map.as_str() {
            return Some(dir.join(path));
        }
        let best = map.as_object().and_then(|o| o.iter().filter_map(|(size, p)| Some((size.parse::<u32>().ok()?, p.as_str()?))).max_by_key(|(s, _)| *s));
        if let Some((_, p)) = best {
            return Some(dir.join(p));
        }
    }
    None
}

/// Read an unpacked extension folder. The id is the Web Store id when the manifest has a `key`, else the id Chromium derives from
/// the folder's path.
pub fn read_extension(dir: &Path, installed: bool) -> Result<Extension, String> {
    let manifest_path = dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let id = match manifest.get("key").and_then(|k| k.as_str()).and_then(base64_decode) {
        Some(der) => id_from_bytes(&der),
        None => id_from_bytes(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().as_bytes()),
    };
    let name = manifest.get("name").and_then(|n| n.as_str()).map(|n| localized(dir, &manifest, n)).unwrap_or_else(|| id.clone());
    let version = manifest.get("version").and_then(|v| v.as_str()).unwrap_or("0").to_owned();
    let popup = manifest.get("action").or_else(|| manifest.get("browser_action")).and_then(|a| a.get("default_popup")).and_then(|p| p.as_str()).map(str::to_owned);
    let permits = |key: &str| manifest.get(key).and_then(|p| p.as_array()).is_some_and(|p| p.iter().any(|x| x.as_str() == Some("nativeMessaging")));
    let native_messaging = permits("permissions") || permits("optional_permissions");
    Ok(Extension { id, name, version, dir: dir.to_path_buf(), popup, icon: best_icon(dir, &manifest), installed, native_messaging })
}

/// Dotted version numbers compared piece by piece as numbers (`8.12.9` < `8.12.10`).
pub fn version_key(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Where a profile keeps the extensions installed from the Web Store: `extensions/<id>/<version>/`.
pub fn install_root(profile_dir: &Path) -> PathBuf {
    profile_dir.join("extensions")
}

/// The newest installed version of each extension in the profile (a folder `extensions/<id>/<version>/` with a manifest).
pub fn installed_dirs(profile_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(ids) = std::fs::read_dir(install_root(profile_dir)) else { return out };
    for id in ids.flatten().filter(|e| e.path().is_dir()) {
        let newest = std::fs::read_dir(id.path()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join("manifest.json").is_file()).max_by_key(|p| version_key(&p.file_name().unwrap_or_default().to_string_lossy()));
        out.extend(newest);
    }
    out.sort();
    out
}

/// The extensions to load: the ones installed in the profile (except `disable`d ids), then `paths` (with `~` expanded).
/// Problems (a folder with no manifest) come back as warnings; the rest still loads.
pub fn collect(profile_dir: &Path, cfg: &ExtensionsConfig, home: &Path) -> (Vec<Extension>, Vec<String>) {
    let mut out: Vec<Extension> = Vec::new();
    let mut warnings = Vec::new();
    let candidates = installed_dirs(profile_dir).into_iter().map(|d| (d, true)).chain(cfg.paths.iter().map(|p| (crate::downloads::expand_tilde(p, home), false)));
    for (dir, installed) in candidates {
        match read_extension(&dir, installed) {
            Ok(e) if cfg.disable.contains(&e.id) => {}
            Ok(e) if out.iter().any(|x| x.id == e.id) => warnings.push(format!("{} is already loaded; skipped {}", e.id, dir.display())),
            Ok(e) if e.dir.to_string_lossy().contains(',') => warnings.push(format!("{} not loaded: Chromium cannot load a folder with a comma in its path", e.dir.display())),
            Ok(e) => out.push(e),
            Err(e) => warnings.push(e),
        }
    }
    (out, warnings)
}

/// Should a window show this extension's toolbar button (and so its popup)? A workspace window shows every loaded extension except the
/// `hide` list; an app window shows only the `show` list. A window with its own storage (`storage: isolated`) has no extensions at all
/// (Chromium loads them only into the profile's shared storage: measured), so it shows none. This decides what browsah *shows*: an
/// extension's content scripts run in every window that shares the profile's storage whatever these lists say.
pub fn shown_in_window(id: &str, cfg: &ExtensionsConfig, is_app: bool, isolated: bool) -> bool {
    if isolated {
        return false;
    }
    if is_app {
        cfg.show.iter().any(|s| s == id)
    } else {
        !cfg.hide.iter().any(|h| h == id)
    }
}

/// Put `--load-extension=` for these folders into the Chromium switches. Chromium keeps only the last `--load-extension`, so one
/// the person gave in `chromium.flags` or on the command line is merged with ours instead of being added next to it.
pub fn merge_load_flag(flags: &mut Vec<String>, dirs: &[PathBuf]) {
    if dirs.is_empty() {
        return;
    }
    let mut all: Vec<String> = Vec::new();
    flags.retain(|f| match f.strip_prefix("--load-extension=") {
        Some(v) => {
            all.extend(v.split(',').filter(|s| !s.is_empty()).map(str::to_owned));
            false
        }
        None => true,
    });
    for d in dirs {
        let d = d.to_string_lossy().into_owned();
        if !all.contains(&d) {
            all.push(d);
        }
    }
    flags.push(format!("--load-extension={}", all.join(",")));
}

/// The extensions this process loaded, for the window (the toolbar buttons). Set once at start-up.
static LOADED: OnceLock<Vec<Extension>> = OnceLock::new();

pub fn set_loaded(list: Vec<Extension>) {
    let _ = LOADED.set(list);
}

pub fn loaded() -> &'static [Extension] {
    LOADED.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Folders where Chrome and Chromium-based browsers keep native messaging host manifests (user-level first), under the user's
/// config folder `config` (`~/.config`).
pub fn host_search_dirs(config: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["google-chrome", "google-chrome-beta", "google-chrome-unstable", "chromium", "BraveSoftware/Brave-Browser", "microsoft-edge", "vivaldi"].iter().map(|b| config.join(b).join("NativeMessagingHosts")).collect();
    dirs.extend(["/etc/opt/chrome/native-messaging-hosts", "/etc/opt/edge/native-messaging-hosts", "/etc/brave/native-messaging-hosts"].iter().map(PathBuf::from));
    dirs
}

/// One native messaging host manifest found in a browser's folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    /// The host's name (also the manifest's file name without `.json`).
    pub name: String,
    pub text: String,
    pub from: PathBuf,
}

/// The host manifests in `dirs` that one of `ids` may use: `allowed_origins` names `chrome-extension://<id>/`, the file is called
/// like the host, and the program it runs exists. The first folder wins when two have the same host.
pub fn matching_hosts(dirs: &[PathBuf], ids: &[String]) -> Vec<Host> {
    let mut out: Vec<Host> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let name = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let named = v.get("name").and_then(|n| n.as_str()) == Some(name.as_str());
            let allowed = v.get("allowed_origins").and_then(|o| o.as_array()).is_some_and(|o| o.iter().filter_map(|x| x.as_str()).any(|origin| ids.iter().any(|id| origin == format!("chrome-extension://{id}/"))));
            let program = v.get("path").and_then(|p| p.as_str()).is_some_and(|p| Path::new(p).is_absolute() && Path::new(p).exists());
            if named && allowed && program && !out.iter().any(|h| h.name == name) {
                out.push(Host { name, text, from: file });
            }
        }
    }
    out
}

/// Where Chromium looks for a profile's native messaging hosts: `NativeMessagingHosts/` in the user data folder (the profile's `cef/`).
pub fn host_dir(cef_dir: &Path) -> PathBuf {
    cef_dir.join("NativeMessagingHosts")
}

/// Copy host manifests into the profile's folder (a copy, not a link, and unchanged). Returns the names that were written or
/// updated; a manifest already there with the same text is left alone.
pub fn copy_hosts(cef_dir: &Path, hosts: &[Host]) -> Vec<String> {
    let dir = host_dir(cef_dir);
    let mut written = Vec::new();
    if hosts.is_empty() || std::fs::create_dir_all(&dir).is_err() {
        return written;
    }
    for h in hosts {
        let target = dir.join(format!("{}.json", h.name));
        if std::fs::read_to_string(&target).ok().as_deref() != Some(h.text.as_str()) && std::fs::write(&target, &h.text).is_ok() {
            written.push(h.name.clone());
        }
    }
    written
}

// ---- installing from the Chrome Web Store ------------------------------------------------------

const STORE: &str = "https://clients2.google.com/service/update2/crx";
/// Most a download may be: extensions are a few megabytes, the biggest ones tens.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

/// The Chromium version this browsah embeds, as the store wants it (`prodversion`: it withholds extensions that need a newer one).
fn chromium_version() -> String {
    use cef::sys::{CHROME_VERSION_BUILD, CHROME_VERSION_MAJOR, CHROME_VERSION_MINOR, CHROME_VERSION_PATCH};
    format!("{CHROME_VERSION_MAJOR}.{CHROME_VERSION_MINOR}.{CHROME_VERSION_BUILD}.{CHROME_VERSION_PATCH}")
}

/// Where to download the newest package of an extension (the store redirects to the file).
pub fn download_url(id: &str, chromium: &str) -> String {
    format!("{STORE}?response=redirect&prodversion={chromium}&acceptformat=crx3&x=id%3D{id}%26installsource%3Dondemand%26uc")
}

/// Where to ask whether a newer version than `have` exists (the answer is a small XML document).
pub fn update_check_url(id: &str, have: &str, chromium: &str) -> String {
    format!("{STORE}?prodversion={chromium}&acceptformat=crx3&x=id%3D{id}%26v%3D{have}%26uc")
}

/// The newest version the store says there is, from an update check's answer; `None` when it says there is nothing newer, has
/// no such extension, or the answer is not what was expected.
pub fn parse_update_check(xml: &str) -> Option<String> {
    let start = xml.find("<updatecheck")?;
    let tag = &xml[start..xml[start..].find('>').map(|e| start + e)?];
    let attr = |name: &str| -> Option<&str> {
        let from = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        tag[from..].split('"').next()
    };
    (attr("status")? == "ok").then(|| attr("version").map(str::to_owned))?
}

/// An extension id from what the person typed: the id itself, or a Chrome Web Store address that contains it.
pub fn id_from_text(text: &str) -> Option<String> {
    let is_id = |s: &str| s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b));
    let text = text.trim();
    if is_id(text) {
        return Some(text.to_owned());
    }
    let url = text.strip_prefix("https://")?;
    let (host, path) = url.split_once('/')?;
    let store = host == "chromewebstore.google.com" || host == "chrome.google.com";
    store.then(|| path.split(['/', '?', '#']).find(|s| is_id(s)).map(str::to_owned))?
}

/// The newest version of an extension installed in the profile.
pub fn installed_version(profile_dir: &Path, id: &str) -> Option<String> {
    installed_dirs(profile_dir).into_iter().filter(|d| d.parent().and_then(|p| p.file_name()).is_some_and(|n| n == id)).find_map(|d| read_extension(&d, true).ok()).map(|e| e.version)
}

/// Check a downloaded package and put it in the profile as `extensions/<id>/<version>/`, with the signing key in its manifest
/// so it keeps its id. The two newest versions are kept (a running browser may still be using the previous one).
pub fn install_package(profile_dir: &Path, id: &str, bytes: &[u8]) -> Result<Extension, String> {
    let crx = crate::crx::parse(bytes, id)?;
    crate::log_info!("extension {id}: package signature verified ({} bytes, signed by the key of {})", bytes.len(), crx.id);
    let root = install_root(profile_dir).join(id);
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let temp = root.join(format!(".installing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let result = (|| {
        crate::crx::unpack(&crx.zip, &temp)?;
        let manifest_path = temp.join("manifest.json");
        let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| format!("the package has no manifest: {e}"))?;
        std::fs::write(&manifest_path, crate::crx::add_key(&manifest, &crx.key)?).map_err(|e| e.to_string())?;
        let ext = read_extension(&temp, true)?;
        if ext.id != id {
            return Err(format!("the unpacked extension has the id {}, not {id}", ext.id));
        }
        // The version names a folder: only digits and dots (what Chrome requires of it too).
        if ext.version.is_empty() || !ext.version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(format!("the version {:?} is not a number like 1.2.3", ext.version));
        }
        let target = root.join(&ext.version);
        if target.exists() {
            return Err(format!("version {} is already installed", ext.version));
        }
        std::fs::rename(&temp, &target).map_err(|e| e.to_string())?;
        read_extension(&target, true)
    })();
    let _ = std::fs::remove_dir_all(&temp);
    let ext = result?;
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join("manifest.json").is_file()).collect();
    versions.sort_by_key(|p| version_key(&p.file_name().unwrap_or_default().to_string_lossy()));
    for old in versions.iter().rev().skip(2) {
        crate::log_info!("extension {id}: removed the old version {}", old.file_name().unwrap_or_default().to_string_lossy());
        let _ = std::fs::remove_dir_all(old);
    }
    crate::log_info!("extension installed: {} {} ({id}) in {}", ext.name, ext.version, ext.dir.display());
    Ok(ext)
}

fn download(id: &str) -> Result<Vec<u8>, String> {
    let url = download_url(id, &chromium_version());
    crate::log_info!("extension {id}: downloading from the Chrome Web Store ({url})");
    let body = crate::webapp::fetch(&url, MAX_DOWNLOAD).map(|f| f.body).map_err(|e| format!("cannot download {id} from the Chrome Web Store: {e}"))?;
    crate::log_info!("extension {id}: downloaded {} bytes", body.len());
    Ok(body)
}

/// Download an extension from the Chrome Web Store and install it in the profile.
pub fn install(profile_dir: &Path, id: &str) -> Result<Extension, String> {
    install_package(profile_dir, id, &download(id)?)
}

/// Install a newer version if the store has one (`None`: the installed one is the newest).
pub fn update(profile_dir: &Path, id: &str) -> Result<Option<Extension>, String> {
    let have = installed_version(profile_dir, id).ok_or_else(|| format!("{id} is not installed"))?;
    crate::log_info!("extension {id}: asking the Chrome Web Store for a version newer than {have}");
    let answer = crate::webapp::fetch(&update_check_url(id, &have, &chromium_version()), 1_000_000).map_err(|e| format!("cannot ask the Chrome Web Store about {id}: {e}"))?;
    match parse_update_check(&String::from_utf8_lossy(&answer.body)) {
        Some(newest) if version_key(&newest) > version_key(&have) => {
            crate::log_info!("extension {id}: the store has {newest}, installed {have}: updating");
            install(profile_dir, id).map(Some)
        }
        Some(newest) => {
            crate::log_info!("extension {id}: up to date (installed {have}, the store has {newest})");
            Ok(None)
        }
        None => {
            crate::log_info!("extension {id}: up to date (installed {have}; the store reports nothing newer)");
            Ok(None)
        }
    }
}

/// Remove an installed extension (all its versions). `false` if it was not installed.
pub fn remove(profile_dir: &Path, id: &str) -> Result<bool, String> {
    let dir = install_root(profile_dir).join(id);
    if !dir.is_dir() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::log_info!("extension removed: {id} ({})", dir.display());
    Ok(true)
}

// ---- the command line ----------------------------------------------------------------------------

const USAGE: &str = "usage: browsah extension add <ID | Chrome Web Store address> [--profile=ID]
       browsah extension list [--profile=ID]
       browsah extension update [ID] [--profile=ID]
       browsah extension remove <ID> --yes [--profile=ID]";

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned))
}

/// `browsah extension ...`: install, list, update and remove the profile's extensions. Needs no CEF and no window; a running
/// browser loads the changes at its next start.
pub fn cli(args: &[String]) -> i32 {
    let profile_id = flag_value(args, "--profile").unwrap_or_else(|| crate::profile::DEFAULT_ID.to_owned());
    let profile = match crate::profile::Profile::resolve(&profile_id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    // The journal gets what the command does (install, update, remove) under this profile, like the browser's own log.
    let level = crate::config::load_effective(&crate::config::default_path(), Some(&profile.dir.join("profile.yaml"))).map(|l| l.config.logging.level).unwrap_or(crate::log::Level::Info);
    crate::log::init(&profile.id, level);
    let words: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    let installed = |dir: &Path| read_extension(dir, true);
    match args.first().map(String::as_str) {
        Some("add") => {
            let Some(text) = words.first() else {
                eprintln!("{USAGE}");
                return 2;
            };
            let Some(id) = id_from_text(text) else {
                eprintln!("{text:?} is not an extension id (32 letters a to p) or a Chrome Web Store address");
                return 2;
            };
            match install(&profile.dir, &id) {
                Ok(e) => {
                    println!("installed {} {} ({}); restart browsah to load it", e.name, e.version, e.id);
                    let hosts = matching_hosts(&host_search_dirs(&gtk::glib::user_config_dir()), std::slice::from_ref(&e.id));
                    for h in hosts {
                        println!("it uses the native messaging host {} (copied from {} at start-up)", h.name, h.from.display());
                    }
                    0
                }
                Err(e) => {
                    crate::log_error!("extension add {id} failed: {e}");
                    1
                }
            }
        }
        Some("list") => {
            for dir in installed_dirs(&profile.dir) {
                match installed(&dir) {
                    Ok(e) => println!("{}  {:<12} {}", e.id, e.version, e.name),
                    Err(e) => eprintln!("{e}"),
                }
            }
            0
        }
        Some("update") => {
            let ids: Vec<String> = match words.first() {
                Some(t) => match id_from_text(t) {
                    Some(id) => vec![id],
                    None => {
                        eprintln!("{t:?} is not an extension id");
                        return 2;
                    }
                },
                None => installed_dirs(&profile.dir).iter().filter_map(|d| installed(d).ok()).map(|e| e.id).collect(),
            };
            let mut code = 0;
            for id in ids {
                match update(&profile.dir, &id) {
                    Ok(Some(e)) => println!("updated {} to {}; restart browsah to load it", e.name, e.version),
                    Ok(None) => println!("{id} is up to date"),
                    Err(e) => {
                        crate::log_error!("extension update {id} failed: {e}");
                        code = 1;
                    }
                }
            }
            code
        }
        Some("remove") => {
            let Some(id) = words.first().and_then(|t| id_from_text(t)) else {
                eprintln!("{USAGE}");
                return 2;
            };
            if !args.iter().any(|a| a == "--yes") {
                eprintln!("this deletes the extension {id} from the profile; add --yes to do it");
                return 2;
            }
            match remove(&profile.dir, &id) {
                Ok(true) => {
                    println!("removed {id}");
                    0
                }
                Ok(false) => {
                    eprintln!("{id} is not installed");
                    1
                }
                Err(e) => {
                    crate::log_error!("extension remove {id} failed: {e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1Password's public key and the id the Chrome Web Store gives it (measured: an unpacked copy of the Chrome-installed
    /// extension, whose manifest carries this key, loaded under this id).
    const OP_KEY: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAnHpaUll4uWujpAdbIXOQY2WE6hk8PllsYsnoUaj5qHXwv4IB6A9pONqGaTL2KL20u6E6XVhncY6Ae6SQSBQqiIkgjPsiG0NDNsDlju/kzBnfimKFC/bpzOrqFqbhswQHifnet5uHlpG97whTzLO3ka0M5aqB9V9mD/0qVXvNgAVVnSTULH254YqpeCcAhmsKiFZSL6OrOZmCp8kZ/OeOUK9iYWYylL7VcOXVrZf10EPrlaCNXzVk7K35dPuQ7svhA0Pgju3kngB4RLa5Iojhw3IT+B5+m8pisjOSd1oKMrRmhGs7rDhF5IEtAiVxqVp7uOOMPQj3vrbMDAzf7vqLtQIDAQAB";
    const OP_ID: &str = "aeblfdkhhhdcdjpifhhbdiojplfjncoa";

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browsah-ext-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn extensions_id_from_the_public_key_matches_the_web_store() {
        assert_eq!(id_from_bytes(&base64_decode(OP_KEY).unwrap()), OP_ID);
    }

    #[test]
    fn extensions_base64_round_trips() {
        for n in 0..8 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_decode(OP_KEY).map(|d| base64_encode(&d)).as_deref(), Some(OP_KEY));
        assert!(base64_decode("not base64!").is_none());
    }

    #[test]
    fn extensions_read_manifest_name_popup_icon_and_keyed_id() {
        let d = temp("read");
        write(&d.join("manifest.json"), &format!(r#"{{"manifest_version":3,"name":"__MSG_extName__","version":"1.2.3","default_locale":"en","key":"{OP_KEY}","action":{{"default_popup":"popup/index.html"}},"icons":{{"16":"a16.png","128":"a128.png","48":"a48.png"}}}}"#));
        write(&d.join("_locales/en/messages.json"), r#"{"extName":{"message":"My Vault"}}"#);
        let e = read_extension(&d, true).unwrap();
        assert_eq!((e.id.as_str(), e.name.as_str(), e.version.as_str()), (OP_ID, "My Vault", "1.2.3"));
        assert_eq!(e.popup.as_deref(), Some("popup/index.html"));
        assert_eq!(e.icon, Some(d.join("a128.png")), "the largest icon");
        assert!(e.installed);
        assert!(!e.native_messaging);
    }

    #[test]
    fn extensions_native_messaging_permission_is_read_from_the_manifest() {
        let d = temp("nm");
        write(&d.join("manifest.json"), r#"{"name":"x","version":"1","permissions":["storage","nativeMessaging"]}"#);
        assert!(read_extension(&d, false).unwrap().native_messaging);
        write(&d.join("manifest.json"), r#"{"name":"x","version":"1","optional_permissions":["nativeMessaging"]}"#);
        assert!(read_extension(&d, false).unwrap().native_messaging, "an optional permission counts");
        write(&d.join("manifest.json"), r#"{"name":"x","version":"1","permissions":["storage"]}"#);
        assert!(!read_extension(&d, false).unwrap().native_messaging);
    }

    #[test]
    fn extensions_without_a_key_get_the_id_derived_from_the_path() {
        let d = temp("nokey");
        write(&d.join("manifest.json"), r#"{"manifest_version":3,"name":"x","version":"1"}"#);
        let e = read_extension(&d, false).unwrap();
        assert_eq!(e.id, id_from_bytes(d.canonicalize().unwrap().to_string_lossy().as_bytes()));
        assert_eq!(e.id.len(), 32);
        assert!(e.id.bytes().all(|b| (b'a'..=b'p').contains(&b)));
        assert!(read_extension(&d.join("missing"), false).is_err());
    }

    #[test]
    fn extensions_newest_installed_version_loads_and_disabled_and_duplicates_are_skipped() {
        let profile = temp("collect");
        for v in ["8.12.9", "8.12.10"] {
            write(&install_root(&profile).join(OP_ID).join(v).join("manifest.json"), &format!(r#"{{"name":"op","version":"{v}","key":"{OP_KEY}"}}"#));
        }
        let extra = temp("collect-extra");
        write(&extra.join("manifest.json"), r#"{"name":"mine","version":"1"}"#);
        let cfg = ExtensionsConfig { paths: vec![extra.to_string_lossy().into_owned(), "/nonexistent/x".into()], ..Default::default() };
        let (list, warnings) = collect(&profile, &cfg, Path::new("/home/me"));
        assert_eq!(list.iter().map(|e| e.version.as_str()).collect::<Vec<_>>(), ["8.12.10", "1"], "numeric version order, then paths");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let off = ExtensionsConfig { disable: vec![OP_ID.into()], ..cfg };
        assert_eq!(collect(&profile, &off, Path::new("/home/me")).0.len(), 1);
    }

    #[test]
    fn extensions_buttons_workspaces_show_all_but_hidden_apps_show_only_allowed_isolated_show_none() {
        let cfg = ExtensionsConfig { hide: vec!["aaa".into()], show: vec!["bbb".into()], ..Default::default() };
        // A workspace (or the main window): everything except `hide`; `show` means nothing there.
        assert!(!shown_in_window("aaa", &cfg, false, false));
        assert!(shown_in_window("bbb", &cfg, false, false));
        assert!(shown_in_window("ccc", &cfg, false, false), "shown by default");
        // An app: nothing except `show`; `hide` means nothing there.
        assert!(shown_in_window("bbb", &cfg, true, false));
        assert!(!shown_in_window("aaa", &cfg, true, false));
        assert!(!shown_in_window("ccc", &cfg, true, false), "hidden by default");
        // Its own storage: no extensions are loaded there, so no button, whatever the lists say.
        assert!(!shown_in_window("ccc", &cfg, false, true));
        assert!(!shown_in_window("bbb", &cfg, true, true));
    }

    #[test]
    fn extensions_load_flag_merges_with_one_the_person_gave() {
        let mut flags = vec!["--ozone-platform=x11".to_owned(), "--load-extension=/a,/b".into()];
        merge_load_flag(&mut flags, &[PathBuf::from("/b"), PathBuf::from("/c")]);
        assert_eq!(flags, ["--ozone-platform=x11", "--load-extension=/a,/b,/c"]);
        let mut none = vec!["--x".to_owned()];
        merge_load_flag(&mut none, &[]);
        assert_eq!(none, ["--x"], "nothing to load, no switch");
    }

    #[test]
    fn extensions_native_hosts_are_matched_by_origin_name_and_program() {
        let browser = temp("hosts-a");
        let other = temp("hosts-b");
        let program = std::env::current_exe().unwrap();
        let host = |name: &str, origin: &str, path: &str| format!(r#"{{"name":"{name}","path":"{path}","type":"stdio","allowed_origins":["chrome-extension://{origin}/"]}}"#);
        let program = program.to_string_lossy();
        write(&browser.join("com.a.json"), &host("com.a", OP_ID, &program));
        write(&browser.join("com.other.json"), &host("com.other", "pppppppppppppppppppppppppppppppp", &program));
        write(&browser.join("com.wrongname.json"), &host("com.something", OP_ID, &program));
        write(&browser.join("com.noprogram.json"), &host("com.noprogram", OP_ID, "/nonexistent/prog"));
        write(&other.join("com.a.json"), &host("com.a", OP_ID, "/ignored/duplicate"));
        let hosts = matching_hosts(&[browser.clone(), other, PathBuf::from("/nonexistent")], &[OP_ID.to_owned()]);
        assert_eq!(hosts.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["com.a"]);
        assert_eq!(hosts[0].from, browser.join("com.a.json"), "the first folder wins");
        // Copying writes once, then leaves an identical file alone.
        let cef = temp("hosts-cef");
        assert_eq!(copy_hosts(&cef, &hosts), ["com.a"]);
        assert_eq!(std::fs::read_to_string(host_dir(&cef).join("com.a.json")).unwrap(), hosts[0].text);
        assert!(copy_hosts(&cef, &hosts).is_empty());
    }

    #[test]
    fn extensions_host_folders_cover_chrome_and_chromium() {
        let dirs = host_search_dirs(Path::new("/home/me/.config"));
        assert!(dirs.contains(&PathBuf::from("/home/me/.config/google-chrome/NativeMessagingHosts")));
        assert!(dirs.contains(&PathBuf::from("/home/me/.config/chromium/NativeMessagingHosts")));
    }

    #[test]
    fn extensions_store_addresses_and_update_answers() {
        assert_eq!(id_from_text(OP_ID).as_deref(), Some(OP_ID));
        assert_eq!(id_from_text(&format!("https://chromewebstore.google.com/detail/1password/{OP_ID}?hl=en")).as_deref(), Some(OP_ID));
        assert_eq!(id_from_text(&format!("https://chrome.google.com/webstore/detail/1password/{OP_ID}")).as_deref(), Some(OP_ID));
        assert_eq!(id_from_text(&format!("https://evil.example/detail/x/{OP_ID}")), None, "only the store's own addresses");
        assert_eq!(id_from_text("not an id"), None);
        assert_eq!(id_from_text(&OP_ID.to_uppercase()), None);
        let url = download_url(OP_ID, "156.0.8078.88");
        assert!(url.starts_with("https://clients2.google.com/service/update2/crx?response=redirect&prodversion=156.0.8078.88"), "{url}");
        assert!(url.contains(&format!("x=id%3D{OP_ID}")));
        assert!(update_check_url(OP_ID, "1.2.3", "156.0.0.0").contains("%26v%3D1.2.3"));
        let ok = r#"<?xml version="1.0"?><gupdate xmlns="http://www.google.com/update2/response" protocol="2.0"><app appid="x" status="ok"><updatecheck codebase="https://a/b.crx" hash_sha256="h" status="ok" version="8.12.46.29"/></app></gupdate>"#;
        assert_eq!(parse_update_check(ok).as_deref(), Some("8.12.46.29"));
        assert_eq!(parse_update_check(r#"<gupdate><app><updatecheck status="noupdate"/></app></gupdate>"#), None);
        assert_eq!(parse_update_check(r#"<gupdate><app appid="x" status="error-unknownApplication"/></gupdate>"#), None);
        assert_eq!(parse_update_check("<html>"), None);
    }

    #[test]
    fn extensions_install_keeps_the_store_id_the_two_newest_versions_and_refuses_bad_packages() {
        use crate::crx::tests::{ecdsa_key, ecdsa_package_with, zip_of};
        let profile = temp("install");
        let key = ecdsa_key();
        let build = |version: &str| {
            let manifest = format!(r#"{{"manifest_version":3,"name":"Demo","version":"{version}","action":{{"default_popup":"p.html"}}}}"#);
            ecdsa_package_with(&key, &zip_of(&[("manifest.json", manifest.as_str()), ("p.html", "<p>hi")]))
        };
        let (id, v1) = build("1.0.0");
        let ext = install_package(&profile, &id, &v1).unwrap();
        assert_eq!((ext.id.as_str(), ext.version.as_str(), ext.popup.as_deref()), (id.as_str(), "1.0.0", Some("p.html")), "the signing key in the manifest gives the store id");
        assert_eq!(installed_version(&profile, &id).as_deref(), Some("1.0.0"));
        assert!(install_package(&profile, &id, &v1).unwrap_err().contains("already installed"));
        assert!(install_package(&profile, "pppppppppppppppppppppppppppppppp", &v1).is_err(), "a package for another id");
        for v in ["1.0.1", "1.0.10"] {
            install_package(&profile, &id, &build(v).1).unwrap();
        }
        let mut kept: Vec<String> = std::fs::read_dir(install_root(&profile).join(&id)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        kept.sort();
        assert_eq!(kept, ["1.0.1", "1.0.10"], "the two newest, in numeric order, and no leftovers");
        assert_eq!(installed_version(&profile, &id).as_deref(), Some("1.0.10"));
        let (id2, bad) = ecdsa_package_with(&ecdsa_key(), &zip_of(&[("manifest.json", r#"{"name":"x","version":"../evil"}"#)]));
        assert!(install_package(&profile, &id2, &bad).unwrap_err().contains("not a number"));
        assert!(!install_root(&profile).join(&id2).join("../evil").exists());
        assert!(remove(&profile, &id).unwrap());
        assert!(!remove(&profile, &id).unwrap());
    }
}
