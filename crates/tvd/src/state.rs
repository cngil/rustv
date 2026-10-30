//! Shared daemon state and command execution.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

use crate::apps::AppManager;
use crate::audio::{self, VolumeAction};
use crate::cast::Cast;
use crate::config::Config;
use crate::input::{Input, KeyAction, Mods};
use crate::keymap;
use crate::lock::Lock;
use crate::media;
use crate::osd::Osd;
use crate::overlay::Overlay;
use crate::pairing;
use crate::power::{self, Action as PowerAction};
use crate::protocol::{AppInfo, AudioInfo, Command, LockInfo, RunningInfo, ServerMsg, SettingsInfo, StateSnapshot};
use crate::settings;
use crate::sleep;
use crate::volumes::{self, Volumes};

/// Largest pointer move accepted in one message.
const MAX_DELTA: i32 = 1000;
/// How long the pairing code stays on the TV when a remote asks for it.
const PAIRING_TIME: Duration = Duration::from_secs(120);
/// How long the screens show that the TV is going away before it goes.
const GOODBYE: Duration = Duration::from_secs(2);
/// How long a new sway session gets before tvd sets the display up again.
const SESSION_BACK: Duration = Duration::from_secs(5);

/// Only plain keypad navigation is routed as data to the locked shell.
fn lock_navigation(command: &Command) -> Option<&str> {
    let Command::Key { code, action, shift: false, ctrl: false, alt: false, meta: false } = command else {
        return None;
    };
    if matches!(action, KeyAction::Up) {
        return None;
    }
    (matches!(code.as_str(), "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Enter" | "Escape" | "Backspace")
        || (code.starts_with("Digit") && code.len() == 6 && code.as_bytes()[5].is_ascii_digit()))
    .then_some(code)
}

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
    pub cec: crate::cec::Cec,
    lock: Lock,
    osd: Osd,
    /// Holds the screen's gamma below full brightness (`display.brightness`).
    dimmer: crate::osd::Dimmer,
    typer: crate::osd::Typer,
    night_audio: crate::night_audio::NightAudio,
    /// What a media player plays, and its cover picture's path.
    now_playing: std::sync::Mutex<Option<(crate::media::NowPlaying, Option<std::path::PathBuf>)>>,
    /// The display trial's prompt over an app, which covers the shell's.
    trial_prompt: crate::osd::Prompt,
    volumes: tokio::sync::Mutex<Volumes>,
    /// When the sleep timer fires (seconds since the epoch), and a counter that cancels older timers.
    sleep_at: std::sync::Mutex<Option<u64>>,
    sleep_timer: AtomicU64,
    /// One settings change at a time: each reads, edits and rewrites config.toml.
    settings_write: tokio::sync::Mutex<()>,
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
    /// The TV output as last read from sway, and the mode tvd set last (the
    /// output reports none while the screen is off).
    display: std::sync::Mutex<crate::display::Info>,
    display_set: std::sync::Mutex<Option<crate::display::Mode>>,
    /// A mode on trial, and a counter that ends older trials.
    display_trial: std::sync::Mutex<Option<Trial>>,
    display_trial_id: AtomicU64,
    pub keyboard: crate::keyboard::Keyboard,
    /// A power action that takes the TV away, while it does.
    power: std::sync::Mutex<Option<PowerAction>>,
}

/// A display mode being tried, and the one to go back to.
struct Trial {
    mode: crate::display::Mode,
    previous: Option<crate::display::Mode>,
    until: u64,
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
            cec: crate::cec::Cec::default(),
            lock: Lock::load(),
            osd: Osd::default(),
            dimmer: crate::osd::Dimmer::default(),
            typer: crate::osd::Typer::default(),
            trial_prompt: crate::osd::Prompt::default(),
            night_audio: crate::night_audio::NightAudio::default(),
            now_playing: std::sync::Mutex::new(None),
            volumes: tokio::sync::Mutex::new(Volumes::load()),
            sleep_at: std::sync::Mutex::new(None),
            sleep_timer: AtomicU64::new(0),
            settings_write: tokio::sync::Mutex::new(()),
            config_path,
            clients: AtomicUsize::new(0),
            last_input: AtomicU64::new(now()),
            last_activity: AtomicU64::new(now()),
            asleep: AtomicBool::new(false),
            display: std::sync::Mutex::new(crate::display::Info::default()),
            display_set: std::sync::Mutex::new(None),
            display_trial: std::sync::Mutex::new(None),
            display_trial_id: AtomicU64::new(0),
            keyboard: crate::keyboard::Keyboard::default(),
            power: std::sync::Mutex::new(None),
        });
        sleep::watch(Arc::downgrade(&state));
        watch_volume(Arc::downgrade(&state));
        crate::gamepad::watch(Arc::downgrade(&state));
        crate::cec::watch(Arc::downgrade(&state));
        state.apply_cast();
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
        let cfg = self.config();
        pairing::url(&cfg.server.bind, &self.token, cfg.server.pairing_address)
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
            lock: LockInfo { on_sleep: cfg.lock.on_sleep, pin_set: self.lock.has_pin() },
            locale: cfg.locale.clone(),
            theme: cfg.theme.clone(),
            gamepad: cfg.gamepad.clone(),
            presets: crate::theme::presets(),
            overlay: cfg.overlay.clone(),
            cec: cfg.cec.clone(),
            display: crate::display::Info {
                mode: cfg.display.mode.clone(),
                brightness: cfg.display.brightness,
                warmth: cfg.display.warmth,
                ..self.display.lock().unwrap().clone()
            },
            keyboard: cfg.keyboard.clone(),
            night: cfg.night.clone(),
            system: cfg.system.clone(),
            wallpaper: crate::wallpaper::info(&cfg.wallpaper),
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

    /// `kept`: tvd checked the value itself (a display mode that stayed on screen),
    /// so keys the remote may not set are allowed.
    async fn change_setting(self: &Arc<Self>, key: String, value: serde_json::Value, kept: bool) -> Result<()> {
        let _one_at_a_time = self.settings_write.lock().await;
        // Night audio needs plugins tvd cannot install; say so before saving a switch that cannot work.
        if matches!(key.as_str(), "audio.night" | "night.audio")
            && value == serde_json::Value::Bool(true)
            && !crate::night_audio::plugins_present()
        {
            return Err(crate::night_audio::missing());
        }
        let path = self.config_path.clone();
        let log_key = key.clone();
        let fresh = tokio::task::spawn_blocking(move || {
            if kept {
                settings::apply_kept(&path, &key, &value)
            } else {
                settings::apply(&path, &key, &value)
            }
        })
        .await??;
        let (overlay_moved, cast_changed, brightness_changed, screen_changed, audio_changed) = {
            let mut cfg = self.config.write().unwrap();
            let brightness_changed = (cfg.display.brightness - fresh.display.brightness).abs() > f32::EPSILON;
            let screen_changed = cfg.display.warmth != fresh.display.warmth || cfg.night != fresh.night;
            let audio_changed = cfg.audio.night != fresh.audio.night || cfg.night != fresh.night;
            // The overlay reads position and locale when it starts, so either change restarts it.
            let moved = cfg.overlay.position != fresh.overlay.position || cfg.locale != fresh.locale;
            cfg.locale = fresh.locale;
            cfg.theme = fresh.theme;
            cfg.gamepad = fresh.gamepad;
            let cast_changed = cfg.cast != fresh.cast;
            cfg.cast = fresh.cast;
            cfg.lock = fresh.lock;
            // Only the sections the remote can edit; apps and the server need a restart.
            cfg.shell = fresh.shell;
            cfg.screensaver = fresh.screensaver;
            cfg.audio = fresh.audio;
            cfg.sleep = fresh.sleep;
            cfg.overlay = fresh.overlay;
            cfg.display = fresh.display;
            cfg.keyboard = fresh.keyboard;
            cfg.cec = fresh.cec;
            cfg.wallpaper = fresh.wallpaper;
            cfg.night = fresh.night;
            cfg.system = fresh.system;
            (moved, cast_changed, brightness_changed, screen_changed, audio_changed)
        };
        if audio_changed {
            self.apply_night_audio(true).await;
            let _ = self.events.send(ServerMsg::Audio(self.audio_info().await));
        }
        if brightness_changed {
            self.show_brightness();
        } else if screen_changed {
            self.apply_screen();
        }
        if cast_changed {
            self.apply_cast();
        }
        info!(key = %log_key, "setting changed");

        let (shell, screensaver) = {
            let cfg = self.config();
            (cfg.shell.clone(), cfg.screensaver.clone())
        };
        let _ = self.events.send(ServerMsg::Shell(shell));
        let _ = self.events.send(ServerMsg::Screensaver(screensaver));
        let wallpaper = crate::wallpaper::info(&self.config().wallpaper);
        let _ = self.events.send(ServerMsg::Wallpaper(wallpaper));
        let (locale, theme) = {
            let cfg = self.config();
            (cfg.locale.clone(), cfg.theme.clone())
        };
        let _ = self.events.send(ServerMsg::Locale(locale));
        let _ = self.events.send(ServerMsg::Theme(crate::theme::resolve(&theme)));
        let _ = self.events.send(ServerMsg::Audio(self.audio_info().await));
        let _ = self.events.send(ServerMsg::Settings(Box::new(self.settings_info())));
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
                hidden: self.apps.is_hidden(&app.id),
                source: app.source(),
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
            locked: self.lock.is_locked(),
            sleep_in: self.sleep_at.lock().unwrap().map(|at| at.saturating_sub(now())),
            display_trial: self.display_trial.lock().unwrap().as_ref().map(|trial| crate::protocol::DisplayTrial {
                mode: trial.mode.to_string(),
                seconds: trial.until.saturating_sub(now()),
            }),
            keyboard: self.keyboard.is_open(),
            remotes: self.remotes.load(Ordering::Relaxed),
            clients: self.clients.load(Ordering::Relaxed),
            power: *self.power.lock().unwrap(),
        }
    }

    pub async fn notify(&self) {
        self.switch_volume().await;
        self.show_trial_prompt().await;
        let snapshot = self.snapshot().await;
        let _ = self.events.send(ServerMsg::State(snapshot));
    }

    /// After startup: the screen may still be off if tvd stopped while the TV slept,
    /// the config may ask for a display mode, and an app adopted at startup is
    /// the one whose volume counts.
    pub async fn after_start(self: &Arc<Self>) {
        self.set_up_display().await;
        self.apply_night_audio(true).await;
        // A held key whose `up` never came is let go.
        let state = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                state.input.release_held(crate::input::MAX_HOLD);
            }
        });
        // What plays, every 2 seconds, for the home screen and the remotes.
        let state = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                state.read_now_playing().await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        // Night mode follows the clock.
        let state = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                state.apply_screen();
                state.apply_night_audio(false).await;
            }
        });
        if let Some((id, _)) = self.apps.running().await {
            self.volumes.lock().await.current = id;
        }
    }

    /// Screen on, in the configured mode, brightness and warmth: at start and
    /// after a new sway session.
    async fn set_up_display(&self) {
        if let Err(err) = sleep::display_power(true).await {
            tracing::debug!("turning the screen on: {err:#}");
        }
        let wanted = self.config().display.mode.clone();
        if let Ok(mode) = crate::display::Mode::parse(&wanted) {
            if let Err(err) = self.set_display_mode(mode).await {
                warn!("display.mode {wanted}: {err:#}");
            }
        }
        self.read_display().await;
        self.apply_screen();
    }

    /// One of the power menu's actions, if `system.power` offers it.
    async fn power(self: &Arc<Self>, action: PowerAction) -> Result<()> {
        if !self.config().system.power.contains(&action) {
            return Err(crate::i18n::error("error.power.off", &[]));
        }
        match action {
            PowerAction::RestartApp => {
                let (id, _) = self.apps.running().await.ok_or_else(|| crate::i18n::error("error.power.no_app", &[]))?;
                info!(app = %id, "restarting the app");
                // Launching closes the open app first.
                self.apps.launch(&id).await
            }
            PowerAction::RestartShell => power::restart_shell().await,
            _ => self.take_tv_away(action).await,
        }
    }

    /// Restarts the TV session or the box, or shuts it down: the screens say
    /// so first, and the open app gets to close on its own terms.
    async fn take_tv_away(self: &Arc<Self>, action: PowerAction) -> Result<()> {
        {
            let mut going = self.power.lock().unwrap();
            if going.is_some() {
                return Ok(());
            }
            *going = Some(action);
        }
        info!(action = action.name(), "the TV is going away");
        self.notify().await;
        if !power::dry_run() {
            self.apps.stop().await;
        }
        if action == PowerAction::Shutdown && self.config().cec.power_tv && !power::dry_run() {
            self.cec.tv_standby().await;
        }
        tokio::time::sleep(GOODBYE).await;
        let done = match action {
            PowerAction::RestartSession => power::end_session().await,
            _ => power::system(action).await,
        };
        if let Err(err) = done {
            *self.power.lock().unwrap() = None;
            self.notify().await;
            return Err(err);
        }
        // A reboot or shutdown ends tvd too; a new session (or a dry run) comes back.
        if matches!(action, PowerAction::Reboot | PowerAction::Shutdown) && !power::dry_run() {
            return Ok(());
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(SESSION_BACK).await;
            if action == PowerAction::RestartSession && !power::dry_run() {
                state.set_up_display().await;
            }
            *state.power.lock().unwrap() = None;
            info!("the TV is back");
            state.notify().await;
        });
        Ok(())
    }

    /// Opens or closes the on-screen keyboard, in the theme's colors and the TV's language.
    pub async fn set_keyboard(&self, open: bool) {
        if open && self.lock.is_locked() {
            return;
        }
        if open == self.keyboard.is_open() {
            return;
        }
        if open {
            let (enabled, palette, lang) = {
                let cfg = self.config();
                (cfg.keyboard.enabled, crate::theme::resolve(&cfg.theme), cfg.locale.language)
            };
            if !enabled {
                return;
            }
            self.keyboard.open(&palette, crate::i18n::code(lang));
        } else {
            self.keyboard.close();
        }
        self.notify().await;
    }

    /// Types text into the focused app: any character through the Wayland virtual
    /// keyboard, else what the US layout has through uinput.
    async fn type_text(&self, text: &str) -> Result<()> {
        if let Err(err) = self.typer.text(text).await {
            debug!("virtual keyboard: {err:#}; typing with the US layout");
            let skipped = self.input.text(text)?;
            if skipped > 0 {
                warn!(skipped, "characters without a key on the US layout were skipped");
            }
        }
        Ok(())
    }

    /// A key while the keyboard is open. Returns whether the keyboard took it.
    async fn keyboard_key(&self, code: &str) -> Result<bool> {
        let (palette, lang) = {
            let cfg = self.config();
            (crate::theme::resolve(&cfg.theme), cfg.locale.language)
        };
        let Some(outputs) = self.keyboard.press(code, &palette, crate::i18n::code(lang)) else { return Ok(false) };
        for output in outputs {
            match output {
                crate::keyboard::Output::Type(text) => self.type_text(&text).await?,
                crate::keyboard::Output::Press(code) => {
                    let key = keymap::from_code(code).with_context(|| format!("no key {code}"))?;
                    self.input.key(key, Mods::default())?;
                }
                crate::keyboard::Output::Close => self.set_keyboard(false).await,
            }
        }
        Ok(true)
    }

    /// Reads the TV output from sway into `display`, for the remotes and the shell.
    async fn read_display(&self) {
        match crate::display::read().await {
            Ok((output, current, modes)) => {
                let current = current.or(*self.display_set.lock().unwrap());
                let mut info = self.display.lock().unwrap();
                info.output = output;
                info.current = current.map(|mode| mode.to_string());
                info.modes = modes.iter().map(ToString::to_string).collect();
            }
            Err(err) => tracing::debug!("reading the display: {err:#}"),
        }
    }

    async fn set_display_mode(&self, mode: crate::display::Mode) -> Result<()> {
        let (output, current, modes) = crate::display::read().await?;
        if !modes.contains(&mode) {
            anyhow::bail!("the TV does not offer {mode}");
        }
        if current != Some(mode) {
            crate::display::apply(&output, mode).await?;
            info!(%mode, %output, "display mode set");
        }
        *self.display_set.lock().unwrap() = Some(mode);
        Ok(())
    }

    /// Puts `mode` on screen for a few seconds; without `display_keep` the
    /// previous one comes back, so an unsupported mode cannot leave the TV black.
    async fn try_display_mode(self: &Arc<Self>, mode: &str) -> Result<()> {
        let mode = crate::display::Mode::parse(mode)?;
        let before = {
            let trial = self.display_trial.lock().unwrap();
            match trial.as_ref() {
                // Trying another while one is on trial still goes back to the one before both.
                Some(trial) => trial.previous,
                None => None,
            }
        };
        let previous = match before {
            Some(mode) => Some(mode),
            None => crate::display::read().await?.1.or(*self.display_set.lock().unwrap()),
        };
        self.set_display_mode(mode).await?;
        let id = self.display_trial_id.fetch_add(1, Ordering::Relaxed) + 1;
        *self.display_trial.lock().unwrap() =
            Some(Trial { mode, previous, until: now() + crate::display::TRIAL_SECONDS });
        self.read_display().await;
        let _ = self.events.send(ServerMsg::Settings(Box::new(self.settings_info())));
        self.notify().await;
        // Count down on every screen, then go back unless someone kept it.
        let state = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if state.display_trial_id.load(Ordering::Relaxed) != id {
                    return;
                }
                let over = state.display_trial.lock().unwrap().as_ref().map_or(true, |t| now() >= t.until);
                if over {
                    info!("display mode not kept, going back");
                    state.end_display_trial().await;
                    return;
                }
                state.notify().await;
            }
        });
        Ok(())
    }

    /// The shell asks to keep a mode on trial on the home screen; over an app,
    /// `rustv-osd prompt` asks instead, with the same words.
    async fn show_trial_prompt(&self) {
        let trial = self.display_trial.lock().unwrap().as_ref().map(|t| (t.mode, t.until));
        let running = self.apps.running().await.is_some();
        let Some((mode, until)) = trial.filter(|_| running) else {
            self.trial_prompt.hide();
            return;
        };
        let (palette, lang) = {
            let cfg = self.config();
            (crate::theme::resolve(&cfg.theme), crate::i18n::code(cfg.locale.language))
        };
        let text = |key: &str| rustv_locales::text(lang, key).to_string();
        let seconds = until.saturating_sub(now()).to_string();
        // "1920 × 1080, 60 Hz", as the shell writes it.
        let shown = mode.to_string();
        let detail = match shown.split_once('@') {
            Some((size, hz)) => format!("{}, {hz} Hz", size.replace('x', " × ")),
            None => shown,
        };
        self.trial_prompt.show(serde_json::json!({
            "title": text("remote.display.keep_title"),
            "detail": detail,
            "countdown": rustv_locales::format(lang, "tv.trial.countdown", &[("seconds", &seconds)]),
            "keys": [["OK", text("common.keep")], [text("common.back"), text("common.go_back")]],
            "palette": palette,
        }));
    }

    async fn keep_display_mode(self: &Arc<Self>) -> Result<()> {
        self.display_trial_id.fetch_add(1, Ordering::Relaxed);
        let Some(trial) = self.display_trial.lock().unwrap().take() else { return Ok(()) };
        info!(mode = %trial.mode, "display mode kept");
        self.change_setting("display.mode".into(), trial.mode.to_string().into(), true).await
    }

    /// Back to the mode before the trial.
    async fn end_display_trial(&self) {
        self.display_trial_id.fetch_add(1, Ordering::Relaxed);
        let trial = self.display_trial.lock().unwrap().take();
        if let Some(previous) = trial.and_then(|trial| trial.previous) {
            if let Err(err) = self.set_display_mode(previous).await {
                warn!("going back to {previous}: {err:#}");
            }
        }
        self.read_display().await;
        let _ = self.events.send(ServerMsg::Settings(Box::new(self.settings_info())));
        self.notify().await;
    }

    /// AirPlay as configured, but never while locked: a mirrored phone would
    /// show over the lock screen.
    fn apply_cast(self: &Arc<Self>) {
        let mut cast = self.config().cast.clone();
        if self.lock.is_locked() {
            cast.airplay = false;
        }
        self.cast.apply(&cast, Arc::downgrade(self));
    }

    /// The volume bar over an open app (the shell draws its own on the home screen).
    pub async fn show_volume(&self, volume: &audio::Volume) {
        if !self.config().shell.volume_overlay || self.apps.running().await.is_none() {
            return;
        }
        let (palette, lang, max) = {
            let cfg = self.config();
            (crate::theme::resolve(&cfg.theme), crate::i18n::code(cfg.locale.language), cfg.audio.max_volume)
        };
        let percent = (volume.level * 100.0).round().to_string();
        let level = crate::osd::Level {
            label: rustv_locales::text(lang, if volume.muted { "tv.muted" } else { "tv.volume" }).to_string(),
            value: if volume.muted { 0.0 } else { volume.level / max.max(0.01) },
            text: rustv_locales::format(lang, "unit.percent", &[("n", &percent)]),
            muted: volume.muted,
        };
        self.osd.show(level, &palette);
    }

    /// Puts the screen at the brightness and warmth it should have now, night mode included.
    fn apply_screen(&self) {
        let (brightness, warmth) = crate::night::screen(&self.config(), crate::night::local_minutes());
        self.dimmer.set(brightness, warmth);
    }

    pub fn is_locked(&self) -> bool {
        self.lock.is_locked()
    }

    /// What plays now, as the screens were last told.
    pub fn now_playing(&self) -> Option<crate::media::NowPlaying> {
        self.now_playing.lock().unwrap().as_ref().map(|(now, _)| now.clone())
    }

    /// The cover picture of what plays, if the player keeps it in a file.
    pub fn now_playing_art(&self) -> Option<std::path::PathBuf> {
        self.now_playing.lock().unwrap().as_ref().and_then(|(_, art)| art.clone())
    }

    async fn read_now_playing(&self) {
        let read = if self.config().shell.now_playing { crate::media::now_playing().await } else { None };
        let changed = {
            let mut last = self.now_playing.lock().unwrap();
            let changed = last.as_ref().map(|(now, _)| now) != read.as_ref().map(|(now, _)| now);
            *last = read;
            changed
        };
        if changed {
            let _ = self.events.send(ServerMsg::NowPlaying { now: self.now_playing() });
        }
    }

    /// Night audio on or off as the settings and the clock say. `retry` tries
    /// again after a failure, for a settings change.
    async fn apply_night_audio(&self, retry: bool) {
        let on = {
            let cfg = self.config();
            cfg.audio.night || cfg.night.audio && crate::night::active(&cfg, crate::night::local_minutes())
        };
        self.night_audio.want(on, retry).await;
    }

    /// Applies the brightness and shows it on the bar, over apps and the home screen alike.
    fn show_brightness(&self) {
        let (brightness, palette, lang) = {
            let cfg = self.config();
            (cfg.display.brightness, crate::theme::resolve(&cfg.theme), crate::i18n::code(cfg.locale.language))
        };
        self.apply_screen();
        let percent = (brightness * 100.0).round().to_string();
        let level = crate::osd::Level {
            label: rustv_locales::text(lang, "tv.brightness").to_string(),
            value: brightness,
            text: rustv_locales::format(lang, "unit.percent", &[("n", &percent)]),
            muted: false,
        };
        self.osd.show(level, &palette);
    }

    /// When another app (or the home screen) comes up, keep the volume the last
    /// one ended at, and bring back the level the new one had.
    async fn switch_volume(&self) {
        let id = self.apps.running().await.map_or_else(|| volumes::HOME.to_string(), |(id, _)| id);
        let mut saved = self.volumes.lock().await;
        if saved.current == id {
            return;
        }
        let old = std::mem::replace(&mut saved.current, id.clone());
        let audio = self.config().audio.clone();
        if !audio.per_app_volume {
            return;
        }
        if let Ok(volume) = audio::get().await {
            saved.set(&old, volume.level);
        }
        if let Some(level) = saved.get(&id) {
            if let Err(err) = audio::apply(VolumeAction::Set, Some(level), &audio).await {
                warn!("restoring the volume for {id}: {err:#}");
            }
        }
    }

    /// Sleep in `minutes`, or cancel the timer with 0.
    async fn set_sleep_timer(self: &Arc<Self>, minutes: u32) {
        let id = self.sleep_timer.fetch_add(1, Ordering::Relaxed) + 1;
        *self.sleep_at.lock().unwrap() = (minutes > 0).then(|| now() + u64::from(minutes) * 60);
        if minutes > 0 {
            info!(minutes, "sleep timer set");
            let state = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(u64::from(minutes) * 60)).await;
                // A newer timer (or a cancel) replaced this one.
                if state.sleep_timer.load(Ordering::Relaxed) == id {
                    info!("sleep timer done");
                    state.go_to_sleep().await;
                }
            });
        }
        self.notify().await;
    }

    /// Called every 30 s: puts the TV to sleep once it has been unused long enough.
    pub async fn check_sleep(self: &Arc<Self>) {
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
    pub async fn go_to_sleep(self: &Arc<Self>) {
        crate::shell_controls::clear();
        self.set_keyboard(false).await;
        // Whatever put the TV to sleep, a pending timer has nothing left to do.
        self.sleep_timer.fetch_add(1, Ordering::Relaxed);
        *self.sleep_at.lock().unwrap() = None;
        let lock_on_sleep = self.config().lock.on_sleep;
        if lock_on_sleep && self.lock.has_pin() && self.lock.lock().is_ok() {
            self.apply_cast();
        }
        let cfg = self.config().sleep.clone();
        info!(close_apps = cfg.close_apps, "going to sleep");
        if cfg.close_apps {
            self.apps.stop().await;
        }
        if let Err(err) = sleep::display_power(false).await {
            warn!("turning the screen off: {err:#}");
        }
        if self.config().cec.power_tv {
            self.cec.tv_standby().await;
        }
        self.asleep.store(true, Ordering::Relaxed);
        self.notify().await;
    }

    /// Something other than the remote used the TV (a phone started casting).
    pub async fn external_activity(&self) {
        self.last_activity.store(now(), Ordering::Relaxed);
        self.wake().await;
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep.load(Ordering::Relaxed)
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
        if self.config().cec.power_tv {
            self.cec.tv_on().await;
        }
        self.notify().await;
        true
    }

    pub async fn execute(self: &Arc<Self>, cmd: Command) -> Result<()> {
        if cmd.is_input() {
            self.last_input.store(now(), Ordering::Relaxed);
        }
        self.last_activity.store(now(), Ordering::Relaxed);
        // Letting go of a held key is always fine, and never wakes or unlocks anything.
        if let Command::Key { code, action: KeyAction::Up, .. } = &cmd {
            if let Some(key) = keymap::from_code(code) {
                self.input.key_up(key)?;
            }
            return Ok(());
        }
        // The key that wakes the TV only wakes it, like on the screensaver.
        if !matches!(cmd, Command::Sleep) && self.wake().await && cmd.is_input() {
            return Ok(());
        }
        if self.lock.is_locked() {
            if let Some(code) = lock_navigation(&cmd) {
                let _ = self.events.send(ServerMsg::ShellKey { code: code.into() });
                return Ok(());
            }
        }
        if self.lock.is_locked() && !matches!(cmd, Command::Unlock { .. } | Command::Sleep) {
            return Err(crate::i18n::error("error.locked", &[]));
        }
        if crate::shell_controls::POINTER.load(Ordering::Relaxed) {
            if let Command::Key { code, .. } = &cmd {
                match code.as_str() {
                    "ArrowLeft" => self.input.motion(-30, 0)?,
                    "ArrowRight" => self.input.motion(30, 0)?,
                    "ArrowUp" => self.input.motion(0, -30)?,
                    "ArrowDown" => self.input.motion(0, 30)?,
                    "PageUp" => self.input.scroll(0, 1)?,
                    "PageDown" => self.input.scroll(0, -1)?,
                    "Enter" => self.input.button(crate::input::Button::Left, Default::default()).await?,
                    "ContextMenu" => self.input.button(crate::input::Button::Right, Default::default()).await?,
                    "Escape" => {
                        crate::shell_controls::POINTER.store(false, Ordering::Relaxed);
                    }
                    _ => {}
                }
                return Ok(());
            }
        }
        if let Command::Key { code, .. } = &cmd {
            if code == "ContextMenu" && self.apps.running().await.is_some() && !crate::shell_controls::active() {
                crate::shell_controls::show(true).await?;
                let _ = self.events.send(ServerMsg::ShellControls { open: true });
                return Ok(());
            }
        }
        // While the on-screen keyboard is open, the arrows, OK and Back move over it.
        if let Command::Key { code, .. } = &cmd {
            if self.keyboard.is_open() && self.keyboard_key(code).await? {
                return Ok(());
            }
        }
        // A mode on trial: OK keeps it and Back goes back, over an app as on the home screen.
        if let Command::Key { code, .. } = &cmd {
            if self.display_trial.lock().unwrap().is_some() {
                match code.as_str() {
                    "Enter" | "NumpadEnter" => return self.keep_display_mode().await,
                    "Escape" | "BrowserBack" => {
                        self.end_display_trial().await;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
        match cmd {
            Command::Key { code, shift, ctrl, alt, meta, action } => {
                let key = keymap::from_code(&code).with_context(|| format!("unknown key code {code:?}"))?;
                let mods = Mods { shift, ctrl, alt, meta };
                // A held key with modifiers is rare; it is simply pressed.
                if action == KeyAction::Down && !(shift || ctrl || alt || meta) {
                    self.input.key_down(key)?;
                } else {
                    self.input.key(key, mods)?;
                }
            }
            Command::Text { text } => self.type_text(&text).await?,
            Command::Move { dx, dy } => {
                self.input.motion(dx.clamp(-MAX_DELTA, MAX_DELTA), dy.clamp(-MAX_DELTA, MAX_DELTA))?;
            }
            Command::Scroll { dx, dy } => self.input.scroll(dx.clamp(-50, 50), dy.clamp(-50, 50))?,
            Command::Button { button, action } => self.input.button(button, action).await?,
            Command::Launch { id } => {
                crate::shell_controls::clear();
                self.apps.launch(&id).await?;
            }
            Command::Stop => {
                crate::shell_controls::clear();
                self.apps.stop().await;
            }
            Command::Volume { action, level } => {
                let audio = self.config().audio.clone();
                audio::apply(action, level, &audio).await?;
                if let Ok(volume) = audio::get().await {
                    self.show_volume(&volume).await;
                    let mut saved = self.volumes.lock().await;
                    if audio.per_app_volume {
                        let current = saved.current.clone();
                        saved.set(&current, volume.level);
                    }
                }
                self.notify().await;
            }
            Command::Media { action, seconds } => {
                if let media::Outcome::Keys(keys) = media::control(action, seconds).await {
                    for code in keys {
                        let key = keymap::from_code(code).with_context(|| format!("no key {code}"))?;
                        self.input.key(key, Mods::default())?;
                    }
                }
            }
            Command::SleepTimer { minutes } => self.set_sleep_timer(minutes).await,
            Command::WifiConnect { ssid, password } => {
                info!(%ssid, "joining Wi-Fi");
                crate::wireless::connect(&ssid, password.as_deref()).await?;
            }
            Command::Bluetooth { action, address } => {
                crate::wireless::bluetooth_action(action, address.as_deref()).await?;
                // Audio devices show up as outputs once connected.
                let _ = self.events.send(ServerMsg::Audio(self.audio_info().await));
            }
            Command::AudioOutput { id } => {
                if !self.night_audio.switch_output(id).await? {
                    audio::set_output(id).await?;
                }
                let _ = self.events.send(ServerMsg::Audio(self.audio_info().await));
                self.notify().await;
            }
            Command::Reorder { ids } => {
                self.apps.reorder(&ids)?;
                let _ = self.events.send(ServerMsg::Apps { apps: self.app_infos() });
            }
            Command::HideApp { id, hidden } => {
                self.apps.set_hidden(&id, hidden)?;
                let _ = self.events.send(ServerMsg::Apps { apps: self.app_infos() });
            }
            Command::Set { key, value } => self.change_setting(key, value, false).await?,
            Command::DisplayMode { mode } => self.try_display_mode(&mode).await?,
            Command::DisplayKeep => self.keep_display_mode().await?,
            Command::DisplayRevert => self.end_display_trial().await,
            Command::Keyboard { open } => {
                if crate::shell_controls::active() && open != Some(false) {
                    crate::shell_controls::show(false).await?;
                }
                let open = open.unwrap_or(!self.keyboard.is_open());
                self.set_keyboard(open).await;
            }
            Command::Sleep => self.go_to_sleep().await,
            Command::Power { action } => self.power(action).await?,
            Command::PointerMode { enabled } => {
                if enabled && crate::shell_controls::active() {
                    crate::shell_controls::show(false).await?;
                    let _ = self.events.send(ServerMsg::ShellControls { open: false });
                }
                crate::shell_controls::POINTER.store(enabled, Ordering::Relaxed);
            }
            Command::ShellControls { open } => {
                crate::shell_controls::show(open).await?;
                let _ = self.events.send(ServerMsg::ShellControls { open });
            }
            Command::Lock => {
                self.lock.lock()?;
                crate::shell_controls::clear();
                self.set_keyboard(false).await;
                info!("locked");
                self.apps.stop().await;
                self.apply_cast();
                self.notify().await;
            }
            Command::Unlock { pin } => {
                self.lock.unlock(&pin)?;
                info!("unlocked");
                self.apply_cast();
                self.notify().await;
            }
            Command::SetPin { current, pin } => {
                self.lock.set_pin(current.as_deref(), pin.as_deref())?;
                info!(pin_set = self.lock.has_pin(), "PIN changed");
                let _ = self.events.send(ServerMsg::Settings(Box::new(self.settings_info())));
                self.notify().await;
            }
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

/// Volume changes made outside rustv (an app's own control, another PipeWire
/// client, an output switch) reach the remotes and the shell within 2 s.
/// One `wpctl get-volume` every 2 s costs next to nothing.
fn watch_volume(state: std::sync::Weak<AppState>) {
    tokio::spawn(async move {
        let mut last = None;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(state) = state.upgrade() else { return };
            let Ok(volume) = audio::get().await else { continue };
            if last.is_some_and(|last| last != volume) {
                state.show_volume(&volume).await;
                state.notify().await;
            }
            last = Some(volume);
        }
    });
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod lock_navigation_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_plain_navigation_reaches_the_lock_screen() {
        for code in
            ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Escape", "Backspace", "Digit0", "Digit9"]
        {
            let command: Command = serde_json::from_value(json!({"type":"key","code":code})).unwrap();
            assert_eq!(lock_navigation(&command), Some(code));
        }
        for value in [
            json!({"type":"key","code":"KeyA"}),
            json!({"type":"key","code":"Enter","ctrl":true}),
            json!({"type":"key","code":"Enter","meta":true}),
            json!({"type":"key","code":"Digit1","action":"up"}),
            json!({"type":"key","code":"DigitX"}),
            json!({"type":"text","text":"1234"}),
            json!({"type":"pointer_mode","enabled":true}),
        ] {
            let command: Command = serde_json::from_value(value).unwrap();
            assert_eq!(lock_navigation(&command), None);
        }
    }
}
