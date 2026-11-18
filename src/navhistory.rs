//! A tab's back and forward list, kept with the remembered tabs so it survives closing the window and a tab being put to sleep.
//!
//! CEF has no call to put entries into a browser's navigation list, so a restored tab starts with one entry (its page) and the rest
//! is kept here as two stacks next to it: `back` (the entry nearest the current page is last) and `forward` (the nearest is last).
//! The shell uses the back stack when the browser itself cannot go back, and the forward stack when it cannot go forward. This
//! module is the pure part: the entries, the saved text and how to split and fold lists.

use serde::{Deserialize, Serialize};

/// The most entries remembered for a tab, around the current one.
const MAX_BACK: usize = 30;
const MAX_FORWARD: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub url: String,
    #[serde(default)]
    pub title: String,
}

impl Entry {
    pub fn new(url: &str, title: &str) -> Entry {
        Entry { url: url.to_owned(), title: title.to_owned() }
    }
}

/// An address worth coming back to: a web page or a local file (not a blank page, a script or an internal page).
pub fn restorable(url: &str) -> bool {
    ["http://", "https://", "file://"].iter().any(|p| url.starts_with(p))
}

#[derive(Serialize, Deserialize)]
struct Saved {
    entries: Vec<Entry>,
    index: usize,
}

/// The saved form of a whole list: `back` (nearest last), the current entry, `forward` (nearest last).
pub fn encode(back: &[Entry], current: &Entry, forward: &[Entry]) -> String {
    let back: Vec<Entry> = back.iter().filter(|e| restorable(&e.url)).rev().take(MAX_BACK).cloned().collect::<Vec<_>>().into_iter().rev().collect();
    let mut entries = back.clone();
    entries.push(current.clone());
    entries.extend(forward.iter().filter(|e| restorable(&e.url)).rev().take(MAX_FORWARD).cloned());
    if entries.len() == 1 {
        return String::new(); // nothing to remember beyond the page itself
    }
    serde_json::to_string(&Saved { entries, index: back.len() }).unwrap_or_default()
}

/// The lists from saved text, if it is for the page `current_url` (else the list belongs to something else and is dropped):
/// `(back, forward)`, nearest last in both.
pub fn decode(text: &str, current_url: &str) -> Option<(Vec<Entry>, Vec<Entry>)> {
    let saved: Saved = serde_json::from_str(text).ok()?;
    if saved.entries.get(saved.index).map(|e| e.url.as_str()) != Some(current_url) {
        return None;
    }
    let back = saved.entries[..saved.index].iter().filter(|e| restorable(&e.url)).cloned().collect();
    let forward = saved.entries[saved.index + 1..].iter().rev().filter(|e| restorable(&e.url)).cloned().collect();
    Some((back, forward))
}

/// Fold the entries of a browser that is going away (`entries`, the one at `index` current) into the lists the tab keeps, so the
/// page can be reloaded in a fresh browser with a single entry and nothing is lost. Returns the new `(back, forward)`.
pub fn fold(back: &[Entry], entries: &[Entry], index: usize, forward: &[Entry]) -> (Vec<Entry>, Vec<Entry>) {
    if entries.is_empty() {
        return (back.to_vec(), forward.to_vec());
    }
    let index = index.min(entries.len() - 1);
    let mut b = back.to_vec();
    b.extend(entries[..index].iter().filter(|e| restorable(&e.url)).cloned());
    // The forward stack has the nearest entry last: the browser's own forward entries sit nearer than the tab's.
    let mut f = forward.to_vec();
    f.extend(entries[index + 1..].iter().rev().filter(|e| restorable(&e.url)).cloned());
    (b, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: u32) -> Entry {
        Entry::new(&format!("https://e.example/{n}"), &format!("t{n}"))
    }

    #[test]
    fn a_list_round_trips_for_its_page() {
        let text = encode(&[e(1), e(2)], &e(3), &[e(5), e(4)]);
        let (back, fwd) = decode(&text, "https://e.example/3").expect("decodes");
        assert_eq!(back, vec![e(1), e(2)], "nearest last");
        assert_eq!(fwd, vec![e(5), e(4)], "nearest last");
    }

    #[test]
    fn a_list_for_another_page_is_dropped() {
        let text = encode(&[e(1)], &e(2), &[]);
        assert!(decode(&text, "https://e.example/9").is_none());
        assert!(decode("not json", "https://e.example/2").is_none());
        assert!(decode("", "https://e.example/2").is_none());
    }

    #[test]
    fn a_page_alone_saves_nothing_and_blank_or_script_entries_are_left_out() {
        assert_eq!(encode(&[], &e(1), &[]), "");
        assert_eq!(encode(&[Entry::new("about:blank", ""), Entry::new("javascript:alert(1)", "")], &e(1), &[Entry::new("data:text/html,x", "")]), "");
        let (back, _) = decode(&encode(&[Entry::new("about:blank", ""), e(1)], &e(2), &[]), "https://e.example/2").unwrap();
        assert_eq!(back, vec![e(1)]);
    }

    #[test]
    fn long_lists_keep_the_entries_nearest_the_page() {
        let back: Vec<Entry> = (0..100).map(e).collect();
        let fwd: Vec<Entry> = (200..100 + 200).rev().map(e).collect(); // nearest (200) last
        let (b, f) = decode(&encode(&back, &e(150), &fwd), "https://e.example/150").unwrap();
        assert_eq!((b.len(), f.len()), (MAX_BACK, MAX_FORWARD));
        assert_eq!(b.last(), Some(&e(99)), "the nearest back entry is kept");
        assert_eq!(f.last(), Some(&e(200)), "the nearest forward entry is kept");
    }

    #[test]
    fn folding_a_browsers_entries_into_the_lists() {
        // The browser has [b1, b2, CUR, f1, f2] (f1 nearest); the tab already had earlier [x] and later [z].
        let entries = vec![e(11), e(12), e(13), e(14), e(15)];
        let (back, fwd) = fold(&[e(1)], &entries, 2, &[e(99)]);
        assert_eq!(back, vec![e(1), e(11), e(12)]);
        assert_eq!(fwd, vec![e(99), e(15), e(14)], "the browser's nearer entries are last");
        assert_eq!(fold(&[e(1)], &[], 0, &[e(2)]), (vec![e(1)], vec![e(2)]));
    }
}
