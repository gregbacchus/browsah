//! `sudo browsah system-install`: copy the program to a place only root can change, so programs that check the browser before
//! trusting it will accept it. 1Password's desktop app is the one this is for: its browser integration refuses a browser program
//! that you could modify yourself (measured: "binary permission verification failed" for a build in `target/release`;
//! docs/research/chrome-extensions.md), and a Homebrew install is owned by you.
//!
//! The command prints every change it will make and asks before making them; `--dry-run` only prints. It never touches a profile,
//! an extension or a setting of yours, and the 1Password line in `/etc/1password/custom_allowed_browsers` is added only when
//! `--1password` is given. `sudo browsah system-uninstall` takes it all away again. The planning and copying are separate from the
//! command line so they are tested with temporary folders (no root needed); what root adds, the ownership, is not tested.

use std::io::{BufRead, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const DEFAULT_PREFIX: &str = "/opt/browsah";
pub const DEFAULT_BIN_DIR: &str = "/usr/local/bin";
pub const DEFAULT_ALLOWLIST: &str = "/etc/1password/custom_allowed_browsers";

/// What the launcher file says about itself: `system-uninstall` only removes a launcher that has it.
const MARKER: &str = "# installed by `browsah system-install`";

/// Files that make up a working install, as the release tarball lists them (`.github/workflows/release.yml`).
const REQUIRED: &[&str] = &["browsah", "libcef.so", "icudtl.dat", "v8_context_snapshot.bin", "resources.pak", "chrome_100_percent.pak"];
const OPTIONAL: &[&str] = &["libvk_swiftshader.so", "libvulkan.so.1", "vk_swiftshader_icd.json", "chrome_200_percent.pak", "CREDITS.html", "LICENSE"];

/// Everything the command will do.
#[derive(Debug)]
pub struct Plan {
    /// The folder with the program being copied.
    pub source: PathBuf,
    /// `(path relative to the source and the destination, mode)`.
    pub files: Vec<(PathBuf, u32)>,
    pub bytes: u64,
    pub prefix: PathBuf,
    pub bin_dir: PathBuf,
    /// The 1Password allow-list file to add `browsah` to (only with `--1password`).
    pub allowlist: Option<PathBuf>,
}

impl Plan {
    pub fn launcher_path(&self) -> PathBuf {
        self.bin_dir.join("browsah")
    }
}

/// The launcher script: sets what the program needs to find `libcef.so`, and tells it its own stable paths (as Homebrew's does).
pub fn launcher_script(prefix: &Path, launcher: &Path) -> String {
    format!(
        "#!/bin/sh\n{MARKER}\nexport BROWSAH_LAUNCHER=\"{launcher}\"\nexport BROWSAH_LIBS=\"{prefix}\"\nexport LD_LIBRARY_PATH=\"{prefix}${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}\"\nexec \"{prefix}/browsah\" \"$@\"\n",
        launcher = launcher.display(),
        prefix = prefix.display()
    )
}

/// The files to copy from `source` (the release's list, plus the language files). An error names what is missing.
pub fn plan(source: &Path, prefix: &Path, bin_dir: &Path, allowlist: Option<&Path>) -> Result<Plan, String> {
    let mut files: Vec<(PathBuf, u32)> = Vec::new();
    let mut bytes = 0;
    let mut missing = Vec::new();
    for name in REQUIRED.iter().chain(OPTIONAL) {
        match std::fs::metadata(source.join(name)) {
            Ok(m) if m.is_file() => {
                let executable = *name == "browsah" || name.contains(".so");
                files.push((PathBuf::from(name), if executable { 0o755 } else { 0o644 }));
                bytes += m.len();
            }
            _ if REQUIRED.contains(name) => missing.push(*name),
            _ => {}
        }
    }
    if !missing.is_empty() {
        return Err(format!("{} does not look like a browsah install: missing {}", source.display(), missing.join(", ")));
    }
    if let Ok(entries) = std::fs::read_dir(source.join("locales")) {
        let mut names: Vec<PathBuf> = entries.flatten().filter(|e| e.path().is_file()).map(|e| PathBuf::from("locales").join(e.file_name())).collect();
        names.sort();
        for n in names {
            bytes += std::fs::metadata(source.join(&n)).map(|m| m.len()).unwrap_or(0);
            files.push((n, 0o644));
        }
    }
    if !prefix.is_absolute() || prefix.components().count() < 3 {
        return Err(format!("the install folder must be an absolute path at least two levels deep, like /opt/browsah (got {})", prefix.display()));
    }
    if source == prefix {
        return Err(format!("this is already the installed copy in {}: run system-install from the copy you want to install (Homebrew's `\"$(brew --prefix)/bin/browsah\"`, or your build), not from the installed one (/usr/local/bin/browsah)", prefix.display()));
    }
    if prefix.starts_with(source) || source.starts_with(prefix) {
        return Err(format!("the install folder {} and the folder being copied {} overlap", prefix.display(), source.display()));
    }
    if let Some(list) = allowlist {
        if !list.is_file() {
            return Err(format!("{} does not exist: is the 1Password desktop app installed? (nothing was changed)", list.display()));
        }
    }
    Ok(Plan { source: source.to_path_buf(), files, bytes, prefix: prefix.to_path_buf(), bin_dir: bin_dir.to_path_buf(), allowlist: allowlist.map(Path::to_path_buf) })
}

/// The plan in words: every file or folder that will be created or changed, and why.
pub fn describe(p: &Plan) -> String {
    let mut s = String::new();
    s.push_str("browsah system-install will make these changes:\n\n");
    s.push_str(&format!("  1. Copy the program ({} files, {:.0} MB) from\n       {}\n     to\n       {}\n     owned by root and not writable by anyone else. An older copy there is replaced.\n", p.files.len(), p.bytes as f64 / 1e6, p.source.display(), p.prefix.display()));
    s.push_str(&format!("  2. Write the start-up script {} (sets the library path, then runs {}/browsah).\n     It replaces an earlier script of this command; any other file there is left alone and stops the install.\n", p.launcher_path().display(), p.prefix.display()));
    match &p.allowlist {
        Some(l) => s.push_str(&format!("  3. Add the line `browsah` to {}\n     (1Password's list of browsers it trusts; skipped if it is already there).\n", l.display())),
        None => s.push_str("  3. Nothing for 1Password: add `--1password` to also add `browsah` to its list of trusted browsers,\n     or add the line `browsah` to /etc/1password/custom_allowed_browsers yourself.\n"),
    }
    s.push_str("\nWhy: 1Password only trusts a browser program that is owned by root and that you cannot modify. A build you made, or a\nHomebrew install (owned by you), is refused.\nNothing else changes: your profiles, extensions and settings stay where they are.\n");
    s
}

fn permission_hint(e: &std::io::Error, path: &Path) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        format!("{}: permission denied. This command changes system folders: run it with sudo and the full path of the copy you are installing, for Homebrew `sudo \"$(brew --prefix)/bin/browsah\" system-install`", path.display())
    } else {
        format!("{}: {e}", path.display())
    }
}

fn is_ours(launcher: &Path) -> bool {
    std::fs::read_to_string(launcher).map(|t| t.contains(MARKER)).unwrap_or(false)
}

/// Add `browsah` to the allow-list unless a line already says it. Returns whether the file changed.
fn add_to_allowlist(path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| permission_hint(&e, path))?;
    if text.lines().any(|l| l.trim() == "browsah") {
        return Ok(false);
    }
    let mut new = text;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str("browsah\n");
    std::fs::write(path, new).map_err(|e| permission_hint(&e, path))?;
    Ok(true)
}

fn remove_from_allowlist(path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| permission_hint(&e, path))?;
    if !text.lines().any(|l| l.trim() == "browsah") {
        return Ok(false);
    }
    let kept: String = text.lines().filter(|l| l.trim() != "browsah").map(|l| format!("{l}\n")).collect();
    std::fs::write(path, kept).map_err(|e| permission_hint(&e, path))?;
    Ok(true)
}

/// Carry out the plan. Returns what was done, one line each. The copy is made beside the destination and renamed into place, so
/// a failure part-way leaves the old install as it was.
pub fn apply(p: &Plan) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let launcher = p.launcher_path();
    if launcher.exists() && !is_ours(&launcher) {
        return Err(format!("{} exists and was not made by this command; it is left alone. Remove it, or choose another place with --bin-dir=DIR", launcher.display()));
    }
    let pid = std::process::id();
    let name = p.prefix.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = p.prefix.parent().ok_or("no parent folder")?;
    let staging = parent.join(format!(".{name}.new-{pid}"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| permission_hint(&e, &staging))?;
    let result = (|| -> Result<(), String> {
        for (rel, mode) in &p.files {
            let to = staging.join(rel);
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir).map_err(|e| permission_hint(&e, dir))?;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).map_err(|e| permission_hint(&e, dir))?;
            }
            std::fs::copy(p.source.join(rel), &to).map_err(|e| format!("copying {}: {e}", rel.display()))?;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(*mode)).map_err(|e| permission_hint(&e, &to))?;
        }
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).map_err(|e| permission_hint(&e, &staging))?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let old = parent.join(format!(".{name}.old-{pid}"));
    if p.prefix.exists() {
        std::fs::rename(&p.prefix, &old).map_err(|e| permission_hint(&e, &p.prefix))?;
    }
    if let Err(e) = std::fs::rename(&staging, &p.prefix) {
        let _ = std::fs::rename(&old, &p.prefix); // put the old one back
        let _ = std::fs::remove_dir_all(&staging);
        return Err(permission_hint(&e, &p.prefix));
    }
    let _ = std::fs::remove_dir_all(&old);
    done.push(format!("copied the program to {}", p.prefix.display()));

    std::fs::create_dir_all(&p.bin_dir).map_err(|e| permission_hint(&e, &p.bin_dir))?;
    let temp = p.bin_dir.join(format!(".browsah.new-{pid}"));
    std::fs::write(&temp, launcher_script(&p.prefix, &launcher)).map_err(|e| permission_hint(&e, &temp))?;
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755)).map_err(|e| permission_hint(&e, &temp))?;
    std::fs::rename(&temp, &launcher).map_err(|e| permission_hint(&e, &launcher))?;
    done.push(format!("wrote the start-up script {}", launcher.display()));

    if let Some(list) = &p.allowlist {
        done.push(if add_to_allowlist(list)? { format!("added `browsah` to {}", list.display()) } else { format!("`browsah` was already in {}", list.display()) });
    }
    Ok(done)
}

/// Remove what `apply` made. The install folder is only removed when it looks like one (it has `libcef.so` and `browsah`), and the
/// start-up script only when this command wrote it.
pub fn uninstall(prefix: &Path, bin_dir: &Path, allowlist: Option<&Path>) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    if prefix.join("libcef.so").is_file() && prefix.join("browsah").is_file() && prefix.components().count() >= 3 {
        std::fs::remove_dir_all(prefix).map_err(|e| permission_hint(&e, prefix))?;
        done.push(format!("removed {}", prefix.display()));
    } else if prefix.exists() {
        return Err(format!("{} does not look like a browsah install; left alone", prefix.display()));
    }
    let launcher = bin_dir.join("browsah");
    if launcher.exists() {
        if is_ours(&launcher) {
            std::fs::remove_file(&launcher).map_err(|e| permission_hint(&e, &launcher))?;
            done.push(format!("removed {}", launcher.display()));
        } else {
            done.push(format!("{} was not made by this command; left alone", launcher.display()));
        }
    }
    if let Some(list) = allowlist {
        if list.is_file() && remove_from_allowlist(list)? {
            done.push(format!("removed `browsah` from {}", list.display()));
        }
    }
    Ok(done)
}

/// Is this file owned by root and not writable by anyone else (what 1Password asks of the browser program)?
pub fn root_owned_and_locked(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.uid() == 0 && m.mode() & 0o022 == 0).unwrap_or(false)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned))
}

const USAGE: &str = "usage: sudo BROWSAH system-install [--1password] [--dry-run] [--yes] [--prefix=DIR] [--bin-dir=DIR]
       sudo BROWSAH system-uninstall [--1password] [--yes] [--prefix=DIR] [--bin-dir=DIR]

BROWSAH is the full path of the copy to install: \"$(brew --prefix)/bin/browsah\" for Homebrew, or ./target/release/browsah for a build
(not /usr/local/bin/browsah, which is the installed copy).

system-install copies the program to /opt/browsah (owned by root), writes /usr/local/bin/browsah, and with --1password adds
`browsah` to 1Password's list of trusted browsers. It prints what it will do and asks first (--yes skips the question,
--dry-run only prints). system-uninstall removes them again.";

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok() && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `browsah system-install` and `browsah system-uninstall`.
pub fn cli(command: &str, args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let prefix = PathBuf::from(flag(args, "--prefix").unwrap_or_else(|| DEFAULT_PREFIX.into()));
    let bin_dir = PathBuf::from(flag(args, "--bin-dir").unwrap_or_else(|| DEFAULT_BIN_DIR.into()));
    let allowlist_path = PathBuf::from(flag(args, "--allowlist-file").unwrap_or_else(|| DEFAULT_ALLOWLIST.into()));
    let want_1password = args.iter().any(|a| a == "--1password");
    let yes = args.iter().any(|a| a == "--yes");
    let dry = args.iter().any(|a| a == "--dry-run");
    let allowlist = want_1password.then_some(allowlist_path.as_path());
    if command == "system-uninstall" {
        println!("browsah system-uninstall will remove {} and {}/browsah (only if this command made them){}.", prefix.display(), bin_dir.display(), if want_1password { format!(", and the line `browsah` from {}", allowlist_path.display()) } else { String::new() });
        if !yes && !confirm("Proceed?") {
            println!("nothing was changed");
            return 1;
        }
        return match uninstall(&prefix, &bin_dir, allowlist) {
            Ok(done) => {
                done.iter().for_each(|d| println!("{d}"));
                0
            }
            Err(e) => {
                crate::log_error!("system-uninstall failed: {e}");
                1
            }
        };
    }
    let source = match std::env::current_exe().and_then(|p| p.canonicalize()).map(|p| p.parent().map(Path::to_path_buf)) {
        Ok(Some(dir)) => dir,
        _ => {
            eprintln!("cannot find the folder this program runs from");
            return 1;
        }
    };
    let plan = match plan(&source, &prefix, &bin_dir, allowlist) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    println!("{}", describe(&plan));
    if unsafe { libc::geteuid() } != 0 {
        println!("Note: you are not root. Files written now would be owned by you, which 1Password refuses. Run it with sudo.\n");
    }
    if dry {
        println!("(dry run: nothing was changed)");
        return 0;
    }
    if !yes && !confirm("Proceed?") {
        println!("nothing was changed");
        return 1;
    }
    match apply(&plan) {
        Ok(done) => {
            done.iter().for_each(|d| println!("{d}"));
            println!("\nDone. Next:");
            println!("  - start browsah as {} (a `browsah` earlier in your PATH, such as Homebrew's, is a different copy);", plan.launcher_path().display());
            if want_1password {
                println!("  - restart the 1Password desktop app so that it reads its list of trusted browsers;");
            }
            println!("  - after upgrading browsah, run this command again to update the copy in {}.", plan.prefix.display());
            0
        }
        Err(e) => {
            crate::log_error!("system-install failed: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("browsah-sysinst-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A folder laid out like the release tarball.
    fn fake_source(dir: &Path) {
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        for f in REQUIRED.iter().chain(&["libvulkan.so.1", "CREDITS.html"]) {
            std::fs::write(dir.join(f), format!("content of {f}")).unwrap();
        }
        std::fs::write(dir.join("locales/en-US.pak"), "en").unwrap();
        std::fs::write(dir.join("locales/de.pak"), "de").unwrap();
        // Things a source build has next to the program that an install must not copy.
        std::fs::create_dir_all(dir.join("deps")).unwrap();
        std::fs::write(dir.join("deps/junk.rlib"), "x").unwrap();
        std::fs::write(dir.join("browsah.d"), "x").unwrap();
        std::fs::write(dir.join("chrome-sandbox"), "x").unwrap();
    }

    #[test]
    fn system_install_plans_the_release_files_and_nothing_else() {
        let (src, dst) = (temp("plan-src"), temp("plan-dst"));
        fake_source(&src);
        let p = plan(&src, &dst.join("browsah"), &dst.join("bin"), None).unwrap();
        let names: Vec<String> = p.files.iter().map(|(f, _)| f.to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"browsah".to_owned()) && names.contains(&"libcef.so".to_owned()) && names.contains(&"locales/de.pak".to_owned()) && names.contains(&"CREDITS.html".to_owned()));
        assert!(!names.iter().any(|n| n.contains("junk") || n.ends_with(".d") || n == "chrome-sandbox"), "{names:?}");
        let mode = |n: &str| p.files.iter().find(|(f, _)| f.to_string_lossy() == n).map(|(_, m)| *m);
        assert_eq!((mode("browsah"), mode("libcef.so"), mode("resources.pak")), (Some(0o755), Some(0o755), Some(0o644)));
        assert!(describe(&p).contains("owned by root") && describe(&p).contains("--1password"), "the plan says what and why");
    }

    #[test]
    fn system_install_refuses_a_folder_that_is_not_an_install_and_unsafe_places() {
        let (src, dst) = (temp("bad-src"), temp("bad-dst"));
        std::fs::write(src.join("browsah"), "x").unwrap();
        assert!(plan(&src, &dst.join("browsah"), &dst, None).unwrap_err().contains("missing"));
        fake_source(&src);
        assert!(plan(&src, Path::new("/opt"), &dst, None).is_err(), "too shallow");
        assert!(plan(&src, Path::new("relative/dir"), &dst, None).is_err());
        assert!(plan(&src, &src.join("inside"), &dst, None).unwrap_err().contains("overlap"));
        // Running the installed copy to install itself (its folder is the install folder): say what to do instead.
        let err = plan(&src, &src, &dst, None).unwrap_err();
        assert!(err.contains("already the installed copy") && err.contains("brew --prefix"), "{err}");
        assert!(plan(&src, &dst.join("browsah"), &dst, Some(&dst.join("no-such-file"))).unwrap_err().contains("is the 1Password desktop app installed"));
    }

    #[test]
    fn system_install_copies_replaces_and_writes_the_launcher_and_allowlist() {
        let (src, dst) = (temp("apply-src"), temp("apply-dst"));
        fake_source(&src);
        let (prefix, bin) = (dst.join("opt/browsah"), dst.join("bin"));
        let list = dst.join("allowed");
        std::fs::write(&list, "# comment\nvivaldi-bin").unwrap(); // no final newline
        let p = plan(&src, &prefix, &bin, Some(&list)).unwrap();
        let done = apply(&p).unwrap();
        assert_eq!(done.len(), 3, "{done:?}");
        assert_eq!(std::fs::read_to_string(prefix.join("libcef.so")).unwrap(), "content of libcef.so");
        assert_eq!(std::fs::read_to_string(prefix.join("locales/de.pak")).unwrap(), "de");
        assert_eq!(std::fs::metadata(prefix.join("browsah")).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(std::fs::metadata(prefix.join("resources.pak")).unwrap().permissions().mode() & 0o777, 0o644);
        assert!(!prefix.join("deps").exists() && !prefix.join("chrome-sandbox").exists());
        let script = std::fs::read_to_string(bin.join("browsah")).unwrap();
        assert!(script.contains(&format!("exec \"{}/browsah\"", prefix.display())) && script.contains("BROWSAH_LIBS") && script.contains(MARKER), "{script}");
        assert_eq!(std::fs::metadata(bin.join("browsah")).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(std::fs::read_to_string(&list).unwrap(), "# comment\nvivaldi-bin\nbrowsah\n", "added once, on its own line");
        // Again: replaces the install (an old file is gone), keeps the allow-list as it is, leaves nothing behind.
        std::fs::write(prefix.join("stale-file"), "x").unwrap();
        let again = apply(&plan(&src, &prefix, &bin, Some(&list)).unwrap()).unwrap();
        assert!(again[2].contains("already"), "{again:?}");
        assert!(!prefix.join("stale-file").exists());
        assert_eq!(std::fs::read_to_string(&list).unwrap().matches("browsah").count(), 1);
        let leftovers: Vec<_> = std::fs::read_dir(prefix.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with('.')).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn system_install_leaves_a_launcher_it_did_not_write_alone() {
        let (src, dst) = (temp("own-src"), temp("own-dst"));
        fake_source(&src);
        let bin = dst.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("browsah"), "#!/bin/sh\necho mine\n").unwrap();
        let p = plan(&src, &dst.join("opt/browsah"), &bin, None).unwrap();
        assert!(apply(&p).unwrap_err().contains("not made by this command"));
        assert!(!dst.join("opt/browsah").exists(), "nothing was copied");
        assert_eq!(std::fs::read_to_string(bin.join("browsah")).unwrap(), "#!/bin/sh\necho mine\n");
    }

    #[test]
    fn system_uninstall_removes_only_what_the_install_made() {
        let (src, dst) = (temp("un-src"), temp("un-dst"));
        fake_source(&src);
        let (prefix, bin, list) = (dst.join("opt/browsah"), dst.join("bin"), dst.join("allowed"));
        std::fs::write(&list, "vivaldi-bin\n").unwrap();
        apply(&plan(&src, &prefix, &bin, Some(&list)).unwrap()).unwrap();
        let done = uninstall(&prefix, &bin, Some(&list)).unwrap();
        assert_eq!(done.len(), 3, "{done:?}");
        assert!(!prefix.exists() && !bin.join("browsah").exists());
        assert_eq!(std::fs::read_to_string(&list).unwrap(), "vivaldi-bin\n", "other lines stay");
        // A folder that is not an install is refused; a launcher that is not ours stays.
        std::fs::create_dir_all(dst.join("other")).unwrap();
        std::fs::write(dst.join("other/file"), "x").unwrap();
        assert!(uninstall(&dst.join("other"), &bin, None).unwrap_err().contains("does not look like"));
        assert!(dst.join("other/file").exists());
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("browsah"), "not ours").unwrap();
        let done = uninstall(&dst.join("nothing-here"), &bin, None).unwrap();
        assert!(done[0].contains("left alone") && bin.join("browsah").exists(), "{done:?}");
    }

    #[test]
    fn system_install_knows_a_root_owned_locked_file_from_one_you_own() {
        assert!(root_owned_and_locked(Path::new("/bin/sh")) || std::fs::metadata("/bin/sh").map(|m| m.uid() != 0).unwrap_or(true));
        let d = temp("owner");
        std::fs::write(d.join("f"), "x").unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(!root_owned_and_locked(&d.join("f")), "a file you made is not root-owned");
        }
        assert!(!root_owned_and_locked(&d.join("missing")));
    }

    #[test]
    fn system_install_launcher_script_names_the_stable_paths() {
        let s = launcher_script(Path::new("/opt/browsah"), Path::new("/usr/local/bin/browsah"));
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("BROWSAH_LAUNCHER=\"/usr/local/bin/browsah\"") && s.contains("BROWSAH_LIBS=\"/opt/browsah\"") && s.contains("LD_LIBRARY_PATH=\"/opt/browsah${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\""));
    }
}
