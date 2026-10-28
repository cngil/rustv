//! On-screen keyboard, for whoever has only the TV remote (over HDMI-CEC) or a
//! gamepad and has to type a search or an address.
//!
//! tvd holds the keyboard: while it is open, the arrows, OK and Back move over
//! it instead of reaching the app, and the chosen keys are typed into the app
//! through the virtual keyboard, the same way the phone remote types. The app
//! keeps its focus. `rustv-osd keyboard` only draws what it is told, on sway's
//! overlay layer, so it shows over full screen apps too.
//!
//! The virtual keyboard types the US layout, so the keys are ASCII only.

use std::io::Write;
use std::process::Child;
use std::sync::Mutex;

use anyhow::Result;
use serde_json::json;
use tracing::{debug, warn};

use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Key {
    Char(char),
    Shift,
    Delete,
    Space,
    Enter,
    Close,
    Symbols,
    Letters,
}

impl Key {
    /// How wide it is, in letter widths.
    fn width(self) -> f32 {
        match self {
            Key::Space => 5.0,
            Key::Char(_) => 1.0,
            _ => 1.5,
        }
    }
}

type Row = &'static [Key];

macro_rules! chars {
    ($($c:literal)*) => { &[$(Key::Char($c)),*] };
}

const DIGITS: Row = chars!('1' '2' '3' '4' '5' '6' '7' '8' '9' '0');
const BOTTOM: Row = &[Key::Symbols, Key::Space, Key::Char('.'), Key::Enter, Key::Close];
const LETTERS: [Row; 5] = [
    DIGITS,
    chars!('q' 'w' 'e' 'r' 't' 'y' 'u' 'i' 'o' 'p'),
    chars!('a' 's' 'd' 'f' 'g' 'h' 'j' 'k' 'l'),
    &[
        Key::Shift,
        Key::Char('z'),
        Key::Char('x'),
        Key::Char('c'),
        Key::Char('v'),
        Key::Char('b'),
        Key::Char('n'),
        Key::Char('m'),
        Key::Delete,
    ],
    BOTTOM,
];
const SYMBOLS: [Row; 5] = [
    DIGITS,
    chars!('-' '/' ':' ';' '(' ')' '$' '&' '@' '"'),
    chars!(',' '?' '!' '\'' '#' '%' '*' '+' '='),
    &[
        Key::Char('_'),
        Key::Char('<'),
        Key::Char('>'),
        Key::Char('['),
        Key::Char(']'),
        Key::Char('{'),
        Key::Char('}'),
        Key::Char('~'),
        Key::Delete,
    ],
    &[Key::Letters, Key::Space, Key::Char('.'), Key::Enter, Key::Close],
];

/// What a key press on the keyboard asks tvd to do.
#[derive(Debug, PartialEq)]
pub enum Output {
    /// Type this text into the app.
    Type(String),
    /// Press this key (a DOM code) in the app.
    Press(&'static str),
    Close,
}

#[derive(Default)]
struct Board {
    symbols: bool,
    /// Capital for the next letter only.
    shift: bool,
    row: usize,
    col: usize,
    /// Typed since the keyboard opened, shown above the keys.
    typed: String,
}

impl Board {
    fn rows(&self) -> &'static [Row; 5] {
        if self.symbols {
            &SYMBOLS
        } else {
            &LETTERS
        }
    }

    fn key(&self) -> Key {
        self.rows()[self.row][self.col]
    }

    /// The middle of a key, in letter widths, with each row centered like the renderer does.
    fn center(row: Row, col: usize) -> f32 {
        let widest = 10.0;
        let total: f32 = row.iter().map(|k| k.width()).sum();
        let before: f32 = row[..col].iter().map(|k| k.width()).sum();
        (widest - total) / 2.0 + before + row[col].width() / 2.0
    }

    /// Up or down lands on the key nearest above or below.
    fn go_row(&mut self, row: usize) {
        let rows = self.rows();
        let x = Self::center(rows[self.row], self.col);
        self.col = (0..rows[row].len())
            .min_by(|a, b| {
                let (da, db) = ((Self::center(rows[row], *a) - x).abs(), (Self::center(rows[row], *b) - x).abs());
                da.total_cmp(&db)
            })
            .unwrap_or(0);
        self.row = row;
    }

    /// A DOM key code from the remote; `None` when it is not the keyboard's to handle.
    fn press(&mut self, code: &str) -> Option<Vec<Output>> {
        let rows = self.rows();
        match code {
            "ArrowLeft" => self.col = self.col.saturating_sub(1),
            "ArrowRight" => self.col = (self.col + 1).min(rows[self.row].len() - 1),
            "ArrowUp" if self.row > 0 => self.go_row(self.row - 1),
            "ArrowDown" if self.row + 1 < rows.len() => self.go_row(self.row + 1),
            "ArrowUp" | "ArrowDown" => {}
            "Enter" | "NumpadEnter" | "Space" => return Some(self.activate()),
            "Escape" | "BrowserBack" => return Some(vec![Output::Close]),
            // A Backspace from the phone deletes, like the key on screen.
            "Backspace" => {
                self.typed.pop();
                return Some(vec![Output::Press("Backspace")]);
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    fn activate(&mut self) -> Vec<Output> {
        match self.key() {
            Key::Char(c) => {
                let c = if self.shift { c.to_ascii_uppercase() } else { c };
                self.shift = false;
                self.typed.push(c);
                vec![Output::Type(c.to_string())]
            }
            Key::Space => {
                self.typed.push(' ');
                vec![Output::Type(" ".into())]
            }
            Key::Shift => {
                self.shift = !self.shift;
                Vec::new()
            }
            Key::Delete => {
                self.typed.pop();
                vec![Output::Press("Backspace")]
            }
            // Enter sends the search or the address, which is what the keyboard was for.
            Key::Enter => vec![Output::Press("Enter"), Output::Close],
            Key::Close => vec![Output::Close],
            Key::Symbols | Key::Letters => {
                self.symbols = !self.symbols;
                let len = self.rows()[self.row].len();
                self.col = self.col.min(len - 1);
                Vec::new()
            }
        }
    }

    /// What `rustv-osd keyboard` draws.
    fn render(&self, palette: &Palette, lang: &str) -> serde_json::Value {
        let word = |key: &str| rustv_locales::text(lang, key).to_string();
        let label = |key: Key| -> String {
            match key {
                Key::Char(c) if self.shift => c.to_ascii_uppercase().to_string(),
                Key::Char(c) => c.to_string(),
                Key::Shift => word("osk.shift"),
                Key::Delete => word("osk.delete"),
                Key::Space => word("osk.space"),
                Key::Enter => word("osk.enter"),
                Key::Close => word("osk.close"),
                // The same in every language.
                Key::Symbols => "?123".into(),
                Key::Letters => "ABC".into(),
            }
        };
        let rows: Vec<Vec<serde_json::Value>> = self
            .rows()
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, key)| {
                        json!({
                            "label": label(*key),
                            "width": key.width(),
                            "focused": (r, c) == (self.row, self.col),
                            "active": *key == Key::Shift && self.shift,
                        })
                    })
                    .collect()
            })
            .collect();
        let hint = word("osk.hint");
        json!({ "rows": rows, "preview": self.typed, "hint": hint, "palette": palette })
    }
}

/// The keyboard and its renderer process, while open.
#[derive(Default)]
pub struct Keyboard {
    open: Mutex<Option<(Board, Option<Child>)>>,
}

impl Keyboard {
    pub fn is_open(&self) -> bool {
        self.open.lock().unwrap().is_some()
    }

    /// Opens it on the `q` key, or does nothing if it is open already.
    pub fn open(&self, palette: &Palette, lang: &str) {
        let mut open = self.open.lock().unwrap();
        if open.is_some() {
            return;
        }
        let board = Board { row: 1, ..Board::default() };
        let child = match spawn() {
            Ok(child) => Some(child),
            Err(err) => {
                warn!("on-screen keyboard: {err:#}");
                None
            }
        };
        let mut entry = (board, child);
        send(&mut entry, palette, lang);
        *open = Some(entry);
        debug!("on-screen keyboard open");
    }

    pub fn close(&self) {
        if let Some((_, Some(child))) = self.open.lock().unwrap().take() {
            crate::osd::end(child);
        }
    }

    /// A key from the remote while the keyboard is open. `None` lets it through to the app.
    pub fn press(&self, code: &str, palette: &Palette, lang: &str) -> Option<Vec<Output>> {
        let mut open = self.open.lock().unwrap();
        let entry = open.as_mut()?;
        let outputs = entry.0.press(code)?;
        send(entry, palette, lang);
        Some(outputs)
    }
}

/// `rustv-osd keyboard`, the renderer.
fn spawn() -> Result<Child> {
    crate::osd::spawn("keyboard")
}

fn send(entry: &mut (Board, Option<Child>), palette: &Palette, lang: &str) {
    let line = entry.0.render(palette, lang).to_string();
    if let Some(stdin) = entry.1.as_mut().and_then(|child| child.stdin.as_mut()) {
        if let Err(err) = writeln!(stdin, "{line}") {
            debug!("on-screen keyboard: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        Board { row: 1, ..Board::default() }
    }

    #[test]
    fn types_letters_with_a_one_shot_shift() {
        let mut b = board();
        // q, then right to w.
        assert_eq!(b.press("Enter"), Some(vec![Output::Type("q".into())]));
        b.press("ArrowRight");
        assert_eq!(b.press("Enter"), Some(vec![Output::Type("w".into())]));
        // Down twice from w lands on z's row; left to Shift, on, then z is a capital once.
        b.press("ArrowDown");
        b.press("ArrowDown");
        while b.key() != Key::Shift {
            b.press("ArrowLeft");
        }
        b.press("Enter");
        b.press("ArrowRight");
        assert_eq!(b.press("Enter"), Some(vec![Output::Type("Z".into())]));
        assert_eq!(b.press("Enter"), Some(vec![Output::Type("z".into())]));
        assert_eq!(b.typed, "qwZz");
    }

    #[test]
    fn rows_of_other_lengths_keep_the_column() {
        let mut b = board();
        // p is the last key; below it is l, the last of the shorter row.
        while b.key() != Key::Char('p') {
            b.press("ArrowRight");
        }
        b.press("ArrowDown");
        assert_eq!(b.key(), Key::Char('l'));
        // The wide Space bar sits under the middle letters.
        b.go_row(3);
        b.col = 4;
        b.press("ArrowDown");
        assert_eq!(b.key(), Key::Space);
    }

    #[test]
    fn enter_sends_and_closes_back_closes_and_other_keys_pass() {
        let mut b = board();
        b.row = 4;
        b.col = 3;
        assert_eq!(b.key(), Key::Enter);
        assert_eq!(b.press("Enter"), Some(vec![Output::Press("Enter"), Output::Close]));
        assert_eq!(b.press("Escape"), Some(vec![Output::Close]));
        assert_eq!(b.press("MediaPlayPause"), None);
    }

    #[test]
    fn symbols_and_back() {
        let mut b = board();
        b.row = 4;
        b.col = 0;
        b.press("Enter");
        assert!(b.symbols);
        b.press("ArrowUp");
        b.press("ArrowUp");
        b.press("ArrowUp");
        assert_eq!(b.press("Enter"), Some(vec![Output::Type("-".into())]));
        // Delete takes the last character off the line too.
        b.typed = "ab".into();
        assert_eq!(b.press("Backspace"), Some(vec![Output::Press("Backspace")]));
        assert_eq!(b.typed, "a");
    }

    #[test]
    fn every_key_can_be_typed() {
        // The virtual keyboard types the US layout; anything else would be skipped silently.
        for rows in [LETTERS, SYMBOLS] {
            for key in rows.iter().flat_map(|row| row.iter()) {
                if let Key::Char(c) = key {
                    assert!(c.is_ascii_graphic(), "{c:?}");
                    assert!(crate::keymap::from_char(*c).is_some(), "{c:?} has no key");
                    assert!(crate::keymap::from_char(c.to_ascii_uppercase()).is_some());
                }
            }
        }
    }
}
