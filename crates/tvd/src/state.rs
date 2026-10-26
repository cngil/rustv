//! Shared daemon state and command execution.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use crate::apps::AppManager;
use crate::audio;
use crate::config::Config;
use crate::input::{Input, Mods};
use crate::keymap;
use crate::overlay::Overlay;
use crate::protocol::{AppInfo, AudioInfo, Command, RunningInfo, ServerMsg, StateSnapshot};

/// Largest pointer move accepted in one message.
const MAX_DELTA: i32 = 1000;

pub struct AppState {
    pub token: String,
    pub config: Config,
    pub input: Input,
    pub apps: Arc<AppManager>,
    pub events: broadcast::Sender<ServerMsg>,
    pub overlay: Overlay,
    /// Open WebSocket connections.
    pub clients: AtomicUsize,
    last_input: AtomicU64,
}

impl AppState {
    pub fn new(mut config: Config, config_path: PathBuf, token: String, input: Input) -> Arc<Self> {
        let (changed_tx, mut changed_rx) = mpsc::unbounded_channel();
        let apps = std::mem::take(&mut config.apps);
        let browser = std::mem::take(&mut config.browser);
        let manager = Arc::new(AppManager::new(apps, browser, changed_tx));
        let (events, _) = broadcast::channel(64);

        let state = Arc::new(Self {
            token,
            config,
            input,
            apps: manager,
            events,
            overlay: Overlay::new(config_path),
            clients: AtomicUsize::new(0),
            last_input: AtomicU64::new(now()),
        });
        if state.config.overlay.enabled {
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
        AudioInfo { max_volume: self.config.audio.max_volume, step: self.config.audio.step, outputs }
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
            clients: self.clients.load(Ordering::Relaxed),
        }
    }

    pub async fn notify(&self) {
        let snapshot = self.snapshot().await;
        let _ = self.events.send(ServerMsg::State(snapshot));
    }

    pub async fn execute(self: &Arc<Self>, cmd: Command) -> Result<()> {
        if cmd.is_input() {
            self.last_input.store(now(), Ordering::Relaxed);
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
                audio::apply(action, level, &self.config.audio).await?;
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
