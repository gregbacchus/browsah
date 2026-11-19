//! Tab groups: the model and its pure helpers. A group has a name and a colour; its tabs sit next to each other
//! in the tab bar and carry the group's colour as a dot. A group can be collapsed to a single tab that stands for
//! it. The window side (menus, collapsing, the tab bar) is `app/groups.rs`.

/// One group of a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Stable within the workspace (1 and up; 0 means "no group").
    pub id: u32,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
    pub collapsed: bool,
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

/// A small coloured dot as an SVG, for a tab's icon.
pub fn dot_svg(color: &str) -> String {
    let c: String = color.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '#').collect();
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\"><circle cx=\"8\" cy=\"8\" r=\"6.5\" fill=\"#202020\"/><circle cx=\"8\" cy=\"8\" r=\"5\" fill=\"{c}\" stroke=\"#ffffff\" stroke-width=\"1\"/></svg>")
}

/// Where a tab joining `group` should sit: right after the group's last tab. `order` is the group id of every
/// page in the tab bar, left to right (0: none); `pinned` pages come first and are not moved into. Returns the
/// position the page should have *after* it has been taken out of `from`.
pub fn join_position(order: &[u32], group: u32, from: usize, pinned: usize) -> usize {
    // The order without the tab that is moving.
    let rest: Vec<u32> = order.iter().enumerate().filter(|(i, _)| *i != from).map(|(_, g)| *g).collect();
    let at = match rest.iter().rposition(|g| *g == group) {
        Some(last) => last + 1,
        None => from, // the group has no other tab: stay where it is
    };
    at.max(pinned)
}

/// The tab bar title of a collapsed group: its name and how many tabs it holds.
pub fn collapsed_title(name: &str, tabs: usize) -> String {
    let name = if name.trim().is_empty() { "Group" } else { name.trim() };
    format!("\u{25b8} {name} ({tabs})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: u32, color: &str) -> Group {
        Group { id, name: format!("g{id}"), color: color.to_owned(), collapsed: false }
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
    fn a_joining_tab_goes_after_the_groups_last_tab() {
        // pages: [pinned 0, a(1), b(0), c(1), d(0)]; move d (index 4) into group 1: after c, which is index 3 -> 4.
        assert_eq!(join_position(&[0, 1, 0, 1, 0], 1, 4, 1), 4);
        // move b (index 2) into group 1: without b the order is [0,1,1,0]; last of group 1 is index 2, so position 3.
        assert_eq!(join_position(&[0, 1, 0, 1, 0], 1, 2, 1), 3);
        // a tab already in the group and last stays where it is.
        assert_eq!(join_position(&[0, 1, 1], 1, 2, 1), 2);
        // an empty group: it stays put, but never inside the pinned pages.
        assert_eq!(join_position(&[0, 0, 0], 7, 2, 1), 2);
        assert_eq!(join_position(&[0, 0, 0], 7, 0, 1), 1);
    }

    #[test]
    fn dot_and_title() {
        let dot = dot_svg("#3584e4");
        assert!(dot.contains("fill=\"#3584e4\""));
        assert!(dot.contains("fill=\"#202020\"") && dot.contains("stroke=\"#ffffff\""));
        assert!(!dot_svg("#fff\" onload=\"x").contains("onload="), "no markup gets through");
        assert_eq!(collapsed_title("News", 3), "\u{25b8} News (3)");
        assert_eq!(collapsed_title("  ", 1), "\u{25b8} Group (1)");
    }
}
