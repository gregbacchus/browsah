//! Translating GTK input into what CEF expects: modifier flags, key codes, wheel deltas, cursor
//! names. Pure functions so they can be unit tested without a display.
//!
//! Key codes follow the Windows virtual-key numbering, which is what CEF's `windows_key_code`
//! uses on every platform. The punctuation mapping assumes a US-style layout; other layouts get
//! the right text (CEF takes it from `character`) but may report a different `KeyboardEvent.code`.

use std::sync::atomic::{AtomicU64, Ordering};
use cef::CursorType;
use gtk::gdk::{self, Key, ModifierType};

/// `cef_event_flags_t` values (stable CEF API).
pub mod flags {
    pub const CAPS_LOCK_ON: u32 = 1 << 0;
    pub const SHIFT_DOWN: u32 = 1 << 1;
    pub const CONTROL_DOWN: u32 = 1 << 2;
    pub const ALT_DOWN: u32 = 1 << 3;
    pub const LEFT_MOUSE_BUTTON: u32 = 1 << 4;
    pub const MIDDLE_MOUSE_BUTTON: u32 = 1 << 5;
    pub const RIGHT_MOUSE_BUTTON: u32 = 1 << 6;
    pub const COMMAND_DOWN: u32 = 1 << 7;
}

/// GTK modifier state to CEF event flags (keyboard modifiers and held mouse buttons).
pub fn modifiers(state: ModifierType) -> u32 {
    let mut f = 0;
    for (gtk, cef) in [
        (ModifierType::SHIFT_MASK, flags::SHIFT_DOWN),
        (ModifierType::CONTROL_MASK, flags::CONTROL_DOWN),
        (ModifierType::ALT_MASK, flags::ALT_DOWN),
        (ModifierType::SUPER_MASK, flags::COMMAND_DOWN),
        (ModifierType::LOCK_MASK, flags::CAPS_LOCK_ON),
        (ModifierType::BUTTON1_MASK, flags::LEFT_MOUSE_BUTTON),
        (ModifierType::BUTTON2_MASK, flags::MIDDLE_MOUSE_BUTTON),
        (ModifierType::BUTTON3_MASK, flags::RIGHT_MOUSE_BUTTON),
    ] {
        if state.contains(gtk) {
            f |= cef;
        }
    }
    f
}

/// Windows virtual-key code for a GDK key, or 0 if there is none.
pub fn windows_key_code(key: Key) -> i32 {
    // Named keys first.
    let named = match key {
        Key::BackSpace => 0x08,
        Key::Tab | Key::ISO_Left_Tab => 0x09,
        Key::Return | Key::KP_Enter => 0x0D,
        Key::Escape => 0x1B,
        Key::space | Key::KP_Space => 0x20,
        Key::Page_Up | Key::KP_Page_Up => 0x21,
        Key::Page_Down | Key::KP_Page_Down => 0x22,
        Key::End | Key::KP_End => 0x23,
        Key::Home | Key::KP_Home => 0x24,
        Key::Left | Key::KP_Left => 0x25,
        Key::Up | Key::KP_Up => 0x26,
        Key::Right | Key::KP_Right => 0x27,
        Key::Down | Key::KP_Down => 0x28,
        Key::Insert | Key::KP_Insert => 0x2D,
        Key::Delete | Key::KP_Delete => 0x2E,
        Key::Shift_L | Key::Shift_R => 0x10,
        Key::Control_L | Key::Control_R => 0x11,
        Key::Alt_L | Key::Alt_R => 0x12,
        Key::Super_L | Key::Super_R | Key::Meta_L | Key::Meta_R => 0x5B,
        Key::Caps_Lock => 0x14,
        Key::Num_Lock => 0x90,
        Key::Scroll_Lock => 0x91,
        Key::Pause => 0x13,
        Key::Menu => 0x5D,
        Key::KP_Multiply => 0x6A,
        Key::KP_Add => 0x6B,
        Key::KP_Subtract => 0x6D,
        Key::KP_Decimal => 0x6E,
        Key::KP_Divide => 0x6F,
        Key::KP_0 => 0x60,
        Key::KP_1 => 0x61,
        Key::KP_2 => 0x62,
        Key::KP_3 => 0x63,
        Key::KP_4 => 0x64,
        Key::KP_5 => 0x65,
        Key::KP_6 => 0x66,
        Key::KP_7 => 0x67,
        Key::KP_8 => 0x68,
        Key::KP_9 => 0x69,
        Key::F1 => 0x70,
        Key::F2 => 0x71,
        Key::F3 => 0x72,
        Key::F4 => 0x73,
        Key::F5 => 0x74,
        Key::F6 => 0x75,
        Key::F7 => 0x76,
        Key::F8 => 0x77,
        Key::F9 => 0x78,
        Key::F10 => 0x79,
        Key::F11 => 0x7A,
        Key::F12 => 0x7B,
        _ => 0,
    };
    if named != 0 {
        return named;
    }
    let Some(c) = key.to_unicode() else { return 0 };
    match c {
        'a'..='z' => c.to_ascii_uppercase() as i32,
        'A'..='Z' | '0'..='9' => c as i32,
        // Shifted digits report the digit key.
        '!' => 0x31,
        '@' => 0x32,
        '#' => 0x33,
        '$' => 0x34,
        '%' => 0x35,
        '^' => 0x36,
        '&' => 0x37,
        '*' => 0x38,
        '(' => 0x39,
        ')' => 0x30,
        ';' | ':' => 0xBA,
        '=' | '+' => 0xBB,
        ',' | '<' => 0xBC,
        '-' | '_' => 0xBD,
        '.' | '>' => 0xBE,
        '/' | '?' => 0xBF,
        '`' | '~' => 0xC0,
        '[' | '{' => 0xDB,
        '\\' | '|' => 0xDC,
        ']' | '}' => 0xDD,
        '\'' | '"' => 0xDE,
        _ => 0,
    }
}

/// The character a key press types, if any. Control characters other than Enter are not text:
/// Backspace, Tab, Escape and Delete reach the page as key events only.
pub fn typed_char(key: Key, state: ModifierType) -> Option<char> {
    // Ctrl or Alt chords are shortcuts, not text (AltGr arrives as a different keyval/level).
    if state.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK) {
        return None;
    }
    if matches!(key, Key::Return | Key::KP_Enter) {
        return Some('\r');
    }
    let c = key.to_unicode()?;
    (!c.is_control()).then_some(c)
}

/// Whether the key press is one of the shell's own shortcuts, which must not reach the page.
pub fn is_shell_shortcut(key: Key, state: ModifierType) -> bool {
    let ctrl = state.contains(ModifierType::CONTROL_MASK);
    let alt = state.contains(ModifierType::ALT_MASK);
    match key {
        Key::l | Key::L | Key::r | Key::R | Key::t | Key::T | Key::w | Key::W | Key::d | Key::D | Key::h | Key::H => ctrl,
        Key::Tab | Key::ISO_Left_Tab | Key::Page_Up | Key::Page_Down => ctrl,
        Key::_1 | Key::_2 | Key::_3 | Key::_4 | Key::_5 | Key::_6 | Key::_7 | Key::_8 | Key::_9 | Key::_0 => alt,
        Key::F5 | Key::F6 => true,
        Key::Left | Key::Right => alt,
        _ => false,
    }
}

/// Pixels to scroll for one GTK scroll delta. A mouse wheel notch is `1.0` in `Wheel` units
/// (`input.wheel_notch` px); touchpads report `Surface` units, already pixels. The sign is
/// flipped because CEF counts wheel-up as positive. `k` is the render scale: the page is zoomed
/// by it, so the same logical distance is `k` times as many view pixels.
/// Scroll tuning (`input.*` in the config), stored as f64 bits so it can be set once at start-up.
static WHEEL_NOTCH_BITS: AtomicU64 = AtomicU64::new(0);
static TOUCHPAD_GAIN_BITS: AtomicU64 = AtomicU64::new(0);

static TOUCHPAD_ACCEL_BITS: AtomicU64 = AtomicU64::new(0);

pub fn set_scroll_tuning(wheel_notch: f64, touchpad_speed: f64, touchpad_acceleration: f64) {
    // Stored as the factor plus one so that an exact 0 is distinguishable from "unset".
    TOUCHPAD_ACCEL_BITS.store((touchpad_acceleration + 1.0).to_bits(), Ordering::Relaxed);
    WHEEL_NOTCH_BITS.store(wheel_notch.to_bits(), Ordering::Relaxed);
    TOUCHPAD_GAIN_BITS.store(touchpad_speed.to_bits(), Ordering::Relaxed);
}

/// A tuning value, or `default` while none has been set (unit tests).
fn tuning(bits: &AtomicU64, default: f64) -> f64 {
    match bits.load(Ordering::Relaxed) {
        0 => default,
        b => f64::from_bits(b),
    }
}

/// Speed-dependent scaling of one touchpad scroll event of `delta` px: fast swipes (big events)
/// are amplified so they cover much more than slow ones.
pub fn touchpad_acceleration(delta: f64) -> f64 {
    let a = match TOUCHPAD_ACCEL_BITS.load(Ordering::Relaxed) {
        0 => 1.025,
        b => f64::from_bits(b),
    };
    1.0 + (a - 1.0) * delta.abs()
}

/// View pixels to scroll for one GTK scroll delta (fractional; the caller keeps the remainder).
pub fn wheel_delta_f(delta: f64, unit: gdk::ScrollUnit, k: f32) -> f64 {
    let px = match unit {
        gdk::ScrollUnit::Wheel => delta * tuning(&WHEEL_NOTCH_BITS, 100.0),
        _ => delta * tuning(&TOUCHPAD_GAIN_BITS, 6.0),
    };
    -px * k as f64
}

#[cfg(test)]
pub fn wheel_delta(delta: f64, unit: gdk::ScrollUnit, k: f32) -> i32 {
    wheel_delta_f(delta, unit, k).round() as i32
}

/// Take the whole pixels out of an accumulated scroll, keeping the fraction for next time.
pub fn take_whole(acc: &mut f64) -> i32 {
    let whole = acc.trunc();
    *acc -= whole;
    whole as i32
}

/// CSS cursor name (as GTK knows it) for a CEF cursor type.
pub fn cursor_name(t: CursorType) -> &'static str {
    match t {
        CursorType::HAND => "pointer",
        CursorType::IBEAM => "text",
        CursorType::VERTICALTEXT => "vertical-text",
        CursorType::CROSS => "crosshair",
        CursorType::WAIT => "wait",
        CursorType::PROGRESS => "progress",
        CursorType::HELP => "help",
        CursorType::MOVE => "move",
        CursorType::CELL => "cell",
        CursorType::CONTEXTMENU => "context-menu",
        CursorType::ALIAS => "alias",
        CursorType::COPY => "copy",
        CursorType::NODROP | CursorType::NOTALLOWED => "not-allowed",
        CursorType::GRAB => "grab",
        CursorType::GRABBING => "grabbing",
        CursorType::ZOOMIN => "zoom-in",
        CursorType::ZOOMOUT => "zoom-out",
        CursorType::NONE => "none",
        CursorType::EASTRESIZE => "e-resize",
        CursorType::WESTRESIZE => "w-resize",
        CursorType::NORTHRESIZE => "n-resize",
        CursorType::SOUTHRESIZE => "s-resize",
        CursorType::NORTHEASTRESIZE => "ne-resize",
        CursorType::NORTHWESTRESIZE => "nw-resize",
        CursorType::SOUTHEASTRESIZE => "se-resize",
        CursorType::SOUTHWESTRESIZE => "sw-resize",
        CursorType::EASTWESTRESIZE | CursorType::COLUMNRESIZE => "ew-resize",
        CursorType::NORTHSOUTHRESIZE | CursorType::ROWRESIZE => "ns-resize",
        CursorType::NORTHEASTSOUTHWESTRESIZE => "nesw-resize",
        CursorType::NORTHWESTSOUTHEASTRESIZE => "nwse-resize",
        _ => "default",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_named_keys() {
        assert_eq!(windows_key_code(Key::a), 0x41);
        assert_eq!(windows_key_code(Key::A), 0x41);
        assert_eq!(windows_key_code(Key::z), 0x5A);
        assert_eq!(windows_key_code(Key::_0), 0x30);
        assert_eq!(windows_key_code(Key::_9), 0x39);
        assert_eq!(windows_key_code(Key::Return), 0x0D);
        assert_eq!(windows_key_code(Key::BackSpace), 0x08);
        assert_eq!(windows_key_code(Key::Tab), 0x09);
        assert_eq!(windows_key_code(Key::ISO_Left_Tab), 0x09);
        assert_eq!(windows_key_code(Key::Escape), 0x1B);
        assert_eq!(windows_key_code(Key::Left), 0x25);
        assert_eq!(windows_key_code(Key::Down), 0x28);
        assert_eq!(windows_key_code(Key::Delete), 0x2E);
        assert_eq!(windows_key_code(Key::F1), 0x70);
        assert_eq!(windows_key_code(Key::F12), 0x7B);
        assert_eq!(windows_key_code(Key::KP_5), 0x65);
        assert_eq!(windows_key_code(Key::Shift_L), 0x10);
        assert_eq!(windows_key_code(Key::Control_R), 0x11);
    }

    #[test]
    fn punctuation_shifted_and_unshifted_share_a_code() {
        assert_eq!(windows_key_code(Key::semicolon), windows_key_code(Key::colon));
        assert_eq!(windows_key_code(Key::minus), 0xBD);
        assert_eq!(windows_key_code(Key::underscore), 0xBD);
        assert_eq!(windows_key_code(Key::slash), windows_key_code(Key::question));
        assert_eq!(windows_key_code(Key::exclam), 0x31);
        assert_eq!(windows_key_code(Key::parenright), 0x30);
        assert_eq!(windows_key_code(Key::bracketleft), 0xDB);
        assert_eq!(windows_key_code(Key::quotedbl), 0xDE);
    }

    #[test]
    fn unknown_keys_have_no_code() {
        assert_eq!(windows_key_code(Key::VoidSymbol), 0);
    }

    #[test]
    fn typed_characters() {
        let none = ModifierType::empty();
        assert_eq!(typed_char(Key::a, none), Some('a'));
        assert_eq!(typed_char(Key::A, ModifierType::SHIFT_MASK), Some('A'));
        assert_eq!(typed_char(Key::space, none), Some(' '));
        assert_eq!(typed_char(Key::eacute, none), Some('é'));
        assert_eq!(typed_char(Key::Return, none), Some('\r'));
        assert_eq!(typed_char(Key::BackSpace, none), None);
        assert_eq!(typed_char(Key::Tab, none), None);
        assert_eq!(typed_char(Key::Escape, none), None);
        assert_eq!(typed_char(Key::Delete, none), None);
        assert_eq!(typed_char(Key::Left, none), None);
        assert_eq!(typed_char(Key::F5, none), None);
        // Chords are shortcuts, not text.
        assert_eq!(typed_char(Key::c, ModifierType::CONTROL_MASK), None);
        assert_eq!(typed_char(Key::x, ModifierType::ALT_MASK), None);
    }

    #[test]
    fn modifier_flags() {
        assert_eq!(modifiers(ModifierType::empty()), 0);
        assert_eq!(modifiers(ModifierType::SHIFT_MASK | ModifierType::CONTROL_MASK), flags::SHIFT_DOWN | flags::CONTROL_DOWN);
        assert_eq!(modifiers(ModifierType::ALT_MASK), flags::ALT_DOWN);
        assert_eq!(modifiers(ModifierType::BUTTON1_MASK), flags::LEFT_MOUSE_BUTTON);
        assert_eq!(modifiers(ModifierType::BUTTON3_MASK), flags::RIGHT_MOUSE_BUTTON);
    }

    #[test]
    fn shell_shortcuts_stay_with_the_shell() {
        let ctrl = ModifierType::CONTROL_MASK;
        assert!(is_shell_shortcut(Key::l, ctrl));
        assert!(is_shell_shortcut(Key::R, ctrl));
        assert!(is_shell_shortcut(Key::F5, ModifierType::empty()));
        assert!(is_shell_shortcut(Key::Left, ModifierType::ALT_MASK));
        // Tab shortcuts.
        assert!(is_shell_shortcut(Key::t, ctrl));
        assert!(is_shell_shortcut(Key::d, ctrl) && is_shell_shortcut(Key::H, ctrl), "bookmark and history");
        assert!(is_shell_shortcut(Key::W, ctrl));
        assert!(is_shell_shortcut(Key::Tab, ctrl));
        assert!(is_shell_shortcut(Key::Page_Down, ctrl));
        assert!(is_shell_shortcut(Key::_3, ModifierType::ALT_MASK));
        assert!(!is_shell_shortcut(Key::_3, ModifierType::empty()));
        assert!(!is_shell_shortcut(Key::Tab, ModifierType::empty()));
        // Plain keys and other chords go to the page.
        assert!(!is_shell_shortcut(Key::l, ModifierType::empty()));
        assert!(!is_shell_shortcut(Key::Left, ModifierType::empty()));
        assert!(!is_shell_shortcut(Key::c, ctrl));
        assert!(!is_shell_shortcut(Key::a, ctrl));
    }

    #[test]
    fn wheel_deltas() {
        // One notch down at 1x: 100 px, negated for CEF.
        assert_eq!(wheel_delta(1.0, gdk::ScrollUnit::Wheel, 1.0), -100);
        assert_eq!(wheel_delta(-2.0, gdk::ScrollUnit::Wheel, 1.0), 200);
        // The render scale multiplies it.
        assert_eq!(wheel_delta(1.0, gdk::ScrollUnit::Wheel, 2.0), -200);
        // Touchpad deltas are pixels, with a gain.
        assert_eq!(wheel_delta(10.0, gdk::ScrollUnit::Surface, 1.0), -60);
        let mut acc = 2.7;
        assert_eq!(take_whole(&mut acc), 2);
        assert!((acc - 0.7).abs() < 1e-9);
        let mut acc = -2.7;
        assert_eq!(take_whole(&mut acc), -2);
        assert_eq!(wheel_delta(0.0, gdk::ScrollUnit::Wheel, 2.0), 0);
    }

    #[test]
    fn fast_swipes_are_amplified() {
        assert_eq!(touchpad_acceleration(0.0), 1.0);
        assert!((touchpad_acceleration(40.0) - 2.0).abs() < 1e-9 && touchpad_acceleration(40.0) > touchpad_acceleration(5.0));
        assert_eq!(touchpad_acceleration(-10.0), touchpad_acceleration(10.0));
    }

    #[test]
    fn cursor_names() {
        assert_eq!(cursor_name(CursorType::POINTER), "default");
        assert_eq!(cursor_name(CursorType::HAND), "pointer");
        assert_eq!(cursor_name(CursorType::IBEAM), "text");
        assert_eq!(cursor_name(CursorType::EASTWESTRESIZE), "ew-resize");
        assert_eq!(cursor_name(CursorType::NOTALLOWED), "not-allowed");
    }
}
