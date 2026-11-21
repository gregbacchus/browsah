//! `browsah install refresh`: what to run after an upgrade (`brew upgrade`, or a new build) to bring the installed pieces up to date, in
//! one command. Today that is the root-owned copy in `/opt/browsah` (`system-upgrade`, which asks for the password through sudo); a codec
//! runtime needs nothing, because it is only CEF and the new program is simply started with it (docs/research/2026-11-21-install-refresh.md).
//! The steps are built as plain commands (`plan`), so they are tested without running anything.

use crate::system_install::{self, DEFAULT_PREFIX};
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
}

/// The steps for what is installed. `pass` is what `system-upgrade` is given (`--yes`, `--dry-run`, `--force`, `--from=`, `--prefix=`).
pub fn plan(exe: &Path, found: &Found, pass: &[String]) -> Vec<Step> {
    let mut steps = Vec::new();
    if found.record.is_some() {
        let mut c = vec![exe.display().to_string(), "system-upgrade".to_owned()];
        c.extend(pass.iter().cloned());
        steps.push(Step { what: "update the root-owned copy", command: c });
    }
    steps
}

const USAGE: &str = "usage: browsah install refresh [--dry-run] [--yes] [--force] [--from=DIR] [--prefix=DIR]

After an upgrade (brew upgrade, or a new build), brings the other installed pieces up to date in one go:
  - the root-owned copy in /opt/browsah (browsah system-upgrade; sudo asks for your password).
A codec runtime (browsah codecs install) is only CEF and needs nothing. Only what is installed is touched; --dry-run prints the commands.";

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
    let found = Found { record: system_install::read_record(&prefix), prefix };
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find this program: {e}");
            return 1;
        }
    };
    let steps = plan(&exe, &found, &pass);
    if steps.is_empty() {
        println!("Nothing to refresh: there is no root-owned copy in {} (made by `system-install`).", found.prefix.display());
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
            println!("   failed (exit {code}); stopping.");
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

    #[test]
    fn nothing_installed_means_no_steps() {
        let found = Found { record: None, prefix: DEFAULT_PREFIX.into() };
        assert!(plan(Path::new("/brew/bin/browsah"), &found, &[]).is_empty());
    }

    #[test]
    fn a_root_owned_copy_is_upgraded_with_the_options_given() {
        let record = system_install::Record { source: "/brew/opt/browsah/libexec".into(), version: "0.4.1".into(), one_password: true, bin_dir: "/usr/local/bin".into() };
        let found = Found { record: Some(record), prefix: DEFAULT_PREFIX.into() };
        let cmds: Vec<String> = plan(Path::new("/brew/bin/browsah"), &found, &["--yes".to_owned()]).iter().map(|s| s.command.join(" ")).collect();
        assert_eq!(cmds, vec!["/brew/bin/browsah system-upgrade --yes"]);
    }
}
