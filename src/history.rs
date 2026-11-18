//! History helpers shared by the history window and the `browsah history` command.

use crate::profile::Profile;
use crate::store::{Scope, Store};
use gtk::glib;

/// "5 min ago", "yesterday 14:03", or the date: how long ago `at` (unix seconds) was.
/// How a closed tab's reason reads in a list: `user` is "Closed", `window` "Window closed", `sweep:Stale` "Swept: Stale".
pub fn reason_text(reason: &str) -> String {
    match reason {
        "user" => "Closed".into(),
        "window" => "Window closed".into(),
        r => match r.strip_prefix("sweep:") {
            Some(rule) => format!("Swept: {rule}"),
            None => r.to_owned(),
        },
    }
}

pub fn when(at: i64, now: i64) -> String {
    let d = now - at;
    if d < 60 {
        return "just now".into();
    }
    if d < 3600 {
        return format!("{} min ago", d / 60);
    }
    if d < 86_400 {
        return format!("{} h ago", d / 3600);
    }
    glib::DateTime::from_unix_local(at).ok().and_then(|t| t.format("%Y-%m-%d %H:%M").ok()).map(|s| s.to_string()).unwrap_or_else(|| at.to_string())
}

const USAGE: &str = "usage:
  browsah history [QUERY] [--profile ID] [--workspace ID | --ephemeral] [--limit N]
  browsah history clear [--profile ID] [--older-than-days N] --yes

Lists the profile's visited pages, newest first, or deletes them (all, or older than N days).
The profile must not be running to clear it.";

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

/// `browsah history ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let profile = match Profile::resolve(&flag_value(args, "--profile").unwrap_or_else(|| crate::profile::DEFAULT_ID.to_owned())) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let db = profile.dir.join("data.sqlite");
    if !db.exists() {
        println!("no history yet");
        return 0;
    }
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    if args.first().map(String::as_str) == Some("clear") {
        if !args.iter().any(|a| a == "--yes") {
            eprintln!("this DELETES history (visited pages and closed tabs) of profile {:?}.\nrun again with --yes to continue\n\n{USAGE}", profile.id);
            return 1;
        }
        if crate::profile::is_running(&profile) {
            eprintln!("profile {:?} is running; close it first", profile.id);
            return 1;
        }
        let before = flag_value(args, "--older-than-days").and_then(|d| d.parse::<i64>().ok()).map(|d| now - d * 86_400);
        match store.clear_history(before) {
            Ok(n) => {
                println!("deleted {n} visits");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    } else {
        let ws = flag_value(args, "--workspace");
        let scope = match (&ws, args.iter().any(|a| a == "--ephemeral")) {
            (Some(w), _) => Scope::Workspace(w),
            (None, true) => Scope::Ephemeral,
            (None, false) => Scope::All,
        };
        let limit = flag_value(args, "--limit").and_then(|l| l.parse().ok()).unwrap_or(50);
        let query = crate::workspace::first_positional(args).filter(|a| a.as_str() != "clear").cloned().unwrap_or_default();
        for v in store.search_visits(&query, scope, limit) {
            println!("{:<16} {:<12} {}  {}", when(v.at, now), v.workspace.as_deref().unwrap_or("(ephemeral)"), if v.title.is_empty() { "-" } else { &v.title }, v.url);
        }
        0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn close_reasons_read_well() {
        assert_eq!(super::reason_text("user"), "Closed");
        assert_eq!(super::reason_text("window"), "Window closed");
        assert_eq!(super::reason_text("sweep:Stale searches"), "Swept: Stale searches");
        assert_eq!(super::reason_text("other"), "other");
    }

    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(when(1000, 1010), "just now");
        assert_eq!(when(1000, 1000 + 5 * 60), "5 min ago");
        assert_eq!(when(1000, 1000 + 3 * 3600 + 5), "3 h ago");
        assert!(when(1000, 1000 + 3 * 86_400).starts_with("1970-"), "older than a day: a date");
    }
}
