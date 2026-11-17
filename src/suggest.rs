//! Address-bar suggestions: what to offer while an address is being typed. Pure ranking over bookmarks, open
//! tabs and history; the popover that shows them is `app/suggest.rs`.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bookmark,
    Tab,
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub kind: Kind,
}

/// The address without its scheme and a leading `www.`, lower case: what a person starts typing.
fn bare(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url).to_lowercase();
    rest.strip_prefix("www.").map(str::to_owned).unwrap_or(rest)
}

fn matches(query: &str, url: &str, title: &str) -> bool {
    let (url, title) = (url.to_lowercase(), title.to_lowercase());
    query.split_whitespace().all(|w| url.contains(w) || title.contains(w))
}

/// Rank suggestions for `query` (already trimmed). `visits` are history entries newest first. Each address
/// appears once, as its best kind. At most `limit` are returned, best first.
pub fn rank(query: &str, bookmarks: &[(String, String)], tabs: &[(String, String)], visits: &[(String, String)], limit: usize) -> Vec<Suggestion> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    // address -> (score, title, kind)
    let mut best: HashMap<String, (i32, String, Kind)> = HashMap::new();
    let mut consider = |url: &str, title: &str, kind: Kind, extra: i32| {
        if !matches(&q, url, title) {
            return;
        }
        let b = bare(url);
        let mut score = match kind {
            Kind::Bookmark => 50,
            Kind::Tab => 35,
            Kind::History => 10,
        } + extra;
        if b.starts_with(&q) {
            score += 25; // what was typed is the start of the address
        } else if b.split(['/', '?', '#']).next().is_some_and(|host| host.starts_with(&q)) {
            score += 20;
        }
        if title.to_lowercase().split_whitespace().any(|w| w.starts_with(&q)) {
            score += 8;
        }
        match best.get_mut(url) {
            Some(old) if old.0 >= score => {}
            Some(old) => *old = (score, title.to_owned(), kind),
            None => {
                best.insert(url.to_owned(), (score, title.to_owned(), kind));
            }
        }
    };
    for (u, t) in bookmarks {
        consider(u, t, Kind::Bookmark, 0);
    }
    for (u, t) in tabs {
        consider(u, t, Kind::Tab, 0);
    }
    // History: how often and how recently (the list is newest first).
    let mut counts: HashMap<&str, i32> = HashMap::new();
    for (u, _) in visits {
        *counts.entry(u.as_str()).or_default() += 1;
    }
    let mut seen = std::collections::HashSet::new();
    for (i, (u, t)) in visits.iter().enumerate() {
        if seen.insert(u.as_str()) {
            let recency = (10 - (i as i32 / 5)).max(0);
            consider(u, t, Kind::History, counts[u.as_str()].min(10) + recency);
        }
    }
    let mut out: Vec<(i32, Suggestion)> = best.into_iter().map(|(url, (score, title, kind))| (score, Suggestion { url, title, kind })).collect();
    // Best first; ties by address so the order is stable.
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.url.cmp(&b.1.url)));
    out.into_iter().take(limit).map(|(_, s)| s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(u: &str, t: &str) -> (String, String) {
        (u.to_owned(), t.to_owned())
    }

    #[test]
    fn nothing_is_offered_for_nothing() {
        assert!(rank("  ", &[p("https://a.example/", "A")], &[], &[], 5).is_empty());
    }

    #[test]
    fn matches_the_address_or_the_title_and_every_word() {
        let v = [p("https://docs.example.com/guide", "The Guide"), p("https://other.example/", "Unrelated")];
        assert_eq!(rank("guide", &[], &[], &v, 5).len(), 1);
        assert_eq!(rank("docs guide", &[], &[], &v, 5).len(), 1, "words match the address and the title together");
        assert!(rank("docs missing", &[], &[], &v, 5).is_empty());
    }

    #[test]
    fn what_you_type_at_the_start_of_the_address_comes_first() {
        let v = [p("https://example.org/news", "Latest news"), p("https://news.example/", "News site")];
        let r = rank("news", &[], &[], &v, 5);
        assert_eq!(r[0].url, "https://news.example/", "the host starts with it");
        // `www.` and the scheme are not typed.
        let r = rank("wiki", &[], &[], &[p("https://www.wikipedia.org/", "Wikipedia")], 5);
        assert_eq!(r.len(), 1);
        assert!(rank("wikip", &[], &[], &[p("https://www.wikipedia.org/", "x")], 5)[0].kind == Kind::History);
    }

    #[test]
    fn bookmarks_beat_tabs_beat_history_for_the_same_address() {
        let u = "https://a.example/";
        let r = rank("a.example", &[p(u, "Bookmarked")], &[p(u, "Open")], &[p(u, "Visited")], 5);
        assert_eq!(r.len(), 1, "once");
        assert_eq!((r[0].kind, r[0].title.as_str()), (Kind::Bookmark, "Bookmarked"));
        let r = rank("a.example", &[], &[p(u, "Open")], &[p(u, "Visited")], 5);
        assert_eq!(r[0].kind, Kind::Tab);
    }

    #[test]
    fn often_and_recently_visited_comes_first() {
        let mut v = vec![p("https://site.example/rare", "Rare")];
        for _ in 0..4 {
            v.push(p("https://site.example/often", "Often"));
        }
        let r = rank("site.example", &[], &[], &v, 5);
        assert_eq!(r[0].url, "https://site.example/often");
        // The limit applies after ranking.
        assert_eq!(rank("site.example", &[], &[], &v, 1).len(), 1);
    }
}
