//! Connection to tvd: one WebSocket, reconnecting forever.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use slint::Weak;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::settings::Settings;
use crate::{AppEntry, Shell};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Apps {
        apps: Vec<AppInfo>,
    },
    State {
        running: Option<serde_json::Value>,
        volume: Option<Volume>,
    },
    Audio {
        max_volume: f32,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct AppInfo {
    id: String,
    name: String,
    #[serde(default)]
    icon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
struct Volume {
    level: f32,
    muted: bool,
}

pub async fn run(settings: Settings, ui: Weak<Shell>, mut commands: mpsc::UnboundedReceiver<String>) {
    let mut delay = Duration::from_millis(500);
    loop {
        match session(&settings, &ui, &mut commands).await {
            Ok(()) => delay = Duration::from_millis(500),
            Err(err) => eprintln!("rustv-shell: {err:#}"),
        }
        crate::set_connected(&ui, false);
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(5));
    }
}

async fn session(settings: &Settings, ui: &Weak<Shell>, commands: &mut mpsc::UnboundedReceiver<String>) -> Result<()> {
    let token = settings.token()?;
    let url = format!("ws://{}/api/v1/ws?token={token}", settings.addr);
    let (socket, _) = tokio_tungstenite::connect_async(&url).await.context("connecting to tvd")?;
    crate::set_connected(ui, true);

    let (mut sink, mut stream) = socket.split();
    let mut volume: Option<Volume> = None;
    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => handle(settings, &token, ui, &text, &mut volume).await,
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(err)) => return Err(err.into()),
                Some(Ok(_)) => {}
            },
            command = commands.recv() => match command {
                Some(command) => sink.send(Message::Text(command)).await?,
                None => return Ok(()),
            },
        }
    }
}

async fn handle(settings: &Settings, token: &str, ui: &Weak<Shell>, text: &str, volume: &mut Option<Volume>) {
    let Ok(msg) = serde_json::from_str::<Incoming>(text) else { return };
    match msg {
        Incoming::Apps { apps } => {
            let addr = settings.addr.clone();
            let token = token.to_string();
            let entries = tokio::task::spawn_blocking(move || {
                apps.into_iter()
                    .map(|app| AppEntry {
                        icon: if app.icon { fetch_icon(&addr, &token, &app.id).ok() } else { None },
                        id: app.id,
                        name: app.name,
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            crate::set_apps(ui, entries);
        }
        Incoming::State { running, volume: new_volume } => {
            crate::set_running(ui, running.is_some());
            if let Some(new) = new_volume {
                // The first value only initializes; later changes show the overlay.
                let show = volume.is_some_and(|old| old != new);
                *volume = Some(new);
                crate::set_volume(ui, new.level, new.muted, show);
            }
        }
        Incoming::Audio { max_volume } => crate::set_max_volume(ui, max_volume),
        Incoming::Other => {}
    }
}

/// Downloads an app icon into the runtime directory and returns its path.
fn fetch_icon(addr: &str, token: &str, id: &str) -> Result<PathBuf> {
    let response = ureq::get(&format!("http://{addr}/api/v1/apps/{id}/icon"))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(5))
        .call()?;
    let ext = match response.content_type() {
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        _ => "png",
    };
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)?;

    let dir =
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("rustv-shell");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}
