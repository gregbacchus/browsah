//! Permissions: what a page may do (use the camera, know your location, show notifications...).
//!
//! `permissions:` in the configuration (application, profile or workspace layer) says, per kind of
//! permission and optionally per site, whether to **allow**, **deny** or **ask**. This module is the
//! model: the kinds, how a request is judged against the configuration, and the mapping from Chromium's
//! permission flags. Asking, and remembering the answer, are in `app/permissions.rs` and the store.

use crate::workspace::scope_matches;
use serde::{Deserialize, Serialize};

/// What to do about a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Choice {
    Allow,
    Deny,
    Ask,
}

/// A kind of permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Camera,
    Microphone,
    Location,
    Notifications,
    Clipboard,
    /// Sharing the screen or a window.
    Screen,
    Midi,
    /// Anything else Chromium asks about.
    Other,
}

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::Camera, Kind::Microphone, Kind::Location, Kind::Notifications, Kind::Clipboard, Kind::Screen, Kind::Midi, Kind::Other];

    /// The name used in the configuration and the permissions list.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Camera => "camera",
            Kind::Microphone => "microphone",
            Kind::Location => "location",
            Kind::Notifications => "notifications",
            Kind::Clipboard => "clipboard",
            Kind::Screen => "screen",
            Kind::Midi => "midi",
            Kind::Other => "other",
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// What the page wants to do, for a question ("wants to ...").
    pub fn wants(self) -> &'static str {
        match self {
            Kind::Camera => "use your camera",
            Kind::Microphone => "use your microphone",
            Kind::Location => "know your location",
            Kind::Notifications => "show notifications",
            Kind::Clipboard => "read what you copied",
            Kind::Screen => "share your screen",
            Kind::Midi => "use your MIDI devices",
            Kind::Other => "use a browser feature that needs permission",
        }
    }

    /// The kinds in a media-access request (Chromium's bit set).
    pub fn from_media_mask(mask: u32) -> Vec<Kind> {
        let mut out = Vec::new();
        if mask & MEDIA_VIDEO != 0 {
            out.push(Kind::Camera);
        }
        if mask & MEDIA_AUDIO != 0 {
            out.push(Kind::Microphone);
        }
        if mask & (MEDIA_DESKTOP_AUDIO | MEDIA_DESKTOP_VIDEO) != 0 {
            out.push(Kind::Screen);
        }
        out
    }

    /// The media bits a kind stands for.
    pub fn media_bits(self) -> u32 {
        match self {
            Kind::Camera => MEDIA_VIDEO,
            Kind::Microphone => MEDIA_AUDIO,
            Kind::Screen => MEDIA_DESKTOP_AUDIO | MEDIA_DESKTOP_VIDEO,
            _ => 0,
        }
    }

    /// The kinds in a permission-prompt request (Chromium's bit set). Anything not recognised is `Other`.
    pub fn from_prompt_mask(mask: u32) -> Vec<Kind> {
        const CAMERA: u32 = 2 | 4; // pan-tilt-zoom, camera stream
        const MIC: u32 = 4096;
        const LOCATION: u32 = 256;
        const NOTIFICATIONS: u32 = 32768;
        const CLIPBOARD: u32 = 16;
        const MIDI: u32 = 8192;
        let known = CAMERA | MIC | LOCATION | NOTIFICATIONS | CLIPBOARD | MIDI;
        let mut out = Vec::new();
        for (bits, kind) in [(CAMERA, Kind::Camera), (MIC, Kind::Microphone), (LOCATION, Kind::Location), (NOTIFICATIONS, Kind::Notifications), (CLIPBOARD, Kind::Clipboard), (MIDI, Kind::Midi)] {
            if mask & bits != 0 {
                out.push(kind);
            }
        }
        if mask & !known != 0 || out.is_empty() {
            out.push(Kind::Other);
        }
        out
    }
}

// Chromium's media-access bits (`cef_media_access_permission_types_t`).
const MEDIA_AUDIO: u32 = 1;
const MEDIA_VIDEO: u32 = 2;
const MEDIA_DESKTOP_AUDIO: u32 = 4;
const MEDIA_DESKTOP_VIDEO: u32 = 8;

/// The `permissions:` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct PermissionsConfig {
    /// For any kind with no setting of its own.
    pub default: Choice,
    pub camera: Option<Choice>,
    pub microphone: Option<Choice>,
    pub location: Option<Choice>,
    pub notifications: Option<Choice>,
    pub clipboard: Option<Choice>,
    pub screen: Option<Choice>,
    pub midi: Option<Choice>,
    /// Rules for particular sites; the first matching rule that says something about a kind decides it.
    pub sites: Vec<SiteConfig>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self { default: Choice::Ask, camera: None, microphone: None, location: None, notifications: None, clipboard: None, screen: None, midi: None, sites: Vec::new() }
    }
}

/// A rule for particular sites (`permissions.sites[]`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
#[cfg_attr(test, serde(deny_unknown_fields))] // tests catch typos in the examples
pub struct SiteConfig {
    /// Address patterns, as for a pinned tab's scope (`meet.example.com`, `*.zoom.us`).
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    pub default: Option<Choice>,
    pub camera: Option<Choice>,
    pub microphone: Option<Choice>,
    pub location: Option<Choice>,
    pub notifications: Option<Choice>,
    pub clipboard: Option<Choice>,
    pub screen: Option<Choice>,
    pub midi: Option<Choice>,
}

fn pick(kind: Kind, default: Option<Choice>, camera: Option<Choice>, microphone: Option<Choice>, location: Option<Choice>, notifications: Option<Choice>, clipboard: Option<Choice>, screen: Option<Choice>, midi: Option<Choice>) -> Option<Choice> {
    match kind {
        Kind::Camera => camera,
        Kind::Microphone => microphone,
        Kind::Location => location,
        Kind::Notifications => notifications,
        Kind::Clipboard => clipboard,
        Kind::Screen => screen,
        Kind::Midi => midi,
        Kind::Other => None,
    }
    .or(default)
}

impl SiteConfig {
    fn says(&self, kind: Kind) -> Option<Choice> {
        pick(kind, self.default, self.camera, self.microphone, self.location, self.notifications, self.clipboard, self.screen, self.midi)
    }
}

impl PermissionsConfig {
    /// What the configuration says about `kind` for a page at `origin` (like `https://meet.example.com`).
    pub fn decide(&self, origin: &str, kind: Kind) -> Choice {
        let url = if origin.matches('/').count() <= 2 { format!("{origin}/") } else { origin.to_owned() };
        for site in &self.sites {
            if site.matches.iter().any(|p| scope_matches(p, &url)) {
                if let Some(c) = site.says(kind) {
                    return c;
                }
            }
        }
        pick(kind, None, self.camera, self.microphone, self.location, self.notifications, self.clipboard, self.screen, self.midi).unwrap_or(self.default)
    }
}

/// Answers remembered from questions, by origin and kind, for the code that runs on CEF's thread (the database is the GTK thread's).
/// Filled at start-up and when an answer is remembered.
static REMEMBERED: std::sync::Mutex<Option<std::collections::HashMap<(String, String), bool>>> = std::sync::Mutex::new(None);

pub fn note_remembered(origin: &str, kind: &str, allow: bool) {
    if let Ok(mut g) = REMEMBERED.lock() {
        g.get_or_insert_with(Default::default).insert((origin.trim_end_matches('/').to_owned(), kind.to_owned()), allow);
    }
}

fn remembered(origin: &str, kind: &str) -> Option<bool> {
    REMEMBERED.lock().ok()?.as_ref()?.get(&(origin.trim_end_matches('/').to_owned(), kind.to_owned())).copied()
}

/// What Chromium's own setting for `kind` at `origin` should be, so that a page that looks (`Notification.permission`) sees what
/// browsah will answer: `Some(true)` granted, `Some(false)` denied, `None` still to be asked (Chromium's default). The
/// configuration decides, then an answer remembered from a question. Covers the kinds a page can read without asking
/// (notifications, location, clipboard, MIDI) and camera and microphone: a page that looks first (`permissions.query`, or
/// `enumerateDevices`, which hides device ids and labels until the microphone/camera is granted) must see what the media request
/// will be answered, or it concludes there is no device (Microsoft Teams shows "no microphone/camera found"). Screen capture is not
/// mirrored (setting its content setting crashes Chromium).
pub fn mirrored_choice(cfg: &PermissionsConfig, origin: &str, kind: Kind) -> Option<bool> {
    if !matches!(kind, Kind::Notifications | Kind::Location | Kind::Clipboard | Kind::Midi | Kind::Camera | Kind::Microphone) {
        return None;
    }
    match cfg.decide(origin, kind) {
        Choice::Allow => Some(true),
        Choice::Deny => Some(false),
        Choice::Ask => remembered(origin, kind.name()),
    }
}

/// The host of an origin, for messages.
pub fn origin_host(origin: &str) -> &str {
    origin.split_once("://").map(|(_, r)| r).unwrap_or(origin).split(['/', '?']).next().unwrap_or(origin)
}

// ---- `browsah permissions` ----------------------------------------------------------------------------

const USAGE: &str = "usage:
  browsah permissions list [--profile ID]
  browsah permissions reset [--origin URL] [--profile ID] --yes

`list` shows what has been remembered from questions (site, kind, answer). `reset` forgets one site's
answers (--origin, as shown by `list`) or all of them; it is refused while the profile is running.";

/// `browsah permissions ...`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned)).or_else(|| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()));
    let profile = match crate::profile::Profile::resolve(&value("--profile").unwrap_or_else(|| crate::profile::DEFAULT_ID.to_owned())) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let db = profile.dir.join("data.sqlite");
    let sub = args.first().map(String::as_str);
    if !matches!(sub, Some("list" | "reset")) {
        eprintln!("{USAGE}");
        return 2;
    }
    if !db.exists() {
        println!("nothing remembered yet");
        return 0;
    }
    let store = match crate::store::Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if sub == Some("list") {
        let all = store.remembered_permissions();
        if all.is_empty() {
            println!("nothing remembered yet");
        }
        for (origin, kind, choice, _) in all {
            println!("{origin:<40} {kind:<14} {choice}");
        }
        return 0;
    }
    if !args.iter().any(|a| a == "--yes") {
        eprintln!("this forgets remembered permission answers of profile {:?}; run again with --yes\n\n{USAGE}", profile.id);
        return 1;
    }
    if crate::profile::is_running(&profile) {
        eprintln!("profile {:?} is running; close it first", profile.id);
        return 1;
    }
    match store.forget_permissions(value("--origin").as_deref()) {
        Ok(n) => {
            println!("forgot {n} answer{}", if n == 1 { "" } else { "s" });
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_chromium_is_told_follows_the_configuration_then_what_was_remembered() {
        let cfg: PermissionsConfig = serde_yaml_ng::from_str("default: ask\nnotifications: allow\nlocation: deny\nsites:\n  - match: ['blocked.example']\n    notifications: deny\n").unwrap();
        assert_eq!(mirrored_choice(&cfg, "https://a.example", Kind::Notifications), Some(true));
        assert_eq!(mirrored_choice(&cfg, "https://blocked.example", Kind::Notifications), Some(false), "a site rule wins");
        assert_eq!(mirrored_choice(&cfg, "https://a.example", Kind::Location), Some(false));
        // Ask: nothing known, then a remembered answer (with or without the trailing slash Chromium gives).
        assert_eq!(mirrored_choice(&cfg, "https://asked.example", Kind::Clipboard), None);
        note_remembered("https://asked.example/", "clipboard", true);
        assert_eq!(mirrored_choice(&cfg, "https://asked.example", Kind::Clipboard), Some(true));
        note_remembered("https://asked.example", "clipboard", false);
        assert_eq!(mirrored_choice(&cfg, "https://asked.example/", Kind::Clipboard), Some(false), "the last answer stands");
        // Camera and microphone are mirrored too (Teams reads them before it asks); screen capture is not.
        let allow_all: PermissionsConfig = serde_yaml_ng::from_str("default: allow").unwrap();
        assert_eq!(mirrored_choice(&allow_all, "https://a.example", Kind::Camera), Some(true));
        assert_eq!(mirrored_choice(&allow_all, "https://a.example", Kind::Microphone), Some(true));
        assert_eq!(mirrored_choice(&allow_all, "https://a.example", Kind::Screen), None);
        let teams: PermissionsConfig = serde_yaml_ng::from_str("sites:\n  - match: ['teams.microsoft.com', 'teams.live.com']\n    camera: allow\n    microphone: allow\n").unwrap();
        assert_eq!(mirrored_choice(&teams, "https://teams.live.com", Kind::Camera), Some(true));
        assert_eq!(mirrored_choice(&teams, "https://teams.microsoft.com", Kind::Microphone), Some(true));
        assert_eq!(mirrored_choice(&teams, "https://other.example", Kind::Microphone), None, "ask: left to the handler");
        // A host written as `*.x` does not cover `x` itself, so a rule meant for Teams must name or `**.` it.
        let wild: PermissionsConfig = serde_yaml_ng::from_str("sites:\n  - match: ['*.microsoft.com']\n    microphone: allow\n").unwrap();
        assert_eq!(wild.decide("https://teams.microsoft.com", Kind::Microphone), Choice::Allow);
        assert_eq!(wild.decide("https://microsoft.com", Kind::Microphone), Choice::Ask);
        assert_eq!(mirrored_choice(&allow_all, "https://a.example", Kind::Notifications), Some(true));
    }

    use super::*;

    fn cfg(yaml: &str) -> PermissionsConfig {
        serde_yaml_ng::from_str(yaml).unwrap()
    }

    #[test]
    fn defaults_to_asking() {
        let c = PermissionsConfig::default();
        for k in Kind::ALL {
            assert_eq!(c.decide("https://a.example", k), Choice::Ask);
        }
    }

    #[test]
    fn kinds_override_the_default_and_sites_override_kinds() {
        let c = cfg("default: deny\ncamera: ask\nnotifications: allow\nsites:\n  - match: ['meet.example.com', '*.zoom.us']\n    camera: allow\n    microphone: allow\n  - match: ['news.example.com']\n    default: deny\n    notifications: allow\n  - match: ['**.example.com']\n    notifications: ask\n");
        assert_eq!(c.decide("https://x.org", Kind::Location), Choice::Deny, "the default");
        assert_eq!(c.decide("https://x.org", Kind::Camera), Choice::Ask, "a kind");
        assert_eq!(c.decide("https://x.org", Kind::Notifications), Choice::Allow);
        assert_eq!(c.decide("https://meet.example.com", Kind::Camera), Choice::Allow, "a site beats the kind");
        assert_eq!(c.decide("https://us.zoom.us", Kind::Microphone), Choice::Allow);
        assert_eq!(c.decide("https://meet.example.com", Kind::Location), Choice::Deny, "a site that says nothing about a kind: the kind's setting or the default");
        // A matching rule that does not mention the kind lets later rules decide.
        assert_eq!(c.decide("https://meet.example.com", Kind::Notifications), Choice::Ask, "the third rule, not the kind");
        // The first rule that says something wins; a site `default` covers the other kinds.
        assert_eq!(c.decide("https://news.example.com", Kind::Notifications), Choice::Allow);
        assert_eq!(c.decide("https://news.example.com", Kind::Camera), Choice::Deny, "its own default, not the kind's ask");
        // The port and path do not matter, and the origin form is accepted.
        assert_eq!(c.decide("https://meet.example.com:8443", Kind::Camera), Choice::Allow);
    }

    #[test]
    fn chromium_flags_map_to_kinds() {
        assert_eq!(Kind::from_media_mask(1), [Kind::Microphone]);
        assert_eq!(Kind::from_media_mask(2), [Kind::Camera]);
        assert_eq!(Kind::from_media_mask(3), [Kind::Camera, Kind::Microphone]);
        assert_eq!(Kind::from_media_mask(4 | 8), [Kind::Screen]);
        assert_eq!(Kind::from_prompt_mask(32768), [Kind::Notifications]);
        assert_eq!(Kind::from_prompt_mask(256), [Kind::Location]);
        assert_eq!(Kind::from_prompt_mask(4 | 4096), [Kind::Camera, Kind::Microphone]);
        assert_eq!(Kind::from_prompt_mask(1048576), [Kind::Other], "a flag we do not name");
        assert_eq!(Kind::from_prompt_mask(256 | 1048576), [Kind::Location, Kind::Other]);
        assert_eq!(Kind::from_prompt_mask(0), [Kind::Other]);
        assert_eq!(Kind::Screen.media_bits(), 12);
        assert_eq!(Kind::Location.media_bits(), 0);
    }

    #[test]
    fn names_round_trip() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.name()), Some(k));
        }
        assert_eq!(Kind::parse("teleport"), None);
        assert_eq!(origin_host("https://meet.example.com:8443/x"), "meet.example.com:8443");
        assert_eq!(origin_host("file://"), "");
    }
}
