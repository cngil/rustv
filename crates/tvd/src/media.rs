//! Play, pause, skip and seek in whatever is playing, and what that is.
//!
//! Players that speak MPRIS (the Linux media player D-Bus interface: Firefox,
//! Jellyfin, VLC, mpv with its plugin and many more) are told directly, through
//! `busctl` from systemd. Anything else gets the usual keys instead. The same
//! players say what they play: title, artist, position and a cover picture.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// What a player is playing, for the home screen and the remote.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NowPlaying {
    pub title: String,
    /// Artists, joined; empty for a film.
    pub artist: String,
    /// The player's own name, such as "Mozilla Firefox".
    pub player: String,
    pub playing: bool,
    /// Seconds.
    pub position: Option<f64>,
    pub length: Option<f64>,
    /// Changes with the cover; absent when there is no local cover picture.
    /// The picture itself is at `/api/v1/now_playing/art`.
    pub art: Option<String>,
}

/// What the playing player plays, else a paused one; `None` when nothing is
/// loaded. Also the cover picture's path, when the player keeps it in a file.
pub async fn now_playing() -> Option<(NowPlaying, Option<PathBuf>)> {
    let list = busctl(&["--user".into(), "list".into(), "--no-legend".into()]).await.ok()?;
    let mut paused = None;
    for player in parse_players(&list) {
        let Some(props) = get_all(&player, PLAYER).await else { continue };
        let Some((mut now, art)) = parse_now_playing(&props) else { continue };
        if let Some(identity) = get_all(&player, "org.mpris.MediaPlayer2").await {
            now.player = identity["Identity"]["data"].as_str().unwrap_or_default().to_string();
        }
        if now.playing {
            return Some((now, art));
        }
        paused.get_or_insert((now, art));
    }
    paused
}

async fn get_all(player: &str, interface: &str) -> Option<Value> {
    let args = ["--user", "--json=short", "call", player, PATH, "org.freedesktop.DBus.Properties", "GetAll", "s"];
    let mut args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    args.push(interface.into());
    let out = busctl(&args).await.ok()?;
    let reply: Value = serde_json::from_str(&out).ok()?;
    Some(reply["data"][0].clone())
}

/// Reads `GetAll` of `org.mpris.MediaPlayer2.Player`, as busctl prints it in JSON.
fn parse_now_playing(props: &Value) -> Option<(NowPlaying, Option<PathBuf>)> {
    let status = props["PlaybackStatus"]["data"].as_str()?;
    if status == "Stopped" {
        return None;
    }
    let meta = &props["Metadata"]["data"];
    let title = meta["xesam:title"]["data"].as_str().unwrap_or_default().trim().to_string();
    if title.is_empty() {
        return None;
    }
    let artist = meta["xesam:artist"]["data"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    // Microseconds, as a signed or unsigned 64-bit number depending on the player.
    let seconds = |value: &Value| value["data"].as_f64().filter(|us| *us > 0.0).map(|us| us / 1e6);
    let art = meta["mpris:artUrl"]["data"].as_str().and_then(local_file);
    let now = NowPlaying {
        title,
        artist,
        player: String::new(),
        playing: status == "Playing",
        position: seconds(&props["Position"]),
        length: seconds(&meta["mpris:length"]),
        art: art.as_ref().map(|path| {
            // A short tag for the cover, so screens fetch it again only when it changes.
            let mut hash: u32 = 2166136261;
            for byte in path.as_os_str().as_encoded_bytes() {
                hash = (hash ^ u32::from(*byte)).wrapping_mul(16777619);
            }
            format!("{hash:08x}")
        }),
    };
    Some((now, art))
}

/// A `file://` URL as a path; covers on the internet are never fetched.
fn local_file(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    // Percent-escapes, as in "My%20Music".
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' && tail.len() >= 2 {
            if let Ok(value) = u8::from_str_radix(std::str::from_utf8(&tail[..2]).ok()?, 16) {
                bytes.push(value);
                rest = &tail[2..];
                continue;
            }
        }
        bytes.push(byte);
        rest = tail;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// The picture's type from its first bytes; anything that is not a picture is refused.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        _ => None,
    }
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
    fn reads_what_plays() {
        let props: Value = serde_json::from_str(
            r#"{"PlaybackStatus":{"type":"s","data":"Playing"},"Position":{"type":"x","data":61500000},
               "Metadata":{"type":"a{sv}","data":{"xesam:title":{"type":"s","data":"Blue in Green"},
                 "xesam:artist":{"type":"as","data":["Miles Davis","Bill Evans"]},
                 "mpris:length":{"type":"t","data":337000000},
                 "mpris:artUrl":{"type":"s","data":"file:///tmp/firefox-mpris/My%20Cover.png"}}}}"#,
        )
        .unwrap();
        let (now, art) = parse_now_playing(&props).unwrap();
        assert_eq!(
            (now.title.as_str(), now.artist.as_str(), now.playing),
            ("Blue in Green", "Miles Davis, Bill Evans", true)
        );
        assert_eq!((now.position, now.length), (Some(61.5), Some(337.0)));
        assert_eq!(art, Some(PathBuf::from("/tmp/firefox-mpris/My Cover.png")));
        assert!(now.art.is_some());
    }

    #[test]
    fn nothing_when_stopped_or_untitled_and_no_internet_covers() {
        let stopped: Value = serde_json::from_str(
            r#"{"PlaybackStatus":{"data":"Stopped"},"Metadata":{"data":{"xesam:title":{"data":"x"}}}}"#,
        )
        .unwrap();
        assert!(parse_now_playing(&stopped).is_none());
        let untitled: Value =
            serde_json::from_str(r#"{"PlaybackStatus":{"data":"Paused"},"Metadata":{"data":{}}}"#).unwrap();
        assert!(parse_now_playing(&untitled).is_none());
        assert_eq!(local_file("https://example.com/cover.jpg"), None);
        assert_eq!(image_type(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(image_type(b"-----BEGIN OPENSSH PRIVATE KEY"), None);
    }

    #[test]
    fn falls_back_to_keys() {
        assert_eq!(fallback_keys(Action::Seek, -10), ["ArrowLeft"]);
        assert_eq!(fallback_keys(Action::PlayPause, 0), ["Space"]);
    }
}
