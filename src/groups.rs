//! Tab groups: the model and its pure helpers. A group has a name and a colour and takes the room of one tab in the
//! tab bar: the tab that is showing, painted in the group's colour, with a drop-down listing the group's other tabs.
//! The window side (the drop-down, menus, the tab bar) is `app/groups.rs`.

/// One group of a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Stable within the workspace (1 and up; 0 means "no group").
    pub id: u32,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
}

/// The colours offered, as `(name, hex)`.
pub const PALETTE: &[(&str, &str)] = &[
    ("Blue", "#3584e4"),
    ("Green", "#2ec27e"),
    ("Yellow", "#f5c211"),
    ("Orange", "#ff7800"),
    ("Red", "#e01b24"),
    ("Purple", "#9141ac"),
    ("Brown", "#986a44"),
    ("Grey", "#77767b"),
];

/// The first palette colour no group uses (then they start again).
pub fn next_color(groups: &[Group]) -> String {
    PALETTE.iter().map(|(_, c)| *c).find(|c| !groups.iter().any(|g| g.color == *c)).unwrap_or(PALETTE[groups.len() % PALETTE.len()].1).to_owned()
}

/// The next unused group id.
pub fn next_id(groups: &[Group]) -> u32 {
    groups.iter().map(|g| g.id).max().unwrap_or(0) + 1
}

/// A colour as a CSS class suffix (letters and digits only, so nothing a config file holds can become markup).
pub fn color_class(color: &str) -> String {
    let hex: String = color.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    format!("browsah-group-{hex}")
}

/// A small coloured dot as an SVG, for a tab's icon.
pub fn dot_svg(color: &str) -> String {
    let c: String = color.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '#').collect();
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\"><circle cx=\"8\" cy=\"8\" r=\"6.5\" fill=\"#202020\"/><circle cx=\"8\" cy=\"8\" r=\"5\" fill=\"{c}\" stroke=\"#ffffff\" stroke-width=\"1\"/></svg>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: u32, color: &str) -> Group {
        Group { id, name: format!("g{id}"), color: color.to_owned() }
    }

    #[test]
    fn colours_and_ids_are_handed_out_in_turn() {
        assert_eq!(next_color(&[]), "#3584e4");
        assert_eq!(next_color(&[g(1, "#3584e4")]), "#2ec27e");
        assert_eq!(next_id(&[]), 1);
        assert_eq!(next_id(&[g(1, "x"), g(4, "y")]), 5);
        let all: Vec<Group> = PALETTE.iter().enumerate().map(|(i, (_, c))| g(i as u32 + 1, c)).collect();
        assert!(PALETTE.iter().any(|(_, c)| *c == next_color(&all)), "when all are used the palette starts again");
    }

    #[test]
    fn colour_classes_hold_only_hex_digits() {
        assert_eq!(color_class("#E01B24"), "browsah-group-E01B24");
        assert_eq!(color_class("#fff\" x{}"), "browsah-group-fffx".replace('x', ""));
    }

    #[test]
    fn the_dot_is_safe() {
        let dot = dot_svg("#3584e4");
        assert!(dot.contains("fill=\"#3584e4\""));
        assert!(dot.contains("fill=\"#202020\"") && dot.contains("stroke=\"#ffffff\""));
        assert!(!dot_svg("#fff\" onload=\"x").contains("onload="), "no markup gets through");
    }
}
