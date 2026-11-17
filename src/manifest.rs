//! What a page says about itself as an app (its web app manifest), and how to read it two ways: from what
//! Chromium's DevTools answers (`Page.getAppManifest`, used when installing from a tab) and from the manifest file
//! itself (used by `browsah app add`, which has no browser). Both give the same `AppInfo`, so an app made either way
//! comes out the same. Pure and tested; fetching is in `webapp.rs`, the install dialog in `app/install.rs`.

use crate::webapp::AppDisplay;

/// What a page says about itself as an app.
#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    pub name: String,
    pub start_url: String,
    /// Scope patterns for `scope` (empty: the start page's site).
    pub scope: Vec<String>,
    pub display: AppDisplay,
    /// `#rrggbb`, from the manifest's theme colour.
    pub color: Option<String>,
    /// The manifest's `id`, resolved.
    pub manifest_id: Option<String>,
    /// Icon candidates, best first.
    pub icons: Vec<String>,
    /// Whether a manifest was found (otherwise this is just the page).
    pub from_manifest: bool,
}

/// `rgba(51,102,153,1)` to `#336699`.
pub fn rgba_to_hex(text: &str) -> Option<String> {
    let inner = text.trim().strip_prefix("rgba(")?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(|p| p.trim().parse::<f64>().ok());
    let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
    Some(format!("#{:02x}{:02x}{:02x}", r.round().clamp(0.0, 255.0) as u8, g.round().clamp(0.0, 255.0) as u8, b.round().clamp(0.0, 255.0) as u8))
}

/// A CSS colour written as `#rgb` or `#rrggbb` (what a manifest's `theme_color` usually is) as `#rrggbb`.
pub fn hex_colour(text: &str) -> Option<String> {
    let h = text.trim().strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => Some(format!("#{}", h.to_ascii_lowercase())),
        3 => Some(format!("#{}", h.chars().flat_map(|c| [c, c]).collect::<String>().to_ascii_lowercase())),
        _ => None,
    }
}

/// A scope address as a pattern for `scope`: the site, plus the path when it is not the root.
pub fn scope_pattern(scope_url: &str) -> Option<String> {
    let rest = scope_url.split("://").nth(1)?;
    let (host_port, path) = rest.split_once('/').map(|(h, p)| (h, p.split(['?', '#']).next().unwrap_or(""))).unwrap_or((rest, ""));
    let host = host_port.rsplit('@').next()?.split(':').next()?.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(if path.is_empty() { host } else { format!("{host}/{path}*") })
}

/// The size an icon candidate claims (`192x192` is 192; `any` and `0x0` are vector or unknown: large).
pub fn icon_size(sizes: &str) -> u32 {
    sizes.split_whitespace().filter_map(|s| s.split_once('x').and_then(|(w, h)| Some(w.parse::<u32>().ok()?.min(h.parse::<u32>().ok()?)))).map(|n| if n == 0 { 1024 } else { n }).max().unwrap_or(1024)
}

/// Read what `Page.getAppManifest` answered (`json`): `data` is the manifest as written, `manifest` the parsed
/// one with every address resolved. Icons marked only `maskable` or `monochrome` are not used (GNOME does not mask).
pub fn parse_manifest(json: &str) -> Option<AppInfo> {
    let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(json).ok()?;
    let m = v.get("manifest")?;
    let text = |val: &serde_yaml_ng::Value, key: &str| val.get(key).and_then(|x| x.as_str()).map(str::to_owned).filter(|s| !s.trim().is_empty());
    let raw: Option<serde_yaml_ng::Value> = v.get("data").and_then(|d| d.as_str()).and_then(|d| serde_yaml_ng::from_str(d).ok());
    let start_url = text(m, "startUrl")?;
    let name = text(m, "name").or_else(|| raw.as_ref().and_then(|r| text(r, "name"))).or_else(|| raw.as_ref().and_then(|r| text(r, "short_name")))?;
    let display = match text(m, "display").as_deref() {
        Some("kMinimalUi") => AppDisplay::Minimal,
        Some("kBrowser") => AppDisplay::Browser,
        _ => AppDisplay::Standalone,
    };
    // Icons: the parsed list has the resolved addresses, the raw one the purposes, in the same order.
    let raw_icons = raw.as_ref().and_then(|r| r.get("icons")).and_then(|i| i.as_sequence()).cloned().unwrap_or_default();
    let mut icons: Vec<(u32, String)> = Vec::new();
    for (i, icon) in m.get("icons").and_then(|i| i.as_sequence()).cloned().unwrap_or_default().iter().enumerate() {
        let Some(url) = text(icon, "url") else { continue };
        let purpose = raw_icons.get(i).and_then(|r| text(r, "purpose")).unwrap_or_else(|| "any".into());
        if !purpose.split_whitespace().any(|p| p == "any") {
            continue;
        }
        icons.push((icon_size(&text(icon, "sizes").unwrap_or_default()), url));
    }
    icons.sort_by(|a, b| b.0.cmp(&a.0));
    Some(AppInfo {
        name,
        scope: text(m, "scope").and_then(|s| scope_pattern(&s)).into_iter().collect(),
        start_url,
        display,
        color: text(m, "themeColor").and_then(|c| rgba_to_hex(&c)),
        manifest_id: text(m, "id"),
        icons: icons.into_iter().map(|(_, u)| u).collect(),
        from_manifest: true,
    })
}

/// What to say about a page that has no manifest: its title and address.
pub fn page_info(title: &str, address: &str) -> AppInfo {
    let name = if title.trim().is_empty() { crate::cookies::host_of(address) } else { title.trim().to_owned() };
    AppInfo { name, start_url: address.to_owned(), scope: Vec::new(), display: AppDisplay::Standalone, color: None, manifest_id: None, icons: Vec::new(), from_manifest: false }
}

// ---- the manifest file itself -------------------------------------------------------------------------

/// `rel` resolved against `base` (an absolute http or https address), as a browser resolves a link.
pub fn resolve(base: &str, rel: &str) -> Option<String> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Some(base.split('#').next()?.to_owned());
    }
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return Some(rel.to_owned());
    }
    let (scheme, rest) = base.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    if let Some(r) = rel.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    let (authority, path_query) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".to_owned()));
    let origin = format!("{scheme}://{authority}");
    let path_query = path_query.split('#').next().unwrap_or("");
    let (path, query) = path_query.split_once('?').map(|(p, q)| (p, Some(q))).unwrap_or((path_query, None));
    if let Some(q) = rel.strip_prefix('?') {
        return Some(format!("{origin}{path}?{}", q.split('#').next().unwrap_or("")));
    }
    if rel.starts_with('#') {
        return Some(format!("{origin}{path}{}", query.map(|q| format!("?{q}")).unwrap_or_default()));
    }
    let (rel_path, rel_rest) = match rel.find(['?', '#']) {
        Some(i) => (&rel[..i], &rel[i..]),
        None => (rel, ""),
    };
    let rel_rest = rel_rest.split('#').next().unwrap_or("");
    let joined = if rel_path.starts_with('/') { rel_path.to_owned() } else { format!("{}{}", &path[..path.rfind('/').map_or(0, |i| i + 1)], rel_path) };
    let mut segments: Vec<&str> = Vec::new();
    let parts: Vec<&str> = joined.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        match *part {
            "." => {}
            ".." => {
                segments.pop();
            }
            "" if i == 0 => {}
            p => segments.push(p),
        }
    }
    let mut out = format!("{origin}/{}", segments.join("/"));
    if matches!(parts.last(), Some(&"") | Some(&".") | Some(&"..")) && !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(rel_rest);
    Some(out)
}

/// The attributes of the first tag in `tag` (the text between `<` and `>`), names lower case.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = tag.as_bytes();
    let mut i = tag.find(char::is_whitespace).unwrap_or(tag.len());
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = tag[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                i += 1;
                let s = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                value = tag[s..i].to_owned();
                i += 1;
            } else {
                let s = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = tag[s..i].to_owned();
            }
        }
        out.push((name, value));
    }
    out
}

/// A page's own words: the address of its manifest (a `<link rel="manifest">`, resolved against `page_url`) and its title.
pub fn page_links(html: &str, page_url: &str) -> (Option<String>, Option<String>) {
    let lower = html.to_ascii_lowercase();
    let mut manifest = None;
    let mut from = 0;
    while let Some(i) = lower[from..].find("<link") {
        let start = from + i + 1;
        let Some(end) = lower[start..].find('>') else { break };
        let attrs = attributes(&html[start..start + end]);
        let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        if get("rel").is_some_and(|r| r.split_whitespace().any(|w| w.eq_ignore_ascii_case("manifest"))) {
            if let Some(h) = get("href").and_then(|h| resolve(page_url, h)) {
                manifest = Some(h);
                break;
            }
        }
        from = start + end;
    }
    let title = lower.find("<title").and_then(|i| {
        let open_end = lower[i..].find('>')? + i + 1;
        let close = lower[open_end..].find("</title")? + open_end;
        let t = html[open_end..close].split_whitespace().collect::<Vec<_>>().join(" ");
        Some(t).filter(|t| !t.is_empty())
    });
    (manifest, title)
}

/// The directory of an address (everything up to the last `/` of its path, without query or fragment).
fn directory(url: &str) -> Option<String> {
    let no_frag = url.split(['?', '#']).next()?;
    let (scheme, rest) = no_frag.split_once("://")?;
    Some(match rest.rfind('/') {
        Some(i) => format!("{scheme}://{}", &rest[..=i]),
        None => format!("{scheme}://{rest}/"),
    })
}

fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    Some(format!("{scheme}://{}", rest.split(['/', '?', '#']).next()?))
}

/// Read a manifest file (`json`, fetched from `manifest_url`) for the page at `page_url`, the way Chromium does:
/// `name` or `short_name`, `start_url` (the page's own address if absent), a `scope` that must contain the start
/// page (else the start page's directory), `display`, `theme_color`, `id` and the icons. Icons marked only `maskable`
/// or `monochrome` are not used (GNOME does not mask). `None` when it is not a usable manifest (no name).
pub fn parse_raw_manifest(json: &str, manifest_url: &str, page_url: &str) -> Option<AppInfo> {
    let v: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    let text = |key: &str| v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let name = text("name").or_else(|| text("short_name"))?;
    let start_url = text("start_url").and_then(|s| resolve(manifest_url, &s)).map(|s| s.split('#').next().unwrap_or("").to_owned()).unwrap_or_else(|| page_url.to_owned());
    let default_scope = directory(&start_url);
    let scope_url = text("scope").and_then(|s| resolve(manifest_url, &s)).filter(|s| start_url.starts_with(s.split(['?', '#']).next().unwrap_or(""))).or(default_scope);
    let display = match text("display").as_deref() {
        Some("minimal-ui") => AppDisplay::Minimal,
        Some("browser") => AppDisplay::Browser,
        _ => AppDisplay::Standalone,
    };
    let mut icons: Vec<(u32, String)> = Vec::new();
    for icon in v.get("icons").and_then(|i| i.as_array()).into_iter().flatten() {
        let purpose = icon.get("purpose").and_then(|p| p.as_str()).unwrap_or("any");
        if !purpose.split_whitespace().any(|p| p == "any") {
            continue;
        }
        let Some(url) = icon.get("src").and_then(|s| s.as_str()).and_then(|s| resolve(manifest_url, s)) else { continue };
        icons.push((icon_size(icon.get("sizes").and_then(|s| s.as_str()).unwrap_or("")), url));
    }
    icons.sort_by(|a, b| b.0.cmp(&a.0)); // stable: equal sizes keep the manifest's order
    Some(AppInfo {
        name,
        scope: scope_url.and_then(|s| scope_pattern(&s)).into_iter().collect(),
        manifest_id: Some(text("id").and_then(|i| origin(&start_url).and_then(|o| resolve(&format!("{o}/"), &i))).unwrap_or_else(|| start_url.clone())),
        start_url,
        display,
        color: text("theme_color").and_then(|c| hex_colour(&c)),
        icons: icons.into_iter().map(|(_, u)| u).collect(),
        from_manifest: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r##"{"url":"http://localhost:8765/manifest.webmanifest","errors":[],"data":"{\"name\":\"Spike Application\",\"icons\":[{\"src\":\"icon.svg\",\"purpose\":\"any\"},{\"src\":\"icon-192.png\",\"sizes\":\"192x192\",\"purpose\":\"maskable\"},{\"src\":\"big.png\",\"sizes\":\"512x512\"}]}","parsed":{"scope":"http://localhost:8765/"},"manifest":{"display":"kMinimalUi","icons":[{"url":"http://localhost:8765/icon.svg","sizes":"0x0","type":"image/svg+xml"},{"url":"http://localhost:8765/icon-192.png","sizes":"192x192"},{"url":"http://localhost:8765/big.png","sizes":"512x512"}],"id":"http://localhost:8765/app","name":"Spike Application","scope":"http://localhost:8765/app/","startUrl":"http://localhost:8765/index.html?source=pwa","themeColor":"rgba(51,102,153,1)"}}"##;

    #[test]
    fn a_devtools_answer_becomes_app_details() {
        let a = parse_manifest(ANSWER).unwrap();
        assert_eq!(a.name, "Spike Application");
        assert_eq!(a.start_url, "http://localhost:8765/index.html?source=pwa");
        assert_eq!(a.scope, ["localhost/app/*"]);
        assert_eq!(a.display, AppDisplay::Minimal);
        assert_eq!(a.color.as_deref(), Some("#336699"));
        assert_eq!(a.manifest_id.as_deref(), Some("http://localhost:8765/app"));
        assert_eq!(a.icons, ["http://localhost:8765/icon.svg", "http://localhost:8765/big.png"]);
    }

    #[test]
    fn addresses_resolve_like_a_browsers_links() {
        let b = "https://a.example/x/y/page.html?q=1#top";
        for (rel, want) in [
            ("icon.png", "https://a.example/x/y/icon.png"),
            ("./icon.png", "https://a.example/x/y/icon.png"),
            ("../icon.png", "https://a.example/x/icon.png"),
            ("../../../icon.png", "https://a.example/icon.png"),
            ("/m.json", "https://a.example/m.json"),
            ("//cdn.example/m.json", "https://cdn.example/m.json"),
            ("https://b.example/m.json", "https://b.example/m.json"),
            ("?z=2", "https://a.example/x/y/page.html?z=2"),
            ("#frag", "https://a.example/x/y/page.html?q=1"),
            ("dir/", "https://a.example/x/y/dir/"),
            (".", "https://a.example/x/y/"),
            ("m.json?v=3", "https://a.example/x/y/m.json?v=3"),
            ("", "https://a.example/x/y/page.html?q=1"),
        ] {
            assert_eq!(resolve(b, rel).as_deref(), Some(want), "{rel}");
        }
        assert_eq!(resolve("https://a.example", "m.json").as_deref(), Some("https://a.example/m.json"));
        assert_eq!(resolve("ftp://a.example/", "m.json"), None);
    }

    #[test]
    fn a_page_names_its_manifest_and_title() {
        let html = "<!doctype html><HTML><head><title>\n  My   Mail \n</title><link rel=\"icon\" href=\"/f.ico\"><LINK REL='Manifest' HREF=app/manifest.json crossorigin></head>";
        let (m, t) = page_links(html, "https://mail.example/inbox/");
        assert_eq!(m.as_deref(), Some("https://mail.example/inbox/app/manifest.json"));
        assert_eq!(t.as_deref(), Some("My Mail"));
        assert_eq!(page_links("<link rel=\"stylesheet\" href=a.css><p>hi", "https://x/").0, None);
        assert_eq!(page_links("<link rel=\"alternate manifest\" href=\"/m\">", "https://x.example/p").0.as_deref(), Some("https://x.example/m"));
    }

    #[test]
    fn a_manifest_file_gives_what_chromium_gives() {
        let json = r##"{"name":"Mail","short_name":"M","start_url":"/inbox?src=pwa#x","scope":"/","display":"minimal-ui","theme_color":"#369","id":"/mail",
            "icons":[{"src":"a.svg","purpose":"any"},{"src":"m192.png","sizes":"192x192","purpose":"maskable"},{"src":"b.png","sizes":"512x512"},{"src":"c.png","sizes":"96x96 48x48","purpose":"any maskable"}]}"##;
        let a = parse_raw_manifest(json, "https://mail.example/static/manifest.json", "https://mail.example/").unwrap();
        assert_eq!(a.name, "Mail");
        assert_eq!(a.start_url, "https://mail.example/inbox?src=pwa");
        assert_eq!(a.scope, ["mail.example"]);
        assert_eq!(a.display, AppDisplay::Minimal);
        assert_eq!(a.color.as_deref(), Some("#336699"));
        assert_eq!(a.manifest_id.as_deref(), Some("https://mail.example/mail"));
        assert_eq!(a.icons, ["https://mail.example/static/a.svg", "https://mail.example/static/b.png", "https://mail.example/static/c.png"], "vector first, then by size; maskable-only is dropped");
        assert!(a.from_manifest);
    }

    #[test]
    fn a_manifest_without_much_gets_the_defaults_the_spec_gives() {
        let a = parse_raw_manifest("{\"short_name\":\"Notes\"}", "https://n.example/app/m.json", "https://n.example/app/index.html").unwrap();
        assert_eq!((a.name.as_str(), a.start_url.as_str()), ("Notes", "https://n.example/app/index.html"), "the page itself is the start page");
        assert_eq!(a.scope, ["n.example/app/*"], "the start page's directory");
        assert_eq!((a.display, a.color.clone(), a.icons.len()), (AppDisplay::Standalone, None, 0));
        assert_eq!(a.manifest_id.as_deref(), Some("https://n.example/app/index.html"));
        // A scope that does not contain the start page is ignored.
        let b = parse_raw_manifest("{\"name\":\"X\",\"start_url\":\"/a/start\",\"scope\":\"/b/\"}", "https://x.example/m.json", "https://x.example/").unwrap();
        assert_eq!(b.scope, ["x.example/a/*"]);
        assert!(parse_raw_manifest("{\"start_url\":\"/\"}", "https://x.example/m.json", "https://x.example/").is_none(), "a name is required");
        assert!(parse_raw_manifest("not json", "https://x.example/m.json", "https://x.example/").is_none());
    }

    #[test]
    fn colours_and_pages() {
        assert_eq!(hex_colour("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(hex_colour(" #2A6F97 ").as_deref(), Some("#2a6f97"));
        assert_eq!(hex_colour("red"), None);
        assert_eq!(hex_colour("#12345"), None);
        assert_eq!(page_info("", "https://x.example/p").name, "x.example");
        assert_eq!(page_info("  Hi  ", "https://x.example/p").name, "Hi");
    }

    #[test]
    fn no_manifest_means_no_details() {
        assert!(parse_manifest(r#"{"url":"","errors":["no manifest"]}"#).is_none());
        assert!(parse_manifest("not json {").is_none());
        assert!(parse_manifest(r#"{"manifest":{"name":"X"}}"#).is_none(), "a start URL is needed");
    }

    #[test]
    fn pieces() {
        assert_eq!(rgba_to_hex("rgba(255,255,255,1)").as_deref(), Some("#ffffff"));
        assert_eq!(rgba_to_hex("rgba(0, 10, 20, 0.5)").as_deref(), Some("#000a14"));
        assert_eq!(rgba_to_hex("red"), None);
        assert_eq!(scope_pattern("https://Mail.Example.com:8443/").as_deref(), Some("mail.example.com"));
        assert_eq!(scope_pattern("https://a.example/app/").as_deref(), Some("a.example/app/*"));
        assert_eq!(scope_pattern("nonsense"), None);
        assert_eq!(icon_size("48x48 96x96"), 96);
        assert_eq!(icon_size("any"), 1024);
    }
}
