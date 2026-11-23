//! Tab sweeping: tidying up tabs that have not been used, by rules a workspace configures.
//!
//! A tab's use is a list of events (it was shown, its page was being worked in), each with a weight.
//! "Idle for" is the time since the last event; the **score** adds the events up with older ones
//! counting for less, `sum(weight * 0.5^(age / half_life))`, so a tab visited often keeps a high score
//! for a long while after its last visit and a tab looked at once fades quickly. Rules compare both.
//!
//! This module is the model: durations, usage, rules and the decision. Applying a decision (closing or
//! discarding a tab, checking for unsaved form input and playing media first) is in `app/sweep.rs`.

use crate::profile::Profile;
use crate::store::{Store, TabRecord};
use crate::workspace::{scope_matches, Workspace};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// ---- durations --------------------------------------------------------------------------------------

/// Parse `45s`, `15m`, `2h` or `7d` (a whole number and a unit) into seconds.
pub fn parse_duration(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let (digits, unit) = t.split_at(t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len()));
    let n: i64 = digits.parse().map_err(|_| format!("{text:?} is not a duration (use a number and a unit: 30s, 15m, 2h, 7d)"))?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(format!("{text:?} is not a duration (use a number and a unit: 30s, 15m, 2h, 7d)")),
    };
    n.checked_mul(mult).filter(|s| *s > 0).ok_or_else(|| format!("{text:?} must be longer than zero"))
}

// ---- usage ------------------------------------------------------------------------------------------

/// How a tab has been used: (unix seconds, weight) events, newest last.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    events: VecDeque<(i64, f32)>,
}

/// Events kept per tab (older ones have long faded from any score).
const MAX_EVENTS: usize = 120;

impl Usage {
    /// A tab that is new at `now`.
    pub fn starting(now: i64) -> Usage {
        let mut u = Usage::default();
        u.record(now, 1.0);
        u
    }

    pub fn record(&mut self, at: i64, weight: f32) {
        self.events.push_back((at, weight));
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    /// The time of the last use, if there was one.
    pub fn last_used(&self) -> Option<i64> {
        self.events.back().map(|e| e.0)
    }

    /// `sum(weight * 0.5^(age / half_life))` at `now`.
    pub fn score(&self, now: i64, half_life: i64) -> f64 {
        self.events.iter().map(|(t, w)| f64::from(*w) * 0.5f64.powf((now - t).max(0) as f64 / half_life.max(1) as f64)).sum()
    }

    /// `time:weight,time:weight` for the database.
    pub fn to_text(&self) -> String {
        self.events.iter().map(|(t, w)| format!("{t}:{w}")).collect::<Vec<_>>().join(",")
    }

    pub fn from_text(text: &str) -> Usage {
        let events = text
            .split(',')
            .filter_map(|e| {
                let (t, w) = e.split_once(':')?;
                Some((t.parse().ok()?, w.parse().ok()?))
            })
            .collect();
        Usage { events }
    }
}

// ---- configuration ----------------------------------------------------------------------------------

/// The `sweep:` section (in `config.yaml`, `profile.yaml`, `workspace.yaml` or an app's `app.yaml`; a lower layer's keys win and
/// `rules+` appends to the rules above).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct SweepConfig {
    /// Sweeping is off unless this is `true`.
    pub enabled: bool,
    /// How often the rules are checked (`30s` to `1d`).
    pub interval: String,
    pub rules: Vec<RuleConfig>,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self { enabled: false, interval: "5m".into(), rules: Vec::new() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct RuleConfig {
    /// Shown in the history of closed tabs (`sweep:<name>`) and messages.
    #[serde(default)]
    pub name: Option<String>,
    /// Which tabs: address patterns as for a pinned tab's scope; none means every tab.
    #[serde(default, rename = "match")]
    pub matches: MatchConfig,
    /// The tab has not been used for at least this long.
    #[serde(default)]
    pub idle_for: Option<String>,
    /// The tab's decayed use score is below this.
    #[serde(default)]
    pub score_below: Option<f64>,
    /// How fast use is forgotten when computing the score (default `12h`).
    #[serde(default)]
    pub half_life: Option<String>,
    /// What to do: `close` (default) or `discard` (keep the tab, free its page; it reloads when shown).
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct MatchConfig {
    pub url: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Close,
    Discard,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    urls: Vec<String>,
    idle_for: Option<i64>,
    score_below: Option<f64>,
    half_life: i64,
    pub action: Action,
    /// Whether a pinned tab may be swept by this rule (only the automatic sleep rule: pinned tabs are never closed).
    pub pinned_ok: bool,
}

/// The name of the rule `sleep_tabs_after` adds.
pub const SLEEP_RULE: &str = "sleep";

impl Rule {
    /// Check a rule from the configuration. `position` (from 1) names unnamed rules.
    pub fn compile(cfg: &RuleConfig, position: usize) -> Result<Rule, String> {
        let name = cfg.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("rule {position}"));
        let idle_for = cfg.idle_for.as_deref().map(parse_duration).transpose().map_err(|e| format!("idle_for: {e}"))?;
        let half_life = cfg.half_life.as_deref().map(parse_duration).transpose().map_err(|e| format!("half_life: {e}"))?.unwrap_or(12 * 3600);
        if idle_for.is_none() && cfg.score_below.is_none() {
            return Err("needs idle_for or score_below (otherwise it would sweep every tab at once)".into());
        }
        if cfg.score_below.is_some_and(|s| !s.is_finite() || s <= 0.0) {
            return Err("score_below must be a number above zero".into());
        }
        let action = match cfg.action.as_deref() {
            None | Some("close") => Action::Close,
            Some("discard") => Action::Discard,
            Some(a) => return Err(format!("unknown action {a:?} (use close or discard)")),
        };
        Ok(Rule { name, urls: cfg.matches.url.clone(), idle_for, score_below: cfg.score_below, half_life, action, pinned_ok: false })
    }

    /// The rule behind `sleep_tabs_after`: any tab idle for `idle` seconds is put to sleep (pinned ones too: they are kept).
    pub fn sleep(idle: i64) -> Rule {
        Rule { name: SLEEP_RULE.into(), urls: Vec::new(), idle_for: Some(idle), score_below: None, half_life: 12 * 3600, action: Action::Discard, pinned_ok: true }
    }

    /// Does the rule apply to this tab now? All its conditions must hold. A tab with no recorded use
    /// counts as idle since `fallback` (when it was created).
    pub fn fires(&self, url: &str, usage: &Usage, now: i64, fallback: i64) -> bool {
        self.verdict(url, usage, now, fallback).is_ok()
    }

    /// Like `fires`, but says which condition stopped the rule (for the log).
    /// The tab's decayed use score by this rule's `half_life`.
    pub fn score_of(&self, usage: &Usage, now: i64) -> f64 {
        usage.score(now, self.half_life)
    }

    pub fn verdict(&self, url: &str, usage: &Usage, now: i64, fallback: i64) -> Result<(), String> {
        if !self.urls.is_empty() && !self.urls.iter().any(|p| scope_matches(p, url)) {
            return Err("address does not match".into());
        }
        let last = usage.last_used().unwrap_or(fallback);
        if let Some(d) = self.idle_for.filter(|d| now - last < *d) {
            // The score is shown too, though it has not been compared yet: it is what the rule would look at next.
            let score = self.score_below.map(|l| format!(", score {:.2} (limit {l})", usage.score(now, self.half_life))).unwrap_or_default();
            return Err(format!("idle {} < {}{score}", short_duration(now - last), short_duration(d)));
        }
        if let Some(limit) = self.score_below {
            let score = usage.score(now, self.half_life);
            if score >= limit {
                return Err(format!("score {score:.2} >= {limit}"));
            }
        }
        Ok(())
    }
}

/// `90` is `1m30s`, `7500` is `2h5m`: a duration for a log line.
pub fn short_duration(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{}s", secs / 60, secs % 60),
        3600..=86399 => format!("{}h{}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d{}h", secs / 86400, secs % 86400 / 3600),
    }
}

/// All the rules of a workspace and how often to check them.
#[derive(Debug, Clone, Default)]
pub struct Sweeper {
    pub rules: Vec<Rule>,
    pub interval_secs: i64,
    pub warnings: Vec<String>,
}

impl Sweeper {
    /// The rules of `sweep` (when it is enabled) and, last, the automatic sleep rule for `sleep_after` (`30m`, `2h`...); `None`
    /// when there is neither. A sleep rule shortens the check interval so a short time is honoured (half of it, between 30 s and
    /// the interval).
    pub fn from_config(cfg: &SweepConfig, sleep_after: Option<&str>) -> Option<Sweeper> {
        let mut s = Sweeper { interval_secs: 300, ..Sweeper::default() };
        if cfg.enabled {
            match parse_duration(&cfg.interval) {
                Ok(i) if (30..=86_400).contains(&i) => s.interval_secs = i,
                Ok(_) => s.warnings.push(format!("sweep.interval {:?} must be between 30s and 1d; using 5m", cfg.interval)),
                Err(e) => s.warnings.push(format!("sweep.interval: {e}; using 5m")),
            }
            for (i, r) in cfg.rules.iter().enumerate() {
                match Rule::compile(r, i + 1) {
                    Ok(rule) => s.rules.push(rule),
                    Err(e) => s.warnings.push(format!("sweep rule {}{}: {e}; skipped", i + 1, r.name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default())),
                }
            }
        }
        if let Some(text) = sleep_after.map(str::trim).filter(|t| !t.is_empty()) {
            match parse_duration(text) {
                Ok(idle) => {
                    s.rules.push(Rule::sleep(idle));
                    s.interval_secs = s.interval_secs.min((idle / 2).max(30));
                }
                Err(e) => s.warnings.push(format!("sleep_tabs_after: {e}; tabs are not put to sleep")),
            }
        }
        (!s.rules.is_empty()).then_some(s)
    }

    /// The first rule that fires for a tab (a pinned tab only by a rule that may take pinned tabs).
    /// For the log: how each rule judged the tab ("Stale: idle 5m0s < 1h0m; Everything: fires").
    pub fn explain(&self, url: &str, usage: &Usage, now: i64, created: i64, pinned: bool) -> String {
        if self.rules.is_empty() {
            return "no rules".into();
        }
        self.rules
            .iter()
            .map(|r| match (pinned && !r.pinned_ok, r.verdict(url, usage, now, created)) {
                (true, _) => format!("{}: not for pinned tabs", r.name),
                (false, Ok(())) => format!("{}: fires (score {:.2})", r.name, r.score_of(usage, now)),
                (false, Err(why)) => format!("{}: {why}", r.name),
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn decide(&self, url: &str, usage: &Usage, now: i64, created: i64, pinned: bool) -> Option<&Rule> {
        self.rules.iter().find(|r| (!pinned || r.pinned_ok) && r.fires(url, usage, now, created))
    }
}

// ---- sweeping a workspace that is not open ----------------------------------------------------------

/// The tabs a sweep closed lately, newest first: reason `sweep:...`, at most `within` seconds old, at most `max` of them.
pub fn recently_swept(closed: &[crate::store::ClosedTab], now: i64, within: i64, max: usize) -> Vec<&crate::store::ClosedTab> {
    closed.iter().filter(|t| t.reason.starts_with("sweep:") && now - t.at <= within).take(max).collect()
}

/// The outcome of sweeping a workspace's remembered tabs.
#[derive(Debug, Default)]
pub struct StoredSweep {
    pub kept: Vec<TabRecord>,
    /// Tabs to close, with the name of the rule that fired.
    pub swept: Vec<(TabRecord, String)>,
}

/// Apply a workspace's rules to its remembered tabs (no window is open, so nothing is being looked at
/// and nothing can hold unsaved input: the pages are not running). Pinned tabs come from the
/// configuration each time and are left alone, and a `discard` rule has nothing to do because the tabs
/// are already asleep. A tab with no recorded use is treated as used now (so it is not swept at once).
///
/// **The last tab of a workspace is never swept:** if the rules would remove every tab and the
/// workspace has no pinned tabs (`pinned_in_config`) to stay behind, the most recently used of the tabs
/// they would remove is kept.
pub fn sweep_records(sweeper: &Sweeper, records: &[TabRecord], now: i64, pinned_in_config: usize) -> StoredSweep {
    let mut out = StoredSweep::default();
    for r in records {
        let fired = (!r.pinned).then(|| sweeper.decide(&r.url, &Usage::from_text(&r.usage), now, now, r.pinned)).flatten().filter(|rule| rule.action == Action::Close);
        match fired {
            Some(rule) => out.swept.push((r.clone(), rule.name.clone())),
            None => out.kept.push(r.clone()),
        }
    }
    if out.kept.is_empty() && pinned_in_config == 0 && !out.swept.is_empty() {
        let last = out.swept.iter().enumerate().max_by_key(|(_, (r, _))| Usage::from_text(&r.usage).last_used().unwrap_or(i64::MIN)).map(|(i, _)| i).unwrap_or(0);
        let (rec, _) = out.swept.remove(last);
        out.kept.push(rec);
    }
    out
}

/// Sweep the remembered tabs of `ws` (which must not have an open window). With `apply` false only
/// the notes are produced ("would close ..."). Swept tabs are removed from the workspace and kept in
/// the history of closed tabs as `sweep:<rule>`, subject to the workspace's history settings.
pub fn sweep_stored(profile: &Profile, ws: &Workspace, store: &Store, app_file: &std::path::Path, now: i64, apply: bool) -> Vec<String> {
    let Some(sweeper) = Sweeper::from_config(&effective_sweep(profile, ws, app_file), None) else { return Vec::new() };
    let records = store.load_tabs(&ws.id);
    let result = sweep_records(&sweeper, &records, now, ws.config.tabs.pinned.len());
    if result.swept.is_empty() {
        return Vec::new();
    }
    let history = crate::config::load_window(app_file, &profile.dir.join("profile.yaml"), Some(&ws.dir.join("workspace.yaml"))).map(|l| l.config.history).unwrap_or_default();
    let mut notes = Vec::new();
    crate::log_info!("sweep of closed workspace {:?}: {} remembered tab(s), {} to close, {} kept{}", ws.id, records.len(), result.swept.len(), result.kept.len(), if apply { "" } else { " (dry run)" });
    for (rec, rule) in &result.swept {
        crate::log_info!("sweep of closed workspace {:?}: rule {rule:?} {} {:?} (last used {:?}, pinned {})", ws.id, if apply { "closes" } else { "would close" }, rec.url, Usage::from_text(&rec.usage).last_used(), rec.pinned);
        notes.push(format!("{} {} ({rule})", if apply { "closed" } else { "would close" }, rec.url));
    }
    if apply {
        if store.save_tabs(&ws.id, &result.kept).is_err() {
            crate::log_error!("sweep of closed workspace {:?}: cannot save the remaining tabs; nothing was swept", ws.id);
            return vec!["cannot save the workspace's tabs; nothing was swept".into()];
        }
        for (rec, rule) in &result.swept {
            let web = rec.url.starts_with("http://") || rec.url.starts_with("https://");
            if web && history.enabled && !history.exclude.iter().any(|p| scope_matches(p, &rec.url)) {
                let _ = store.record_closed(Some(&ws.id), &rec.url, &rec.title, now, &format!("sweep:{rule}"), None);
            }
            let _ = std::fs::remove_file(ws.dir.join("shots").join(format!("{}.png", rec.uid)));
        }
    }
    notes
}

/// The `sweep` section a workspace ends up with: the application's, the profile's and the workspace's own layered (the
/// workspace's keys win, `rules+` appends). A file that cannot be read gives the defaults (no sweeping).
pub fn effective_sweep(profile: &Profile, ws: &Workspace, app_file: &std::path::Path) -> SweepConfig {
    crate::config::load_window(app_file, &profile.dir.join("profile.yaml"), Some(&ws.config_file())).map(|l| l.config.sweep).unwrap_or_default()
}

/// Every workspace of a profile that has sweeping rules (its own or inherited).
pub fn workspaces_with_sweeping(profile: &Profile, app_file: &std::path::Path) -> Vec<Workspace> {
    Workspace::list(profile).into_iter().filter(|w| Sweeper::from_config(&effective_sweep(profile, w, app_file), None).is_some()).collect()
}

const CLI_USAGE: &str = "usage: browsah sweep [--profile ID] [--workspace ID] [--dry-run]

Applies each workspace's sweeping rules (`sweep:` in workspace.yaml, or inherited from profile.yaml and config.yaml) to its remembered tabs, for
workspaces that are not open: tabs idle past a close rule are removed and kept in the history of
closed tabs. A running profile does this itself; use this for a profile that is not running, from a
cron job or a systemd timer. --dry-run only says what would be swept.";

/// `browsah sweep ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned)).or_else(|| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()));
    if args.iter().any(|a| a == "--help") {
        println!("{CLI_USAGE}");
        return 0;
    }
    let profile = match Profile::resolve(&value("--profile").unwrap_or_else(crate::profile::cli_default_id)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let dry = args.iter().any(|a| a == "--dry-run");
    if !dry && crate::profile::is_running(&profile) {
        eprintln!("profile {:?} is running and sweeps its closed workspaces itself; use --dry-run to look", profile.id);
        return 1;
    }
    let db = profile.dir.join("data.sqlite");
    if !db.exists() {
        println!("nothing to sweep (no stored tabs yet)");
        return 0;
    }
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let only = value("--workspace");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut any = false;
    for ws in workspaces_with_sweeping(&profile, &crate::config::default_path()).into_iter().filter(|w| only.as_ref().is_none_or(|o| *o == w.id)) {
        for note in sweep_stored(&profile, &ws, &store, &crate::config::default_path(), now, !dry) {
            any = true;
            println!("{}: {note}", ws.id);
        }
    }
    if !any {
        println!("nothing to sweep");
    }
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_recent_sweeps_are_listed_newest_first_and_capped() {
        let t = |id, at, reason: &str| crate::store::ClosedTab { id, workspace: None, url: format!("https://x/{id}"), title: String::new(), at, reason: reason.into(), group: None };
        let all = vec![t(5, 990, "sweep:A"), t(4, 980, "user"), t(3, 900, "sweep:B"), t(2, 100, "sweep:C"), t(1, 99, "window")];
        let ids = |within, max| recently_swept(&all, 1000, within, max).iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(500, 10), vec![5, 3], "swept ones only, and not the old one");
        assert_eq!(ids(5000, 10), vec![5, 3, 2]);
        assert_eq!(ids(5000, 1), vec![5]);
    }

    #[test]
    fn durations_for_the_log() {
        assert_eq!(super::short_duration(45), "45s");
        assert_eq!(super::short_duration(90), "1m30s");
        assert_eq!(super::short_duration(7500), "2h5m");
        assert_eq!(super::short_duration(200_000), "2d7h");
    }

    use super::*;

    fn rule(yaml: &str) -> Result<Rule, String> {
        Rule::compile(&serde_yaml_ng::from_str(yaml).unwrap(), 1)
    }

    fn rec(uid: u64, url: &str, usage: &str, pinned: bool) -> TabRecord {
        TabRecord { uid, url: url.into(), title: format!("t{uid}"), pinned, selected: uid == 1, usage: usage.into(), group: 0, history: String::new() }
    }

    #[test]
    fn stored_tabs_are_swept_by_their_remembered_use() {
        let cfg: SweepConfig = serde_yaml_ng::from_str("enabled: true\nrules:\n  - name: Sleepy\n    match: { url: ['x.com'] }\n    idle_for: 1m\n    action: discard\n  - name: Old\n    idle_for: 1h\n").unwrap();
        let s = Sweeper::from_config(&cfg, None).unwrap();
        let now = 100_000;
        let records = vec![
            rec(1, "https://a.com/old", "1000:1", false),        // idle for ~27 hours: swept (even though it was the selected one)
            rec(2, "https://a.com/fresh", "99900:1", false),     // used a moment ago
            rec(3, "https://a.com/pinned", "1000:1", true),      // pinned: left alone
            rec(4, "https://x.com/asleep", "1000:1", false),     // the first rule to fire is the discard one: nothing to do, the tab is already asleep
            rec(5, "https://a.com/unknown", "", false),          // no recorded use: treated as used now
        ];
        let out = sweep_records(&s, &records, now, 0);
        let swept: Vec<(u64, &str)> = out.swept.iter().map(|(r, n)| (r.uid, n.as_str())).collect();
        assert_eq!(swept, [(1, "Old")]);
        assert_eq!(out.kept.iter().map(|r| r.uid).collect::<Vec<_>>(), [2, 3, 4, 5]);
    }

    #[test]
    fn sleep_after_adds_a_last_rule_that_discards_any_idle_tab_pinned_ones_too() {
        let off = SweepConfig::default();
        assert!(Sweeper::from_config(&off, None).is_none() && Sweeper::from_config(&off, Some("")).is_none());
        // Alone (sweeping off): one discard rule, and the check interval follows the time (half of it, 30 s to 5 min).
        let s = Sweeper::from_config(&off, Some("20m")).unwrap();
        assert_eq!((s.rules.len(), s.rules[0].name.as_str(), s.rules[0].action, s.interval_secs), (1, SLEEP_RULE, Action::Discard, 300));
        assert_eq!(Sweeper::from_config(&off, Some("1m")).unwrap().interval_secs, 30);
        assert_eq!(Sweeper::from_config(&off, Some("10m")).unwrap().interval_secs, 300);
        let (now, url) = (100_000, "https://a.com/");
        let idle = Usage::from_text("90000:1"); // 10000 s ago
        let recent = Usage::from_text("99900:1");
        let s = Sweeper::from_config(&off, Some("1h")).unwrap();
        assert!(s.decide(url, &idle, now, now, false).is_some(), "idle for 2h 46m: asleep");
        assert!(s.decide(url, &recent, now, now, false).is_none(), "used a moment ago");
        assert!(s.decide(url, &idle, now, now, true).is_some(), "a pinned tab may sleep (it is kept)");
        // After the configured rules, which win; and a pinned tab is still safe from those.
        let cfg: SweepConfig = serde_yaml_ng::from_str("enabled: true\nrules:\n  - name: Old\n    idle_for: 1h\n").unwrap();
        let s = Sweeper::from_config(&cfg, Some("30m")).unwrap();
        assert_eq!(s.rules.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Old", SLEEP_RULE]);
        assert_eq!(s.decide(url, &idle, now, now, false).map(|r| r.name.as_str()), Some("Old"));
        assert_eq!(s.decide(url, &idle, now, now, true).map(|r| r.name.as_str()), Some(SLEEP_RULE), "pinned: not closed, but may sleep");
        // The sleep rule does nothing to stored (already asleep) tabs.
        let out = sweep_records(&s, &[rec(1, url, "1000:1", false), rec(2, url, "1000:1", true)], now, 1);
        assert_eq!(out.swept.iter().map(|(r, _)| r.uid).collect::<Vec<_>>(), [1]);
        // A bad duration is a warning, not a rule.
        let bad = Sweeper::from_config(&cfg, Some("soon")).unwrap();
        assert_eq!(bad.rules.len(), 1);
        assert!(bad.warnings.iter().any(|w| w.contains("sleep_tabs_after")), "{:?}", bad.warnings);
    }

    #[test]
    fn a_google_search_idle_for_days_is_closed_by_a_stale_searches_rule_not_put_to_sleep() {
        // A real configuration: `*.google.com` (a subdomain), a short half life, a close action, and `sleep_tabs_after` as well.
        let yaml = "enabled: true\ninterval: 5m\nrules:\n  - name: Stale searches\n    match: { url: [\"*.google.com\", \"duckduckgo.com\"] }\n    idle_for: 1d\n    score_below: 0.5\n    half_life: 15m\n    action: close\n  - name: Rarely used opened links\n    match:\n      url:\n        - directed.fibery.io\n        - gitlab.com/directed-group/*\n    idle_for: 1d\n    score_below: 0.5\n    half_life: 6h\n    action: close\n";
        let cfg: SweepConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let s = Sweeper::from_config(&cfg, Some("5m")).unwrap();
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(s.rules.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Stale searches", "Rarely used opened links", SLEEP_RULE]);
        let url = "https://www.google.com/search?sca_esv=e6431f&udm=2&q=workspaces+icon&sa=X&biw=2042&bih=1080&dpr=1.25";
        let now = 1_000_000;
        let idle_3d7h = Usage::from_text(&format!("{}:1", now - (3 * 86_400 + 7 * 3600)));
        assert_eq!(s.decide(url, &idle_3d7h, now, now, false).map(|r| (r.name.as_str(), r.action)), Some(("Stale searches", Action::Close)));
        // Used within the day: not closed by the rule, but the sleep rule (5 minutes) still puts it to sleep.
        let recent = Usage::from_text(&format!("{}:1", now - 3600));
        assert_eq!(s.decide(url, &recent, now, now, false).map(|r| r.name.as_str()), Some(SLEEP_RULE));
    }

    #[test]
    fn a_workspace_inherits_the_profiles_sweep_rules() {
        use crate::workspace::WorkspaceConfig;
        let root = std::env::temp_dir().join(format!("browsah-sweep-inherit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws-own")).unwrap();
        std::fs::create_dir_all(root.join("ws-plain")).unwrap();
        std::fs::create_dir_all(root.join("ws-off")).unwrap();
        std::fs::write(root.join("profile.yaml"), "sweep:\n  enabled: true\n  interval: 10m\n  rules:\n    - name: Profile rule\n      idle_for: 2d\n").unwrap();
        std::fs::write(root.join("ws-own/workspace.yaml"), "sweep:\n  rules+:\n    - name: Own rule\n      idle_for: 1h\n      action: discard\n").unwrap();
        std::fs::write(root.join("ws-off/workspace.yaml"), "sweep:\n  enabled: false\n").unwrap();
        let profile = Profile { id: "p".into(), config: Default::default(), dir: root.clone() };
        let ws = |id: &str| Workspace { id: id.into(), config: WorkspaceConfig::default(), dir: root.join(format!("ws-{id}")) };
        let names = |id: &str| {
            let cfg = effective_sweep(&profile, &ws(id), &root.join("no-config.yaml"));
            (cfg.enabled, cfg.interval.clone(), Sweeper::from_config(&cfg, None).map(|s| s.rules.iter().map(|r| r.name.clone()).collect::<Vec<_>>()))
        };
        // No sweep section of its own: the profile's, with the profile's interval.
        assert_eq!(names("plain"), (true, "10m".into(), Some(vec!["Profile rule".to_owned()])));
        // `rules+` adds to the profile's rules; the inherited `enabled` and `interval` stay.
        assert_eq!(names("own"), (true, "10m".into(), Some(vec!["Profile rule".to_owned(), "Own rule".to_owned()])));
        // A workspace can switch the inherited sweeping off.
        assert_eq!(names("off"), (false, "10m".into(), None));
        // And the workspaces that have rules are found through the inheritance.
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn profile_sleep_and_workspace_rules_both_reach_the_window() {
        // `sleep_tabs_after` in profile.yaml, the sweep rules in the workspace's own file (what a window is built from).
        let root = std::env::temp_dir().join(format!("browsah-sweep-layers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("default")).unwrap();
        std::fs::write(root.join("profile.yaml"), "sleep_tabs_after: 5m\n").unwrap();
        std::fs::write(root.join("default/workspace.yaml"), "sweep:\n  enabled: true\n  interval: 5m\n  rules:\n    - name: Stale searches\n      match: { url: [\"*.google.com\", \"duckduckgo.com\"] }\n      idle_for: 1d\n      score_below: 0.5\n      half_life: 15m\n      action: close\n").unwrap();
        let loaded = crate::config::load_window(&root.join("no-config.yaml"), &root.join("profile.yaml"), Some(&root.join("default/workspace.yaml"))).unwrap();
        let s = Sweeper::from_config(&loaded.config.sweep, loaded.config.sleep_tabs_after.as_deref()).unwrap();
        assert_eq!(s.rules.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Stale searches", SLEEP_RULE], "{:?}", loaded.warnings);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_last_tab_of_a_workspace_is_never_swept() {
        let cfg: SweepConfig = serde_yaml_ng::from_str("enabled: true\nrules:\n  - idle_for: 1h\n").unwrap();
        let s = Sweeper::from_config(&cfg, None).unwrap();
        let now = 100_000;
        // One tab, long idle: kept.
        let one = sweep_records(&s, &[rec(1, "https://a.com/", "1000:1", false)], now, 0);
        assert!(one.swept.is_empty() && one.kept.len() == 1);
        // Every tab idle: all but the most recently used go.
        let all = sweep_records(&s, &[rec(1, "https://a.com/1", "1000:1", false), rec(2, "https://a.com/2", "5000:1", false), rec(3, "https://a.com/3", "2000:1", false)], now, 0);
        assert_eq!(all.kept.iter().map(|r| r.uid).collect::<Vec<_>>(), [2], "the one used last stays");
        assert_eq!(all.swept.iter().map(|(r, _)| r.uid).collect::<Vec<_>>(), [1, 3]);
        // Pinned tabs from the configuration stay behind, so nothing needs saving.
        let pinned = sweep_records(&s, &[rec(1, "https://a.com/1", "1000:1", false)], now, 1);
        assert!(pinned.kept.is_empty() && pinned.swept.len() == 1);
        // A stored pinned record is a tab that stays too.
        let with_record = sweep_records(&s, &[rec(1, "https://a.com/1", "1000:1", false), rec(2, "https://a.com/2", "1000:1", true)], now, 0);
        assert_eq!(with_record.kept.len(), 1 + 0, "the pinned one stays, so the other can go") ;
        assert_eq!(with_record.swept.len(), 1);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("45s").unwrap(), 45);
        assert_eq!(parse_duration("15m").unwrap(), 900);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration(" 7d ").unwrap(), 604_800);
        for bad in ["", "5", "m", "1.5h", "-2h", "3w", "0m", "h5", "5 m"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn score_fades_with_the_half_life() {
        let mut u = Usage::default();
        u.record(1000, 1.0);
        assert!((u.score(1000, 3600) - 1.0).abs() < 1e-9);
        assert!((u.score(1000 + 3600, 3600) - 0.5).abs() < 1e-9);
        assert!((u.score(1000 + 7200, 3600) - 0.25).abs() < 1e-9);
        // A longer half-life forgets more slowly.
        assert!(u.score(1000 + 3600, 7200) > u.score(1000 + 3600, 3600));
        // Often-used tabs keep a higher score than once-used ones for the same last use.
        let mut often = Usage::default();
        for i in 0..10 {
            often.record(1000 + i * 600, 1.0);
        }
        let mut once = Usage::default();
        once.record(1000 + 9 * 600, 1.0);
        assert!(often.score(10_000, 3600) > once.score(10_000, 3600) * 2.0);
        assert_eq!(often.last_used(), once.last_used());
        // Events from the future (a clock change) do not inflate the score.
        assert!((u.score(500, 3600) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn usage_round_trips_and_is_bounded() {
        let mut u = Usage::default();
        for i in 0..(MAX_EVENTS as i64 + 30) {
            u.record(i, 0.5);
        }
        assert_eq!(u.events.len(), MAX_EVENTS);
        assert_eq!(u.last_used(), Some(MAX_EVENTS as i64 + 29));
        let back = Usage::from_text(&u.to_text());
        assert_eq!(back, u);
        assert_eq!(Usage::from_text(""), Usage::default());
        assert_eq!(Usage::from_text("garbage,1:x,:2,3:0.5").events.len(), 1);
    }

    #[test]
    fn rules_need_a_condition_and_a_valid_action() {
        assert!(rule("idle_for: 1h").is_ok());
        assert!(rule("score_below: 0.5").is_ok());
        assert!(rule("match: { url: ['a.com'] }").unwrap_err().contains("idle_for or score_below"));
        assert!(rule("idle_for: soon").unwrap_err().starts_with("idle_for:"));
        assert!(rule("idle_for: 1h\nhalf_life: 0m").unwrap_err().starts_with("half_life:"));
        assert!(rule("idle_for: 1h\naction: explode").unwrap_err().contains("close or discard"));
        assert!(rule("score_below: -1").is_err());
        assert_eq!(rule("idle_for: 1h\naction: discard").unwrap().action, Action::Discard);
        assert_eq!(rule("idle_for: 1h").unwrap().action, Action::Close);
        assert_eq!(rule("idle_for: 1h").unwrap().name, "rule 1");
    }

    #[test]
    fn a_rule_fires_only_when_every_condition_holds() {
        let r = rule("match: { url: ['*.google.com'] }\nidle_for: 1h\nscore_below: 0.3\nhalf_life: 1h\n").unwrap();
        let mut u = Usage::default();
        u.record(0, 1.0);
        let url = "https://www.google.com/search?q=x";
        // Idle for two hours: score 0.25 (< 0.3) and past the hour.
        assert!(r.fires(url, &u, 7200, 0));
        // Idle long enough but still a high score (used a lot recently before going idle).
        let mut busy = Usage::default();
        for i in 0..20 {
            busy.record(7000 + i, 1.0);
        }
        assert!(!r.fires(url, &busy, 7000 + 3700, 0), "score too high");
        // Low score but not idle long enough.
        assert!(!r.fires(url, &u, 1800, 0), "not idle for an hour");
        // Wrong site.
        assert!(!r.fires("https://example.com/", &u, 7200, 0));
        // No pattern: any tab.
        assert!(rule("idle_for: 1h").unwrap().fires("https://example.com/", &u, 7200, 0));
        // A tab with no use at all counts as idle since it was created.
        let none = Usage::default();
        assert!(rule("idle_for: 1h").unwrap().fires("https://x.com/", &none, 5000, 1000));
        assert!(!rule("idle_for: 1h").unwrap().fires("https://x.com/", &none, 2000, 1000));
    }

    #[test]
    fn the_sweeper_picks_the_first_firing_rule_and_reports_mistakes() {
        let cfg: SweepConfig = serde_yaml_ng::from_str(
            "enabled: true\ninterval: 1m\nrules:\n  - name: Search\n    match: { url: ['google.com'] }\n    idle_for: 1h\n  - name: Everything\n    idle_for: 7d\n    action: discard\n  - name: Broken\n    action: close\n",
        )
        .unwrap();
        let s = Sweeper::from_config(&cfg, None).unwrap();
        assert_eq!(s.interval_secs, 60);
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.warnings.len(), 1);
        assert!(s.warnings[0].contains("rule 3") && s.warnings[0].contains("Broken"));
        let mut u = Usage::default();
        u.record(0, 1.0);
        assert_eq!(s.decide("https://google.com/", &u, 4000, 0, false).map(|r| r.name.as_str()), Some("Search"));
        assert!(s.decide("https://other.com/", &u, 4000, 0, false).is_none());
        assert_eq!(s.decide("https://other.com/", &u, 700_000, 0, false).map(|r| r.action), Some(Action::Discard));
        // Off, or nothing valid: no sweeper.
        assert!(Sweeper::from_config(&SweepConfig::default(), None).is_none());
        let off: SweepConfig = serde_yaml_ng::from_str("rules:\n  - idle_for: 1h\n").unwrap();
        assert!(Sweeper::from_config(&off, None).is_none(), "disabled by default");
        let bad: SweepConfig = serde_yaml_ng::from_str("enabled: true\ninterval: 5s\nrules:\n  - idle_for: 1h\n").unwrap();
        assert_eq!(Sweeper::from_config(&bad, None).unwrap().interval_secs, 300, "an interval that is too short falls back");
    }
}
