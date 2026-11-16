//! URL routing: where a link goes.
//!
//! Every link the browser is asked to open goes through the router: links handed over by other
//! applications, links clicked in a page, and (if a rule asks for it) typed addresses. The router
//! looks at the address, where the request came from, what kind of navigation it is and how it asks
//! to be opened, and answers with an action: carry on as usual, open in a workspace, open in an
//! ephemeral window, hand to the desktop, hand to another profile, or block.
//!
//! Rules are written under `routing:` in the application file (`config.yaml`), a profile's
//! `profile.yaml` and a workspace's `workspace.yaml`; see docs/config/routing.md. The first rule that
//! matches wins; a workspace's rules are tried first (for tabs in that workspace), then the
//! profile's, then the application's.
//!
//! The router is a pure function of immutable data, so it can answer on CEF's thread; the
//! `browsah route test` command exercises it without starting a browser.

use crate::profile::{Profile, DEFAULT_ID};
use crate::workspace::{scope_matches, Workspace};
use serde::Deserialize;
use serde_yaml_ng::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

// ---- the request and the answer ---------------------------------------------------------------

/// Where a request came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Another application (the desktop handed us the link).
    External,
    /// A tab of this browser, in a workspace (`None`: an ephemeral window).
    Tab(Option<String>),
}

/// What sort of navigation it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A link handed over by another application.
    External,
    /// A link clicked in a page (a user gesture, not a form post, not a redirect).
    Link,
    /// An address typed in the address bar or opened from a bookmark: anything else the browser
    /// itself navigates to.
    Typed,
}

/// How the page asked to open it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// In the same tab (an ordinary link).
    SameTab,
    /// In a new tab (`target="_blank"`, a middle click, `Ctrl`+click).
    NewTab,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    pub source: Source,
    pub kind: Kind,
    pub target: Target,
}

/// What to do with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Carry on as if there were no routing (a navigation proceeds where it is; an external link
    /// opens as a tab in the active window).
    Allow,
    Block,
    /// A window with no workspace.
    Ephemeral,
    /// The desktop's handler for the address. Only for schemes other than http and https (a web
    /// link would come straight back here).
    System,
    Workspace(String),
    /// Another profile (and optionally one of its workspaces).
    Profile { profile: String, workspace: Option<String> },
}

impl Action {
    /// Parse the `open:` text of a rule.
    pub fn parse(text: &str) -> Result<Action, String> {
        let t = text.trim();
        match t {
            "allow" | "current" => Ok(Action::Allow),
            "block" => Ok(Action::Block),
            "ephemeral" => Ok(Action::Ephemeral),
            "system" => Ok(Action::System),
            _ => {
                if let Some(id) = t.strip_prefix("workspace:") {
                    if crate::profile::valid_id(id) {
                        return Ok(Action::Workspace(id.to_owned()));
                    }
                } else if let Some(rest) = t.strip_prefix("profile:") {
                    let (profile, workspace) = match rest.split_once('/') {
                        Some((p, w)) => (p, w.strip_prefix("workspace:")),
                        None => (rest, None),
                    };
                    let ws_ok = workspace.is_none_or(crate::profile::valid_id) && !(rest.contains('/') && workspace.is_none());
                    if crate::profile::valid_id(profile) && ws_ok {
                        return Ok(Action::Profile { profile: profile.to_owned(), workspace: workspace.map(str::to_owned) });
                    }
                }
                Err(format!("unknown action {text:?} (use allow, block, ephemeral, system, workspace:<id>, profile:<id> or profile:<id>/workspace:<id>)"))
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Action::Allow => "carry on as usual (allow)".into(),
            Action::Block => "block".into(),
            Action::Ephemeral => "open in an ephemeral window".into(),
            Action::System => "hand to the desktop".into(),
            Action::Workspace(w) => format!("open in workspace {w:?}"),
            Action::Profile { profile, workspace: None } => format!("open in profile {profile:?}"),
            Action::Profile { profile, workspace: Some(w) } => format!("open in profile {profile:?}, workspace {w:?}"),
        }
    }
}

/// The answer, with the rule that gave it (for `route test`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub action: Action,
    /// Which rule matched: layer, position in that layer (from 1) and its name.
    pub rule: Option<(String, usize, Option<String>)>,
}

// ---- configuration ----------------------------------------------------------------------------

/// The `routing:` section of a configuration file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct RoutingConfig {
    /// What to do with links from other applications when no rule matches (default: open as a tab
    /// in the active window).
    pub external: Option<String>,
    pub rules: Vec<RuleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct RuleConfig {
    /// Shown by `route test`.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "match")]
    pub matches: MatchConfig,
    /// The action (see `Action::parse`).
    pub open: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct MatchConfig {
    /// Address patterns (`host` or `host/path`, see `workspace::scope_matches`); any one matches.
    pub url: Vec<String>,
    /// URL schemes (`mailto`, `zoommtg`); any one matches.
    pub scheme: Vec<String>,
    /// Sources: `external`, `workspace:<id>`, `ephemeral` (a window with no workspace); any one matches.
    pub from: Vec<String>,
    /// Kinds of navigation; default `link` and `external`.
    pub kind: Vec<Kind>,
    /// How the page asks to open it; default any.
    pub target: Vec<Target>,
}

/// A source selector in a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FromSel {
    External,
    Ephemeral,
    Workspace(String),
}

#[derive(Debug, Clone)]
struct Rule {
    name: Option<String>,
    urls: Vec<String>,
    schemes: Vec<String>,
    from: Vec<FromSel>,
    kinds: Vec<Kind>,
    targets: Vec<Target>,
    action: Action,
}

impl Rule {
    fn compile(cfg: &RuleConfig) -> Result<Rule, String> {
        let from = cfg
            .matches
            .from
            .iter()
            .map(|f| match f.trim() {
                "external" => Ok(FromSel::External),
                "ephemeral" => Ok(FromSel::Ephemeral),
                t => t
                    .strip_prefix("workspace:")
                    .filter(|id| crate::profile::valid_id(id))
                    .map(|id| FromSel::Workspace(id.to_owned()))
                    .ok_or_else(|| format!("unknown source {f:?} (use external, ephemeral or workspace:<id>)")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let kinds = if cfg.matches.kind.is_empty() { vec![Kind::Link, Kind::External] } else { cfg.matches.kind.clone() };
        Ok(Rule {
            name: cfg.name.clone(),
            urls: cfg.matches.url.clone(),
            schemes: cfg.matches.scheme.iter().map(|s| s.to_lowercase()).collect(),
            from,
            kinds,
            targets: cfg.matches.target.clone(),
            action: Action::parse(&cfg.open)?,
        })
    }

    /// `None` if the rule matches, else why not.
    fn mismatch(&self, req: &Request) -> Option<&'static str> {
        if !self.kinds.contains(&req.kind) {
            return Some("kind");
        }
        if !self.targets.is_empty() && !self.targets.iter().any(|t| *t == req.target) {
            return Some("target");
        }
        if !self.from.is_empty() {
            let ok = self.from.iter().any(|f| match (f, &req.source) {
                (FromSel::External, Source::External) => true,
                (FromSel::Ephemeral, Source::Tab(None)) => true,
                (FromSel::Workspace(w), Source::Tab(Some(src))) => w == src,
                _ => false,
            });
            if !ok {
                return Some("source");
            }
        }
        let scheme = req.url.split_once(':').map(|(s, _)| s.to_lowercase()).unwrap_or_default();
        if !self.schemes.is_empty() && !self.schemes.contains(&scheme) {
            return Some("scheme");
        }
        if !self.urls.is_empty() && !self.urls.iter().any(|p| scope_matches(p, &req.url)) {
            return Some("url");
        }
        None
    }
}

/// All the rules of a profile process.
#[derive(Debug, Clone, Default)]
pub struct Router {
    by_workspace: HashMap<String, Vec<Rule>>,
    profile: Vec<Rule>,
    app: Vec<Rule>,
    external: Option<Action>,
    /// A label for the profile layer (`route test` output).
    profile_label: String,
    /// Problems found while loading (invalid rules are skipped).
    pub warnings: Vec<String>,
}

/// Read the `routing:` section of a YAML file (none if the file or section is missing).
fn read_routing(path: &Path, warnings: &mut Vec<String>) -> RoutingConfig {
    let Ok(text) = std::fs::read_to_string(path) else { return RoutingConfig::default() };
    let value: Value = match serde_yaml_ng::from_str(&text) {
        Ok(v) => v,
        Err(_) => return RoutingConfig::default(), // the configuration loader reports bad files
    };
    let Some(section) = value.get("routing") else { return RoutingConfig::default() };
    match serde_yaml_ng::from_value(section.clone()) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!("{}: routing: {e}; its rules are ignored", path.display()));
            RoutingConfig::default()
        }
    }
}

fn compile_all(layer: &str, file: &Path, cfg: &RoutingConfig, warnings: &mut Vec<String>) -> Vec<Rule> {
    cfg.rules
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match Rule::compile(r) {
            Ok(rule) => Some(rule),
            Err(e) => {
                warnings.push(format!("{}: {layer} rule {}{}: {e}; skipped", file.display(), i + 1, r.name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()));
                None
            }
        })
        .collect()
}

impl Router {
    /// Load the rules of the application file and, if given, of a profile and its workspaces.
    pub fn load(app_file: &Path, profile: Option<&Profile>) -> Router {
        let mut router = Router { profile_label: profile.map(|p| p.id.clone()).unwrap_or_default(), ..Router::default() };
        let mut warnings = Vec::new();
        let app_cfg = read_routing(app_file, &mut warnings);
        router.app = compile_all("application", app_file, &app_cfg, &mut warnings);
        let mut external = app_cfg.external.clone();
        if let Some(p) = profile {
            let file = p.dir.join("profile.yaml");
            let cfg = read_routing(&file, &mut warnings);
            router.profile = compile_all("profile", &file, &cfg, &mut warnings);
            if cfg.external.is_some() {
                external = cfg.external.clone();
            }
            for w in Workspace::list(p) {
                let file = w.dir.join("workspace.yaml");
                let cfg = read_routing(&file, &mut warnings);
                let rules = compile_all(&format!("workspace {}", w.id), &file, &cfg, &mut warnings);
                if !rules.is_empty() {
                    router.by_workspace.insert(w.id.clone(), rules);
                }
            }
        }
        router.external = external.and_then(|e| match Action::parse(&e) {
            Ok(a) => Some(a),
            Err(err) => {
                warnings.push(format!("routing.external: {err}; ignored"));
                None
            }
        });
        router.warnings = warnings;
        router
    }

    /// Decide where `req` goes.
    pub fn route(&self, req: &Request) -> Decision {
        self.explain(req).0
    }

    /// Decide, and list how every rule fared (layer, position, name, `matched` or the reason it did not).
    pub fn explain(&self, req: &Request) -> (Decision, Vec<(String, usize, Option<String>, String)>) {
        let mut trace = Vec::new();
        let mut layers: Vec<(String, &[Rule])> = Vec::new();
        if let Source::Tab(Some(w)) = &req.source {
            if let Some(rules) = self.by_workspace.get(w) {
                layers.push((format!("workspace {w}"), rules));
            }
        }
        let profile_name = if self.profile_label.is_empty() { "profile".to_owned() } else { format!("profile {}", self.profile_label) };
        layers.push((profile_name, &self.profile));
        layers.push(("application".to_owned(), &self.app));
        let mut decision = None;
        for (layer, rules) in layers {
            for (i, rule) in rules.iter().enumerate() {
                let outcome = match rule.mismatch(req) {
                    None => "matched".to_owned(),
                    Some(why) => format!("no ({why})"),
                };
                let matched = outcome == "matched";
                trace.push((layer.clone(), i + 1, rule.name.clone(), outcome));
                if matched && decision.is_none() {
                    decision = Some(Decision { action: rule.action.clone(), rule: Some((layer.clone(), i + 1, rule.name.clone())) });
                }
            }
        }
        let decision = decision.unwrap_or_else(|| match (&req.kind, &self.external) {
            (Kind::External, Some(a)) => Decision { action: a.clone(), rule: Some(("routing.external".into(), 0, None)) },
            _ => Decision { action: Action::Allow, rule: None },
        });
        (decision, trace)
    }

    /// Whether a rule that opens `workspace` exists but the workspace does not (for warnings).
    pub fn referenced_workspaces(&self) -> Vec<String> {
        let mut out = Vec::new();
        let all = self.by_workspace.values().flatten().chain(self.profile.iter()).chain(self.app.iter());
        for r in all {
            if let Action::Workspace(w) = &r.action {
                out.push(w.clone());
            }
        }
        if let Some(Action::Workspace(w)) = &self.external {
            out.push(w.clone());
        }
        out.sort();
        out.dedup();
        out
    }
}

// ---- the process-wide router --------------------------------------------------------------------

static CURRENT: RwLock<Option<Arc<Router>>> = RwLock::new(None);

/// Install the router (at start-up and after the configuration is reloaded).
pub fn install(router: Router) {
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(Arc::new(router));
    }
}

/// The router, from any thread. Without one installed everything is allowed.
pub fn route(req: &Request) -> Decision {
    let router = CURRENT.read().ok().and_then(|g| g.clone());
    match router {
        Some(r) => r.route(req),
        None => Decision { action: Action::Allow, rule: None },
    }
}

// ---- `browsah route test` -----------------------------------------------------------------------

const USAGE: &str = "usage: browsah route test <url> [--profile ID] [--from SOURCE] [--kind KIND] [--target TARGET] [--explain]

  --profile ID    use this profile's rules and workspaces; without it only the application's rules
                  apply (how a link from another application is first judged)
  --from SOURCE   external (default), ephemeral, or workspace:<id> (a link clicked in that workspace)
  --kind KIND     external (default for --from external), link (default otherwise) or typed
  --target TARGET same_tab (default) or new_tab
  --explain       show how every rule fared";

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

/// `browsah route ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("test") {
        eprintln!("{USAGE}");
        return 2;
    }
    let rest = &args[1..];
    let Some(url) = crate::workspace::first_positional(rest).cloned() else {
        eprintln!("{USAGE}");
        return 2;
    };
    let profile = match flag_value(rest, "--profile") {
        Some(id) => match Profile::resolve(&id) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        },
        None => None,
    };
    let from = flag_value(rest, "--from").unwrap_or_else(|| "external".into());
    let source = match from.as_str() {
        "external" => Source::External,
        "ephemeral" => Source::Tab(None),
        f => match f.strip_prefix("workspace:") {
            Some(w) => Source::Tab(Some(w.to_owned())),
            None => {
                eprintln!("--from: expected external, ephemeral or workspace:<id>, got {f:?}");
                return 2;
            }
        },
    };
    let kind = match flag_value(rest, "--kind").as_deref() {
        None => if source == Source::External { Kind::External } else { Kind::Link },
        Some("external") => Kind::External,
        Some("link") => Kind::Link,
        Some("typed") => Kind::Typed,
        Some(k) => {
            eprintln!("--kind: expected external, link or typed, got {k:?}");
            return 2;
        }
    };
    let target = match flag_value(rest, "--target").as_deref() {
        None | Some("same_tab") => Target::SameTab,
        Some("new_tab") => Target::NewTab,
        Some(t) => {
            eprintln!("--target: expected same_tab or new_tab, got {t:?}");
            return 2;
        }
    };
    let app_file = crate::config::default_path();
    let router = Router::load(&app_file, profile.as_ref());
    for w in &router.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(p) = &profile {
        let known: Vec<String> = Workspace::list(p).into_iter().map(|w| w.id).collect();
        for w in router.referenced_workspaces() {
            if !known.contains(&w) {
                eprintln!("warning: a rule opens workspace {w:?}, which does not exist in profile {:?}", p.id);
            }
        }
    }
    let req = Request { url: url.clone(), source: source.clone(), kind, target };
    let (decision, trace) = router.explain(&req);
    println!("address:  {url}");
    println!("from:     {from}   kind: {kind:?}   target: {target:?}");
    println!("rules:    {}", match &profile {
        Some(p) => format!("application + profile {:?}{}", p.id, if p.id == DEFAULT_ID { " (default)" } else { "" }),
        None => "application only (no --profile)".to_owned(),
    });
    if rest.iter().any(|a| a == "--explain") {
        for (layer, i, name, outcome) in &trace {
            println!("  {layer} rule {i}{}: {outcome}", name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default());
        }
        if trace.is_empty() {
            println!("  (no rules)");
        }
    }
    match &decision.rule {
        Some((layer, 0, _)) => println!("decision: {}   [{layer}]", decision.action.describe()),
        Some((layer, i, name)) => println!("decision: {}   [{layer} rule {i}{}]", decision.action.describe(), name.as_ref().map(|n| format!(": {n}")).unwrap_or_default()),
        None => println!("decision: {}   [no rule matched]", decision.action.describe()),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(yaml: &str) -> Router {
        let cfg: RoutingConfig = serde_yaml_ng::from_str(yaml).unwrap();
        let mut warnings = Vec::new();
        let rules = compile_all("profile", Path::new("test.yaml"), &cfg, &mut warnings);
        Router { profile: rules, external: cfg.external.as_deref().and_then(|e| Action::parse(e).ok()), warnings, ..Router::default() }
    }

    fn req(url: &str, source: Source, kind: Kind, target: Target) -> Request {
        Request { url: url.into(), source, kind, target }
    }

    fn ext(url: &str) -> Request {
        req(url, Source::External, Kind::External, Target::SameTab)
    }

    fn click(url: &str, ws: &str) -> Request {
        req(url, Source::Tab(Some(ws.into())), Kind::Link, Target::SameTab)
    }

    #[test]
    fn actions_parse() {
        assert_eq!(Action::parse("allow").unwrap(), Action::Allow);
        assert_eq!(Action::parse("current").unwrap(), Action::Allow);
        assert_eq!(Action::parse("workspace:mail").unwrap(), Action::Workspace("mail".into()));
        assert_eq!(Action::parse("profile:work").unwrap(), Action::Profile { profile: "work".into(), workspace: None });
        assert_eq!(Action::parse("profile:work/workspace:mail").unwrap(), Action::Profile { profile: "work".into(), workspace: Some("mail".into()) });
        for bad in ["", "workspace:", "workspace:Bad Id", "profile:", "profile:work/mail", "profile:work/workspace:", "launch", "workspace"] {
            assert!(Action::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn first_matching_rule_wins_and_default_is_allow() {
        let r = rules("rules:\n  - name: mail\n    match: { url: ['mail.example.com'] }\n    open: workspace:mail\n  - match: { url: ['*.example.com'] }\n    open: workspace:other\n");
        let d = r.route(&ext("https://mail.example.com/inbox"));
        assert_eq!(d.action, Action::Workspace("mail".into()));
        assert_eq!(d.rule, Some(("profile".into(), 1, Some("mail".into()))));
        assert_eq!(r.route(&ext("https://x.example.com/")).action, Action::Workspace("other".into()));
        assert_eq!(r.route(&ext("https://unrelated.net/")), Decision { action: Action::Allow, rule: None });
    }

    #[test]
    fn selectors_combine_with_and_and_alternatives_with_or() {
        let r = rules("rules:\n  - match: { url: ['a.com', 'b.com'], from: [external, 'workspace:chat'], target: [new_tab] }\n    open: block\n");
        let blocked = |x: Request| r.route(&x).action == Action::Block;
        assert!(blocked(req("https://a.com/", Source::External, Kind::External, Target::NewTab)));
        assert!(blocked(req("https://b.com/", Source::Tab(Some("chat".into())), Kind::Link, Target::NewTab)));
        assert!(!blocked(req("https://c.com/", Source::External, Kind::External, Target::NewTab)), "url");
        assert!(!blocked(req("https://a.com/", Source::External, Kind::External, Target::SameTab)), "target");
        assert!(!blocked(req("https://a.com/", Source::Tab(Some("docs".into())), Kind::Link, Target::NewTab)), "source");
        assert!(!blocked(req("https://a.com/", Source::Tab(None), Kind::Link, Target::NewTab)), "ephemeral is not a workspace");
    }

    #[test]
    fn kinds_default_to_links_and_external_and_typed_is_opt_in() {
        let r = rules("rules:\n  - match: { url: ['a.com'] }\n    open: block\n  - match: { url: ['t.com'], kind: [typed] }\n    open: block\n");
        let blocked = |x: Request| r.route(&x).action == Action::Block;
        assert!(blocked(ext("https://a.com/")));
        assert!(blocked(click("https://a.com/", "w")));
        assert!(!blocked(req("https://a.com/", Source::Tab(None), Kind::Typed, Target::SameTab)), "typing an address is not routed unless asked");
        assert!(blocked(req("https://t.com/", Source::Tab(None), Kind::Typed, Target::SameTab)));
        assert!(!blocked(click("https://t.com/", "w")));
    }

    #[test]
    fn schemes_and_ephemeral_source() {
        let r = rules("rules:\n  - match: { scheme: [mailto, zoommtg] }\n    open: system\n  - match: { from: [ephemeral] }\n    open: workspace:scratch\n");
        assert_eq!(r.route(&ext("mailto:a@b.c")).action, Action::System);
        assert_eq!(r.route(&ext("ZOOMMTG://x")).action, Action::System);
        assert_eq!(r.route(&req("https://x.com/", Source::Tab(None), Kind::Link, Target::SameTab)).action, Action::Workspace("scratch".into()));
        assert_eq!(r.route(&click("https://x.com/", "w")).action, Action::Allow);
    }

    #[test]
    fn external_default_applies_only_to_external_links() {
        let r = rules("external: workspace:inbox\nrules:\n  - match: { url: ['a.com'] }\n    open: workspace:a\n");
        assert_eq!(r.route(&ext("https://z.com/")).action, Action::Workspace("inbox".into()));
        assert_eq!(r.route(&click("https://z.com/", "w")).action, Action::Allow);
        assert_eq!(r.route(&ext("https://a.com/")).action, Action::Workspace("a".into()), "a rule beats the default");
    }

    #[test]
    fn invalid_rules_are_skipped_with_a_warning() {
        let r = rules("rules:\n  - match: { url: ['a.com'] }\n    open: teleport\n  - match: { from: ['nowhere'] }\n    open: block\n  - match: { url: ['ok.com'] }\n    open: block\n");
        assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
        assert!(r.warnings[0].contains("rule 1") && r.warnings[1].contains("rule 2"));
        assert_eq!(r.route(&ext("https://ok.com/")).action, Action::Block);
    }

    #[test]
    fn workspace_rules_come_first_for_their_own_tabs() {
        let mut r = rules("rules:\n  - match: { url: ['x.com'] }\n    open: workspace:profilewide\n");
        r.by_workspace.insert("chat".into(), vec![Rule::compile(&serde_yaml_ng::from_str("match: { url: ['x.com'] }\nopen: workspace:chatonly\n").unwrap()).unwrap()]);
        assert_eq!(r.route(&click("https://x.com/", "chat")).action, Action::Workspace("chatonly".into()));
        assert_eq!(r.route(&click("https://x.com/", "docs")).action, Action::Workspace("profilewide".into()));
        assert_eq!(r.route(&ext("https://x.com/")).action, Action::Workspace("profilewide".into()), "workspace rules only judge their own tabs' links");
        let (_, trace) = r.explain(&click("https://x.com/", "chat"));
        assert_eq!(trace[0].0, "workspace chat");
    }

    #[test]
    fn explain_lists_why_rules_did_not_match() {
        let r = rules("rules:\n  - name: a\n    match: { url: ['a.com'] }\n    open: block\n  - name: b\n    match: { url: ['b.com'], from: [external] }\n    open: block\n");
        let (d, trace) = r.explain(&click("https://b.com/", "w"));
        assert_eq!(d.action, Action::Allow);
        let outcomes: Vec<&str> = trace.iter().map(|t| t.3.as_str()).collect();
        assert_eq!(outcomes, ["no (url)", "no (source)"]);
    }

    #[test]
    fn docs_cover_every_routing_option() {
        let docs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/config/routing.md")).expect("docs/config/routing.md exists");
        fn fields<T: serde::de::DeserializeOwned>() -> Vec<String> {
            let err = serde_yaml_ng::from_str::<T>("{ __nope__: 1 }").err().unwrap().to_string();
            err.split("expected").nth(1).unwrap_or("").split('`').skip(1).step_by(2).map(str::to_owned).collect()
        }
        let mut paths: Vec<String> = fields::<RoutingConfig>().into_iter().map(|f| format!("routing.{f}")).collect();
        paths.extend(fields::<RuleConfig>().into_iter().map(|f| format!("routing.rules[].{f}")));
        paths.extend(fields::<MatchConfig>().into_iter().map(|f| format!("routing.rules[].match.{f}")));
        assert!(paths.len() >= 9, "{paths:?}");
        for p in paths {
            assert!(docs.contains(&format!("| `{p}`")), "routing option {p} is not documented in docs/config/routing.md");
        }
    }

    #[test]
    fn example_routing_loads_without_warnings() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut warnings = Vec::new();
        for f in ["examples/config.yaml", "examples/profile.yaml", "examples/workspace.yaml"] {
            let cfg = read_routing(&root.join(f), &mut warnings);
            let compiled = compile_all("example", &root.join(f), &cfg, &mut warnings);
            assert_eq!(compiled.len(), cfg.rules.len(), "{f}");
        }
        assert!(warnings.is_empty(), "{warnings:?}");
        let profile_rules = read_routing(&root.join("examples/profile.yaml"), &mut warnings);
        assert!(!profile_rules.rules.is_empty(), "examples/profile.yaml should show routing rules");
    }
}
