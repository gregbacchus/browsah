//! The message shown when the CEF that was loaded does not fit this browsah (an old codec runtime after an upgrade): CEF is not started
//! yet, so this is a plain dialog with the problem and what to run; the caller then exits.

use gtk::{gio, glib};

/// Show `title` and `detail` in a dialog and wait until it is closed.
pub fn show_cef_mismatch(title: &str, detail: &str) {
    let dialog = gtk::AlertDialog::builder().modal(true).message(title).detail(detail).buttons(["Quit"]).cancel_button(0).default_button(0).build();
    let main_loop = glib::MainLoop::new(None, false);
    let done = main_loop.clone();
    dialog.choose(None::<&gtk::Window>, gio::Cancellable::NONE, move |_| done.quit());
    main_loop.run();
}
