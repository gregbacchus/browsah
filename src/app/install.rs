//! Installing a page as a web app: read its web manifest through DevTools (`Page.getAppManifest`), show what
//! was found, and create the workspace, launcher entry and icon (`webapp.rs`). The manifest is the site's
//! own claim, so nothing is saved until the user confirms the name and address shown.

use super::{view::BrowserView, Shell};
use crate::webapp;
use crate::workspace::AppDisplay;
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

/// What a page says about itself as an app.
#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    pub name: String,
    pub start_url: String,
    /// Scope patterns for `app.scope` (empty: the start page's site).
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

/// A scope address as a pattern for `app.scope`: the site, plus the path when it is not the root.
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
fn icon_size(sizes: &str) -> u32 {
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

/// Ask the page for its manifest. `None` when it has none (or the browser does not answer).
pub async fn read_manifest(view: &Rc<BrowserView>) -> Option<AppInfo> {
    let id = view.cdp("Page.getAppManifest");
    let mut waited = 0;
    while waited < 5000 {
        if let Some((ok, json)) = view.cdp_result(id) {
            return if ok { parse_manifest(&json) } else { None };
        }
        glib::timeout_future(Duration::from_millis(50)).await;
        waited += 50;
    }
    None
}

/// Fetch the first icon that downloads, as PNG bytes.
async fn fetch_icon(view: &Rc<BrowserView>, icons: &[String]) -> Option<Vec<u8>> {
    for url in icons.iter().take(3) {
        view.download_image(url);
        let mut waited = 0;
        while waited < 5000 && view.image_result(url).is_none() {
            glib::timeout_future(Duration::from_millis(50)).await;
            waited += 50;
        }
        if let Some(png) = view.image_png(url) {
            return Some(png);
        }
    }
    None
}

/// Create the web app: its workspace directory (with the icon), launcher entry and icon. Returns its id.
pub async fn save_app(view: &Rc<BrowserView>, info: &AppInfo, name: &str) -> Result<String, String> {
    let profile = super::PROFILE.with(|p| p.borrow().clone()).ok_or("no profile")?;
    let existing: std::collections::HashSet<String> = crate::workspace::Workspace::list(&profile).into_iter().map(|w| w.id).collect();
    let base = webapp::id_from_name(name);
    let id = std::iter::once(base.clone()).chain((2..100).map(|n| format!("{base}-{n}"))).find(|c| !existing.contains(c)).ok_or("no free id")?;
    let icon = fetch_icon(view, &info.icons).await;
    let color = info.color.as_deref().and_then(|c| crate::profile::ColorPair::parse(c).ok());
    let display = match info.display {
        AppDisplay::Standalone => "standalone",
        AppDisplay::Minimal => "minimal",
        AppDisplay::Browser => "browser",
    };
    let text = webapp::config_text(Some(name), color.as_ref(), &info.start_url, &info.scope, display, icon.as_ref().map(|_| "icon.png"), info.manifest_id.as_deref());
    let dir = profile.workspaces_dir().join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("workspace.yaml"), text).map_err(|e| e.to_string())?;
    if let Some(png) = &icon {
        std::fs::write(dir.join("icon.png"), png).map_err(|e| e.to_string())?;
    }
    let w = crate::workspace::Workspace::resolve(&profile, &id)?;
    let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| e.to_string())?;
    let libs = exe.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    webapp::install(&profile, &w, &exe, &libs).map_err(|e| format!("created {id:?}, but its launcher entry failed: {e}"))?;
    Ok(id)
}

impl Shell {
    /// "Install as web app": read the page's manifest, ask the user to confirm the name, then create the app.
    pub fn offer_install_app(self: &Rc<Self>, tab: Rc<super::Tab>) {
        let shell = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let view = tab.view();
            let (title, address) = {
                let s = tab.state.borrow();
                (s.title.clone(), s.address.clone())
            };
            if !(address.starts_with("http://") || address.starts_with("https://")) {
                shell.toast("Only a web page can be installed as an app");
                return;
            }
            let info = read_manifest(&view).await.unwrap_or_else(|| page_info(&title, &address));
            let _ = shell.confirm_install(view, info);
        });
    }

    pub(super) fn confirm_install(self: &Rc<Self>, view: Rc<BrowserView>, info: AppInfo) -> adw::AlertDialog {
        let dialog = adw::AlertDialog::new(Some("Install as web app"), None);
        let source = if info.from_manifest { "from the site's manifest" } else { "this site has no manifest, so this is the page itself" };
        dialog.set_body(&format!("{}\n\nOpens on {} ({source}).", info.name, info.start_url));
        let entry = gtk::Entry::builder().text(&info.name).activates_default(true).build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Cancel"), ("install", "Install")]);
        dialog.set_response_appearance("install", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("install"));
        dialog.set_close_response("cancel");
        let shell = self.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "install" {
                return;
            }
            let (shell, view, info, name) = (shell.clone(), view.clone(), info.clone(), entry.text().trim().to_owned());
            let name = if name.is_empty() { info.name.clone() } else { name };
            glib::MainContext::default().spawn_local(async move {
                match save_app(&view, &info, &name).await {
                    Ok(id) => {
                        shell.toast(&format!("Installed {name}; find it in your launcher"));
                        if let Some(app) = shell.window.application() {
                            gtk::gio::prelude::ActionGroupExt::activate_action(&app, "open-workspace", Some(&id.to_variant()));
                        }
                    }
                    Err(e) => shell.toast(&format!("Could not install: {e}")),
                }
            });
        });
        dialog.present(Some(&self.window));
        dialog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `Page.getAppManifest` answered for `bench/pwa/manifest.webmanifest` (shortened).
    const ANSWER: &str = r##"{"url":"http://localhost:8765/manifest.webmanifest","errors":[],"data":"{\"name\":\"Spike Application\",\"icons\":[{\"src\":\"icon.svg\",\"purpose\":\"any\"},{\"src\":\"icon-192.png\",\"sizes\":\"192x192\",\"purpose\":\"maskable\"},{\"src\":\"big.png\",\"sizes\":\"512x512\"}]}","parsed":{"scope":"http://localhost:8765/"},"manifest":{"display":"kMinimalUi","icons":[{"url":"http://localhost:8765/icon.svg","sizes":"0x0","type":"image/svg+xml"},{"url":"http://localhost:8765/icon-192.png","sizes":"192x192"},{"url":"http://localhost:8765/big.png","sizes":"512x512"}],"id":"http://localhost:8765/app","name":"Spike Application","scope":"http://localhost:8765/app/","startUrl":"http://localhost:8765/index.html?source=pwa","themeColor":"rgba(51,102,153,1)"}}"##;

    #[test]
    fn a_manifest_answer_becomes_app_details() {
        let a = parse_manifest(ANSWER).expect("parses");
        assert_eq!(a.name, "Spike Application");
        assert_eq!(a.start_url, "http://localhost:8765/index.html?source=pwa");
        assert_eq!(a.scope, ["localhost/app/*"]);
        assert_eq!(a.display, AppDisplay::Minimal);
        assert_eq!(a.color.as_deref(), Some("#336699"));
        assert_eq!(a.manifest_id.as_deref(), Some("http://localhost:8765/app"));
        // The maskable-only icon is skipped; the vector one ("any", size unknown) comes first, then the 512 px one.
        assert_eq!(a.icons, ["http://localhost:8765/icon.svg", "http://localhost:8765/big.png"]);
        assert!(a.from_manifest);
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
        let p = page_info("  ", "https://news.example/x");
        assert_eq!((p.name.as_str(), p.from_manifest), ("news.example", false));
        assert_eq!(page_info("News", "https://news.example/").name, "News");
    }
}
