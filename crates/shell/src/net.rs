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
    ShellControls {
        open: bool,
    },
    Error {
        message: String,
    },
    ShellKey {
        code: String,
    },
    Apps {
        apps: Vec<AppInfo>,
    },
    State {
        running: Option<serde_json::Value>,
        volume: Option<Volume>,
        #[serde(default)]
        asleep: bool,
        #[serde(default)]
        show_pairing: bool,
        #[serde(default)]
        locked: bool,
        #[serde(default)]
        overlay: bool,
        #[serde(default)]
        display_trial: Option<Trial>,
        /// A power action taking the TV away: `restart_session`, `reboot` or `shutdown`.
        #[serde(default)]
        power: Option<String>,
    },
    Pairing {
        url: String,
        size: usize,
        modules: String,
    },
    Audio {
        max_volume: f32,
    },
    Shell(Visibility),
    NowPlaying {
        now: Option<NowPlaying>,
    },
    /// Everything the settings show, as tvd sends it to the remotes; read by menu.rs.
    Settings(serde_json::Value),
    Screensaver(ScreensaverConfig),
    Wallpaper(WallpaperConfig),
    Locale(crate::locale::Locale),
    // Boxed: the palette is much larger than the other messages.
    Theme(Box<crate::Colors>),
    #[serde(other)]
    Other,
}

/// The `[shell]` section of the rustv config, as sent by tvd.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Visibility {
    pub greeting: bool,
    pub clock: bool,
    pub date: bool,
    pub hints: bool,
    pub volume_overlay: bool,
    pub sounds: bool,
    pub sound_volume: f32,
    /// The Settings tile at the end of the app row.
    pub settings: bool,
    /// The Power tile after it.
    pub power: bool,
}

impl Default for Visibility {
    fn default() -> Self {
        Self {
            greeting: true,
            clock: true,
            date: true,
            hints: true,
            volume_overlay: true,
            sounds: true,
            sound_volume: 0.5,
            settings: true,
            power: true,
        }
    }
}

/// The `[screensaver]` section of the rustv config, as sent by tvd.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScreensaverConfig {
    pub enabled: bool,
    pub idle_minutes: f32,
    /// "random", "aurora", "orbits", "clock" or "images".
    pub style: String,
    pub clock: bool,
    pub date: bool,
}

impl Default for ScreensaverConfig {
    fn default() -> Self {
        Self { enabled: true, idle_minutes: 5.0, style: "random".into(), clock: true, date: true }
    }
}

/// The `[wallpaper]` section, as sent by tvd: settings and how many pictures there are.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    pub home: bool,
    pub change_minutes: f32,
    pub slide_seconds: f32,
    pub shuffle: bool,
    pub count: usize,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self { home: true, change_minutes: 30.0, slide_seconds: 30.0, shuffle: true, count: 0 }
    }
}

/// A display mode on trial, from the `state` message.
#[derive(Debug, Clone, Deserialize)]
pub struct Trial {
    pub mode: String,
    pub seconds: u64,
}

#[derive(Debug, Deserialize)]
struct AppInfo {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    source: String,
    id: String,
    name: String,
    #[serde(default)]
    icon: bool,
    /// Taken off the home screen from the remote.
    #[serde(default)]
    hidden: bool,
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
    let url = format!("ws://{}/api/v1/ws?token={token}&client=shell", settings.addr);
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
                Some(command) => {
                    let mut value: serde_json::Value = serde_json::from_str(&command)?;
                    let dialog_id = value.as_object_mut().and_then(|value| value.remove("_dialog_id")).and_then(|value| value.as_u64());
                    let command = value.to_string();
                    if value["type"] == "_screenshot" {
                        let (addr, token, ui) = (settings.addr.clone(), token.clone(), ui.clone());
                        tokio::task::spawn_blocking(move || {
                            let result = (|| -> Result<PathBuf> {
                                let response = ureq::get(&format!("http://{addr}/api/v1/screenshot"))
                                    .set("Authorization", &format!("Bearer {token}"))
                                    .timeout(Duration::from_secs(15)).call()?;
                                let base = std::env::var_os("HOME").map(PathBuf::from).context("No home directory")?.join("Pictures/rustv");
                                std::fs::create_dir_all(&base)?;
                                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
                                let path = base.join(format!("screenshot-{stamp}.png"));
                                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
                                std::io::copy(&mut response.into_reader(), &mut file)?;
                                Ok(path)
                            })();
                            crate::screenshot_result(&ui, result.map_err(|_| "tv.controls.screenshot_failed".into()));
                        });
                    } else if value["type"] == "_get" {
                        let panel = value["panel"].as_str().unwrap_or("").to_string();
                        if !matches!(panel.as_str(), "apps" | "audio" | "wifi" | "bluetooth" | "network" | "system") { continue; }
                        let (addr, token, ui) = (settings.addr.clone(), token.clone(), ui.clone());
                        tokio::task::spawn_blocking(move || {
                            let result = (|| -> Result<serde_json::Value> {
                                let response = ureq::get(&format!("http://{addr}/api/v1/{panel}{}", if panel == "wifi" { "?rescan=true" } else { "" }))
                                    .set("Authorization", &format!("Bearer {token}"))
                                    .timeout(Duration::from_secs(20)).call()?;
                                Ok(serde_json::from_reader(response.into_reader())?)
                            })();
                            crate::panel_result(&ui, panel, result.map_err(|_| "tv.controls.load_failed".into()));
                        });
                    } else if dialog_id.is_some() || matches!(value["type"].as_str(), Some("set_pin" | "unlock" | "wifi_connect")) {
                        let (addr, token, ui) = (settings.addr.clone(), token.clone(), ui.clone());
                        tokio::task::spawn_blocking(move || {
                            let result = ureq::post(&format!("http://{addr}/api/v1/command"))
                                .set("Authorization", &format!("Bearer {token}"))
                                .set("Content-Type", "application/json")
                                .timeout(Duration::from_secs(15)).send_string(&command);
                            let error = match result {
                                Ok(_) => None,
                                Err(ureq::Error::Status(_, response)) => {
                                    let mut body = String::new();
                                    let _ = std::io::Read::read_to_string(&mut response.into_reader(), &mut body);
                                    let body: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                                    Some(body["error"].as_str().unwrap_or("common.something_went_wrong").to_string())
                                }
                                Err(_) => Some("tv.controls.connection_failed".to_string()),
                            };
                            crate::pin_result(&ui, dialog_id, error);
                        });
                    } else {
                        sink.send(Message::Text(command)).await?;
                    }
                },
                None => return Ok(()),
            },
        }
    }
}

async fn handle(settings: &Settings, token: &str, ui: &Weak<Shell>, text: &str, volume: &mut Option<Volume>) {
    let Ok(msg) = serde_json::from_str::<Incoming>(text) else { return };
    match msg {
        Incoming::ShellControls { open } => crate::show_controls(ui, open),
        Incoming::Error { message } => crate::command_error(ui, message),
        Incoming::ShellKey { code } => crate::lock_key(ui, code),
        Incoming::Apps { apps } => {
            let addr = settings.addr.clone();
            let token = token.to_string();
            let entries = tokio::task::spawn_blocking(move || {
                apps.into_iter()
                    .filter(|app| !app.hidden)
                    .map(|app| AppEntry {
                        icon: if app.icon { fetch_icon(&addr, &token, &app.id).ok() } else { None },
                        id: app.id,
                        name: app.name,
                        details: format!("{} · {}", app.kind, app.source),
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            crate::set_apps(ui, entries);
        }
        Incoming::State {
            running,
            volume: new_volume,
            asleep,
            show_pairing,
            locked,
            overlay,
            display_trial,
            power,
        } => {
            crate::set_overlay(ui, overlay);
            crate::set_power(ui, power);
            crate::set_trial(ui, display_trial);
            crate::set_locked(ui, locked);
            crate::set_running_id(ui, running.as_ref().and_then(|app| app["id"].as_str()).map(str::to_owned));
            crate::set_running(ui, running.is_some());
            crate::set_asleep(ui, asleep);
            crate::set_show_pairing(ui, show_pairing);
            if let Some(new) = new_volume {
                // The first value only initializes; later changes show the overlay.
                let show = volume.is_some_and(|old| old != new);
                *volume = Some(new);
                crate::set_volume(ui, new.level, new.muted, show);
            }
        }
        Incoming::Audio { max_volume } => crate::set_max_volume(ui, max_volume),
        Incoming::NowPlaying { now } => {
            // The cover only when it changed; tvd tags it.
            let tag = now.as_ref().and_then(|now| now.art.clone());
            let art = match tag {
                Some(tag) if Some(&tag) != ART.lock().unwrap().as_ref().map(|(old, _)| old) => {
                    let (addr, token) = (settings.addr.clone(), token.to_string());
                    let path = tokio::task::spawn_blocking(move || fetch_art(&addr, &token).ok()).await.ok().flatten();
                    *ART.lock().unwrap() = Some((tag, path.clone()));
                    path
                }
                Some(_) => ART.lock().unwrap().as_ref().and_then(|(_, path)| path.clone()),
                None => None,
            };
            crate::set_now_playing(ui, now, art);
        }
        Incoming::Shell(visibility) => crate::set_visibility(ui, visibility),
        Incoming::Settings(settings) => crate::set_settings(ui, settings),
        Incoming::Screensaver(cfg) => crate::set_screensaver(ui, cfg),
        Incoming::Wallpaper(cfg) => {
            let _ = ui.upgrade_in_event_loop(move |ui| crate::wallpaper::configure(&ui, cfg));
        }
        Incoming::Locale(locale) => crate::set_locale(ui, locale),
        Incoming::Theme(colors) => crate::set_theme(ui, *colors),
        Incoming::Pairing { url, size, modules } => crate::set_pairing(ui, url, size, modules),
        Incoming::Other => {}
    }
}

/// What a media player plays, from tvd's `now_playing` message.
#[derive(Debug, Clone, Deserialize)]
pub struct NowPlaying {
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub player: String,
    #[serde(default)]
    pub position: Option<f64>,
    #[serde(default)]
    pub length: Option<f64>,
    #[serde(default)]
    pub art: Option<String>,
}

/// The last cover's tag and where it was saved.
static ART: std::sync::Mutex<Option<(String, Option<PathBuf>)>> = std::sync::Mutex::new(None);

/// Downloads the cover of what plays into the runtime directory.
fn fetch_art(addr: &str, token: &str) -> Result<PathBuf> {
    let response = ureq::get(&format!("http://{addr}/api/v1/now_playing/art"))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(5))
        .call()?;
    let ext = match response.content_type() {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)?;
    let dir =
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("rustv-shell");
    std::fs::create_dir_all(&dir)?;
    // A new name per cover, so Slint does not show a cached older one.
    for old in std::fs::read_dir(&dir)?.flatten() {
        if old.file_name().to_string_lossy().starts_with("cover-") {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let path = dir.join(format!("cover-{stamp}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
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
