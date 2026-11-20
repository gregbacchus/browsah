//! The "all options" view of a configuration file: the documented example for a level (application,
//! profile, workspace) with every option shown and explained, the options the real file sets switched on
//! with their values and everything else commented out. Pure text work (no GTK), so it is tested.
//!
//! The examples in `examples/` are the templates, so the comments are the ones the documentation already
//! carries. The result is a generated *copy*: the person can look at it, edit it and save it over the real
//! file if they want to (the window is `app/fullconfig.rs`; `browsah config full` prints it).

use serde_yaml_ng::Value;

const APP_TEMPLATE: &str = include_str!("../examples/config.yaml");
const PROFILE_TEMPLATE: &str = include_str!("../examples/profile.yaml");
const WORKSPACE_TEMPLATE: &str = include_str!("../examples/workspace.yaml");
const WEBAPP_TEMPLATE: &str = include_str!("../examples/webapp.yaml");

/// Which file is being shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `~/.config/browsah/config.yaml`
    Application,
    /// A profile's `profile.yaml`
    Profile,
    /// A workspace's `workspace.yaml`
    Workspace,
    /// A web app's `app.yaml`
    App,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Application => "Application",
            Level::Profile => "Profile",
            Level::Workspace => "Workspace",
            Level::App => "Web app",
        }
    }

    /// The file's name, for the header comment.
    fn file_name(self) -> &'static str {
        match self {
            Level::Application => "config.yaml",
            Level::Profile => "profile.yaml",
            Level::Workspace => "workspace.yaml",
            Level::App => "app.yaml",
        }
    }

    /// Options this level's file ignores (they belong to a lower layer's process).
    fn ignored(self) -> &'static [&'static str] {
        match self {
            Level::Application => &[],
            Level::Profile => crate::config::APP_ONLY,
            Level::Workspace | Level::App => crate::config::WORKSPACE_IGNORED,
        }
    }
}

// ---- the template tree --------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Node {
    /// Comment and blank lines before the key, as written.
    lead: Vec<String>,
    indent: usize,
    /// The key as written in the template (quotes kept).
    key: String,
    /// The value on the key's line, without its trailing comment.
    inline: String,
    /// The trailing `# ...` comment on the key's line.
    trailing: Option<String>,
    /// Nested options, when the key holds a map written one key per line.
    children: Vec<Node>,
    /// Further lines of a value that is not a plain map (a list, a multi-line flow map).
    block: Vec<String>,
}

impl Node {
    fn is_map(&self) -> bool {
        self.inline.is_empty() && self.block.is_empty() && !self.children.is_empty()
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Split `text` at a comment: a `#` outside quotes that starts the text or follows a space.
pub(crate) fn split_comment(text: &str) -> (&str, Option<&str>) {
    let (mut single, mut double) = (false, false);
    let mut prev = ' ';
    for (i, c) in text.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single && prev != '\\' => double = !double,
            '#' if !single && !double && prev.is_whitespace() => return (text[..i].trim_end(), Some(text[i..].trim_end())),
            _ => {}
        }
        prev = c;
    }
    (text.trim_end(), None)
}

/// `key:` or `"key with: colon":` at the start of `trimmed`: the key as written and the rest of the line.
fn split_key(trimmed: &str) -> Option<(&str, &str)> {
    if trimmed.starts_with('#') || trimmed.starts_with("- ") || trimmed == "-" {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')? + 1;
        let after = trimmed[end + 1..].strip_prefix(':')?;
        return Some((&trimmed[..end + 1], after));
    }
    let colon = trimmed.find(':')?;
    let (key, after) = (&trimmed[..colon], &trimmed[colon + 1..]);
    if key.is_empty() || key.contains(' ') || !(after.is_empty() || after.starts_with(' ')) {
        return None;
    }
    Some((key, after))
}

fn parse_nodes(lines: &[&str], i: &mut usize, indent: usize) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut lead: Vec<String> = Vec::new();
    while *i < lines.len() {
        let line = lines[*i];
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lead.push(line.to_owned());
            *i += 1;
            continue;
        }
        let ind = indent_of(line);
        if ind < indent {
            break;
        }
        let Some((key, after)) = split_key(trimmed).filter(|_| ind == indent) else {
            // Something this reader does not understand at this level: keep it with the previous node.
            if let Some(last) = nodes.last_mut() {
                let last: &mut Node = last;
                last.block.append(&mut lead);
                last.block.push(line.to_owned());
            }
            *i += 1;
            continue;
        };
        *i += 1;
        let (inline, trailing) = split_comment(after.trim_start());
        let mut node = Node {
            lead: std::mem::take(&mut lead),
            indent,
            key: key.to_owned(),
            inline: inline.to_owned(),
            trailing: trailing.map(str::to_owned),
            children: Vec::new(),
            block: Vec::new(),
        };
        // What follows more deeply indented belongs to this key.
        let start = *i;
        let mut j = *i;
        while j < lines.len() && (lines[j].trim().is_empty() || indent_of(lines[j]) > indent) {
            j += 1;
        }
        // Trailing blank lines belong to whatever comes next.
        while j > start && lines[j - 1].trim().is_empty() {
            j -= 1;
        }
        if node.inline.is_empty() && j > start {
            let first = lines[start..j].iter().find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).copied().unwrap_or("");
            let child_indent = indent_of(first);
            if split_key(first.trim_start()).is_some() && !first.trim_start().starts_with('-') {
                let mut k = start;
                node.children = parse_nodes(&lines[..j], &mut k, child_indent);
                // Comments after the last child stay with the block.
                node.block = lines[k..j].iter().map(|s| (*s).to_owned()).collect();
                *i = j;
                nodes.push(node);
                continue;
            }
        }
        node.block = lines[start..j].iter().map(|s| (*s).to_owned()).collect();
        *i = j;
        nodes.push(node);
    }
    // Comments at the end of the block go with the last node.
    if !lead.is_empty() {
        if let Some(last) = nodes.last_mut() {
            last.block.append(&mut lead);
        }
    }
    nodes
}

fn parse_template(text: &str) -> Vec<Node> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    parse_nodes(&lines, &mut i, 0)
}

fn bare(key: &str) -> &str {
    key.trim_matches('"').trim_end_matches('+')
}

/// Remove the option at a dotted path from a tree.
fn remove_path(nodes: &mut Vec<Node>, path: &str) {
    match path.split_once('.') {
        Some((head, rest)) => {
            if let Some(n) = nodes.iter_mut().find(|n| bare(&n.key) == head) {
                remove_path(&mut n.children, rest);
            }
        }
        None => nodes.retain(|n| bare(&n.key) != path),
    }
}

/// Add what `over` has that `base` lacks. Where both have a value, `over_wins` decides whose it is.
fn merge(base: &mut Vec<Node>, over: Vec<Node>, over_wins: bool) {
    for o in over {
        match base.iter_mut().find(|b| bare(&b.key) == bare(&o.key)) {
            Some(b) if b.is_map() && o.is_map() => merge(&mut b.children, o.children, over_wins),
            Some(b) => {
                if over_wins {
                    *b = o;
                }
            }
            None => base.push(o),
        }
    }
}

fn template_for(level: Level) -> Vec<Node> {
    let mut app = parse_template(APP_TEMPLATE);
    for path in level.ignored() {
        remove_path(&mut app, path);
    }
    match level {
        Level::Application => app,
        Level::Profile => {
            let mut nodes = parse_template(PROFILE_TEMPLATE);
            merge(&mut nodes, app, true);
            nodes
        }
        Level::Workspace => {
            let mut nodes = parse_template(WORKSPACE_TEMPLATE);
            merge(&mut nodes, app, true);
            nodes
        }
        Level::App => {
            let mut nodes = parse_template(WEBAPP_TEMPLATE);
            merge(&mut nodes, app, true);
            nodes
        }
    }
}

// ---- rendering ----------------------------------------------------------------------------------------

fn lookup<'a>(user: Option<&'a Value>, key: &str) -> Option<(String, &'a Value)> {
    let map = user?.as_mapping()?;
    for k in [key.to_owned(), format!("{key}+")] {
        if let Some(v) = map.get(k.as_str()) {
            return Some((k, v));
        }
    }
    None
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_else(|_| format!("{s:?}")),
        _ => String::new(),
    }
}

/// The value on one line when it is short enough to read that way.
fn flow(v: &Value) -> Option<String> {
    match v {
        Value::Sequence(items) if items.iter().all(|i| !matches!(i, Value::Sequence(_) | Value::Mapping(_))) => {
            let s = format!("[{}]", items.iter().map(scalar).collect::<Vec<_>>().join(", "));
            (s.len() <= 80).then_some(s)
        }
        Value::Mapping(m) if m.is_empty() => Some("{}".into()),
        Value::Mapping(_) | Value::Sequence(_) => None,
        v => Some(scalar(v)),
    }
}

/// `key: value`, or `key:` and an indented block, written at `indent`.
fn emit_value(out: &mut Vec<String>, indent: usize, key: &str, v: &Value, trailing: Option<&str>) {
    let pad = " ".repeat(indent);
    let comment = trailing.map(|c| format!("  {c}")).unwrap_or_default();
    if let Some(one) = flow(v) {
        out.push(format!("{pad}{key}: {one}{comment}"));
        return;
    }
    out.push(format!("{pad}{key}:{comment}"));
    let body = serde_yaml_ng::to_string(v).unwrap_or_default();
    for l in body.lines() {
        out.push(format!("{pad}{}{l}", "  "));
    }
}

/// Switch a line off. `col0`: the `# ` goes at the start of the line (inside something already switched off, so the
/// whole block uncomments by removing `# ` and keeps its indentation); otherwise at the option's own indent.
fn comment_out(line: &str, indent: usize, col0: bool) -> String {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return line.to_owned();
    }
    if col0 {
        return format!("# {line}");
    }
    let at = indent.min(indent_of(line));
    format!("{}# {}", " ".repeat(at), &line[at..])
}

fn render(nodes: &[Node], user: Option<&Value>, out: &mut Vec<String>, used: &mut Vec<String>, col0: bool) {
    for n in nodes {
        out.extend(n.lead.iter().cloned());
        let key = bare(&n.key);
        let found = lookup(user, key);
        let pad = " ".repeat(n.indent);
        match found {
            Some((actual, v)) => {
                used.push(actual.clone());
                let shown_key = if actual.ends_with('+') { actual.clone() } else { n.key.clone() };
                match (n.is_map(), v) {
                    (true, Value::Mapping(m)) if !m.is_empty() => {
                        out.push(format!("{pad}{shown_key}:{}", n.trailing.as_deref().map(|c| format!("  {c}")).unwrap_or_default()));
                        let mut inner = Vec::new();
                        render(&n.children, Some(v), &mut inner, &mut Vec::new(), false);
                        // Keys the template does not know are kept, after the known ones.
                        let known: Vec<String> = n.children.iter().flat_map(|c| [bare(&c.key).to_owned(), format!("{}+", bare(&c.key))]).collect();
                        out.append(&mut inner);
                        for (k, val) in m {
                            let k = k.as_str().unwrap_or("");
                            if !known.iter().any(|x| x == k) {
                                emit_value(out, n.indent + 2, k, val, None);
                            }
                        }
                        out.extend(n.block.iter().cloned());
                    }
                    _ => {
                        emit_value(out, n.indent, &shown_key, v, n.trailing.as_deref());
                    }
                }
            }
            None => {
                let comment = n.trailing.as_deref().map(|c| format!("  {c}")).unwrap_or_default();
                let head = |text: String| if col0 { format!("# {pad}{text}") } else { format!("{pad}# {text}") };
                if n.is_map() {
                    out.push(head(format!("{}:{comment}", n.key)));
                    render(&n.children, None, out, &mut Vec::new(), true);
                    out.extend(n.block.iter().map(|l| comment_out(l, n.indent, true)));
                } else {
                    let value = if n.inline.is_empty() { String::new() } else { format!(" {}", n.inline) };
                    out.push(head(format!("{}:{value}{comment}", n.key)));
                    out.extend(n.block.iter().map(|l| comment_out(l, n.indent, col0)));
                }
            }
        }
    }
}

/// The "all options" text for `level`, with what `existing` (the real file's text, if there is one) sets.
/// `title` names the file in the header (a profile or workspace name).
pub fn generate(level: Level, title: &str, existing: Option<&str>) -> Result<String, String> {
    let user: Option<Value> = match existing.map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => Some(serde_yaml_ng::from_str(text).map_err(|e| format!("the file cannot be read: {e}"))?),
        None => None,
    };
    if let Some(v) = &user {
        if !v.is_mapping() && !v.is_null() {
            return Err("the file is not a map of options".into());
        }
    }
    let nodes = template_for(level);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# {} configuration ({}): every option, explained.", level.label(), title));
    lines.push(format!("# Generated by browsah as a copy of {}. Lines that start with \"# key:\" are options this file does", level.file_name()));
    lines.push("# not set (the value shown is the documented example, usually the default); remove the \"# \" to set one.".into());
    lines.push("# Lines without it are what the file sets today. Comments you wrote in the file are not carried over.".into());
    lines.push("# Documentation: Main menu > Documentation, or docs/config/README.md.".into());
    lines.push(String::new());
    let mut used = Vec::new();
    render(&nodes, user.as_ref(), &mut lines, &mut used, false);
    // Top-level options the template does not have.
    if let Some(m) = user.as_ref().and_then(Value::as_mapping) {
        let known: Vec<String> = nodes.iter().flat_map(|c| [bare(&c.key).to_owned(), format!("{}+", bare(&c.key))]).collect();
        let extra: Vec<(&Value, &Value)> = m.iter().filter(|(k, _)| !known.iter().any(|x| Some(x.as_str()) == k.as_str())).collect();
        if !extra.is_empty() {
            lines.push(String::new());
            lines.push("# Other settings in the file:".into());
            for (k, v) in extra {
                emit_value(&mut lines, 0, k.as_str().unwrap_or(""), v, None);
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn nothing_set_gives_a_file_with_no_settings() {
        for level in [Level::Application, Level::Profile, Level::Workspace, Level::App] {
            let text = generate(level, "x", None).unwrap();
            assert!(value(&text).is_null(), "{level:?} must set nothing:\n{text}");
            assert!(text.contains("# window:") || level != Level::Application, "{text}");
            assert!(text.contains("rendering"), "{level:?}");
        }
    }

    /// Showing an example file as if it were the real one must keep every value it sets.
    #[test]
    fn what_a_file_sets_is_kept_exactly() {
        for (level, file) in [
            (Level::Application, "examples/config.yaml"),
            (Level::Profile, "examples/profile.yaml"),
            (Level::Workspace, "examples/workspace.yaml"),
            (Level::App, "examples/webapp.yaml"),
        ] {
            let original = std::fs::read_to_string(root().join(file)).unwrap();
            let text = generate(level, "x", Some(&original)).unwrap();
            assert_eq!(value(&text), value(&original), "{file} changed by the expansion:\n{text}");
        }
    }

    #[test]
    fn unset_options_are_shown_with_their_comments_and_set_ones_are_live() {
        let text = generate(Level::Profile, "Work", Some("name: Work\nrendering:\n  frame_rate: 30\ninput: { wheel_notch: 80 }\nchromium:\n  flags+: ['--a']\nunknown_thing: 1\n")).unwrap();
        let v = value(&text);
        assert_eq!(v["name"].as_str(), Some("Work"));
        assert_eq!(v["rendering"]["frame_rate"].as_i64(), Some(30));
        assert_eq!(v["input"]["wheel_notch"].as_i64(), Some(80));
        assert_eq!(v["chromium"]["flags+"][0].as_str(), Some("--a"));
        assert_eq!(v["unknown_thing"].as_i64(), Some(1));
        assert!(text.contains("# search_url:"), "an unset option is shown commented out:\n{text}");
        assert!(text.contains("max fps") || text.contains("frame rate") || text.contains("fps"), "{text}");
        assert!(!text.contains("# frame_rate:"), "a set option is not also shown commented out");
        assert!(!text.contains("# watch_config"), "application-only options are not offered in a profile");
    }

    #[test]
    fn a_workspace_is_not_offered_what_it_ignores() {
        let text = generate(Level::Workspace, "Mail", None).unwrap();
        for ignored in ["# watch_config:", "# default_profile:", "# chromium:", "# gpu:", "# nav:", "# sweep:", "# app:", "# url:"] {
            let shown = text.contains(ignored);
            assert_eq!(shown, matches!(ignored, "# nav:" | "# sweep:"), "{ignored}:\n{text}");
        }
        assert!(!text.contains("device_resolution") && !text.contains("# mode:"), "{text}");
        // A web app's file offers its own keys (including `nav`, used with `display: custom`), the layered options such as `sweep` and
        // `sleep_tabs_after`, and none of a workspace's tab rules.
        let app = generate(Level::App, "Mail", None).unwrap();
        for (key, shown) in [("# url:", true), ("# multiple_windows:", true), ("# storage:", true), ("# nav:", true), ("# tabs:", false), ("# sweep:", true), ("# sleep_tabs_after:", true), ("# gpu:", false)] {
            assert_eq!(app.contains(key), shown, "{key}:\n{app}");
        }
    }

    /// The expansion of a real file must still load with the real types.
    #[test]
    fn the_result_loads_with_the_real_types() {
        let app = generate(Level::Application, "x", Some("search_url: 'https://a/?q={query}'\nwindow: { frame: { thickness: 6 } }\ncookies:\n  rules:\n    - match: [a.example]\n      action: deny\n")).unwrap();
        let c = crate::config::Config::parse(&app).unwrap();
        assert_eq!((c.search_url.as_str(), c.window.frame.thickness, c.cookies.rules.len()), ("https://a/?q={query}", 6, 1));
        let ws = generate(Level::Workspace, "x", Some("name: Mail\nnav: { back: false }\ntabs:\n  pinned:\n    - url: https://a.example/\n")).unwrap();
        let w = crate::workspace::parse_config(&ws).unwrap();
        assert!(!w.nav.back && w.tabs.pinned.len() == 1 && w.name.as_deref() == Some("Mail"));
        let p = generate(Level::Profile, "x", Some("name: Work\ndefault_workspace: mail\n")).unwrap();
        assert_eq!(crate::profile::parse_config(&p).unwrap().name.as_deref(), Some("Work"));
    }

    #[test]
    fn comment_splitting_respects_quotes() {
        assert_eq!(split_comment("a  # c"), ("a", Some("# c")));
        assert_eq!(split_comment("\"#4a2f80\"  # c"), ("\"#4a2f80\"", Some("# c")));
        assert_eq!(split_comment("'a # b'"), ("'a # b'", None));
        assert_eq!(split_comment("[\"#a\", \"#b\"]"), ("[\"#a\", \"#b\"]", None));
    }

    #[test]
    fn a_broken_file_is_reported_not_replaced() {
        assert!(generate(Level::Application, "x", Some("window: [")).is_err());
        assert!(generate(Level::Application, "x", Some("- a\n- b\n")).is_err());
    }
}
