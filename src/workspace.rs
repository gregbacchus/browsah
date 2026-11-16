//! Workspaces: a named window of tabs that belongs to a profile.
//!
//! A workspace has its own directory, `<profile>/workspaces/<id>/`, holding `workspace.yaml`
//! (hand edited, see docs/config/workspace.md). Its tabs, bookmarks and so on live in the profile's
//! database (later stages), keyed by the workspace id. All workspaces of a profile are windows of
//! the profile's one process and share its application id.

use crate::profile::{valid_id, ColorPair, Profile};
use serde::Deserialize;
use std::path::{Path, PathBuf};

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
}

/// The keys of `workspace.yaml` that are not configuration overrides.
pub const IDENTITY_KEYS: &[&str] = &["name", "color"];

#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub config: WorkspaceConfig,
    pub dir: PathBuf,
}

impl Workspace {
    pub fn display_name(&self) -> &str {
        self.config.name.as_deref().unwrap_or(&self.id)
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

fn first_positional(args: &[String]) -> Option<&String> {
    // Skip `--flag value` pairs and `--flag=value`.
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            i += if a.contains('=') || a == "--yes" { 1 } else { 2 };
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
    if dir.join("workspace.yaml").exists() {
        eprintln!("workspace {id:?} already exists in profile {:?}", profile.id);
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
    fn docs_cover_every_workspace_option() {
        let docs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/config/workspace.md")).expect("docs/config/workspace.md exists");
        let err = serde_yaml_ng::from_str::<WorkspaceConfig>("{ __nope__: 1 }").err().unwrap().to_string();
        let fields: Vec<&str> = err.split("expected").nth(1).unwrap_or("").split('`').skip(1).step_by(2).collect();
        assert!(!fields.is_empty());
        for f in fields {
            assert!(docs.contains(&format!("| `{f}`")), "workspace option {f} is not documented in docs/config/workspace.md");
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
