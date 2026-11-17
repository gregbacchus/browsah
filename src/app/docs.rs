//! The "Documentation" window: the configuration docs rendered as a small site (`crate::docsite`) and shown in a
//! window with no workspace, like any other page.

use super::{Shell, NEW_EPHEMERAL, PROFILE};
use std::path::PathBuf;
use std::rc::Rc;

/// Where the rendered site lives: per profile and version, so processes never write over each other's pages.
pub fn site_dir() -> PathBuf {
    let id = PROFILE.with(|p| p.borrow().as_ref().map(|p| p.id.clone())).unwrap_or_else(|| "default".into());
    gtk::glib::user_cache_dir().join("browsah").join(format!("docs-{id}-{}", env!("CARGO_PKG_VERSION")))
}

impl Shell {
    /// Render the documentation and show it in a window. Returns the window, or `None` after a message.
    pub fn open_docs(self: &Rc<Self>) -> Option<Rc<Shell>> {
        let index = match crate::docsite::write_site(&site_dir()) {
            Ok(i) => i,
            Err(e) => {
                self.toast(&format!("Could not write the documentation: {e}"));
                return None;
            }
        };
        let url = format!("file://{}", index.display());
        let Some(open) = NEW_EPHEMERAL.with(|f| f.borrow().clone()) else {
            self.open_tab(&url, true);
            return None;
        };
        Some(open(&url))
    }
}
