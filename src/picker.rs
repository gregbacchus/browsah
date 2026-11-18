//! The workspace picker's model: how its entries are ordered and what a colour badge shows (pure, tested). The popover is
//! `app/picker.rs`.

use crate::profile::ColorPair;
use std::collections::HashMap;

/// How the picker lists the workspaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Order {
    /// The most recently used first (the default).
    #[default]
    LastUsed,
    /// By name, `A` to `Z`.
    Alphabetical,
}

impl Order {
    /// The word stored in the profile's preferences.
    pub fn as_str(self) -> &'static str {
        match self {
            Order::LastUsed => "last_used",
            Order::Alphabetical => "alphabetical",
        }
    }

    /// The order stored by [`Order::as_str`]; anything else (or nothing) is the default.
    pub fn from_pref(value: Option<&str>) -> Order {
        match value {
            Some("alphabetical") => Order::Alphabetical,
            _ => Order::LastUsed,
        }
    }
}

/// One workspace in the picker.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub color: Option<ColorPair>,
}

/// Put the entries in order. `last_used` maps a workspace id to when it was last used (seconds); a workspace never used comes
/// after every used one, and ties (and the whole list, for `Alphabetical`) fall back to the name without regard to case, then the id.
pub fn sort(entries: &mut [Entry], order: Order, last_used: &HashMap<String, i64>) {
    let by_name = |a: &Entry, b: &Entry| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id));
    match order {
        Order::Alphabetical => entries.sort_by(by_name),
        Order::LastUsed => entries.sort_by(|a, b| {
            let (ta, tb) = (last_used.get(&a.id), last_used.get(&b.id));
            tb.cmp(&ta).then_with(|| by_name(a, b)) // `Some` sorts after `None`, so the reverse puts used ones first
        }),
    }
}

/// The colour of an entry's badge for the light or dark appearance, if the workspace has a colour.
pub fn badge_color(color: Option<&ColorPair>, dark: bool) -> Option<&str> {
    color.map(|c| if dark { c.dark.as_str() } else { c.light.as_str() })
}

/// A badge as an SVG: a disc in the workspace's colour with a faint outline so a dark or light colour shows on either
/// appearance; a workspace with no colour gets an empty ring.
pub fn badge_svg(color: Option<&str>) -> String {
    let ring = "stroke=\"#808080\" stroke-opacity=\"0.55\" stroke-width=\"1\"";
    let body = match color.map(|c| c.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '#').collect::<String>()) {
        Some(c) => format!("<circle cx=\"8\" cy=\"8\" r=\"6\" fill=\"{c}\" {ring}/>"),
        None => format!("<circle cx=\"8\" cy=\"8\" r=\"5.5\" fill=\"none\" {ring}/>"),
    };
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\">{body}</svg>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, name: &str) -> Entry {
        Entry { id: id.into(), name: name.into(), color: None }
    }

    fn ids(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn alphabetical_ignores_case_and_ties_use_the_id() {
        let mut v = vec![e("w", "work"), e("b", "Banking"), e("a2", "alpha"), e("a1", "Alpha"), e("z", "Zed")];
        sort(&mut v, Order::Alphabetical, &HashMap::new());
        assert_eq!(ids(&v), ["a1", "a2", "b", "w", "z"]);
    }

    #[test]
    fn last_used_puts_the_newest_first_and_never_used_last() {
        let mut v = vec![e("a", "Alpha"), e("b", "Beta"), e("c", "Gamma"), e("d", "Delta"), e("e", "Echo")];
        let used: HashMap<String, i64> = [("c".to_owned(), 300), ("a".to_owned(), 100), ("e".to_owned(), 300)].into();
        sort(&mut v, Order::LastUsed, &used);
        // c (Gamma) and e (Echo) tie at 300 and go by name, Echo first; then a; then the never-used b and d by name.
        assert_eq!(ids(&v), ["e", "c", "a", "b", "d"]);
        // With nothing recorded it is alphabetical.
        sort(&mut v, Order::LastUsed, &HashMap::new());
        assert_eq!(ids(&v), ["a", "b", "d", "e", "c"]);
    }

    #[test]
    fn the_order_round_trips_through_the_stored_word() {
        for o in [Order::LastUsed, Order::Alphabetical] {
            assert_eq!(Order::from_pref(Some(o.as_str())), o);
        }
        assert_eq!(Order::from_pref(None), Order::LastUsed);
        assert_eq!(Order::from_pref(Some("nonsense")), Order::LastUsed);
    }

    #[test]
    fn badges_follow_the_appearance_and_a_missing_colour_is_a_ring() {
        let c = ColorPair::parse("#4a2f80 #7a56c2").unwrap();
        assert_eq!(badge_color(Some(&c), false), Some("#4a2f80"));
        assert_eq!(badge_color(Some(&c), true), Some("#7a56c2"));
        assert_eq!(badge_color(None, true), None);
        assert!(badge_svg(Some("#4a2f80")).contains("fill=\"#4a2f80\""));
        assert!(badge_svg(None).contains("fill=\"none\""));
        // Nothing but a colour gets into the markup.
        assert!(!badge_svg(Some("#fff\"/><x")).contains("<x"));
    }
}
