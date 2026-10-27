//! Messages shared by the WebSocket and the REST API.
//!
//! Every client (web remote, TV shell, scripts, agents) speaks the same
//! JSON commands, tagged by `type`.

use serde::{Deserialize, Serialize};

use crate::audio::{Output, Volume, VolumeAction};
use crate::input::{Button, ButtonAction};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Press a key. `code` is a DOM `KeyboardEvent.code`, such as `ArrowUp` or `KeyA`.
    Key {
        code: String,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        meta: bool,
    },
    /// Type text (US layout).
    Text {
        text: String,
    },
    /// Move the pointer by a relative amount.
    Move {
        dx: i32,
        dy: i32,
    },
    Scroll {
        #[serde(default)]
        dx: i32,
        #[serde(default)]
        dy: i32,
    },
    Button {
        button: Button,
        #[serde(default)]
        action: ButtonAction,
    },
    Launch {
        id: String,
    },
    /// Close the running app and return to the shell.
    Stop,
    Volume {
        action: VolumeAction,
        #[serde(default)]
        level: Option<f32>,
    },
    /// Make an output (from the `audio` message) the default one.
    AudioOutput {
        id: u32,
    },
    /// Put the apps in this order (ids). Apps left out keep their place at the end.
    Reorder {
        ids: Vec<String>,
    },
    /// Change one setting, such as `screensaver.idle_minutes`; saved to the config file.
    Set {
        key: String,
        value: serde_json::Value,
    },
    /// Play/pause, next, previous, or jump `seconds` in whatever is playing.
    Media {
        action: crate::media::Action,
        #[serde(default)]
        seconds: i64,
    },
    /// Join a Wi-Fi network; `password` only for one not saved yet.
    WifiConnect {
        ssid: String,
        #[serde(default)]
        password: Option<String>,
    },
    /// Bluetooth: power, scan, pair, connect, disconnect or remove (`address` for devices).
    Bluetooth {
        action: crate::wireless::BluetoothAction,
        #[serde(default)]
        address: Option<String>,
    },
    /// Go to sleep in `minutes`; 0 cancels the timer.
    SleepTimer {
        #[serde(default)]
        minutes: u32,
    },
    /// Show the pairing QR code on the TV for two minutes, to add another device.
    ShowPairing,
    /// Lock the TV (needs a PIN); until `unlock`, every other command is refused.
    Lock,
    Unlock {
        pin: String,
    },
    /// Set, change or (without `pin`) remove the PIN. Needs `current` once a PIN is set.
    SetPin {
        #[serde(default)]
        current: Option<String>,
        #[serde(default)]
        pin: Option<String>,
    },
    /// Turn the screen off now (and close the app if `sleep.close_apps`); any input wakes it.
    Sleep,
    /// Show or hide the debug overlay; without `visible` it toggles.
    Overlay {
        #[serde(default)]
        visible: Option<bool>,
    },
}

impl Command {
    /// Commands that can take seconds and must not hold up input handling.
    pub fn is_slow(&self) -> bool {
        matches!(
            self,
            Command::Launch { .. }
                | Command::Stop
                | Command::Volume { .. }
                | Command::AudioOutput { .. }
                | Command::Set { .. }
                | Command::Media { .. }
                | Command::WifiConnect { .. }
                | Command::Bluetooth { .. }
                | Command::Sleep
        )
    }

    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Command::Key { .. }
                | Command::Text { .. }
                | Command::Move { .. }
                | Command::Scroll { .. }
                | Command::Button { .. }
                | Command::Media { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    State(StateSnapshot),
    Apps {
        apps: Vec<AppInfo>,
    },
    Audio(AudioInfo),
    /// What the on-screen shell should show (`[shell]` in the config).
    Shell(crate::config::Shell),
    /// Screensaver settings for the shell (`[screensaver]` in the config).
    Screensaver(crate::config::Screensaver),
    /// Your own pictures (`[wallpaper]`), without their paths; the shell fetches them by number.
    Wallpaper(crate::wallpaper::Info),
    /// Everything the remote's settings panel shows.
    Settings(SettingsInfo),
    /// Language and formats (`[locale]` in the config).
    Locale(crate::config::Locale),
    /// Colors for the shell and the remotes, worked out from `[theme]`.
    Theme(crate::theme::Palette),
    /// Pairing link and its QR code (`modules`: rows of `size` characters, `1` = dark). Sent to the shell.
    Pairing {
        url: String,
        size: usize,
        modules: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsInfo {
    pub shell: crate::config::Shell,
    pub screensaver: crate::config::Screensaver,
    pub audio: crate::config::Audio,
    pub sleep: crate::config::Sleep,
    pub cast: crate::config::Cast,
    pub lock: LockInfo,
    pub locale: crate::config::Locale,
    pub theme: crate::config::Theme,
    pub gamepad: crate::config::Gamepad,
    /// The presets, with their colors, for the theme picker.
    pub presets: Vec<crate::theme::Preset>,
    pub overlay: crate::config::Overlay,
    pub cec: crate::config::Cec,
    pub wallpaper: crate::wallpaper::Info,
    /// Keys `set` accepts.
    pub editable: &'static [&'static str],
    pub version: &'static str,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockInfo {
    pub on_sleep: bool,
    pub pin_set: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioInfo {
    pub max_volume: f32,
    pub step: f32,
    pub outputs: Vec<Output>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateSnapshot {
    pub running: Option<RunningInfo>,
    pub volume: Option<Volume>,
    /// Seconds since the last remote input.
    pub idle_seconds: u64,
    /// Whether the debug overlay is on screen.
    pub overlay: bool,
    /// Screen off after a long time without use; any input wakes it.
    pub asleep: bool,
    /// Whether the TV should show the pairing QR code now.
    pub show_pairing: bool,
    /// Locked with the PIN: only `unlock` works.
    pub locked: bool,
    /// Seconds until the sleep timer puts the TV to sleep, if one is set.
    pub sleep_in: Option<u64>,
    /// Connected remotes (clients other than the TV shell).
    pub remotes: usize,
    /// Connected WebSocket clients (remotes, the shell).
    pub clients: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunningInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub description: Option<String>,
    /// Whether `/api/v1/apps/{id}/icon` has an image.
    pub icon: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        let cmd: Command = serde_json::from_str(r#"{"type":"key","code":"ArrowUp"}"#).unwrap();
        assert!(matches!(cmd, Command::Key { ref code, shift: false, .. } if code == "ArrowUp"));
        let cmd: Command = serde_json::from_str(r#"{"type":"button","button":"left"}"#).unwrap();
        assert!(matches!(cmd, Command::Button { action: ButtonAction::Click, .. }));
        let cmd: Command = serde_json::from_str(r#"{"type":"stop"}"#).unwrap();
        assert!(cmd.is_slow());
        assert!(serde_json::from_str::<Command>(r#"{"type":"format_disk"}"#).is_err());
    }

    #[test]
    fn serializes_tagged_messages() {
        let msg = ServerMsg::State(StateSnapshot {
            running: None,
            volume: None,
            idle_seconds: 3,
            overlay: false,
            asleep: false,
            show_pairing: false,
            locked: false,
            sleep_in: None,
            remotes: 0,
            clients: 1,
        });
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"type":"state","running":null,"volume":null,"idle_seconds":3,"overlay":false,"asleep":false,"show_pairing":false,"locked":false,"sleep_in":null,"remotes":0,"clients":1}"#
        );
        let cmd: Command = serde_json::from_str(r#"{"type":"overlay"}"#).unwrap();
        assert!(matches!(cmd, Command::Overlay { visible: None }));
    }
}
