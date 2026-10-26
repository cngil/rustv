//! Play, pause, skip and seek in whatever is playing.
//!
//! Players that speak MPRIS (the Linux media player D-Bus interface: Firefox,
//! Jellyfin, VLC, mpv with its plugin and many more) are told directly, through
//! `busctl` from systemd. Anything else gets the usual keys instead.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio::process::Command;

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    PlayPause,
    Next,
    Previous,
    /// Jump by `seconds`, back when negative.
    Seek,
}

/// What happened, so the remote can say so.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Sent to an MPRIS player.
    Player,
    /// No player answered; press these keys instead.
    Keys(&'static [&'static str]),
}

pub async fn control(action: Action, seconds: i64) -> Outcome {
    match mpris(action, seconds).await {
        Ok(()) => Outcome::Player,
        Err(_) => Outcome::Keys(fallback_keys(action, seconds)),
    }
}

/// Keys (DOM codes) most apps understand for the same thing.
fn fallback_keys(action: Action, seconds: i64) -> &'static [&'static str] {
    match action {
        // Space works in web players and most desktop ones; the media key in the rest.
        Action::PlayPause => &["Space"],
        Action::Next => &["MediaTrackNext"],
        Action::Previous => &["MediaTrackPrevious"],
        Action::Seek if seconds < 0 => &["ArrowLeft"],
        Action::Seek => &["ArrowRight"],
    }
}

async fn mpris(action: Action, seconds: i64) -> Result<()> {
    let player = pick_player().await?.context("no MPRIS player")?;
    let mut args = vec!["--user".to_string(), "call".into(), player, PATH.into(), PLAYER.into()];
    match action {
        Action::PlayPause => args.push("PlayPause".into()),
        Action::Next => args.push("Next".into()),
        Action::Previous => args.push("Previous".into()),
        Action::Seek => args.extend(["Seek".into(), "x".into(), (seconds * 1_000_000).to_string()]),
    }
    busctl(&args).await.map(drop)
}

/// The player that is playing, or else the first one there is.
async fn pick_player() -> Result<Option<String>> {
    let list = busctl(&["--user".into(), "list".into(), "--no-legend".into()]).await?;
    let players = parse_players(&list);
    for player in &players {
        let status = busctl(&[
            "--user".into(),
            "get-property".into(),
            player.clone(),
            PATH.into(),
            PLAYER.into(),
            "PlaybackStatus".into(),
        ])
        .await
        .unwrap_or_default();
        if status.contains("\"Playing\"") {
            return Ok(Some(player.clone()));
        }
    }
    Ok(players.into_iter().next())
}

fn parse_players(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| name.starts_with("org.mpris.MediaPlayer2."))
        .map(String::from)
        .collect()
}

async fn busctl(args: &[String]) -> Result<String> {
    let out = Command::new("busctl").args(args).output().await.context("running busctl")?;
    if !out.status.success() {
        bail!("busctl: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_players_on_the_bus() {
        let list = "org.freedesktop.DBus 1 dbus-daemon ahmet :1.0 user@1000.service - -\n\
                    org.mpris.MediaPlayer2.firefox.instance_1_42 4242 firefox ahmet :1.88 app.slice - -\n\
                    :1.89 4300 tvd ahmet :1.89 tvd.service - -\n";
        assert_eq!(parse_players(list), ["org.mpris.MediaPlayer2.firefox.instance_1_42"]);
    }

    #[test]
    fn falls_back_to_keys() {
        assert_eq!(fallback_keys(Action::Seek, -10), ["ArrowLeft"]);
        assert_eq!(fallback_keys(Action::PlayPause, 0), ["Space"]);
    }
}
