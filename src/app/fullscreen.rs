//! Fullscreen: `F11` for the window, and a page's own request (a video's fullscreen button). The bars are hidden
//! while the window fills the screen; `Esc` (handled by Chromium for a page) or `F11` leaves.

use super::Shell;
use adw::prelude::*;
use std::rc::Rc;

impl Shell {
    /// Fill the screen (or stop). `from_page`: a page asked, so the page also has to be told when it is left here.
    pub(super) fn set_fullscreen(self: &Rc<Self>, on: bool, from_page: bool) {
        if from_page {
            self.page_fullscreen.set(on);
        } else if !on {
            // Left with F11: a page that asked for fullscreen must give it up too.
            if self.page_fullscreen.replace(false) {
                if let Some(t) = self.current() {
                    t.view().exit_page_fullscreen();
                }
            }
        }
        self.fullscreen.set(on);
        self.window.set_fullscreened(on);
        self.toolbar.set_reveal_top_bars(!on);
        self.toolbar.set_reveal_bottom_bars(!on || self.find.is_open());
    }

    /// `F11`.
    pub(super) fn toggle_fullscreen(self: &Rc<Self>) {
        let on = !self.fullscreen.get();
        self.set_fullscreen(on, false);
    }

    /// Whether the window is fullscreen (for the self-test).
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.get()
    }

    /// Whether the window's bars are shown (for the self-test).
    pub fn bars_shown(&self) -> bool {
        self.toolbar.reveals_top_bars()
    }
}
