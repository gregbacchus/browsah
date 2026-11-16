//! Command-line options. Anything unrecognised that starts with `--` is passed to Chromium.

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
    /// Do not reload config files when they change (`--no-watch`).
    pub no_watch: bool,
    /// Alternative config file (`--config=PATH`).
    pub config_path: Option<String>,
    /// Extra Chromium switches given on the command line.
    pub flags: Vec<String>,
}

impl Options {
    pub fn parse() -> Self {
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
            profile: std::env::var("BROWSAH_PROFILE").ok().filter(|p| !p.is_empty()),
            no_watch: false,
            config_path: None,
            flags: Vec::new(),
        };
        for a in std::env::args().skip(1) {
            match a.as_str() {
                "--mode=paint" => o.mode = Some(Mode::Paint),
                "--mode=accel" => o.mode = Some(Mode::Accel),
                "--bench=single" => o.bench = Bench::Single,
                "--bench=tabs" => o.bench = Bench::Tabs,
                "--bench=fps" => o.bench = Bench::Fps,
                "--backing=off" => o.backing = Some(false),
                "--selftest" => o.selftest = true,
                "--no-watch" => o.no_watch = true,
                "--latency" => o.latency = true,
                _ if a.starts_with("--tabs=") => o.tabs = a[7..].parse().unwrap_or(0),
                _ if a.starts_with("--tab-mode=") => o.tab_mode = a[11..].to_string(),
                _ if a.starts_with("--url=") => o.url = Some(a[6..].to_string()),
                _ if a.starts_with("--seconds=") => o.seconds = a[10..].parse().unwrap_or(0),
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
