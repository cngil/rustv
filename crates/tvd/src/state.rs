//! Shared daemon state and command execution.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use crate::apps::AppManager;
use crate::audio::{self, VolumeAction};
use crate::cast::Cast;
use crate::config::Config;
use crate::input::{Input, Mods};
use crate::keymap;
use crate::overlay::Overlay;
use crate::pairing;
use crate::protocol::{AppInfo, AudioInfo, Command, RunningInfo, ServerMsg, SettingsInfo, StateSnapshot};
use crate::settings;
use crate::sleep;

/// Largest pointer move accepted in one message.
const MAX_DELTA: i32 = 1000;
/// How long the pairing code stays on the TV when a remote asks for it.
const PAIRING_TIME: Duration = Duration::from_secs(120);

pub struct AppState {
    pub token: String,
    /// Live configuration. The remote can change parts of it, see [`settings`].
    pub config: RwLock<Config>,
    config_path: PathBuf,
    pub input: Input,
    pub apps: Arc<AppManager>,
    pub events: broadcast::Sender<ServerMsg>,
    pub overlay: Overlay,
    cast: Cast,
    /// Open WebSocket connections.
    pub clients: AtomicUsize,
    /// The ones that are remotes, not the TV shell.
    pub remotes: AtomicUsize,
    pairing: pairing::Marker,
    paired: AtomicBool,
    /// The pairing code shows until then (seconds since the epoch).
    pairing_until: AtomicU64,
    last_input: AtomicU64,
    /// Last remote command or sound; what sleep waits on.
    last_activity: AtomicU64,
    asleep: AtomicBool,
}

impl AppState {
    pub fn new(mut config: Config, config_path: PathBuf, token: String, input: Input) -> Arc<Self> {
        let (changed_tx, mut changed_rx) = mpsc::unbounded_channel();
        let apps = std::mem::take(&mut config.apps);
        let browser = std::mem::take(&mut config.browser);
        let manager = Arc::new(AppManager::new(apps, browser, changed_tx));
        let (events, _) = broadcast::channel(64);

        let marker = pairing::Marker::new(&token);
        let paired = AtomicBool::new(marker.is_paired());
        let state = Arc::new(Self {
            pairing: marker,
            paired,
            pairing_until: AtomicU64::new(0),
            remotes: AtomicUsize::new(0),
            token,
            config: RwLock::new(config),
            input,
            apps: manager,
            events,
            overlay: Overlay::new(config_path.clone()),
            cast: Cast::default(),
            config_path,
            clients: AtomicUsize::new(0),
            last_input: AtomicU64::new(now()),
            last_activity: AtomicU64::new(now()),
            asleep: AtomicBool::new(false),
        });
        sleep::watch(Arc::downgrade(&state));
        let cast = state.config().cast.clone();
        state.cast.apply(&cast, Arc::downgrade(&state));
        if state.config().overlay.enabled {
            state.overlay.set_visible(true, Arc::downgrade(&state));
        }

        let weak = Arc::downgrade(&state);
        tokio::spawn(async move {
            while changed_rx.recv().await.is_some() {
                let Some(state) = weak.upgrade() else { break };
                state.notify().await;
            }
        });
        state
    }

    pub fn pairing_url(&self) -> String {
        pairing::url(&self.config().server.bind, &self.token)
    }

    /// A remote (not the shell) connected with the token: pairing worked.
    pub fn remote_connected(&self) {
        if !self.paired.swap(true, Ordering::Relaxed) {
            info!("first remote paired, hiding the pairing code");
            self.pairing.set_paired();
        }
    }

    fn show_pairing(&self) -> bool {
        self.config().shell.pairing_qr
            && (!self.paired.load(Ordering::Relaxed) || now() < self.pairing_until.load(Ordering::Relaxed))
    }

    pub fn config(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap()
    }

    pub fn settings_info(&self) -> SettingsInfo {
        let cfg = self.config();
        SettingsInfo {
            shell: cfg.shell.clone(),
            screensaver: cfg.screensaver.clone(),
            audio: cfg.audio.clone(),
            sleep: cfg.sleep.clone(),
            cast: cfg.cast.clone(),
            overlay: cfg.overlay.clone(),
            editable: settings::EDITABLE,
            version: env!("CARGO_PKG_VERSION"),
            config_path: self.config_path.display().to_string(),
        }
    }

    /// Lowers the volume if it is above the configured limit.
    pub async fn enforce_volume_limit(&self) {
        let audio = self.config().audio.clone();
        match audio::get().await {
            Ok(volume) if volume.level > audio.max_volume + 0.005 => {
                info!(from = volume.level, to = audio.max_volume, "volume above the limit, lowering it");
                if let Err(err) = audio::apply(VolumeAction::Set, Some(audio.max_volume), &audio).await {
                    warn!("lowering volume: {err:#}");
                }
            }
            _ => {}
        }
    }

    async fn change_setting(self: &Arc<Self>, key: String, value: serde_json::Value) -> Result<()> {
        let path = self.config_path.clone();
        let log_key = key.clone();
        let fresh = tokio::task::spawn_blocking(move || settings::apply(&path, &key, &value)).await??;
        let (overlay_moved, cast_changed) = {
            let mut cfg = self.config.write().unwrap();
            let moved = cfg.overlay.position != fresh.overlay.position;
            let cast_changed = cfg.cast != fresh.cast;
            cfg.cast = fresh.cast;
            // Only the sections the remote can edit; apps and the server need a restart.
            cfg.shell = fresh.shell;
            cfg.screensaver = fresh.screensaver;
            cfg.audio = fresh.audio;
            cfg.sleep = fresh.sleep;
            cfg.overlay = fresh.overlay;
            (moved, cast_changed)
        };
        if cast_changed {
            let cast = self.config().cast.clone();
            self.cast.apply(&cast, Arc::downgrade(self));
        }
        info!(key = %log_key, "setting changed");

        let (shell, screensaver) = {
            let cfg = self.config();
            (cfg.shell.clone(), cfg.screensaver.clone())
        };
        let _ = self.events.send(ServerMsg::Shell(shell));
        let _ = self.events.send(ServerMsg::Screensaver(screensaver));
        let _ = self.events.send(ServerMsg::Audio(self.audio_info().await));
        let _ = self.events.send(ServerMsg::Settings(self.settings_info()));
        self.enforce_volume_limit().await;
        if overlay_moved && self.overlay.visible() {
            self.overlay.set_visible(false, Arc::downgrade(self));
            self.overlay.set_visible(true, Arc::downgrade(self));
        }
        self.notify().await;
        Ok(())
    }

    pub fn app_infos(&self) -> Vec<AppInfo> {
        self.apps
            .list()
            .iter()
            .map(|app| AppInfo {
                id: app.id.clone(),
                name: app.name.clone(),
                kind: app.kind_name(),
                description: app.description.clone(),
                icon: app.icon_path().is_some(),
            })
            .collect()
    }

    pub fn icon_path(&self, id: &str) -> Option<PathBuf> {
        self.apps.list().iter().find(|app| app.id == id)?.icon_path()
    }

    pub async fn audio_info(&self) -> AudioInfo {
        let outputs = audio::outputs().await.unwrap_or_else(|err| {
            warn!("listing audio outputs: {err:#}");
            Vec::new()
        });
        let audio = self.config().audio.clone();
        AudioInfo { max_volume: audio.max_volume, step: audio.step, outputs }
    }

    pub async fn snapshot(&self) -> StateSnapshot {
        let volume = match audio::get().await {
            Ok(volume) => Some(volume),
            Err(err) => {
                warn!("reading volume: {err:#}");
                None
            }
        };
        StateSnapshot {
            running: self.apps.running().await.map(|(id, name)| RunningInfo { id, name }),
            volume,
            idle_seconds: now().saturating_sub(self.last_input.load(Ordering::Relaxed)),
            overlay: self.overlay.visible(),
            asleep: self.asleep.load(Ordering::Relaxed),
            show_pairing: self.show_pairing(),
            remotes: self.remotes.load(Ordering::Relaxed),
            clients: self.clients.load(Ordering::Relaxed),
        }
    }

    pub async fn notify(&self) {
        let snapshot = self.snapshot().await;
        let _ = self.events.send(ServerMsg::State(snapshot));
    }

    /// Called every 30 s: puts the TV to sleep once it has been unused long enough.
    pub async fn check_sleep(&self) {
        if sleep::sound_playing() {
            self.last_activity.store(now(), Ordering::Relaxed);
        }
        let cfg = self.config().sleep.clone();
        let idle = now().saturating_sub(self.last_activity.load(Ordering::Relaxed));
        if !cfg.enabled || self.asleep.load(Ordering::Relaxed) || (idle as f32) < cfg.after_minutes * 60.0 {
            return;
        }
        info!(idle_minutes = idle / 60, "unused for a while");
        self.go_to_sleep().await;
    }

    /// Screen off, and the open app closed if the config says so.
    pub async fn go_to_sleep(&self) {
        let cfg = self.config().sleep.clone();
        info!(close_apps = cfg.close_apps, "going to sleep");
        if cfg.close_apps {
            self.apps.stop().await;
        }
        if let Err(err) = sleep::display_power(false).await {
            warn!("turning the screen off: {err:#}");
        }
        self.asleep.store(true, Ordering::Relaxed);
        self.notify().await;
    }

    /// Something other than the remote used the TV (a phone started casting).
    pub async fn external_activity(&self) {
        self.last_activity.store(now(), Ordering::Relaxed);
        self.wake().await;
    }

    /// Turns the screen back on. Returns whether the TV was asleep.
    async fn wake(&self) -> bool {
        if !self.asleep.swap(false, Ordering::Relaxed) {
            return false;
        }
        info!("waking up");
        if let Err(err) = sleep::display_power(true).await {
            warn!("turning the screen on: {err:#}");
        }
        self.notify().await;
        true
    }

    pub async fn execute(self: &Arc<Self>, cmd: Command) -> Result<()> {
        if cmd.is_input() {
            self.last_input.store(now(), Ordering::Relaxed);
        }
        self.last_activity.store(now(), Ordering::Relaxed);
        // The key that wakes the TV only wakes it, like on the screensaver.
        if !matches!(cmd, Command::Sleep) && self.wake().await && cmd.is_input() {
            return Ok(());
        }
        match cmd {
            Command::Key { code, shift, ctrl, alt, meta } => {
                let key = keymap::from_code(&code).with_context(|| format!("unknown key code {code:?}"))?;
                self.input.key(key, Mods { shift, ctrl, alt, meta })?;
            }
            Command::Text { text } => {
                let skipped = self.input.text(&text)?;
                if skipped > 0 {
                    warn!(skipped, "characters without a key on the US layout were skipped");
                }
            }
            Command::Move { dx, dy } => {
                self.input.motion(dx.clamp(-MAX_DELTA, MAX_DELTA), dy.clamp(-MAX_DELTA, MAX_DELTA))?;
            }
            Command::Scroll { dx, dy } => self.input.scroll(dx.clamp(-50, 50), dy.clamp(-50, 50))?,
            Command::Button { button, action } => self.input.button(button, action).await?,
            Command::Launch { id } => self.apps.launch(&id).await?,
            Command::Stop => self.apps.stop().await,
            Command::Volume { action, level } => {
                let audio = self.config().audio.clone();
                audio::apply(action, level, &audio).await?;
                self.notify().await;
            }
            Command::AudioOutput { id } => {
                audio::set_output(id).await?;
                let _ = self.events.send(ServerMsg::Audio(self.audio_info().await));
                self.notify().await;
            }
            Command::Reorder { ids } => {
                self.apps.reorder(&ids)?;
                let _ = self.events.send(ServerMsg::Apps { apps: self.app_infos() });
            }
            Command::Set { key, value } => self.change_setting(key, value).await?,
            Command::Sleep => self.go_to_sleep().await,
            Command::ShowPairing => {
                self.pairing_until.store(now() + PAIRING_TIME.as_secs(), Ordering::Relaxed);
                self.notify().await;
                let state = Arc::clone(self);
                tokio::spawn(async move {
                    tokio::time::sleep(PAIRING_TIME + Duration::from_secs(1)).await;
                    state.notify().await;
                });
            }
            Command::Overlay { visible } => {
                let visible = visible.unwrap_or(!self.overlay.visible());
                self.overlay.set_visible(visible, Arc::downgrade(self));
                self.notify().await;
            }
        }
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
