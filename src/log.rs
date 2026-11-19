//! The program's log goes to the system journal (`journalctl -t browsah`), through journald's native socket, so that "why did
//! that happen" can be answered afterwards without having started the browser from a terminal in debug mode. Every entry has
//! the identifier `browsah`, a priority, and the fields `BROWSAH_PROFILE` and `CODE_TARGET` (the part of the program), so
//! `journalctl -t browsah BROWSAH_PROFILE=default -p info` works. `browsah log` runs journalctl with those filters.
//!
//! What is written is the `logging.level` option (`error`, `warn`, `info` (default), `debug`); `BROWSAH_LOG=debug` overrides it
//! for one run, and `BROWSAH_DEBUG=1` also mirrors every line to the terminal. Warnings and errors always go to the terminal as
//! well. Where there is no journal (no systemd) only the terminal gets them. Use the `log_*!` macros.
//!
//! Works from any thread (CEF's included). Before `init`, and in CEF's child processes, only warnings and errors reach the
//! terminal.

use serde::{Deserialize, Serialize};
use std::fmt::Arguments;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warn" | "warning" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            _ => None,
        }
    }
}

struct Logger {
    level: Level,
    mirror_all: bool,
    profile: String,
    socket: Option<std::os::unix::net::UnixDatagram>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

impl Level {
    /// syslog priority
    fn priority(self) -> u8 {
        match self {
            Level::Error => 3,
            Level::Warn => 4,
            Level::Info => 6,
            Level::Debug => 7,
        }
    }
}

/// Start logging to the journal for `profile`.
pub fn init(profile: &str, level: Level) {
    let level = std::env::var("BROWSAH_LOG").ok().and_then(|v| Level::parse(&v)).unwrap_or(level);
    let mirror_all = std::env::var_os("BROWSAH_DEBUG").is_some();
    let socket = std::os::unix::net::UnixDatagram::unbound().ok().filter(|s| s.connect(JOURNAL_SOCKET).is_ok());
    let have_journal = socket.is_some();
    let _ = LOGGER.set(Logger { level, mirror_all, profile: profile.to_owned(), socket });
    info(module_path!(), format_args!("log started (level {level:?}, pid {}, version {}, journal {})", std::process::id(), env!("CARGO_PKG_VERSION"), if have_journal { "yes" } else { "not available" }));
}

/// Whether a message at `level` would be written (to skip building an expensive message).
pub fn enabled(level: Level) -> bool {
    LOGGER.get().is_some_and(|l| level <= l.level)
}

/// The part of a module path after `browsah::` (`app::sweep` stays, `main` becomes `main`).
fn short(target: &str) -> &str {
    target.strip_prefix("browsah::").unwrap_or(target)
}

/// One journal entry in the native protocol: `NAME=value\n` for plain values, and for a value that may hold newlines
/// `NAME\n` + the length as 8 bytes little endian + the value + `\n`.
fn journal_entry(level: Level, profile: &str, target: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::new();
    // The profile leads the text, so a listing of several profiles (`browsah log` without `--profile`) can be told apart.
    let tagged = format!("[{profile}] {message}");
    let mut message = tagged.as_str();
    let cut;
    if message.len() > 16_000 {
        cut = format!("{}… (cut)", message.chars().take(8_000).collect::<String>());
        message = &cut;
    }
    out.extend_from_slice(format!("PRIORITY={}\nSYSLOG_IDENTIFIER=browsah\nBROWSAH_PROFILE={profile}\nCODE_TARGET={}\n", level.priority(), short(target).replace('\n', " ")).as_bytes());
    out.extend_from_slice(b"MESSAGE\n");
    out.extend_from_slice(&(message.len() as u64).to_le_bytes());
    out.extend_from_slice(message.as_bytes());
    out.push(b'\n');
    out
}

pub fn log(level: Level, target: &str, args: Arguments) {
    let Some(l) = LOGGER.get() else {
        if level <= Level::Warn {
            eprintln!("browsah: {args}");
        }
        return;
    };
    if level <= l.level {
        if let Some(sock) = &l.socket {
            let _ = sock.send(&journal_entry(level, &l.profile, target, &args.to_string()));
        }
    }
    if level <= Level::Warn || (l.mirror_all && level <= l.level.max(Level::Debug)) {
        eprintln!("browsah: {}: {}", short(target), args);
    }
}

pub fn error(target: &str, args: Arguments) {
    log(Level::Error, target, args)
}
pub fn warn(target: &str, args: Arguments) {
    log(Level::Warn, target, args)
}
pub fn info(target: &str, args: Arguments) {
    log(Level::Info, target, args)
}
pub fn debug(target: &str, args: Arguments) {
    log(Level::Debug, target, args)
}

#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { $crate::log::error(module_path!(), format_args!($($t)*)) }; }
#[macro_export]
macro_rules! log_warn { ($($t:tt)*) => { $crate::log::warn(module_path!(), format_args!($($t)*)) }; }
#[macro_export]
macro_rules! log_info { ($($t:tt)*) => { $crate::log::info(module_path!(), format_args!($($t)*)) }; }
#[macro_export]
macro_rules! log_debug { ($($t:tt)*) => { $crate::log::debug(module_path!(), format_args!($($t)*)) }; }

const CLI_USAGE: &str = "usage: browsah log [--profile=ID] [--lines=N] [--grep=TEXT] [--level=LEVEL] [--follow] [--since=TIME]

Shows the program's log from the system journal (the same as `journalctl -t browsah`), the last 50
lines by default. --grep keeps lines matching TEXT (e.g. `sweep`), --level shows that priority and worse (error, warn, info,
debug), --since takes journalctl's times (`1 hour ago`, `today`), --follow keeps printing. Use --all-profiles for every profile.
How much is logged is the `logging.level` option; `BROWSAH_LOG=debug` logs everything for one run.";

/// The journalctl arguments for `browsah log ...` (separate so they can be tested).
fn journalctl_args(args: &[String]) -> Vec<String> {
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned));
    let mut out: Vec<String> = vec!["--no-pager".into(), "-o".into(), "short-iso".into(), "-t".into(), "browsah".into()];
    out.push("-n".into());
    out.push(value("--lines").and_then(|v| v.parse::<u32>().ok()).unwrap_or(50).to_string());
    if let Some(g) = value("--grep") {
        out.push("-g".into());
        out.push(g);
    }
    if let Some(l) = value("--level").and_then(|v| Level::parse(&v)) {
        out.push("-p".into());
        out.push(l.priority().to_string());
    }
    if let Some(t) = value("--since") {
        out.push("--since".into());
        out.push(t);
    }
    if args.iter().any(|a| a == "--follow") {
        out.push("-f".into());
    }
    // Every profile unless one is named (the profile that runs is often not `default`).
    if let Some(p) = value("--profile").filter(|_| !args.iter().any(|a| a == "--all-profiles")) {
        out.push(format!("BROWSAH_PROFILE={p}"));
    }
    out
}

/// `browsah log ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help") {
        println!("{CLI_USAGE}");
        return 0;
    }
    let jargs = journalctl_args(args);
    match std::process::Command::new("journalctl").args(&jargs).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("cannot run journalctl ({e}); the log is in the system journal: journalctl -t browsah");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_order_and_parse() {
        assert!(Level::Error < Level::Warn && Level::Warn < Level::Info && Level::Info < Level::Debug);
        assert_eq!(Level::parse(" Debug "), Some(Level::Debug));
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert_eq!(Level::parse("loud"), None);
    }

    #[test]
    fn a_journal_entry_has_the_fields_and_a_length_prefixed_message() {
        let e = journal_entry(Level::Warn, "work", "browsah::app::sweep", "line one\nline two");
        let text = String::from_utf8_lossy(&e).to_string();
        assert!(text.starts_with("PRIORITY=4\nSYSLOG_IDENTIFIER=browsah\nBROWSAH_PROFILE=work\nCODE_TARGET=app::sweep\nMESSAGE\n"));
        let at = e.windows(8).position(|w| w == 24u64.to_le_bytes()).expect("length");
        assert_eq!(&e[at + 8..], b"[work] line one\nline two\n");
    }

    #[test]
    fn log_command_becomes_journalctl_arguments() {
        let a = |v: &[&str]| journalctl_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let plain = a(&[]);
        assert!(!plain.iter().any(|x| x.starts_with("BROWSAH_PROFILE")) && plain.windows(2).any(|w| w == ["-n", "50"]));
        let some = a(&["--profile=work", "--lines=5", "--grep=sweep", "--level=warn", "--follow"]);
        assert!(some.contains(&"BROWSAH_PROFILE=work".to_string()) && some.windows(2).any(|w| w == ["-g", "sweep"]) && some.windows(2).any(|w| w == ["-p", "4"]) && some.contains(&"-f".to_string()));
        assert!(!a(&["--all-profiles"]).iter().any(|x| x.starts_with("BROWSAH_PROFILE")));
    }

    #[test]
    fn module_paths_lose_the_crate_name() {
        assert_eq!(short("browsah::app::sweep"), "app::sweep");
        assert_eq!(short("main"), "main");
    }
}
