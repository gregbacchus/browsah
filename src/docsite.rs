//! The documentation as a small static site. The markdown under `docs/`, the README and the example files are compiled
//! into the binary (`build.rs` lists them); `write_site` renders them to HTML in a folder, which the "Documentation"
//! window then opens like any other page. Pure (no GTK, no CEF), so it is tested.

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/docs_files.rs"));

const GITHUB: &str = "https://github.com/gregbacchus/browsah/blob/main/";

/// The pages of the "Configuration" group, first; the rest of `docs/config/` follows alphabetically.
const CONFIG_ORDER: &[&str] = &["README", "value-types", "options", "profile", "workspace", "webapp", "routing", "permissions", "history", "sweep", "selector"];

struct Page {
    /// Path of the source, relative to the repository (`docs/config/options.md`).
    src: &'static str,
    title: String,
    body: String,
    /// `(level, text, id)` of the headings.
    headings: Vec<(u8, String, String)>,
    #[cfg_attr(not(test), allow(dead_code))]
    /// Links to other pages as written: `(target path, fragment)`; only those that resolved or should have.
    links: Vec<(String, String)>,
}

/// Where a source file's rendering goes, relative to the site folder.
fn out_path(src: &str) -> String {
    match src.strip_suffix(".md") {
        Some(base) => format!("{base}.html"),
        None => format!("{src}.html"),
    }
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `docs/config` + `../README.md` = `docs/README.md`. `None` when it leaves the repository.
fn join(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// A link from page `from` to `to`, relative, as the browser needs it in the rendered tree.
fn relative(from: &str, to: &str) -> String {
    let depth = dir_of(from).split('/').filter(|p| !p.is_empty()).count();
    format!("{}{}", "../".repeat(depth), to)
}

/// The anchor GitHub gives a heading (the docs were written against it): lower case, spaces to hyphens, punctuation gone.
pub fn slug(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            ' ' | '-' => Some('-'),
            '_' => Some('_'),
            _ => None,
        })
        .collect()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn render_markdown(src: &'static str, text: &str, known: &HashSet<&str>) -> Page {
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    let events: Vec<Event> = parser.collect();
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    let (mut headings, mut links) = (Vec::new(), Vec::new());
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut title = String::new();
    let dir = dir_of(src);
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Heading { level, .. }) => {
                let n = *level as u8;
                let mut j = i + 1;
                let mut plain = String::new();
                while j < events.len() && !matches!(events[j], Event::End(TagEnd::Heading(_))) {
                    if let Event::Text(t) | Event::Code(t) = &events[j] {
                        plain.push_str(t);
                    }
                    j += 1;
                }
                let base = slug(&plain);
                let count = used.entry(base.clone()).or_insert(0);
                let id = if *count == 0 { base.clone() } else { format!("{base}-{count}") };
                *count += 1;
                if n == 1 && title.is_empty() {
                    title = plain.clone();
                }
                headings.push((n, plain, id.clone()));
                out.push(Event::Html(format!("<h{n} id=\"{id}\">").into()));
                out.extend(events[i + 1..j].iter().cloned());
                out.push(Event::Html(format!("</h{n}>\n").into()));
                i = j + 1;
                continue;
            }
            Event::Start(Tag::Link { link_type, dest_url, title: t, id }) => {
                let dest = dest_url.to_string();
                let new = if dest.contains("://") || dest.starts_with("mailto:") || dest.starts_with('#') {
                    dest
                } else {
                    let (path, frag) = dest.split_once('#').map_or((dest.as_str(), ""), |(p, f)| (p, f));
                    match join(dir, path) {
                        Some(target) if known.contains(target.as_str()) => {
                            links.push((target.clone(), frag.to_owned()));
                            let rel = relative(src, &out_path(&target));
                            if frag.is_empty() { rel } else { format!("{rel}#{frag}") }
                        }
                        // The folder's own page: `config/` and the like.
                        Some(target) if known.contains(format!("{target}/README.md").as_str()) => {
                            let t = format!("{target}/README.md");
                            links.push((t.clone(), frag.to_owned()));
                            let rel = relative(src, &out_path(&t));
                            if frag.is_empty() { rel } else { format!("{rel}#{frag}") }
                        }
                        // Something outside the documentation (source code, scripts): the repository's copy.
                        Some(target) if !target.is_empty() => {
                            links.push((target.clone(), String::new()));
                            format!("{GITHUB}{target}")
                        }
                        _ => dest.clone(),
                    }
                };
                out.push(Event::Start(Tag::Link { link_type: *link_type, dest_url: CowStr::from(new), title: t.clone(), id: id.clone() }));
            }
            e => out.push(e.clone()),
        }
        i += 1;
    }
    let mut body = String::new();
    html::push_html(&mut body, out.into_iter());
    if title.is_empty() {
        title = src.rsplit('/').next().unwrap_or(src).trim_end_matches(".md").to_owned();
    }
    Page { src, title, body, headings, links }
}

fn render_example(src: &'static str, text: &str) -> Page {
    let name = src.rsplit('/').next().unwrap_or(src);
    Page {
        src,
        title: name.to_owned(),
        body: format!("<h1 id=\"{}\">{}</h1>\n<p>The file as it is in the repository's <code>{}</code>.</p>\n<pre><code>{}</code></pre>\n", slug(name), escape(src), escape(dir_of(src)), escape(text)),
        headings: Vec::new(),
        links: Vec::new(),
    }
}

fn nav(pages: &[Page], current: &str) -> String {
    let group = |name: &str, srcs: Vec<&Page>, current: &str, out: &mut String| {
        if srcs.is_empty() {
            return;
        }
        out.push_str(&format!("<h2>{}</h2>\n<ul>\n", escape(name)));
        for p in srcs {
            let class = if p.src == current { " class=\"current\"" } else { "" };
            out.push_str(&format!("<li><a{class} href=\"{}\">{}</a>", relative(current, &out_path(p.src)), escape(&p.title)));
            if p.src == current {
                let subs: Vec<_> = p.headings.iter().filter(|h| h.0 == 2).collect();
                if !subs.is_empty() {
                    out.push_str("<ul>");
                    for (_, text, id) in subs {
                        out.push_str(&format!("<li><a href=\"#{id}\">{}</a></li>", escape(text)));
                    }
                    out.push_str("</ul>");
                }
            }
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n");
    };
    let by = |prefix: &str| -> Vec<&Page> { pages.iter().filter(|p| p.src.starts_with(prefix)).collect() };
    let find = |src: &str| pages.iter().find(|p| p.src == src);
    let mut out = String::from("<a class=\"brand\" href=\"");
    out.push_str(&relative(current, "index.html"));
    out.push_str("\">browsah</a>\n");
    group("Start", ["docs/README.md", "README.md"].iter().filter_map(|s| find(s)).collect(), current, &mut out);
    let mut config = by("docs/config/");
    config.sort_by_key(|p| {
        let stem = p.src.rsplit('/').next().unwrap_or("").trim_end_matches(".md");
        (CONFIG_ORDER.iter().position(|c| *c == stem).unwrap_or(usize::MAX), p.src)
    });
    group("Configuration", config, current, &mut out);
    group("Examples", by("examples/"), current, &mut out);
    let mut other: Vec<&Page> = by("docs/research/");
    other.extend(find("docs/roadmap.md"));
    group("Research and plans", other, current, &mut out);
    out
}

const CSS: &str = r#"
:root { color-scheme: light dark; --bg: #fff; --fg: #1d2126; --dim: #5d6670; --line: #d9dde2; --side: #f4f6f8; --accent: #2a6f97; --code: #eef1f4; }
@media (prefers-color-scheme: dark) { :root { --bg: #16191d; --fg: #dde1e6; --dim: #9aa3ad; --line: #2c3238; --side: #1c2024; --accent: #6db3d9; --code: #23282d; } }
* { box-sizing: border-box; }
body { margin: 0; display: flex; min-height: 100vh; background: var(--bg); color: var(--fg); font: 16px/1.6 system-ui, "Segoe UI", sans-serif; }
nav { flex: 0 0 270px; position: sticky; top: 0; align-self: flex-start; height: 100vh; overflow-y: auto; padding: 20px 16px 40px; background: var(--side); border-right: 1px solid var(--line); font-size: 14px; }
nav .brand { display: block; font-weight: 700; font-size: 20px; color: var(--fg); text-decoration: none; margin-bottom: 8px; }
nav h2 { margin: 18px 0 4px; font-size: 11px; letter-spacing: .08em; text-transform: uppercase; color: var(--dim); }
nav ul { list-style: none; margin: 0; padding: 0; }
nav li > ul { margin: 2px 0 6px 12px; border-left: 1px solid var(--line); padding-left: 10px; font-size: 13px; }
nav a { display: block; padding: 2px 0; color: var(--fg); text-decoration: none; }
nav a:hover { color: var(--accent); }
nav a.current { color: var(--accent); font-weight: 600; }
main { flex: 1; min-width: 0; max-width: 920px; padding: 28px 40px 80px; }
h1 { font-size: 2rem; margin-top: 0; } h2 { margin-top: 2.2rem; padding-bottom: 4px; border-bottom: 1px solid var(--line); } h3 { margin-top: 1.6rem; }
a { color: var(--accent); }
code { font: 0.9em ui-monospace, "SF Mono", Menlo, Consolas, monospace; background: var(--code); padding: 1px 5px; border-radius: 4px; }
pre { background: var(--code); padding: 12px 16px; border-radius: 8px; overflow-x: auto; line-height: 1.45; }
pre code { background: none; padding: 0; }
table { border-collapse: collapse; display: block; overflow-x: auto; margin: 1em 0; }
th, td { border: 1px solid var(--line); padding: 6px 10px; text-align: left; vertical-align: top; }
th { background: var(--side); }
blockquote { margin: 1em 0; padding: 0 1em; border-left: 4px solid var(--line); color: var(--dim); }
@media (max-width: 760px) { body { display: block; } nav { position: static; height: auto; } main { padding: 20px 16px 60px; } }
"#;

fn page_html(pages: &[Page], p: &Page) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{CSS}</style></head>\n<body>\n<nav>\n{}</nav>\n<main>\n{}</main>\n</body></html>\n",
        escape(&if p.title.to_lowercase().contains("documentation") { p.title.clone() } else { format!("{} \u{b7} browsah documentation", p.title) }),
        nav(pages, p.src),
        p.body
    )
}

fn pages() -> Vec<Page> {
    let known: HashSet<&str> = FILES.iter().map(|(p, _)| *p).collect();
    FILES.iter().map(|(src, text)| if src.ends_with(".md") { render_markdown(src, text, &known) } else { render_example(src, text) }).collect()
}

/// Render the whole site into `dir` (over what is there: a window may be showing it) and return the page to open first.
pub fn write_site(dir: &Path) -> std::io::Result<PathBuf> {
    let pages = pages();
    for p in &pages {
        let file = dir.join(out_path(p.src));
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, page_html(&pages, p))?;
    }
    let index = dir.join("index.html");
    std::fs::write(&index, "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"0; url=docs/README.html\"><title>browsah documentation</title><a href=\"docs/README.html\">Documentation</a>\n")?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_get_the_anchors_the_docs_use() {
        assert_eq!(slug("Layers and reloading"), "layers-and-reloading");
        assert_eq!(slug("Links that go through a redirector"), "links-that-go-through-a-redirector");
        assert_eq!(slug("`window`"), "window");
        assert_eq!(slug("Startup and search"), "startup-and-search");
        assert_eq!(slug("Colours & more (new)"), "colours--more-new");
    }

    #[test]
    fn links_are_resolved_inside_the_tree() {
        assert_eq!(join("docs/config", "../README.md").as_deref(), Some("docs/README.md"));
        assert_eq!(join("docs/config", "options.md").as_deref(), Some("docs/config/options.md"));
        assert_eq!(join("", "docs/config/README.md").as_deref(), Some("docs/config/README.md"));
        assert_eq!(join("docs", "../../x"), None);
        assert_eq!(relative("docs/config/a.md", "docs/README.html"), "../../docs/README.html");
        assert_eq!(relative("README.md", "docs/README.html"), "docs/README.html");
        assert_eq!(out_path("examples/config.yaml"), "examples/config.yaml.html");
        assert_eq!(out_path("docs/config/options.md"), "docs/config/options.html");
    }

    #[test]
    fn the_docs_are_all_in_the_site_and_their_links_work() {
        let ps = pages();
        assert!(ps.len() >= 25, "{}", ps.len());
        for must in ["docs/README.md", "docs/config/options.md", "docs/config/value-types.md", "examples/config.yaml", "README.md"] {
            assert!(ps.iter().any(|p| p.src == must), "{must} is not in the site");
        }
        let by: HashMap<&str, &Page> = ps.iter().map(|p| (p.src, p)).collect();
        let mut bad = Vec::new();
        for p in &ps {
            assert!(!p.title.is_empty() && !p.body.is_empty(), "{}", p.src);
            for (target, frag) in &p.links {
                // Links into the documentation must hit a page and, with a fragment, one of its headings.
                let in_docs = target.starts_with("docs/") || target.starts_with("examples/") || target == "README.md";
                match by.get(target.as_str()) {
                    Some(t) if !frag.is_empty() && !t.headings.iter().any(|h| &h.2 == frag) => bad.push(format!("{} -> {target}#{frag}: no such heading", p.src)),
                    None if in_docs => bad.push(format!("{} -> {target}: no such page", p.src)),
                    _ => {}
                }
            }
        }
        assert!(bad.is_empty(), "broken links in the documentation: {bad:#?}");
    }

    #[test]
    fn a_page_renders_tables_code_anchors_and_navigation() {
        let dir = std::env::temp_dir().join(format!("browsah-docsite-{}", std::process::id()));
        let index = write_site(&dir).unwrap();
        assert!(index.exists());
        let options = std::fs::read_to_string(dir.join("docs/config/options.html")).unwrap();
        assert!(options.contains("<table>") && options.contains("<code>search_url</code>"), "tables and inline code");
        assert!(options.contains("id=\"startup-and-search\""), "headings carry ids");
        assert!(options.contains("href=\"../../docs/config/profile.html\"") || options.contains("profile.html"), "links point at rendered pages");
        assert!(options.contains("class=\"current\""), "the navigation marks the page");
        assert!(!options.contains(".md\""), "no link is left pointing at a markdown file");
        let ex = std::fs::read_to_string(dir.join("examples/config.yaml.html")).unwrap();
        assert!(ex.contains("<pre><code>") && ex.contains("search_url"), "examples are shown as code");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
