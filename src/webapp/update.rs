//! Explicit manifest updates. Keep provenance separate from the person's app.yaml.
use super::{AppConfig, ManifestInstall};
use crate::{manifest::{self, http_origin, same_origin, AppInfo}, profile::{ColorPair, Profile}, workspace::{first_positional, Workspace}};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

const SOURCE: &str = ".manifest-source.json";
type Fields = BTreeMap<String, Value>;

#[derive(Clone, Serialize, Deserialize)]
struct Icon {
    file: String,
    digest: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Source {
    page: String,
    manifest: String,
    id: String,
    fields: Fields,
    icon: Option<Icon>,
    #[serde(default)]
    custom_icon: bool,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    config_digest: String,
    source: Source,
}

#[derive(Serialize, Deserialize)]
struct Record {
    source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Pending>,
}

fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn color_value(color: Option<&ColorPair>) -> Value {
    color.map_or(Value::Null, |color| json!({"light": color.light, "dark": color.dark}))
}

fn seed_fields(info: &AppInfo) -> Fields {
    let color = info.color.as_deref().and_then(|color| ColorPair::parse(color).ok());
    fields(Some(&info.name), &info.start_url, &info.scope, super::display_name_of(info.display), color.as_ref())
}

fn fields(name: Option<&str>, url: &str, scope: &[String], display: &str, color: Option<&ColorPair>) -> Fields {
    [
        ("name".to_owned(), json!(name)),
        ("url".to_owned(), json!(url)),
        ("scope".to_owned(), json!(scope)),
        ("display".to_owned(), json!(display)),
        ("color".to_owned(), color_value(color)),
    ].into_iter().collect()
}

fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    gtk::glib::file_set_contents(path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_record(dir: &Path, source: Source, pending: Option<Pending>) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(&Record { source, pending }).map_err(|e| e.to_string())?;
    write(&dir.join(SOURCE), &data)
}

pub(super) fn remember_install(dir: &Path, install: &ManifestInstall, icon: Option<(&str, &[u8])>) -> Result<(), String> {
    let info = &install.info;
    let Some(id) = &info.manifest_id else { return Ok(()) };
    let Some(url) = &info.manifest_url else { return Ok(()) };
    write_record(dir, Source {
        page: info.manifest_page.clone().unwrap_or_else(|| install.page_url.clone()), manifest: url.clone(), id: id.clone(), fields: seed_fields(info),
        icon: icon.map(|(file, data)| Icon { file: file.to_owned(), digest: digest(data) }), custom_icon: install.custom_icon,
    }, None)
}

fn read_source(dir: &Path, text: &str) -> Result<Option<Source>, String> {
    match fs::read(dir.join(SOURCE)) {
        Ok(data) => {
            let record: Record = serde_json::from_slice(&data).map_err(|e| format!("invalid manifest metadata: {e}"))?;
            Ok(Some(record.pending.filter(|p| p.config_digest == digest(text.as_bytes())).map_or(record.source, |p| p.source)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read manifest metadata: {e}")),
    }
}

fn manifest_url(source: Option<&Source>, app: &AppConfig) -> Result<String, String> {
    if let Some(source) = source {
        return Ok(source.manifest.clone());
    }
    let page = super::fetch(&app.url, 2_000_000)?;
    if !same_origin(&app.url, &page.url) {
        return Err("the start page redirected to another origin".into());
    }
    manifest::page_links(&String::from_utf8_lossy(&page.body), &page.url).0.ok_or_else(|| "the start page has no discoverable manifest".into())
}

fn fetch_manifest(url: &str, page: &str, id: &str) -> Result<AppInfo, String> {
    if http_origin(url).is_none() || !same_origin(page, id) {
        return Err("the installed manifest origin could not be verified".into());
    }
    let response = super::fetch(url, 1_000_000)?;
    if !same_origin(url, &response.url) {
        return Err("the manifest redirected to another origin".into());
    }
    let info = manifest::parse_raw_manifest(&String::from_utf8_lossy(&response.body), &response.url, page)
        .ok_or("the site returned an invalid manifest")?;
    if info.manifest_id.as_deref() != Some(id) || !same_origin(&info.start_url, id) {
        return Err("the manifest identifies a different web app".into());
    }
    Ok(info)
}

fn choose_fields(old: &Fields, local: &Fields, remote: &Fields) -> Fields {
    remote.iter().filter(|(key, value)| old.get(*key) == local.get(*key) && local.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone())).collect()
}

/// Rewrite only selected top-level keys. Confirm the whole resulting YAML is semantically
/// equivalent to the intended edit before allowing a write.
fn patch_yaml(text: &str, fields: &Fields) -> Result<String, String> {
    let old: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    let mut expected = old.as_mapping().cloned().ok_or("app.yaml must be a mapping")?;
    let mut output = String::new();
    let mut written = Vec::new();
    let mut replacing = false;
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            output.push_str(raw);
            continue;
        }
        if replacing && (line.starts_with(char::is_whitespace) || line.starts_with("- ")) {
            continue;
        }
        replacing = false;
        if line == trimmed {
            if let Some((key, old_value)) = line.split_once(':') {
                if let Some(value) = fields.get(key) {
                    output.push_str(&format!("{key}: {value}"));
                    if let (_, Some(comment)) = crate::fullconfig::split_comment(old_value) {
                        output.push(' ');
                        output.push_str(comment);
                    }
                    output.push_str(ending);
                    written.push(key);
                    replacing = true;
                    continue;
                }
            }
        }
        output.push_str(raw);
    }
    for (key, value) in fields {
        let value_text = value.to_string();
        expected.insert(key.clone().into(), serde_yaml_ng::from_str(&value_text).map_err(|e| e.to_string())?);
        if !written.contains(&key.as_str()) {
            if !output.ends_with('\n') {
                output.push_str(ending);
            }
            output.push_str(&format!("{key}: {value_text}{ending}"));
        }
    }
    let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&output).map_err(|e| format!("cannot safely edit app.yaml: {e}"))?;
    if parsed.as_mapping() != Some(&expected) {
        return Err("cannot safely edit this app.yaml layout".into());
    }
    super::parse_app(&output)?;
    Ok(output)
}

fn icon_matches(dir: &Path, name: Option<&str>, tracked: Option<&Icon>) -> bool {
    match (name, tracked) {
        (None, None) => true,
        (Some(name), Some(icon)) if name == icon.file => fs::read(dir.join(name)).is_ok_and(|bytes| digest(&bytes) == icon.digest),
        _ => false,
    }
}

pub(super) fn cli(profile: &Profile, args: &[String], exe: &Path, libs: &Path) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{}", super::USAGE);
        return 2;
    };
    let apply = args.iter().any(|arg| arg == "--apply");
    match update(profile, id, apply, exe, libs) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("cannot update web app {id:?}: {e}");
            1
        }
    }
}

fn update(profile: &Profile, id: &str, apply: bool, exe: &Path, libs: &Path) -> Result<(), String> {
    let workspace = Workspace::resolve_app(profile, id)?;
    let dir = &workspace.dir;
    let file = dir.join("app.yaml");
    let lock = fs::File::open(dir).map_err(|e| e.to_string())?;
    if apply {
        lock.lock().map_err(|e| e.to_string())?;
    }
    if apply && !fs::symlink_metadata(&file).map_err(|e| e.to_string())?.file_type().is_file() {
        return Err("app.yaml must be a regular file to apply updates".into());
    }
    let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let app = super::parse_app(&text)?;
    let id_from_manifest = app.manifest_id.as_deref().ok_or("this app was not installed from a web manifest")?;
    let previous = read_source(dir, &text)?;
    if previous.as_ref().is_some_and(|source| source.id != id_from_manifest) {
        return Err("the manifest ID in app.yaml differs from the installed identity".into());
    }
    let page = previous.as_ref().map_or(app.url.as_str(), |source| source.page.as_str());
    if !same_origin(page, id_from_manifest) {
        return Err("the installed manifest origin could not be verified".into());
    }
    let url = manifest_url(previous.as_ref(), &app)?;
    let info = fetch_manifest(&url, page, id_from_manifest)?;
    let remote = seed_fields(&info);
    let local = fields(app.name.as_deref(), &app.url, &app.scope, super::display_name_of(app.display), app.color.as_ref());

    let Some(mut source) = previous else {
        println!("This app predates manifest tracking. Existing settings and icon will be kept.");
        for (key, value) in &remote {
            if local.get(key) != Some(value) {
                println!("  {key}: saved {} · manifest {value}", local[key]);
            }
        }
        let icon = app.icon.as_deref().filter(|name| matches!(*name, "icon.png" | "icon.svg")).and_then(|name| {
            let (_, bytes, _) = super::fetch_icon(&info.icons)?;
            let candidate = Icon { file: name.to_owned(), digest: digest(&bytes) };
            icon_matches(dir, Some(name), Some(&candidate)).then_some(candidate)
        });
        if icon.is_none() && !info.icons.is_empty() {
            println!("  icon: the manifest offers an icon; the saved icon is kept during enrollment.");
        }
        if apply {
            write_record(dir, Source {
                page: page.to_owned(), manifest: info.manifest_url.clone().unwrap_or(url), id: id_from_manifest.to_owned(),
                fields: remote, custom_icon: app.icon.is_some() && icon.is_none(), icon,
            }, None)?;
            println!("Manifest tracking enabled.");
        } else {
            println!("Run with --apply to enable tracking for future updates.");
        }
        return Ok(());
    };

    let mut changes = choose_fields(&source.fields, &local, &remote);
    if changes.contains_key("url") || changes.contains_key("scope") {
        let start = changes.get("url").and_then(Value::as_str).unwrap_or(&app.url);
        let scope = changes.get("scope").unwrap_or(&local["scope"]);
        let matches = scope.as_array().is_some_and(|entries| entries.is_empty() || entries.iter().filter_map(Value::as_str)
            .any(|pattern| crate::workspace::scope_matches(pattern, start)));
        if !matches {
            changes.remove("url");
            changes.remove("scope");
        }
    }
    for (key, value) in &remote {
        if local.get(key) != Some(value) && !changes.contains_key(key) {
            println!("  Keeping {key}: {} (manifest offers {value})", local[key]);
        }
    }
    let mut new_icon = None;
    let managed_icon = !source.custom_icon && icon_matches(dir, app.icon.as_deref(), source.icon.as_ref());
    if managed_icon {
        if let Some((ext, bytes, _)) = super::fetch_icon(&info.icons) {
            let hash = digest(&bytes);
            if source.icon.as_ref().is_none_or(|old| old.digest != hash) {
                let file = format!("manifest-icon-{hash}.{ext}");
                changes.insert("icon".to_owned(), json!(file));
                new_icon = Some((Icon { file, digest: hash }, bytes));
            }
        } else if !info.icons.is_empty() {
            println!("  Keeping the installed icon: no manifest icon could be downloaded.");
        } else if source.icon.is_some() {
            changes.insert("icon".to_owned(), Value::Null);
        }
    } else {
        println!("  Keeping the local icon.");
    }

    for (key, value) in &changes {
        if key == "icon" {
            println!("  icon: {}", if value.is_null() { "use the generated icon" } else { "replace from the manifest" });
        } else {
            println!("  {key}: {} → {value}", local[key]);
        }
    }
    if changes.is_empty() {
        println!("No manifest changes to apply.");
    } else if !apply {
        println!("Run with --apply to install these changes.");
    }
    if !apply {
        return Ok(());
    }

    if !changes.is_empty() {
        let updated = patch_yaml(&text, &changes)?;
        let old = source.clone();
        source.manifest = info.manifest_url.unwrap_or(url);
        for (key, value) in &changes {
            if key != "icon" {
                source.fields.insert(key.clone(), value.clone());
            }
        }
        if let Some((icon, bytes)) = new_icon {
            write(&dir.join(&icon.file), &bytes)?;
            source.icon = Some(icon);
        } else if changes.contains_key("icon") {
            source.icon = None;
        }
        // If interrupted, only treat the proposed values as imported once the matching YAML exists.
        write_record(dir, old, Some(Pending { config_digest: digest(updated.as_bytes()), source: source.clone() }))?;
        if fs::read_to_string(&file).map_err(|e| e.to_string())? != text {
            return Err("app.yaml changed during the update; run the command again".into());
        }
        write(&file, updated.as_bytes())?;
        write_record(dir, source, None)?;
    }
    let updated = Workspace::resolve_app(profile, id)?;
    super::install(profile, &updated, exe, libs).map_err(|e| format!("cannot refresh the launcher: {e}"))?;
    if !changes.is_empty() {
        println!("Updated {id} and refreshed its launcher.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_fields_still_matching_the_import_change() {
        let old = fields(Some("Site"), "https://site.example/old", &[], "minimal", None);
        let local = fields(Some("My app"), "https://site.example/old", &["personal.example/*".into()], "custom", None);
        let remote = fields(Some("New site"), "https://site.example/new", &["site.example/new/*".into()], "browser", Some(&ColorPair::parse("#abc").unwrap()));
        let changes = choose_fields(&old, &local, &remote);
        assert_eq!(changes.keys().map(String::as_str).collect::<Vec<_>>(), ["color", "url"]);
    }

    #[test]
    fn patch_preserves_unrelated_yaml_and_comments() {
        let text = "# Local app\nurl: https://site.example/\ncolor: # colour note\n  light: '#aaa'\n  dark: '#bbb'\n# My overrides\nscope:\n- site.example/*\nwatch_config: true # retain\n";
        let update = [
            ("color".to_owned(), Value::Null),
            ("scope".to_owned(), json!(["site.example/mail/*"])),
            ("name".to_owned(), json!("New: name # punctuation")),
        ].into_iter().collect();
        let output = patch_yaml(text, &update).unwrap();
        let app = super::super::parse_app(&output).unwrap();
        assert_eq!(app.scope, ["site.example/mail/*"]);
        assert_eq!(app.name.as_deref(), Some("New: name # punctuation"));
        assert!(app.color.is_none());
        assert!(output.contains("color: null # colour note\n"));
        assert!(output.contains("# My overrides\n"));
        assert!(output.contains("watch_config: true # retain\n"));
        assert!(patch_yaml("{url: 'https://site.example/'}", &update).is_err());
    }

    #[test]
    fn origins_include_scheme_host_and_port() {
        assert!(same_origin("https://EXAMPLE.com:443/path", "https://example.com/"));
        assert!(!same_origin("https://example.com:444/", "https://example.com/"));
        assert!(!same_origin("http://example.com/", "https://example.com/"));
        assert!(!same_origin("https://example.com@another.example/", "https://another.example/"));
    }
}
