//! Downloads: where files go, and what to call them.
//!
//! `downloads:` in the configuration (any layer) chooses the folder and whether to ask for each file.
//! The handler that receives the files is in `app/view.rs`; messages and the "save as" question are
//! in `app/downloads.rs`. This module has the pure parts.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The `downloads:` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct DownloadsConfig {
    /// The folder files are saved in; `~` is your home folder. Without one: your Downloads folder.
    pub directory: Option<String>,
    /// Ask where to save each file (the folder is where the question starts).
    pub ask: bool,
}

impl Default for DownloadsConfig {
    fn default() -> Self {
        Self { directory: None, ask: false }
    }
}

/// The folder to save in: the configured one with `~` expanded, else the Downloads folder, else the
/// home folder.
pub fn resolve_directory(cfg: &DownloadsConfig) -> PathBuf {
    let home = gtk::glib::home_dir();
    match cfg.directory.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => expand_tilde(d, &home),
        None => gtk::glib::user_special_dir(gtk::glib::UserDirectory::Downloads).unwrap_or(home),
    }
}

/// `~` and `~/rest` mean the home folder; everything else is as written.
pub fn expand_tilde(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        home.to_path_buf()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// A file name that is safe to create: no folders, no leading dots that hide it, never empty.
pub fn sanitise_name(suggested: &str) -> String {
    let base = suggested.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().map(|c| if c.is_control() || c == ':' { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() { "download".to_owned() } else { trimmed.to_owned() }
}

/// `dir/name`, or `dir/name (1).ext`, `(2)`... if that exists, so nothing is overwritten.
pub fn unique_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let name = sanitise_name(name);
    let first = dir.join(&name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    (1..10_000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !exists(p))
        .unwrap_or_else(|| dir.join(format!("{stem} ({}){ext}", std::process::id())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tilde_expansion() {
        let home = Path::new("/home/me");
        assert_eq!(expand_tilde("~", home), PathBuf::from("/home/me"));
        assert_eq!(expand_tilde("~/Downloads/x", home), PathBuf::from("/home/me/Downloads/x"));
        assert_eq!(expand_tilde("/srv/files", home), PathBuf::from("/srv/files"));
        assert_eq!(expand_tilde("~other/x", home), PathBuf::from("~other/x"), "only the current user's home");
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(sanitise_name("report.pdf"), "report.pdf");
        assert_eq!(sanitise_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitise_name("C:\\dir\\file.txt"), "file.txt");
        assert_eq!(sanitise_name(".bashrc"), "bashrc");
        assert_eq!(sanitise_name(""), "download");
        assert_eq!(sanitise_name("a:b.txt"), "a_b.txt");
        assert_eq!(sanitise_name("   "), "download");
        assert_eq!(sanitise_name("dir/"), "download");
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        let dir = Path::new("/d");
        let taken: HashSet<PathBuf> = ["/d/note.txt", "/d/note (1).txt", "/d/archive.tar.gz", "/d/README", "/d/.hidden"].iter().map(PathBuf::from).collect();
        let exists = |p: &Path| taken.contains(p);
        assert_eq!(unique_path(dir, "fresh.txt", exists), PathBuf::from("/d/fresh.txt"));
        assert_eq!(unique_path(dir, "note.txt", exists), PathBuf::from("/d/note (2).txt"));
        assert_eq!(unique_path(dir, "archive.tar.gz", exists), PathBuf::from("/d/archive.tar (1).gz"));
        assert_eq!(unique_path(dir, "README", exists), PathBuf::from("/d/README (1)"));
        assert_eq!(unique_path(dir, "../note.txt", exists), PathBuf::from("/d/note (2).txt"), "the name is sanitised first");
    }

    #[test]
    fn default_directory_is_a_real_folder_when_unset() {
        let d = resolve_directory(&DownloadsConfig::default());
        assert!(d.is_absolute());
        let mine = resolve_directory(&DownloadsConfig { directory: Some("~/x".into()), ask: true });
        assert!(mine.ends_with("x") && mine.is_absolute());
        assert_eq!(resolve_directory(&DownloadsConfig { directory: Some("  ".into()), ask: false }), d, "blank means unset");
    }
}
