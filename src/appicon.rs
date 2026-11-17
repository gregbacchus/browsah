//! The generated app icon: a circle in the primary colour with four swishes in a soft contrast colour (pure, tested).
//!
//! The circle shades from a little lighter at the top left to a little darker at the bottom right. The swishes are a
//! tint of the primary colour toward white (on a dark circle) or toward black (on a light one), never pure white or
//! black, which looks too stark. Used for profiles, for web apps with no icon of their own and for the profile picker.

/// The primary colour when nothing is configured: the middle of the profile picker's blue.
pub const DEFAULT_PRIMARY: &str = "#2b5db2";

/// How far the circle's top left is mixed toward white, and its bottom right toward black.
const LIGHTEN: f64 = 0.16;
const DARKEN: f64 = 0.22;
/// How far the swishes' colour is mixed from the primary toward white (dark circle) or black (light circle).
const TINT_TO_WHITE: f64 = 0.84;
const TINT_TO_BLACK: f64 = 0.72;
/// Above this relative luminance the circle counts as light and gets dark swishes.
const LIGHT_ABOVE: f64 = 0.4;

type Rgb = (f64, f64, f64);

/// `#rrggbb` or `#rgb` as channels in 0..=255.
fn parse(hex: &str) -> Option<Rgb> {
    let h = hex.trim().strip_prefix('#')?;
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_owned(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some((f64::from(v >> 16 & 255), f64::from(v >> 8 & 255), f64::from(v & 255)))
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

fn hex((r, g, b): Rgb) -> String {
    let c = |x: f64| x.round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// WCAG relative luminance, 0 (black) to 1 (white).
fn luminance((r, g, b): Rgb) -> f64 {
    let lin = |x: f64| {
        let s = x / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// The three colours of an icon for a primary colour: (top left, bottom right, swishes).
pub fn palette(primary: Option<&str>) -> (String, String, String) {
    let base = primary.and_then(parse).or_else(|| parse(DEFAULT_PRIMARY)).expect("the default colour parses");
    let (white, black) = ((255.0, 255.0, 255.0), (0.0, 0.0, 0.0));
    let swish = if luminance(base) > LIGHT_ABOVE { mix(base, black, TINT_TO_BLACK) } else { mix(base, white, TINT_TO_WHITE) };
    (hex(mix(base, white, LIGHTEN)), hex(mix(base, black, DARKEN)), hex(swish))
}

/// The icon as SVG for a primary colour (`#rrggbb`); `None`, or a colour that does not parse, gives the default blue.
pub fn svg(primary: Option<&str>) -> String {
    let (nw, se, swish) = palette(primary);
    format!(
        "<svg width=\"512\" height=\"512\" viewBox=\"0 0 512 512\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\">\n\
         <defs><linearGradient id=\"bg\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{nw}\"/><stop offset=\"1\" stop-color=\"{se}\"/></linearGradient></defs>\n\
         <circle cx=\"256\" cy=\"256\" r=\"256\" fill=\"url(#bg)\"/>\n\
         <g fill=\"{swish}\">\n\
         <path d=\"M241.331 179.942C241.331 179.942 186.223 235.05 158.669 262.604C144.868 276.404 130.013 335.07 169.775 400.727C113.78 372.682 48.0569 347.013 73.7262 242.969C77.0672 229.429 84.488 217.488 94.8893 208.918C157.294 157.493 224.401 164.863 241.331 179.942Z\"/>\n\
         <path d=\"M179.942 270.287C179.942 270.287 235.05 325.396 262.604 352.949C276.404 366.75 335.07 381.605 400.727 341.843C372.682 397.838 347.013 463.561 242.969 437.892C229.429 434.551 217.488 427.13 208.918 416.729C157.493 354.325 164.863 287.218 179.942 270.287Z\"/>\n\
         <path d=\"M270.287 331.676C270.287 331.676 325.396 276.569 352.949 249.014C366.751 235.214 381.605 176.548 341.843 110.891C397.838 138.936 463.561 164.605 437.892 268.649C434.551 282.189 427.13 294.13 416.729 302.701C354.325 354.125 287.218 346.755 270.287 331.676Z\"/>\n\
         <path d=\"M331.676 241.331C331.676 241.331 276.569 186.223 249.014 158.669C235.214 144.868 176.548 130.013 110.891 169.775C138.936 113.78 164.605 48.0569 268.649 73.7262C282.189 77.0672 294.13 84.488 302.701 94.8893C354.125 157.294 346.755 224.401 331.676 241.331Z\"/>\n\
         </g>\n\
         </svg>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(h: &str) -> Rgb {
        parse(h).unwrap()
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse("#4a2f80"), Some((74.0, 47.0, 128.0)));
        assert_eq!(parse("#fff"), Some((255.0, 255.0, 255.0)));
        assert_eq!(parse("red"), None);
        assert_eq!(parse("#12345"), None);
    }

    #[test]
    fn the_circle_is_lighter_at_the_top_left_and_darker_at_the_bottom_right() {
        let (nw, se, _) = palette(Some("#4a2f80"));
        let (base, nw, se) = (luminance(channels("#4a2f80")), luminance(channels(&nw)), luminance(channels(&se)));
        assert!(nw > base && base > se, "{nw} > {base} > {se}");
        // Slight: well short of white and black.
        assert!(nw < 0.5 && se > 0.01);
    }

    #[test]
    fn swishes_contrast_but_are_never_pure_white_or_black() {
        for primary in ["#4a2f80", "#2b5db2", "#000000", "#111111", "#f5d90a", "#ffffff", "#eeeeee"] {
            let (_, _, swish) = palette(Some(primary));
            assert_ne!(swish, "#ffffff", "{primary}");
            assert_ne!(swish, "#000000", "{primary}");
            let (a, b) = (luminance(channels(primary)), luminance(channels(&swish)));
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(ratio >= 4.0, "{primary} against {swish}: contrast {ratio:.1}");
        }
        // A dark circle gets light swishes and a light circle dark ones.
        assert!(luminance(channels(&palette(Some("#4a2f80")).2)) > 0.5);
        assert!(luminance(channels(&palette(Some("#f5d90a")).2)) < 0.2);
    }

    #[test]
    fn no_colour_or_a_bad_one_gives_the_default() {
        assert_eq!(svg(None), svg(Some("nonsense")));
        assert_eq!(svg(None), svg(Some(DEFAULT_PRIMARY)));
        assert_ne!(svg(None), svg(Some("#4a2f80")));
    }

    #[test]
    fn the_svg_has_the_circle_the_gradient_and_four_swishes() {
        let s = svg(Some("#4a2f80"));
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert_eq!(s.matches("<path").count(), 4);
        assert!(s.contains("<circle") && s.contains("url(#bg)") && s.contains("x2=\"1\" y2=\"1\""));
        let (nw, se, swish) = palette(Some("#4a2f80"));
        assert!(s.contains(&nw) && s.contains(&se) && s.contains(&swish));
    }
}
