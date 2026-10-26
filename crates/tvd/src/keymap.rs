//! Maps web key names and characters to Linux key codes.
//!
//! Key names follow `KeyboardEvent.code` from the DOM (physical keys), so a
//! browser can forward its keyboard unchanged. Characters assume a US layout.

use evdev::Key;

pub fn from_code(code: &str) -> Option<Key> {
    if let Some(letter) = code.strip_prefix("Key") {
        let mut chars = letter.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => letter_key(c),
            _ => None,
        };
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        let mut chars = digit.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => digit_key(c),
            _ => None,
        };
    }
    if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        return function_key(n);
    }
    Some(match code {
        "ArrowUp" => Key::KEY_UP,
        "ArrowDown" => Key::KEY_DOWN,
        "ArrowLeft" => Key::KEY_LEFT,
        "ArrowRight" => Key::KEY_RIGHT,
        "Enter" | "NumpadEnter" => Key::KEY_ENTER,
        "Escape" => Key::KEY_ESC,
        "Backspace" => Key::KEY_BACKSPACE,
        "Tab" => Key::KEY_TAB,
        "Space" => Key::KEY_SPACE,
        "Minus" => Key::KEY_MINUS,
        "Equal" => Key::KEY_EQUAL,
        "BracketLeft" => Key::KEY_LEFTBRACE,
        "BracketRight" => Key::KEY_RIGHTBRACE,
        "Backslash" => Key::KEY_BACKSLASH,
        "Semicolon" => Key::KEY_SEMICOLON,
        "Quote" => Key::KEY_APOSTROPHE,
        "Backquote" => Key::KEY_GRAVE,
        "Comma" => Key::KEY_COMMA,
        "Period" => Key::KEY_DOT,
        "Slash" => Key::KEY_SLASH,
        "Home" => Key::KEY_HOME,
        "End" => Key::KEY_END,
        "PageUp" => Key::KEY_PAGEUP,
        "PageDown" => Key::KEY_PAGEDOWN,
        "Insert" => Key::KEY_INSERT,
        "Delete" => Key::KEY_DELETE,
        "CapsLock" => Key::KEY_CAPSLOCK,
        "ContextMenu" => Key::KEY_COMPOSE,
        "ShiftLeft" => Key::KEY_LEFTSHIFT,
        "ShiftRight" => Key::KEY_RIGHTSHIFT,
        "ControlLeft" => Key::KEY_LEFTCTRL,
        "ControlRight" => Key::KEY_RIGHTCTRL,
        "AltLeft" => Key::KEY_LEFTALT,
        "AltRight" => Key::KEY_RIGHTALT,
        "MetaLeft" => Key::KEY_LEFTMETA,
        "MetaRight" => Key::KEY_RIGHTMETA,
        "MediaPlayPause" => Key::KEY_PLAYPAUSE,
        "MediaStop" => Key::KEY_STOPCD,
        "MediaTrackNext" => Key::KEY_NEXTSONG,
        "MediaTrackPrevious" => Key::KEY_PREVIOUSSONG,
        "BrowserBack" => Key::KEY_BACK,
        "BrowserForward" => Key::KEY_FORWARD,
        _ => return None,
    })
}

/// Key and whether Shift is needed for a character on a US layout.
pub fn from_char(c: char) -> Option<(Key, bool)> {
    if c.is_ascii_alphabetic() {
        return letter_key(c).map(|key| (key, c.is_ascii_uppercase()));
    }
    if c.is_ascii_digit() {
        return digit_key(c).map(|key| (key, false));
    }
    let (key, shift) = match c {
        ' ' => (Key::KEY_SPACE, false),
        '\n' => (Key::KEY_ENTER, false),
        '\t' => (Key::KEY_TAB, false),
        '-' => (Key::KEY_MINUS, false),
        '_' => (Key::KEY_MINUS, true),
        '=' => (Key::KEY_EQUAL, false),
        '+' => (Key::KEY_EQUAL, true),
        '[' => (Key::KEY_LEFTBRACE, false),
        '{' => (Key::KEY_LEFTBRACE, true),
        ']' => (Key::KEY_RIGHTBRACE, false),
        '}' => (Key::KEY_RIGHTBRACE, true),
        '\\' => (Key::KEY_BACKSLASH, false),
        '|' => (Key::KEY_BACKSLASH, true),
        ';' => (Key::KEY_SEMICOLON, false),
        ':' => (Key::KEY_SEMICOLON, true),
        '\'' => (Key::KEY_APOSTROPHE, false),
        '"' => (Key::KEY_APOSTROPHE, true),
        '`' => (Key::KEY_GRAVE, false),
        '~' => (Key::KEY_GRAVE, true),
        ',' => (Key::KEY_COMMA, false),
        '<' => (Key::KEY_COMMA, true),
        '.' => (Key::KEY_DOT, false),
        '>' => (Key::KEY_DOT, true),
        '/' => (Key::KEY_SLASH, false),
        '?' => (Key::KEY_SLASH, true),
        '!' => (Key::KEY_1, true),
        '@' => (Key::KEY_2, true),
        '#' => (Key::KEY_3, true),
        '$' => (Key::KEY_4, true),
        '%' => (Key::KEY_5, true),
        '^' => (Key::KEY_6, true),
        '&' => (Key::KEY_7, true),
        '*' => (Key::KEY_8, true),
        '(' => (Key::KEY_9, true),
        ')' => (Key::KEY_0, true),
        _ => return None,
    };
    Some((key, shift))
}

fn letter_key(c: char) -> Option<Key> {
    Some(match c.to_ascii_uppercase() {
        'A' => Key::KEY_A,
        'B' => Key::KEY_B,
        'C' => Key::KEY_C,
        'D' => Key::KEY_D,
        'E' => Key::KEY_E,
        'F' => Key::KEY_F,
        'G' => Key::KEY_G,
        'H' => Key::KEY_H,
        'I' => Key::KEY_I,
        'J' => Key::KEY_J,
        'K' => Key::KEY_K,
        'L' => Key::KEY_L,
        'M' => Key::KEY_M,
        'N' => Key::KEY_N,
        'O' => Key::KEY_O,
        'P' => Key::KEY_P,
        'Q' => Key::KEY_Q,
        'R' => Key::KEY_R,
        'S' => Key::KEY_S,
        'T' => Key::KEY_T,
        'U' => Key::KEY_U,
        'V' => Key::KEY_V,
        'W' => Key::KEY_W,
        'X' => Key::KEY_X,
        'Y' => Key::KEY_Y,
        'Z' => Key::KEY_Z,
        _ => return None,
    })
}

fn digit_key(c: char) -> Option<Key> {
    Some(match c {
        '0' => Key::KEY_0,
        '1' => Key::KEY_1,
        '2' => Key::KEY_2,
        '3' => Key::KEY_3,
        '4' => Key::KEY_4,
        '5' => Key::KEY_5,
        '6' => Key::KEY_6,
        '7' => Key::KEY_7,
        '8' => Key::KEY_8,
        '9' => Key::KEY_9,
        _ => return None,
    })
}

fn function_key(n: u8) -> Option<Key> {
    Some(match n {
        1 => Key::KEY_F1,
        2 => Key::KEY_F2,
        3 => Key::KEY_F3,
        4 => Key::KEY_F4,
        5 => Key::KEY_F5,
        6 => Key::KEY_F6,
        7 => Key::KEY_F7,
        8 => Key::KEY_F8,
        9 => Key::KEY_F9,
        10 => Key::KEY_F10,
        11 => Key::KEY_F11,
        12 => Key::KEY_F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dom_codes() {
        assert_eq!(from_code("KeyQ"), Some(Key::KEY_Q));
        assert_eq!(from_code("Digit0"), Some(Key::KEY_0));
        assert_eq!(from_code("F11"), Some(Key::KEY_F11));
        assert_eq!(from_code("ArrowLeft"), Some(Key::KEY_LEFT));
        assert_eq!(from_code("KeyQQ"), None);
        assert_eq!(from_code("F13"), None);
        assert_eq!(from_code("Nope"), None);
    }

    #[test]
    fn maps_characters() {
        assert_eq!(from_char('a'), Some((Key::KEY_A, false)));
        assert_eq!(from_char('A'), Some((Key::KEY_A, true)));
        assert_eq!(from_char(':'), Some((Key::KEY_SEMICOLON, true)));
        assert_eq!(from_char('@'), Some((Key::KEY_2, true)));
        assert_eq!(from_char('ş'), None);
    }

    #[test]
    fn every_mapped_key_fits_the_virtual_keyboard() {
        let all = (b' '..=b'~').map(char::from).filter_map(from_char).map(|(k, _)| k);
        for key in all {
            assert!((1..=248).contains(&key.code()), "{key:?}");
        }
    }
}
