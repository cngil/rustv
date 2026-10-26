//! Messages shared by the WebSocket and the REST API.
//!
//! Every client (web remote, TV shell, scripts, agents) speaks the same
//! JSON commands, tagged by `type`.

use serde::{Deserialize, Serialize};

use crate::audio::{Volume, VolumeAction};
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
}

impl Command {
    /// Commands that can take seconds and must not hold up input handling.
    pub fn is_slow(&self) -> bool {
        matches!(self, Command::Launch { .. } | Command::Stop | Command::Volume { .. })
    }

    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Command::Key { .. }
                | Command::Text { .. }
                | Command::Move { .. }
                | Command::Scroll { .. }
                | Command::Button { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    State(StateSnapshot),
    Apps { apps: Vec<AppInfo> },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct StateSnapshot {
    pub running: Option<RunningInfo>,
    pub volume: Option<Volume>,
    /// Seconds since the last remote input.
    pub idle_seconds: u64,
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
        let msg = ServerMsg::State(StateSnapshot { running: None, volume: None, idle_seconds: 3 });
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"type":"state","running":null,"volume":null,"idle_seconds":3}"#
        );
    }
}
