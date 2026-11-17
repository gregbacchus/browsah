//! Cookie rules: which sites' cookies are used from and stored to the profile.
//!
//! `cookies:` in the configuration (any layer, so also per workspace) allows or denies cookies by the
//! address they travel with: a denied site's cookies are neither sent with requests nor saved from
//! responses. The rules are applied by a cookie filter on CEF's network thread (`app/view.rs`).
//! `session` is a middle way: cookies work normally but are deleted when the window closes.
//! (Other site data, such as local storage, cannot be filtered this way; a workspace can instead keep
//! all of it apart with `storage: isolated`.)

use crate::workspace::scope_matches;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    /// Used while the browser is open, then forgotten when the window closes (and not kept from an earlier run).
    Session,
    Deny,
}

/// The `cookies:` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct CookiesConfig {
    /// For sites no rule covers.
    pub default: Action,
    /// Rules, tried in order; the first whose `match` fits the address decides.
    pub rules: Vec<CookieRuleConfig>,
}

impl Default for CookiesConfig {
    fn default() -> Self {
        Self { default: Action::Allow, rules: Vec::new() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct CookieRuleConfig {
    /// Address patterns, as for a pinned tab's scope (`tracker.example.com`, `*.ads.example`); any one may match.
    #[serde(default, rename = "match")]
    pub matches: Vec<String>,
    pub action: Action,
}

impl CookiesConfig {
    /// Whether cookies travelling with a request to `url` may be sent and saved.
    pub fn decide(&self, url: &str) -> Action {
        self.rules.iter().find(|r| r.matches.iter().any(|p| scope_matches(p, url))).map(|r| r.action).unwrap_or(self.default)
    }

    /// True when nothing is filtered (the filter can be skipped).
    /// True when some rule wants cookies forgotten at the end of the session.
    pub fn has_session_rules(&self) -> bool {
        self.default == Action::Session || self.rules.iter().any(|r| r.action == Action::Session)
    }

    pub fn is_open(&self) -> bool {
        self.default == Action::Allow && self.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> CookiesConfig {
        serde_yaml_ng::from_str(yaml).unwrap()
    }

    #[test]
    fn everything_is_allowed_by_default() {
        let c = CookiesConfig::default();
        assert!(c.is_open());
        assert_eq!(c.decide("https://anything.example/"), Action::Allow);
    }

    #[test]
    fn rules_decide_by_address_and_the_first_match_wins() {
        let c = cfg("rules:\n  - match: ['tracker.example.com', '*.ads.example']\n    action: deny\n  - match: ['**.example.com']\n    action: allow\n  - match: ['shop.example.org/cart/*']\n    action: deny\n");
        assert!(!c.is_open());
        assert_eq!(c.decide("https://tracker.example.com/pixel"), Action::Deny);
        assert_eq!(c.decide("https://x.ads.example/"), Action::Deny);
        assert_eq!(c.decide("https://www.example.com/"), Action::Allow);
        assert_eq!(c.decide("https://shop.example.org/cart/1"), Action::Deny, "a path pattern");
        assert_eq!(c.decide("https://shop.example.org/home"), Action::Allow, "outside the path: the default");
        // The first match wins even if a later one would say otherwise.
        assert_eq!(cfg("rules:\n  - match: ['a.com']\n    action: allow\n  - match: ['a.com']\n    action: deny\n").decide("https://a.com/"), Action::Allow);
    }

    #[test]
    fn a_default_of_deny_works_as_an_allow_list() {
        let c = cfg("default: deny\nrules:\n  - match: ['bank.example', 'mail.example.com']\n    action: allow\n");
        assert_eq!(c.decide("https://bank.example/login"), Action::Allow);
        assert_eq!(c.decide("https://news.example/"), Action::Deny);
        assert!(!c.is_open());
    }

    #[test]
    fn session_rules_are_found_and_decided() {
        let c = cfg("rules:\n  - match: ['news.example']\n    action: session\n");
        assert!(c.has_session_rules() && !c.is_open());
        assert_eq!(c.decide("https://news.example/"), Action::Session);
        assert_eq!(c.decide("https://other.example/"), Action::Allow);
        assert!(!CookiesConfig::default().has_session_rules());
        assert!(cfg("default: session\n").has_session_rules());
    }

    #[test]
    fn a_rule_needs_an_action() {
        assert!(serde_yaml_ng::from_str::<CookiesConfig>("rules:\n  - match: ['a.com']\n").is_err());
        assert!(serde_yaml_ng::from_str::<CookiesConfig>("default: maybe\n").is_err());
    }
}
