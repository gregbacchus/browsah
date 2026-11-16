//! Turning address-bar text into a URL: an explicit URL stays as typed, something that looks like
//! a host name gets a scheme, anything else becomes a search.

/// Schemes accepted exactly as typed. `javascript:` is deliberately not here: typing it in the
/// address bar is a classic way to be tricked into running a script, so it is searched for.
const EXPLICIT: &[&str] = &["http://", "https://", "file://", "about:", "data:", "view-source:", "chrome://", "blob:"];

pub fn resolve(input: &str, search_url: &str) -> String {
    let text = input.trim();
    if text.is_empty() {
        return "about:blank".into();
    }
    let lower = text.to_ascii_lowercase();
    if EXPLICIT.iter().any(|s| lower.starts_with(s)) {
        return text.to_owned();
    }
    if !text.chars().any(char::is_whitespace) && looks_like_host(text) {
        let scheme = if is_local(text) { "http://" } else { "https://" };
        return format!("{scheme}{text}");
    }
    search_url.replace("{query}", &percent_encode(text))
}

/// The host part (`host` or `[v6]`) and port of the authority in `text`, without path/query/hash.
fn authority(text: &str) -> &str {
    let end = text.find(['/', '?', '#']).unwrap_or(text.len());
    let auth = &text[..end];
    // Drop any user:password@ prefix.
    auth.rsplit_once('@').map_or(auth, |(_, h)| h)
}

fn split_host_port(auth: &str) -> (&str, Option<&str>) {
    if auth.starts_with('[') {
        return match auth.find(']') {
            Some(i) => (&auth[..=i], auth[i + 1..].strip_prefix(':')),
            None => (auth, None),
        };
    }
    match auth.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (auth, None),
    }
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255))
}

fn valid_port(p: Option<&str>) -> bool {
    p.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn looks_like_host(text: &str) -> bool {
    let (host, port) = split_host_port(authority(text));
    if host.is_empty() || !valid_port(port) {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") || is_ipv4(host) {
        return true;
    }
    if host.starts_with('[') && host.ends_with(']') {
        return host.len() > 2 && host[1..host.len() - 1].bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    }
    // A dotted name whose last label has a letter (so `1.5` is a number, not a host).
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        && labels.last().is_some_and(|l| l.bytes().any(|b| b.is_ascii_alphabetic()))
}

/// Local hosts get plain http (they rarely have certificates).
fn is_local(text: &str) -> bool {
    let (host, _) = split_host_port(authority(text));
    host.eq_ignore_ascii_case("localhost") || is_ipv4(host) || host.starts_with('[')
}

/// Percent-encode for a query string: unreserved characters stay, space becomes `+`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "https://duckduckgo.com/?q={query}";

    fn r(s: &str) -> String {
        resolve(s, S)
    }

    #[test]
    fn explicit_urls_stay_as_typed() {
        assert_eq!(r("https://example.com/a?b=c"), "https://example.com/a?b=c");
        assert_eq!(r("HTTP://Example.com"), "HTTP://Example.com");
        assert_eq!(r("file:///tmp/a.html"), "file:///tmp/a.html");
        assert_eq!(r("about:blank"), "about:blank");
        assert_eq!(r("  https://example.com  "), "https://example.com");
    }

    #[test]
    fn host_names_get_https() {
        assert_eq!(r("example.com"), "https://example.com");
        assert_eq!(r("www.example.co.nz/path?x=1#top"), "https://www.example.co.nz/path?x=1#top");
        assert_eq!(r("example.com:8443/x"), "https://example.com:8443/x");
        assert_eq!(r("user@example.com/path"), "https://user@example.com/path");
    }

    #[test]
    fn local_hosts_get_http() {
        assert_eq!(r("localhost"), "http://localhost");
        assert_eq!(r("localhost:3000/app"), "http://localhost:3000/app");
        assert_eq!(r("192.168.1.1"), "http://192.168.1.1");
        assert_eq!(r("10.0.0.2:8080"), "http://10.0.0.2:8080");
        assert_eq!(r("[::1]:8080"), "http://[::1]:8080");
    }

    #[test]
    fn everything_else_is_a_search() {
        assert_eq!(r("rust lifetimes"), "https://duckduckgo.com/?q=rust+lifetimes");
        assert_eq!(r("intranet"), "https://duckduckgo.com/?q=intranet");
        assert_eq!(r("1.5"), "https://duckduckgo.com/?q=1.5");
        assert_eq!(r("999.1.1.1"), "https://duckduckgo.com/?q=999.1.1.1");
        assert_eq!(r("example.com is down"), "https://duckduckgo.com/?q=example.com+is+down");
        assert_eq!(r("example.com:abc"), "https://duckduckgo.com/?q=example.com%3Aabc");
        assert_eq!(r("c++ & rust"), "https://duckduckgo.com/?q=c%2B%2B+%26+rust");
        assert_eq!(r("café"), "https://duckduckgo.com/?q=caf%C3%A9");
    }

    #[test]
    fn javascript_urls_are_never_navigated_to() {
        assert!(r("javascript:alert(1)").starts_with("https://duckduckgo.com/?q="));
    }

    #[test]
    fn empty_input_is_a_blank_page() {
        assert_eq!(r(""), "about:blank");
        assert_eq!(r("   "), "about:blank");
    }

    #[test]
    fn custom_search_engine() {
        assert_eq!(resolve("hello world", "https://www.google.com/search?q={query}"), "https://www.google.com/search?q=hello+world");
    }
}
