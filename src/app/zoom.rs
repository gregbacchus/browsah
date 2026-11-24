//! Page zoom: `Ctrl +`, `Ctrl -`, `Ctrl 0` and `Ctrl` with the wheel. The zoom is remembered per site (host) in the
//! profile database and put back whenever a page of that site loads.

use super::{store, view::BrowserView, Shell, Tab};
use std::rc::Rc;

/// The zoom factors a step moves between (as in other browsers).
pub const STEPS: &[f64] = &[0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];

/// The next factor up (`dir` > 0) or down (`dir` < 0) from `current`; 1.0 for `dir` = 0.
pub fn next_factor(current: f64, dir: i32) -> f64 {
    if dir == 0 {
        return 1.0;
    }
    if dir > 0 {
        STEPS.iter().copied().find(|s| *s > current + 0.001).unwrap_or(*STEPS.last().unwrap_or(&1.0))
    } else {
        STEPS.iter().rev().copied().find(|s| *s < current - 0.001).unwrap_or(STEPS[0])
    }
}

pub fn percent_text(factor: f64) -> String {
    format!("{}%", (factor * 100.0).round() as i32)
}

/// The host a zoom is remembered for (none for `about:` pages and the like).
fn zoom_host(address: &str) -> Option<String> {
    if !(address.starts_with("http://") || address.starts_with("https://")) {
        return None;
    }
    Some(crate::cookies::host_of(address)).filter(|h| !h.is_empty())
}

impl Shell {
    /// Zoom the selected tab one step (`0`: back to 100 %), remember it for the site and say what it is now.
    pub fn zoom_step(self: &Rc<Self>, dir: i32) {
        let Some(tab) = self.current() else { return };
        let view = tab.focused_view();
        let next = next_factor(view.user_zoom(), dir);
        if dir == 0 {
            view.reset_page_scale(); // a pinch's view zoom goes too
        }
        view.set_user_zoom(next);
        let address = tab.focused_state().borrow().address.clone();
        if let (Some(host), Some(st)) = (zoom_host(&address), store()) {
            let _ = st.set_site_zoom(&host, next);
        }
        self.toast(&format!("Zoom {}", percent_text(next)));
    }

    /// A page of a site loaded or the address changed: use the zoom the person set for that site.
    pub(super) fn restore_site_zoom(&self, tab: &Rc<Tab>) {
        let address = tab.state.borrow().address.clone();
        self.restore_site_zoom_of(&tab.view(), &address);
    }

    /// The same for one view (either pane of a split tab).
    pub(super) fn restore_site_zoom_of(&self, view: &Rc<BrowserView>, address: &str) {
        let factor = zoom_host(address).and_then(|h| store().and_then(|s| s.site_zoom(&h))).unwrap_or(1.0);
        if (view.user_zoom() - factor).abs() > 0.001 {
            view.set_user_zoom(factor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_move_between_the_usual_zoom_levels() {
        assert_eq!(next_factor(1.0, 1), 1.1);
        assert_eq!(next_factor(1.1, 1), 1.25);
        assert_eq!(next_factor(1.0, -1), 0.9);
        assert_eq!(next_factor(1.25, 0), 1.0);
        assert_eq!(next_factor(5.0, 1), 5.0, "the top stays");
        assert_eq!(next_factor(0.25, -1), 0.25, "the bottom stays");
        assert_eq!(next_factor(1.2, 1), 1.25, "an odd value goes to the next step");
        assert_eq!(next_factor(1.2, -1), 1.1);
        assert_eq!(percent_text(1.25), "125%");
        assert_eq!(percent_text(0.67), "67%");
    }

    #[test]
    fn only_web_pages_have_a_remembered_zoom() {
        assert_eq!(zoom_host("https://News.Example:8443/x").as_deref(), Some("news.example"));
        assert_eq!(zoom_host("about:blank"), None);
        assert_eq!(zoom_host("file:///tmp/x.html"), None);
    }
}
