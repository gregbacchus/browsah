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

/// The host a request address points at (lower case, no port or credentials).
pub fn host_of(url: &str) -> String {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("");
    host.rsplit('@').next().unwrap_or(host).split(':').next().unwrap_or(host).to_ascii_lowercase()
}

/// `cookies:` rules that allow `hosts`, ready to paste into a configuration file.
pub fn allow_rules_yaml(hosts: &[String]) -> String {
    let list = hosts.iter().map(|h| format!("\"{h}\"")).collect::<Vec<_>>().join(", ");
    format!("cookies:\n  rules:\n    - match: [{list}]\n      action: allow\n")
}

/// Identifies one session cookie within one cookie store: the store, the cookie's domain (or, if it has
/// none, the host of the request), its name and its path.
pub fn session_key(store: &str, request_url: &str, domain: &str, name: &str, path: &str) -> String {
    let host = host_of(request_url);
    let domain = domain.trim().trim_start_matches('.');
    let domain = if domain.is_empty() { host.as_str() } else { domain };
    format!("{store}\u{1}{}\u{1}{name}\u{1}{path}", domain.to_ascii_lowercase())
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
    fn session_keys_ignore_how_the_domain_is_written() {
        let k = |url: &str, d: &str| session_key("s", url, d, "ck", "/");
        assert_eq!(k("https://News.Example:8443/a?x", ""), k("http://news.example/", ".news.example"));
        assert_ne!(k("https://a.example/", ""), k("https://b.example/", ""));
        assert_ne!(session_key("one", "https://a/", "", "n", "/"), session_key("two", "https://a/", "", "n", "/"), "stores differ");
        assert_ne!(session_key("s", "https://a/", "", "n", "/"), session_key("s", "https://a/", "", "m", "/"));
    }

    #[test]
    fn allow_rules_for_blocked_sites_parse_back() {
        let yaml = allow_rules_yaml(&["a.example".to_owned(), "cdn.b.example".to_owned()]);
        let cfg: crate::config::Config = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(cfg.cookies.decide("https://cdn.b.example/x"), Action::Allow);
        assert_eq!(host_of("https://User@A.Example:8080/p?q"), "a.example");
    }

    #[test]
    fn a_rule_needs_an_action() {
        assert!(serde_yaml_ng::from_str::<CookiesConfig>("rules:\n  - match: ['a.com']\n").is_err());
        assert!(serde_yaml_ng::from_str::<CookiesConfig>("default: maybe\n").is_err());
    }
}
