//! Web apps: a workspace for one site (`app:` in `workspace.yaml`) that has its own icon and its own entry in
//! the desktop's launcher. This module has the desktop integration and the `browsah app` command line; the
//! window itself is an ordinary workspace window (see `workspace.rs`, `apply_app`).

use crate::profile::{self, Profile};
use crate::workspace::{first_positional, Workspace};
use std::path::{Path, PathBuf};

/// The application id of one web app: the profile's id and the app's, which is also the name of its
/// `.desktop` file, so the desktop groups the app's window under its own launcher entry and icon.
pub fn app_id(profile: &Profile, id: &str) -> String {
    format!("{}.{}", profile.app_id(), id)
}

/// The `.desktop` entry that launches a web app.
pub fn desktop_entry(profile: &Profile, w: &Workspace, exe: &Path, libs: &Path) -> String {
    let id = app_id(profile, &w.id);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Comment={name} (browsah, profile {profile_name})\n\
         Exec=env LD_LIBRARY_PATH={libs} {exe} --profile={pid} --workspace={wid}\n\
         Icon={id}\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass={id}\n\
         Categories=Network;\n",
        name = w.display_name(),
        profile_name = profile.display_name(),
        libs = profile::exec_quote(&libs.to_string_lossy()),
        exe = profile::exec_quote(&exe.to_string_lossy()),
        pid = profile.id,
        wid = w.id,
    )
}

/// Write the app's `.desktop` entry and icon (its own icon file if it names one, else a generated one).
pub fn install(profile: &Profile, w: &Workspace, exe: &Path, libs: &Path) -> std::io::Result<()> {
    let id = app_id(profile, &w.id);
    std::fs::create_dir_all(profile::applications_dir())?;
    std::fs::create_dir_all(profile::icons_dir())?;
    std::fs::write(profile::applications_dir().join(format!("{id}.desktop")), desktop_entry(profile, w, exe, libs))?;
    let own = w.config.app.as_ref().and_then(|a| a.icon.as_ref()).map(|f| w.dir.join(f)).filter(|p| p.is_file());
    remove_icons(&id);
    match own {
        Some(file) if file.extension().is_some_and(|e| e == "svg") => {
            std::fs::copy(&file, profile::icons_dir().join(format!("{id}.svg")))?;
        }
        Some(file) => {
            // A raster icon: the theme looks for it by size; this is the one we have.
            let dir = profile::icons_dir().join("../../256x256/apps");
            std::fs::create_dir_all(&dir)?;
            std::fs::copy(&file, dir.join(format!("{id}.png")))?;
        }
        None => {
            std::fs::write(profile::icons_dir().join(format!("{id}.svg")), profile::icon_svg_for(&w.id, w.display_name(), w.config.color.as_ref()))?;
        }
    }
    let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
    let _ = std::process::Command::new("gtk-update-icon-cache").args(["-q", "-t"]).arg(profile::icons_dir().join("../../..")).status();
    Ok(())
}

fn remove_icons(id: &str) {
    let _ = std::fs::remove_file(profile::icons_dir().join(format!("{id}.svg")));
    let _ = std::fs::remove_file(profile::icons_dir().join(format!("../../256x256/apps/{id}.png")));
}

pub fn uninstall(profile: &Profile, id: &str) {
    let app = app_id(profile, id);
    let _ = std::fs::remove_file(profile::applications_dir().join(format!("{app}.desktop")));
    remove_icons(&app);
    let _ = std::process::Command::new("update-desktop-database").arg(profile::applications_dir()).status();
}

// ---- command line -----------------------------------------------------------------------------

const USAGE: &str = "usage:
  browsah app add <id> --url URL [--name NAME] [--scope PATTERN[,PATTERN...]] [--display standalone|minimal|browser]
                       [--color \"LIGHT [DARK]\"] [--icon FILE] [--profile ID]
  browsah app list [--profile ID]
  browsah app remove <id> [--profile ID] --yes
  browsah app install <id> [--profile ID]      (write the launcher entry and icon again)

A web app is a workspace for one site (docs/config/webapp.md). The profile defaults to `default`.
Launch one with: browsah --profile=<profile> --workspace=<id>, or from the desktop's launcher.";

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            return Some(v.to_owned());
        }
    }
    None
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The text of a new web app's `workspace.yaml`.
pub fn config_text(name: Option<&str>, color: Option<&profile::ColorPair>, url: &str, scope: &[String], display: &str, icon: Option<&str>, manifest_id: Option<&str>) -> String {
    let mut t = String::from("# Web app. Options: docs/config/webapp.md and docs/config/workspace.md\n");
    if let Some(n) = name {
        t.push_str(&format!("name: {}\n", quote(n)));
    }
    if let Some(c) = color {
        t.push_str(&format!("color:\n  light: \"{}\"\n  dark: \"{}\"\n", c.light, c.dark));
    }
    t.push_str(&format!("app:\n  url: {}\n", quote(url)));
    if !scope.is_empty() {
        t.push_str(&format!("  scope: [{}]\n", scope.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ")));
    }
    t.push_str(&format!("  display: {display}\n"));
    if let Some(i) = icon {
        t.push_str(&format!("  icon: {}\n", quote(i)));
    }
    if let Some(i) = manifest_id {
        t.push_str(&format!("  id: {}\n", quote(i)));
    }
    t
}

/// An id made from a name: lower case letters and digits, `-` between words, starting with a letter.
pub fn id_from_name(name: &str) -> String {
    let mut id: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_owned();
    if id.chars().next().is_some_and(|c| c.is_ascii_lowercase()) { id } else { format!("app-{id}").trim_end_matches('-').to_owned() }
}

/// `browsah app ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let profile_id = flag_value(args, "--profile").unwrap_or_else(|| profile::DEFAULT_ID.to_owned());
    let profile = match Profile::resolve(&profile_id) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find this program: {e}");
            return 1;
        }
    };
    let libs: PathBuf = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("add") => add(&profile, &args[1..], &exe, &libs),
        Some("list") => {
            for w in Workspace::list(&profile).into_iter().filter(Workspace::is_app) {
                let a = w.config.app.as_ref().expect("an app");
                println!("{:<16} {:<20} {}", w.id, w.display_name(), a.url);
            }
            0
        }
        Some("install") => match first_positional(&args[1..]).map(|id| Workspace::resolve(&profile, id)) {
            Some(Ok(w)) if w.is_app() => match install(&profile, &w, &exe, &libs) {
                Ok(()) => {
                    println!("installed the launcher entry {}.desktop", app_id(&profile, &w.id));
                    0
                }
                Err(e) => {
                    eprintln!("cannot install: {e}");
                    1
                }
            },
            Some(Ok(w)) => {
                eprintln!("{:?} is a workspace, not a web app", w.id);
                1
            }
            Some(Err(e)) => {
                eprintln!("{e}");
                1
            }
            None => {
                eprintln!("{USAGE}");
                2
            }
        },
        Some("remove") => remove(&profile, &args[1..]),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

fn add(profile: &Profile, args: &[String], exe: &Path, libs: &Path) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    if !profile::valid_id(id) {
        eprintln!("{id:?} is not a valid id (lower case letters, digits, - and _, starting with a letter)");
        return 2;
    }
    let Some(url) = flag_value(args, "--url") else {
        eprintln!("--url is required\n{USAGE}");
        return 2;
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        eprintln!("--url must be an http or https address");
        return 2;
    }
    let display = flag_value(args, "--display").unwrap_or_else(|| "standalone".into());
    if !["standalone", "minimal", "browser"].contains(&display.as_str()) {
        eprintln!("--display must be standalone, minimal or browser");
        return 2;
    }
    let color = match flag_value(args, "--color").map(|c| profile::ColorPair::parse(&c)) {
        Some(Err(e)) => {
            eprintln!("--color: {e}");
            return 2;
        }
        Some(Ok(c)) => Some(c),
        None => None,
    };
    let dir = profile.workspaces_dir().join(id);
    if dir.join("workspace.yaml").exists() {
        eprintln!("{id:?} already exists in profile {:?}", profile.id);
        return 1;
    }
    let scope: Vec<String> = flag_value(args, "--scope").map(|s| s.split(',').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let icon_name = match flag_value(args, "--icon") {
        Some(src) => {
            let ext = Path::new(&src).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
            if !["png", "svg"].contains(&ext.as_str()) {
                eprintln!("--icon must be a png or svg file");
                return 2;
            }
            if !Path::new(&src).is_file() {
                eprintln!("--icon: {src} is not a file");
                return 2;
            }
            Some((src, format!("icon.{ext}")))
        }
        None => None,
    };
    let text = config_text(flag_value(args, "--name").as_deref(), color.as_ref(), &url, &scope, &display, icon_name.as_ref().map(|(_, n)| n.as_str()), None);
    let result = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join("workspace.yaml"), text))
        .and_then(|_| match &icon_name {
            Some((src, name)) => std::fs::copy(src, dir.join(name)).map(|_| ()),
            None => Ok(()),
        });
    if let Err(e) = result {
        eprintln!("cannot create the web app: {e}");
        return 1;
    }
    let w = match Workspace::resolve(profile, id) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if let Err(e) = install(profile, &w, exe, libs) {
        eprintln!("created the web app, but cannot write its launcher entry: {e}");
        return 1;
    }
    println!("created web app {id:?} in profile {:?} ({})", profile.id, dir.display());
    println!("launch it from the desktop's launcher, or with: browsah --profile={} --workspace={id}", profile.id);
    0
}

fn remove(profile: &Profile, args: &[String]) -> i32 {
    let Some(id) = first_positional(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    match Workspace::resolve(profile, id) {
        Ok(w) if w.is_app() => {
            if !args.iter().any(|a| a == "--yes") {
                eprintln!("this DELETES web app {id:?} ({}) and its launcher entry.\nrun again with --yes to continue", w.dir.display());
                return 1;
            }
            let code = crate::workspace::cli(&["remove".into(), id.clone(), format!("--profile={}", profile.id), "--yes".into()]);
            if code == 0 {
                uninstall(profile, id);
            }
            code
        }
        Ok(_) => {
            eprintln!("{id:?} is a workspace, not a web app (remove it with: browsah workspace remove)");
            1
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_from_names() {
        assert_eq!(id_from_name("Google Mail"), "google-mail");
        assert_eq!(id_from_name("  Mail & Calendar!! "), "mail-calendar");
        assert_eq!(id_from_name("365 Office"), "app-365-office");
        assert!(profile::valid_id(&id_from_name("Ünïcode Tool")));
    }

    #[test]
    fn written_config_reads_back() {
        let color = profile::ColorPair::parse("#2a6f97 #61a5c2").unwrap();
        let text = config_text(Some("My \"Mail\""), Some(&color), "https://mail.example/inbox", &["mail.example".into(), "login.example/*".into()], "minimal", Some("icon.png"), Some("https://mail.example/"));
        let c = crate::workspace::parse_config(&text).unwrap();
        let a = c.app.as_ref().unwrap();
        assert_eq!((a.url.as_str(), a.scope.len(), a.icon.as_deref(), a.id.as_deref()), ("https://mail.example/inbox", 2, Some("icon.png"), Some("https://mail.example/")));
        assert_eq!(c.name.as_deref(), Some("My \"Mail\""));
        assert!(c.color.is_some());
    }

    #[test]
    fn desktop_entry_names_the_app_and_launches_its_workspace() {
        let p = Profile { id: "work".into(), config: Default::default(), dir: "/tmp/p".into() };
        let w = Workspace { id: "mail".into(), config: crate::workspace::parse_config("name: Mail\napp:\n  url: https://m.example/\n").unwrap(), dir: "/tmp/p/workspaces/mail".into() };
        let e = desktop_entry(&p, &w, Path::new("/opt/browsah/browsah"), Path::new("/opt/browsah"));
        assert!(e.contains("Name=Mail\n") && e.contains("--profile=work --workspace=mail") && e.contains("Icon=be.geee.browsah.work.mail\n") && e.contains("StartupWMClass=be.geee.browsah.work.mail\n"), "{e}");
        assert_eq!(app_id(&p, "mail"), "be.geee.browsah.work.mail");
    }
}
