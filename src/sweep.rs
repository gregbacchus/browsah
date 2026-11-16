//! Tab sweeping: tidying up tabs that have not been used, by rules a workspace configures.
//!
//! A tab's use is a list of events (it was shown, its page was being worked in), each with a weight.
//! "Idle for" is the time since the last event; the **score** adds the events up with older ones
//! counting for less, `sum(weight * 0.5^(age / half_life))`, so a tab visited often keeps a high score
//! for a long while after its last visit and a tab looked at once fades quickly. Rules compare both.
//!
//! This module is the model: durations, usage, rules and the decision. Applying a decision (closing or
//! discarding a tab, checking for unsaved form input and playing media first) is in `app/sweep.rs`.

use crate::workspace::scope_matches;
use serde::Deserialize;
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

/// The `sweep:` section of `workspace.yaml`.
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Default, Deserialize)]
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
}

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
        Ok(Rule { name, urls: cfg.matches.url.clone(), idle_for, score_below: cfg.score_below, half_life, action })
    }

    /// Does the rule apply to this tab now? All its conditions must hold. A tab with no recorded use
    /// counts as idle since `fallback` (when it was created).
    pub fn fires(&self, url: &str, usage: &Usage, now: i64, fallback: i64) -> bool {
        if !self.urls.is_empty() && !self.urls.iter().any(|p| scope_matches(p, url)) {
            return false;
        }
        let last = usage.last_used().unwrap_or(fallback);
        if self.idle_for.is_some_and(|d| now - last < d) {
            return false;
        }
        if self.score_below.is_some_and(|s| usage.score(now, self.half_life) >= s) {
            return false;
        }
        true
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
    /// `None` when sweeping is off or has no valid rule.
    pub fn from_config(cfg: &SweepConfig) -> Option<Sweeper> {
        if !cfg.enabled {
            return None;
        }
        let mut s = Sweeper { interval_secs: 300, ..Sweeper::default() };
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
        (!s.rules.is_empty()).then_some(s)
    }

    /// The first rule that fires for a tab.
    pub fn decide(&self, url: &str, usage: &Usage, now: i64, created: i64) -> Option<&Rule> {
        self.rules.iter().find(|r| r.fires(url, usage, now, created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(yaml: &str) -> Result<Rule, String> {
        Rule::compile(&serde_yaml_ng::from_str(yaml).unwrap(), 1)
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
        let s = Sweeper::from_config(&cfg).unwrap();
        assert_eq!(s.interval_secs, 60);
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.warnings.len(), 1);
        assert!(s.warnings[0].contains("rule 3") && s.warnings[0].contains("Broken"));
        let mut u = Usage::default();
        u.record(0, 1.0);
        assert_eq!(s.decide("https://google.com/", &u, 4000, 0).map(|r| r.name.as_str()), Some("Search"));
        assert!(s.decide("https://other.com/", &u, 4000, 0).is_none());
        assert_eq!(s.decide("https://other.com/", &u, 700_000, 0).map(|r| r.action), Some(Action::Discard));
        // Off, or nothing valid: no sweeper.
        assert!(Sweeper::from_config(&SweepConfig::default()).is_none());
        let off: SweepConfig = serde_yaml_ng::from_str("rules:\n  - idle_for: 1h\n").unwrap();
        assert!(Sweeper::from_config(&off).is_none(), "disabled by default");
        let bad: SweepConfig = serde_yaml_ng::from_str("enabled: true\ninterval: 5s\nrules:\n  - idle_for: 1h\n").unwrap();
        assert_eq!(Sweeper::from_config(&bad).unwrap().interval_secs, 300, "an interval that is too short falls back");
    }
}
