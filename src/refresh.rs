//! `browsah install refresh`: everything an upgrade leaves to do, in one command. After `brew upgrade` (or a new build) the
//! root-owned copy in `/opt/browsah` is older than the program, and an installed codec runtime no longer matches the program's hash,
//! so `system-upgrade` and `codecs install` (a 190 MB download) were two separate chores. This runs, for what is installed:
//!
//! 1. `system-upgrade`: the root-owned copy (asks for the password through sudo);
//! 2. `codecs refresh` for the system codec runtime (through sudo and the installed launcher);
//! 3. `codecs refresh` for this user's codec runtime;
//!
//! where `codecs refresh` keeps the runtime's Chromium files and points it at the new program, with no download. The steps are built
//! as plain commands (`plan`), so they are tested without running anything.

use crate::system_install::{self, DEFAULT_BIN_DIR, DEFAULT_PREFIX};
use std::path::{Path, PathBuf};

/// One command to run, and what it is for.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub what: &'static str,
    pub command: Vec<String>,
}

/// What is installed on this computer.
pub struct Found {
    /// The root-owned copy's note, if there is one in the install folder.
    pub record: Option<system_install::Record>,
    pub prefix: PathBuf,
    /// A codec runtime is installed for the system copy / for this user.
    pub system_runtime: bool,
    pub user_runtime: bool,
}

/// The steps for what is installed. `pass` is what `system-upgrade` is given (`--yes`, `--dry-run`, `--force`, `--from=`, `--prefix=`).
/// Run as root (`as_root`), there is no sudo to add and no user runtime to refresh (it belongs to a user).
pub fn plan(exe: &Path, found: &Found, pass: &[String], as_root: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    let exe = exe.display().to_string();
    if found.record.is_some() {
        let mut c = vec![exe.clone(), "system-upgrade".to_owned()];
        c.extend(pass.iter().cloned());
        steps.push(Step { what: "update the root-owned copy", command: c });
    }
    if found.system_runtime {
        let bin_dir = found.record.as_ref().map(|r| r.bin_dir.clone()).unwrap_or_else(|| PathBuf::from(DEFAULT_BIN_DIR));
        let mut c = if as_root { Vec::new() } else { vec!["sudo".to_owned()] };
        c.extend([bin_dir.join("browsah").display().to_string(), "codecs".into(), "refresh".into()]);
        steps.push(Step { what: "point the system codec runtime at it", command: c });
    }
    if found.user_runtime && !as_root {
        steps.push(Step { what: "point your codec runtime at this program", command: vec![exe, "codecs".into(), "refresh".into()] });
    }
    steps
}

const USAGE: &str = "usage: browsah install refresh [--dry-run] [--yes] [--force] [--from=DIR] [--prefix=DIR]

After an upgrade (brew upgrade, or a new build), brings the other installed pieces up to date in one go:
  1. the root-owned copy in /opt/browsah (browsah system-upgrade; sudo asks for your password),
  2. the system codec runtime, if there is one (browsah codecs refresh, through sudo),
  3. your codec runtime, if there is one (browsah codecs refresh).
The codec runtime's Chromium files are kept: nothing is downloaded. Only what is installed is touched; --dry-run prints the commands.";

/// `browsah install ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("refresh") || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return if args.iter().any(|a| a == "--help" || a == "-h") { 0 } else { 2 };
    }
    let rest = &args[1..];
    let dry = rest.iter().any(|a| a == "--dry-run");
    let pass: Vec<String> = rest.iter().filter(|a| matches!(a.as_str(), "--yes" | "--dry-run" | "--force") || a.starts_with("--from=") || a.starts_with("--prefix=")).cloned().collect();
    if let Some(unknown) = rest.iter().find(|a| !pass.contains(a)) {
        eprintln!("unknown option {unknown}\n\n{USAGE}");
        return 2;
    }
    let prefix = PathBuf::from(rest.iter().find_map(|a| a.strip_prefix("--prefix=")).unwrap_or(DEFAULT_PREFIX));
    let user_base = gtk::glib::user_data_dir().join("browsah/codec-runtime");
    let found = Found {
        record: system_install::read_record(&prefix),
        system_runtime: prefix.join("codec-runtime/current").is_symlink(),
        user_runtime: user_base.join("current").is_symlink(),
        prefix: prefix.clone(),
    };
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find this program: {e}");
            return 1;
        }
    };
    let as_root = unsafe { libc::geteuid() } == 0;
    let steps = plan(&exe, &found, &pass, as_root);
    if steps.is_empty() {
        println!("Nothing to refresh: no root-owned copy in {} and no codec runtime installed.", found.prefix.display());
        return 0;
    }
    let mut failed = false;
    for (i, step) in steps.iter().enumerate() {
        println!("{}. {}:\n   {}", i + 1, step.what, step.command.join(" "));
        if dry {
            continue;
        }
        let code = std::process::Command::new(&step.command[0]).args(&step.command[1..]).status().map(|s| s.code().unwrap_or(1)).unwrap_or_else(|e| {
            eprintln!("cannot run {}: {e}", step.command[0]);
            1
        });
        if code != 0 {
            println!("   failed (exit {code}); the next steps need it, so stopping.");
            failed = true;
            break;
        }
    }
    if dry {
        println!("\n(dry run: nothing was run)");
    } else if !failed {
        println!("\nDone. Restart browsah to use the refreshed copy.");
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> system_install::Record {
        system_install::Record { source: "/brew/opt/browsah/libexec".into(), version: "0.4.1".into(), one_password: true, bin_dir: DEFAULT_BIN_DIR.into() }
    }

    #[test]
    fn nothing_installed_means_no_steps() {
        let found = Found { record: None, prefix: DEFAULT_PREFIX.into(), system_runtime: false, user_runtime: false };
        assert!(plan(Path::new("/brew/bin/browsah"), &found, &[], false).is_empty());
    }

    #[test]
    fn a_full_install_is_refreshed_in_order_with_sudo_only_where_needed() {
        let found = Found { record: Some(record()), prefix: DEFAULT_PREFIX.into(), system_runtime: true, user_runtime: true };
        let steps = plan(Path::new("/brew/bin/browsah"), &found, &["--yes".to_owned()], false);
        let cmds: Vec<String> = steps.iter().map(|s| s.command.join(" ")).collect();
        assert_eq!(cmds, vec!["/brew/bin/browsah system-upgrade --yes", "sudo /usr/local/bin/browsah codecs refresh", "/brew/bin/browsah codecs refresh"]);
    }

    #[test]
    fn root_neither_needs_sudo_nor_touches_a_users_runtime() {
        let found = Found { record: Some(record()), prefix: DEFAULT_PREFIX.into(), system_runtime: true, user_runtime: true };
        let cmds: Vec<String> = plan(Path::new("/x/browsah"), &found, &[], true).iter().map(|s| s.command.join(" ")).collect();
        assert_eq!(cmds, vec!["/x/browsah system-upgrade", "/usr/local/bin/browsah codecs refresh"]);
    }

    #[test]
    fn only_a_user_runtime_is_only_refreshed_without_sudo() {
        let found = Found { record: None, prefix: DEFAULT_PREFIX.into(), system_runtime: false, user_runtime: true };
        let steps = plan(Path::new("/x/browsah"), &found, &[], false);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].command, vec!["/x/browsah", "codecs", "refresh"]);
    }
}
