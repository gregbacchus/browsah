//! The profile selector: a small program that is the system's web browser and asks which profile
//! should open a link.
//!
//! `browsah select [URL]` (what the `be.geee.browsah.desktop` entry runs) shows a list of the
//! profiles; choosing one starts `browsah --profile=<id> URL`, which hands the link to that
//! profile's process (or starts it). It needs neither CEF nor a profile, so it appears at once.
//! The application's routing rules (`routing:` in `config.yaml`) and `default_profile` can answer
//! without asking.
//!
//! `browsah selector install` writes the `.desktop` entry and icon; choosing "Browsah" as the
//! default web browser is then a setting of the desktop.

use crate::profile::{self, Profile};
use crate::router::{Action, Kind, Request, Router, Source, Target};
use adw::prelude::*;
use gtk::{gio, glib};
use std::path::Path;
use std::rc::Rc;

/// The application id of the selector (profiles are `be.geee.browsah.<id>`).
pub const SELECTOR_APP_ID: &str = profile::APP_ID_PREFIX;

/// What to do with a link.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Start (or hand the link to) this profile, optionally into one of its workspaces.
    Launch { profile: String, workspace: Option<String> },
    /// Do nothing (a rule blocked the link).
    Ignore,
    /// Ask.
    Choose,
}

/// Decide without asking if possible: one profile, a configured default, or an application rule
/// that names a profile.
pub fn decide(profile_ids: &[String], default_profile: Option<&str>, rule_action: &Action) -> Plan {
    if let Action::Profile { profile, workspace } = rule_action {
        if profile_ids.contains(profile) {
            return Plan::Launch { profile: profile.clone(), workspace: workspace.clone() };
        }
    }
    if *rule_action == Action::Block {
        return Plan::Ignore;
    }
    if let [only] = profile_ids {
        return Plan::Launch { profile: only.clone(), workspace: None };
    }
    if let Some(d) = default_profile.filter(|d| profile_ids.iter().any(|p| p == d)) {
        return Plan::Launch { profile: d.to_owned(), workspace: None };
    }
    Plan::Choose
}

/// Start `browsah --profile=ID [--workspace=W] [URL]` (which hands over to a running profile).
fn launch(profile: &str, workspace: Option<&str>, url: Option<&str>) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg(format!("--profile={profile}"));
    if let Some(w) = workspace {
        cmd.arg(format!("--workspace={w}"));
    }
    if let Some(u) = url {
        cmd.arg(u);
    }
    cmd.spawn().map(|_| ())
}

/// The short form of a link for the window: its host, or the whole thing if it is not a web address.
fn link_summary(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(rest).to_owned(),
        None => url.chars().take(60).collect(),
    }
}

/// One line of the list.
pub struct Row {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub running: bool,
}

/// Build the chooser window. `on_pick` gets the chosen profile id.
fn build_window(app: &adw::Application, rows: &[Row], url: Option<&str>, on_pick: Rc<dyn Fn(&str)>) -> (adw::ApplicationWindow, gtk::ListBox) {
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).css_classes(["boxed-list"]).build();
    let theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().expect("display"));
    for (i, r) in rows.iter().enumerate() {
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&r.name).as_str()).subtitle(if r.running { "running" } else { "" }).activatable(true).build();
        let icon = if theme.has_icon(&r.icon) { r.icon.as_str() } else { "web-browser-symbolic" };
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(32);
        row.add_prefix(&image);
        if i < 9 {
            row.add_suffix(&gtk::Label::builder().label(format!("{}", i + 1)).css_classes(["dim-label", "caption"]).build());
        }
        // The row's name in the widget name, so the pick does not depend on its position.
        row.set_widget_name(&r.id);
        list.append(&row);
    }
    list.connect_row_activated({
        let on_pick = on_pick.clone();
        move |_, row| on_pick(row.widget_name().as_str())
    });

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_top(12).margin_bottom(18).margin_start(18).margin_end(18).build();
    if let Some(u) = url {
        let label = gtk::Label::builder().label(format!("Open {} with:", link_summary(u))).xalign(0.0).wrap(true).css_classes(["heading"]).build();
        label.set_tooltip_text(Some(u));
        content.append(&label);
    }
    content.append(&list);
    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    let window = adw::ApplicationWindow::builder().application(app).title("Choose a profile").content(&toolbar).default_width(380).resizable(false).build();

    // 1 to 9 choose a profile; Escape closes.
    let keys = gtk::EventControllerKey::new();
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    keys.connect_key_pressed({
        let window = window.downgrade();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(w) = window.upgrade() {
                    w.close();
                }
                return glib::Propagation::Stop;
            }
            if let Some(n) = key.to_unicode().and_then(|c| c.to_digit(10)).filter(|n| (1..=9).contains(n)) {
                if let Some(id) = ids.get(n as usize - 1) {
                    on_pick(id);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
        first.grab_focus();
    }
    (window, list)
}

fn rows_for(profiles: &[Profile]) -> Vec<Row> {
    profiles
        .iter()
        .map(|p| Row { id: p.id.clone(), name: p.display_name().to_owned(), icon: p.app_id(), running: profile::is_running(p) })
        .collect()
}

/// `browsah select [URL]`: choose a profile for a link. Returns the exit code.
pub fn run(url: Option<&str>) -> i32 {
    let app_file = crate::config::default_path();
    let loaded = crate::config::load_for_startup(&app_file, None);
    let profiles = profile::list();
    let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
    // The application's routing rules may already name a profile (or block the link).
    let rule_action = match url {
        Some(u) => Router::load(&app_file, None).route(&Request { url: u.to_owned(), source: Source::External, kind: Kind::External, target: Target::SameTab }).action,
        None => Action::Allow,
    };
    match decide(&ids, loaded.config.default_profile.as_deref(), &rule_action) {
        Plan::Ignore => 0,
        Plan::Launch { profile, workspace } => match launch(&profile, workspace.as_deref(), url) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("browsah: cannot start profile {profile}: {e}");
                1
            }
        },
        Plan::Choose => show(&profiles, url),
    }
}

fn show(profiles: &[Profile], url: Option<&str>) -> i32 {
    let app = adw::Application::builder().application_id(SELECTOR_APP_ID).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    let rows = rows_for(profiles);
    let url = url.map(str::to_owned);
    let code = Rc::new(std::cell::Cell::new(0));
    app.connect_activate({
        let code = code.clone();
        move |app| {
            let on_pick: Rc<dyn Fn(&str)> = Rc::new({
                let (app, url, code) = (app.clone(), url.clone(), code.clone());
                move |id: &str| match launch(id, None, url.as_deref()) {
                    Ok(()) => app.quit(),
                    Err(e) => {
                        eprintln!("browsah: cannot start profile {id}: {e}");
                        code.set(1);
                        app.quit();
                    }
                }
            });
            let (window, _) = build_window(app, &rows, url.as_deref(), on_pick);
            window.present();
        }
    });
    app.run_with_args::<&str>(&[]);
    code.get()
}

/// `browsah select --check`: build the window with made-up profiles, activate the second row and
/// confirm the pick arrives. Needs a display; used by the test suite.
fn check() -> i32 {
    let app = adw::Application::builder().application_id(format!("{SELECTOR_APP_ID}.selectortest")).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    let picked = Rc::new(std::cell::RefCell::new(None::<String>));
    let result = Rc::new(std::cell::Cell::new(1));
    app.connect_activate({
        let (picked, result) = (picked.clone(), result.clone());
        move |app| {
            let rows = vec![
                Row { id: "alpha".into(), name: "Alpha".into(), icon: "no-such-icon".into(), running: false },
                Row { id: "beta".into(), name: "Beta & <Co>".into(), icon: "no-such-icon".into(), running: true },
            ];
            let on_pick: Rc<dyn Fn(&str)> = Rc::new({
                let picked = picked.clone();
                move |id: &str| *picked.borrow_mut() = Some(id.to_owned())
            });
            let (window, list) = build_window(app, &rows, Some("https://example.com/path?q=1"), on_pick);
            window.present();
            let count = (0..).take_while(|i| list.row_at_index(*i).is_some()).count();
            let second = list.row_at_index(1);
            let named = second.as_ref().map(|r| r.widget_name().to_string());
            list.emit_by_name::<()>("row-activated", &[&second.expect("second row")]);
            let ok = count == 2 && named.as_deref() == Some("beta") && picked.borrow().as_deref() == Some("beta");
            println!("{} the chooser lists the profiles and reports the one activated (rows {count}, picked {:?})", if ok { "PASS" } else { "FAIL" }, picked.borrow());
            result.set(if ok { 0 } else { 1 });
            app.quit();
        }
    });
    app.run_with_args::<&str>(&[]);
    result.get()
}

// ---- the desktop entry ---------------------------------------------------------------------------

/// The `.desktop` entry of the selector: the system's web browser, which asks for a profile.
pub fn desktop_entry(exe: &Path, libs: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Browsah\n\
         GenericName=Web Browser\n\
         Comment=Choose a Browsah profile for web links\n\
         Exec=env LD_LIBRARY_PATH={libs} {exe} select %U\n\
         Icon={id}\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass={id}\n\
         Categories=Network;WebBrowser;\n\
         MimeType=text/html;application/xhtml+xml;x-scheme-handler/http;x-scheme-handler/https;\n",
        libs = profile::exec_quote(&libs.to_string_lossy()),
        exe = profile::exec_quote(&exe.to_string_lossy()),
        id = SELECTOR_APP_ID,
    )
}

/// The selector's icon: the default colour of [`crate::appicon`].
pub fn icon_svg() -> String {
    crate::appicon::svg(None)
}

const USAGE: &str = "usage:
  browsah select [URL]          choose a profile for a link (what the desktop entry runs)
  browsah selector install      add the \"Browsah\" desktop entry and icon
  browsah selector uninstall    remove them
  browsah selector status       say whether the entry exists and which browser is the default

After `install`, choose \"Browsah\" as the default web browser in the desktop's settings.";

/// `browsah selector ...`. Returns the exit code.
pub fn selector_cli(args: &[String]) -> i32 {
    let entry = profile::applications_dir().join(format!("{SELECTOR_APP_ID}.desktop"));
    let icon = profile::icons_dir().join(format!("{SELECTOR_APP_ID}.svg"));
    match args.first().map(String::as_str) {
        Some("install") => {
            let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
                eprintln!("cannot find this program");
                return 1;
            };
            let libs = exe.parent().map(Path::to_path_buf).unwrap_or_default();
            let result = std::fs::create_dir_all(profile::applications_dir())
                .and_then(|_| std::fs::create_dir_all(profile::icons_dir()))
                .and_then(|_| std::fs::write(&entry, desktop_entry(&exe, &libs)))
                .and_then(|_| std::fs::write(&icon, icon_svg()));
            if let Err(e) = result {
                eprintln!("cannot write the desktop entry: {e}");
                return 1;
            }
            let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
            println!("installed {}", entry.display());
            println!("Choose \"Browsah\" as the default web browser in your desktop's settings (Default Apps), or run:");
            println!("  xdg-settings set default-web-browser {SELECTOR_APP_ID}.desktop");
            0
        }
        Some("uninstall") => {
            let _ = std::fs::remove_file(&entry);
            let _ = std::fs::remove_file(&icon);
            let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
            println!("removed the Browsah desktop entry");
            0
        }
        Some("status") => {
            println!("desktop entry: {} ({})", if entry.exists() { "installed" } else { "not installed" }, entry.display());
            let default = std::process::Command::new("xdg-settings").args(["get", "default-web-browser"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
            println!("default web browser: {}", default.filter(|d| !d.is_empty()).unwrap_or_else(|| "unknown".into()));
            0
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

/// `browsah select ...`.
pub fn select_cli(args: &[String]) -> i32 {
    if args.first().map(String::as_str) == Some("--check") {
        return check();
    }
    run(args.iter().find(|a| !a.starts_with("--")).map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plan_without_asking() {
        let two = ids(&["default", "work"]);
        // A single profile needs no question.
        assert_eq!(decide(&ids(&["default"]), None, &Action::Allow), Plan::Launch { profile: "default".into(), workspace: None });
        // Several: ask, unless there is a configured default.
        assert_eq!(decide(&two, None, &Action::Allow), Plan::Choose);
        assert_eq!(decide(&two, Some("work"), &Action::Allow), Plan::Launch { profile: "work".into(), workspace: None });
        assert_eq!(decide(&two, Some("gone"), &Action::Allow), Plan::Choose, "a default that no longer exists is ignored");
        // A rule that names a profile wins over everything, with its workspace.
        let rule = Action::Profile { profile: "work".into(), workspace: Some("mail".into()) };
        assert_eq!(decide(&two, Some("default"), &rule), Plan::Launch { profile: "work".into(), workspace: Some("mail".into()) });
        // A rule naming a profile that does not exist is not followed.
        assert_eq!(decide(&two, None, &Action::Profile { profile: "gone".into(), workspace: None }), Plan::Choose);
        // A blocking rule drops the link; other actions (they refer to a profile's workspaces) do not decide.
        assert_eq!(decide(&two, Some("work"), &Action::Block), Plan::Ignore);
        assert_eq!(decide(&two, None, &Action::Workspace("mail".into())), Plan::Choose);
    }

    #[test]
    fn link_summaries() {
        assert_eq!(link_summary("https://example.com/path?q=1"), "example.com");
        assert_eq!(link_summary("http://localhost:8080/x"), "localhost:8080");
        assert_eq!(link_summary("mailto:a@b.c"), "mailto:a@b.c");
        assert_eq!(link_summary("https://a.example"), "a.example");
    }

    #[test]
    fn desktop_entry_is_the_web_browser_and_quotes_paths() {
        let d = desktop_entry(Path::new("/opt/my apps/browsah"), Path::new("/opt/my apps"));
        assert!(d.contains("Name=Browsah\n") && d.contains("select %U"));
        assert!(d.contains("x-scheme-handler/http;") && d.contains("x-scheme-handler/https;"));
        assert!(d.contains("StartupWMClass=be.geee.browsah\n") && d.contains("Icon=be.geee.browsah\n"));
        assert!(d.contains("\"/opt/my apps/browsah\""));
        assert!(icon_svg().starts_with("<svg"));
    }
}
