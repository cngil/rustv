//! Virtual keyboard and pointer through `/dev/uinput`.
//!
//! The devices look like real hardware to the compositor, so remote input
//! works in every app without any app-side support.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, Key, RelativeAxisType};
use serde::Deserialize;

use crate::keymap;

/// Delay between button press and release. Both in one frame are dropped by libinput.
const CLICK_HOLD: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonAction {
    #[default]
    Click,
    Down,
    Up,
}

/// A key press from a remote: down and up at once, or held from `down` to `up`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyAction {
    #[default]
    Press,
    Down,
    Up,
}

/// A key held longer than this lost its `up` (a phone that went to sleep
/// mid-press); it is let go so the TV does not repeat it forever.
pub const MAX_HOLD: Duration = Duration::from_secs(30);

pub struct Input {
    keyboard: Mutex<VirtualDevice>,
    pointer: Mutex<VirtualDevice>,
    /// Keys a remote holds down, and since when.
    held: Mutex<HashMap<Key, Instant>>,
}

impl Input {
    pub fn new() -> Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        // KEY_ESC (1) through KEY_MICMUTE (248): the whole regular keyboard range.
        for code in 1..=248 {
            keys.insert(Key::new(code));
        }
        let keyboard = VirtualDeviceBuilder::new()?.name("rustv keyboard").with_keys(&keys)?.build()?;

        let mut buttons = AttributeSet::<Key>::new();
        for button in [Key::BTN_LEFT, Key::BTN_RIGHT, Key::BTN_MIDDLE] {
            buttons.insert(button);
        }
        let mut axes = AttributeSet::<RelativeAxisType>::new();
        for axis in [
            RelativeAxisType::REL_X,
            RelativeAxisType::REL_Y,
            RelativeAxisType::REL_WHEEL,
            RelativeAxisType::REL_HWHEEL,
        ] {
            axes.insert(axis);
        }
        let pointer = VirtualDeviceBuilder::new()?
            .name("rustv pointer")
            .with_keys(&buttons)?
            .with_relative_axes(&axes)?
            .build()?;

        Ok(Self { keyboard: Mutex::new(keyboard), pointer: Mutex::new(pointer), held: Mutex::new(HashMap::new()) })
    }

    /// Holds `key` down until `key_up`, so the app sees a real long press and
    /// the compositor repeats it, as with a key held on a keyboard.
    pub fn key_down(&self, key: Key) -> Result<()> {
        if self.held.lock().unwrap().insert(key, Instant::now()).is_none() {
            self.keyboard.lock().unwrap().emit(&[key_event(key, 1)])?;
        }
        Ok(())
    }

    /// Lets go of a held key; a key that is not held is ignored (its `down`
    /// only woke the TV, or it was let go already).
    pub fn key_up(&self, key: Key) -> Result<()> {
        if self.held.lock().unwrap().remove(&key).is_some() {
            self.keyboard.lock().unwrap().emit(&[key_event(key, 0)])?;
        }
        Ok(())
    }

    /// Lets go of every key held at least `longer_than`; zero lets go of all.
    pub fn release_held(&self, longer_than: Duration) {
        let stale: Vec<Key> = {
            let mut held = self.held.lock().unwrap();
            let stale: Vec<Key> =
                held.iter().filter(|(_, since)| since.elapsed() >= longer_than).map(|(key, _)| *key).collect();
            for key in &stale {
                held.remove(key);
            }
            stale
        };
        let mut dev = self.keyboard.lock().unwrap();
        for key in stale {
            let _ = dev.emit(&[key_event(key, 0)]);
        }
    }

    pub fn key(&self, key: Key, mods: Mods) -> Result<()> {
        let held: Vec<Key> = [
            (mods.ctrl, Key::KEY_LEFTCTRL),
            (mods.alt, Key::KEY_LEFTALT),
            (mods.shift, Key::KEY_LEFTSHIFT),
            (mods.meta, Key::KEY_LEFTMETA),
        ]
        .into_iter()
        .filter_map(|(on, key)| on.then_some(key))
        .collect();

        let mut dev = self.keyboard.lock().unwrap();
        for m in &held {
            dev.emit(&[key_event(*m, 1)])?;
        }
        dev.emit(&[key_event(key, 1)])?;
        dev.emit(&[key_event(key, 0)])?;
        for m in held.iter().rev() {
            dev.emit(&[key_event(*m, 0)])?;
        }
        Ok(())
    }

    /// Types text with a US layout. Returns the number of characters that have no key.
    pub fn text(&self, text: &str) -> Result<usize> {
        let mut skipped = 0;
        for c in text.chars() {
            match keymap::from_char(c) {
                Some((key, shift)) => self.key(key, Mods { shift, ..Mods::default() })?,
                None => skipped += 1,
            }
        }
        Ok(skipped)
    }

    pub fn motion(&self, dx: i32, dy: i32) -> Result<()> {
        self.pointer.lock().unwrap().emit(&[
            InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, dx),
            InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, dy),
        ])?;
        Ok(())
    }

    pub fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        self.pointer.lock().unwrap().emit(&[
            InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_WHEEL.0, dy),
            InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_HWHEEL.0, dx),
        ])?;
        Ok(())
    }

    pub async fn button(&self, button: Button, action: ButtonAction) -> Result<()> {
        let key = match button {
            Button::Left => Key::BTN_LEFT,
            Button::Right => Key::BTN_RIGHT,
            Button::Middle => Key::BTN_MIDDLE,
        };
        match action {
            ButtonAction::Down => self.pointer_key(key, 1),
            ButtonAction::Up => self.pointer_key(key, 0),
            ButtonAction::Click => {
                self.pointer_key(key, 1)?;
                tokio::time::sleep(CLICK_HOLD).await;
                self.pointer_key(key, 0)
            }
        }
    }

    fn pointer_key(&self, key: Key, value: i32) -> Result<()> {
        self.pointer.lock().unwrap().emit(&[key_event(key, value)])?;
        Ok(())
    }
}

fn key_event(key: Key, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY, key.code(), value)
}
