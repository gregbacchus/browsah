//! Installing a page as a web app: read its web manifest through DevTools (`Page.getAppManifest`), show what
//! was found, and create the workspace, launcher entry and icon (`webapp.rs`). The manifest is the site's
//! own claim, so nothing is saved until the user confirms the name and address shown.

use super::{view::BrowserView, Shell};
use crate::manifest::{page_info, parse_manifest, AppInfo};
use crate::webapp;
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

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

/// Create the web app: its directory (with the icon), launcher entry and icon. Returns its id.
pub async fn save_app(view: &Rc<BrowserView>, info: &AppInfo, name: &str, page_url: &str) -> Result<String, String> {
    let profile = super::PROFILE.with(|p| p.borrow().clone()).ok_or("no profile")?;
    let icon = fetch_icon(view, &info.icons).await.map(|png| ("png", png));
    let (exe, libs) = crate::profile::launcher().map_err(|e| e.to_string())?;
    let spec = webapp::NewApp { name: Some(name.to_owned()), color: info.color.clone(), url: info.start_url.clone(), scope: info.scope.clone(), display: info.display, icon, manifest: info.from_manifest.then(|| webapp::ManifestInstall { info: info.clone(), page_url: page_url.to_owned(), custom_icon: false }) };
    webapp::create(&profile, None, &spec, &exe, &libs)
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
            let _ = shell.confirm_install(view, info, address);
        });
    }

    pub(super) fn confirm_install(self: &Rc<Self>, view: Rc<BrowserView>, info: AppInfo, page_url: String) -> adw::AlertDialog {
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
            let page_url = page_url.clone();
            glib::MainContext::default().spawn_local(async move {
                match save_app(&view, &info, &name, &page_url).await {
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
