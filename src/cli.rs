//! Command-line options. Anything unrecognised that starts with `--` is passed to Chromium.

/// `browsah --help`, `-h` or `help` (printed to stdout, exit 0).
pub const HELP: &str = "browsah: a GTK4 web browser around Chromium (CEF), native Wayland

usage:
  browsah [OPTIONS] [URL | HOST | SEARCH WORDS]     start the browser (or hand the link to the running profile)
  browsah COMMAND [ARGS]                            manage profiles, apps and workspaces, no window needed

options:
  --profile=ID        the profile to run (or `--profile ID`; else BROWSAH_PROFILE, then `default_profile` in config.yaml, then `default`)
  --workspace=ID      open this workspace of the profile
  --app=ID            open this web app of the profile
  --new-window        open a window with no workspace instead of the remembered ones
  --url=URL           the page to open (a bare argument works too)
  --config=PATH       use this application config file
  --no-watch          do not reload config files when they change
  --mode=paint|accel  rendering path (readback or zero-copy dmabuf); overrides `rendering.mode`
  --backing=off       render at 1x instead of device resolution (a diagnostic)
  --selftest          run the scripted self-test and exit (BROWSAH_SELFTEST_ONLY=a,b names sections)
  --latency           measure input latency and exit
  --bench=single|tabs|fps [--tabs=N] [--tab-mode=live|static] [--seconds=N]   rendering benchmarks
  -h, --help, help    show this help
  -V, --version       show the version
  --OTHER             any other --switch is passed to Chromium

commands (all take --profile ID where a profile matters, before or after the command; without it: `default_profile` from config.yaml, else `default`):
  profile add <id> [--name NAME] [--color \"LIGHT [DARK]\"]     | list | install <id> | remove <id> --yes [--keep-data]
  workspace add <id> [--name NAME] [--color ...]               | list | remove <id> --yes
  app add [<id>] --url URL [--name N] [--scope P,...] [--display standalone|minimal|browser] [--color ...] [--icon FILE]
  app update <id> [--apply]                      preview or apply web manifest changes
  system-install [--1password] [--dry-run] [--yes] | system-uninstall      (run with sudo) a root-owned copy in /opt/browsah, for 1Password
  codecs install | refresh | status | uninstall                optional personal H.264/AAC CEF runtime (refresh: after an upgrade, no download)
  install refresh [--dry-run] [--yes] [--force] [--from=DIR]    after an upgrade: update the root-owned copy and the codec runtimes in one go
  system-upgrade [--dry-run] [--yes] [--force] [--from=DIR]                update that copy from the one it came from (it runs sudo itself)
  extension add <id | Chrome Web Store address> | list | update [id] | remove <id> --yes     Chrome extensions
  app list | install <id> | route <id> [--write | --remove] | remove <id> --yes
  config show [--profile=ID] [--config=PATH]                  the effective configuration and where each value came from
  config full [--profile=ID [--workspace=ID]]                  a file with every option, explained
  route test <url> [--from SOURCE] [--kind KIND] [--target T] [--explain]    which routing rule a link would match
  permissions list | reset [--origin URL] --yes                remembered permission answers
  history [QUERY] [--limit N]  | history clear [--older-than-days N] --yes
  log [--lines=N] [--grep=TEXT] [--level=L] [--follow]          the profile's log from the system journal (what happened, and why)
  sweep [--workspace ID] [--dry-run]                           apply sweeping rules to a profile that is not running
  select [URL]                                                 choose a profile for a link (the default-browser entry)
  selector install | uninstall | status                        the default-browser entry and its icon

Everything is documented in the Documentation window (main menu) and in docs/ (docs/README.md).
Needs LD_LIBRARY_PATH to include the directory with libcef.so, unless started from the brew launcher or a desktop entry.";

/// The root options handled before anything starts: the help text or the version, with the exit code, or `None`.
pub fn root_info(args: &[String]) -> Option<(String, i32)> {
    match args.first().map(String::as_str)? {
        "-h" | "--help" | "help" => Some((HELP.to_owned(), 0)),
        "-V" | "--version" => Some((format!("browsah {}", env!("CARGO_PKG_VERSION")), 0)),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `OnPaint`: GPU readback into CPU memory.
    Paint,
    /// `OnAcceleratedPaint`: dmabuf, zero copy.
    Accel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bench {
    None,
    Single,
    Tabs,
    Fps,
}

#[derive(Debug)]
pub struct Options {
    /// Rendering path; overrides `rendering.mode` from the config.
    pub mode: Option<Mode>,
    pub bench: Bench,
    /// Page to open instead of the configured home page.
    pub url: Option<String>,
    /// Benchmarks: stop after this many seconds and print a summary (0 = run until closed).
    pub seconds: u64,
    /// Benchmarks: number of tabs (`--tabs=N`).
    pub tabs: usize,
    /// Page variant for the tab benchmarks: `live` (animated, timers) or `static` (idle).
    pub tab_mode: String,
    /// Overrides `rendering.device_resolution` (`--backing=off` renders at 1x).
    pub backing: Option<bool>,
    /// Run the scripted navigation check and exit (`--selftest`).
    pub selftest: bool,
    /// Measure input latency and exit (`--latency`).
    pub latency: bool,
    /// Profile to run (`--profile=ID`, or `BROWSAH_PROFILE`). Default: `default`; the self-test,
    /// latency and benchmark runs use the throwaway `selftest` profile.
    pub profile: Option<String>,
    /// Workspace to open (`--workspace=ID`).
    pub workspace: Option<String>,
    /// `workspace` names a web app (`--app=ID`), not a workspace.
    pub app: bool,
    /// Open a window with no workspace instead of the remembered workspaces (`--new-window`).
    pub new_window: bool,
    /// Do not reload config files when they change (`--no-watch`).
    pub no_watch: bool,
    /// Alternative config file (`--config=PATH`).
    pub config_path: Option<String>,
    /// Extra Chromium switches given on the command line.
    pub flags: Vec<String>,
}

impl Options {
    pub fn parse() -> Self {
        Self::parse_from(std::env::args().skip(1), std::env::var("BROWSAH_PROFILE").ok())
    }

    /// The options in `args` (without the program name); `env_profile` is `BROWSAH_PROFILE`, which `--profile` overrides.
    pub fn parse_from(args: impl IntoIterator<Item = String>, env_profile: Option<String>) -> Self {
        let mut o = Options {
            mode: None,
            bench: Bench::None,
            url: None,
            seconds: 0,
            tabs: 0,
            tab_mode: "live".into(),
            backing: None,
            selftest: false,
            latency: false,
            profile: env_profile.filter(|p| !p.is_empty()),
            workspace: None,
            app: false,
            new_window: false,
            no_watch: false,
            config_path: None,
            flags: Vec::new(),
        };
        let mut args = args.into_iter();
        while let Some(a) = args.next() {
            match a.as_str() {
                // `--profile ID` (a separate word) as well as `--profile=ID`.
                "--profile" => {
                    if let Some(id) = args.next() {
                        o.profile = Some(id);
                    }
                }
                "--mode=paint" => o.mode = Some(Mode::Paint),
                "--mode=accel" => o.mode = Some(Mode::Accel),
                "--bench=single" => o.bench = Bench::Single,
                "--bench=tabs" => o.bench = Bench::Tabs,
                "--bench=fps" => o.bench = Bench::Fps,
                "--backing=off" => o.backing = Some(false),
                "--selftest" => o.selftest = true,
                "--no-watch" => o.no_watch = true,
                "--new-window" => o.new_window = true,
                "--latency" => o.latency = true,
                _ if a.starts_with("--tabs=") => o.tabs = a[7..].parse().unwrap_or(0),
                _ if a.starts_with("--tab-mode=") => o.tab_mode = a[11..].to_string(),
                _ if a.starts_with("--url=") => o.url = Some(a[6..].to_string()),
                _ if a.starts_with("--seconds=") => o.seconds = a[10..].parse().unwrap_or(0),
                _ if a.starts_with("--workspace=") => o.workspace = Some(a[12..].to_string()),
                _ if a.starts_with("--app=") => {
                    o.workspace = Some(a[6..].to_string());
                    o.app = true;
                }
                _ if a.starts_with("--profile=") => o.profile = Some(a[10..].to_string()),
                _ if a.starts_with("--config=") => o.config_path = Some(a[9..].to_string()),
                _ if a.starts_with("--") => o.flags.push(a),
                // A bare argument is a page to open.
                _ if o.url.is_none() => o.url = Some(a),
                _ => {}
            }
        }
        if matches!(o.bench, Bench::Tabs | Bench::Fps) && o.tabs == 0 {
            o.tabs = 10;
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_and_version_are_answered_before_anything_starts() {
        for flag in ["--help", "-h", "help"] {
            let (text, code) = root_info(&a(&[flag])).expect(flag);
            assert_eq!(code, 0);
            assert!(text.contains("usage:") && text.contains("commands"));
        }
        let (v, _) = root_info(&a(&["--version"])).unwrap();
        assert_eq!(v, format!("browsah {}", env!("CARGO_PKG_VERSION")));
        assert!(root_info(&a(&["-V"])).is_some());
        // Nothing else is intercepted: a URL, a subcommand, a Chromium switch.
        for other in [&["example.com"][..], &["profile", "--help"], &["--disable-gpu"], &[]] {
            assert!(root_info(&a(other)).is_none(), "{other:?}");
        }
    }

    #[test]
    fn profile_option_takes_both_spellings_and_beats_the_environment() {
        let p = |v: &[&str], env: Option<&str>| Options::parse_from(a(v), env.map(str::to_owned)).profile;
        assert_eq!(p(&["--profile=work"], None).as_deref(), Some("work"));
        assert_eq!(p(&["--profile", "work", "example.com"], None).as_deref(), Some("work"));
        assert_eq!(Options::parse_from(a(&["--profile", "work", "example.com"]), None).url.as_deref(), Some("example.com"));
        assert_eq!(p(&["--profile=work"], Some("other")).as_deref(), Some("work"));
        assert_eq!(p(&[], Some("other")).as_deref(), Some("other"));
        assert_eq!(p(&[], None), None);
    }

    #[test]
    fn help_names_every_command_and_option() {
        for command in ["profile", "workspace", "app", "config", "route", "permissions", "history", "sweep", "select", "selector"] {
            assert!(HELP.contains(&format!("\n  {command} ")), "{command}");
        }
        for option in ["--profile=", "--workspace=", "--app=", "--new-window", "--url=", "--config=", "--no-watch", "--mode=", "--backing=off", "--selftest", "--latency", "--bench="] {
            assert!(HELP.contains(option), "{option}");
        }
    }
}
