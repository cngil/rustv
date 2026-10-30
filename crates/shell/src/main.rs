//! `rustv-shell`, the on-screen home screen.
//!
//! A thin client of `tvd`: it renders the app list and sends `launch`
//! commands over the same WebSocket the web remote uses. All state lives in
//! the daemon, so the shell can crash or restart without losing anything.

mod controls;
mod dialog;
mod locale;
mod menu;
mod net;
mod settings;
mod sounds;
mod wallpaper;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Local;
use slint::{
    Color, ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
    Weak,
};
use tokio::sync::mpsc;

slint::include_modules!();

/// How long the volume overlay stays after a change.
const OSD_TIME: Duration = Duration::from_millis(2200);
/// Clears "Opening…" if the app never shows up.
const OPENING_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the shell checks whether it is time for the screensaver.
const IDLE_CHECK: Duration = Duration::from_secs(2);
/// Styles `random` cycles through; your own pictures join them once there are any.
const SCREENSAVER_STYLES: [&str; 4] = ["aurora", "orbits", "clock", "images"];
/// Screensaver frame rates. Slow motion looks the same at 30 fps and costs
/// half of 60; the plain clock only drifts, so 10 fps is plenty.
const ANIMATION_FRAME: Duration = Duration::from_millis(33);
const CLOCK_FRAME: Duration = Duration::from_millis(100);

thread_local! {
    static FEEDBACK_TIMER: Timer = Timer::default();
    static ACTIVE_PANEL: RefCell<String> = const { RefCell::new(String::new()) };
    static OSD_TIMER: Timer = Timer::default();
    static OPENING_TIMER: Timer = Timer::default();
    static LAST_ACTIVITY: Cell<Instant> = Cell::new(Instant::now());
    static RUNNING_ID: RefCell<Option<String>> = const { RefCell::new(None) };
    static APP_RUNNING: Cell<bool> = const { Cell::new(false) };
    static ASLEEP: Cell<bool> = const { Cell::new(false) };
    static SCREENSAVER: RefCell<net::ScreensaverConfig> = RefCell::new(net::ScreensaverConfig::default());
    static SCREENSAVER_COUNT: Cell<usize> = const { Cell::new(0) };
    static FRAME_TIMER: Timer = Timer::default();
    static LOCALE: RefCell<locale::Locale> = RefCell::new(locale::Locale::default());
    static PIN: RefCell<Option<dialog::Dialog>> = const { RefCell::new(None) };
    static PIN_COMMANDS: RefCell<Option<mpsc::UnboundedSender<String>>> = const { RefCell::new(None) };
    static MENU: RefCell<menu::Menu> = RefCell::new(menu::Menu::default());
    /// The apps as tvd sent them, so the Settings tile can be added or taken away later.
    static APPS: RefCell<Vec<AppEntry>> = const { RefCell::new(Vec::new()) };
    static POWER_TILE: Cell<bool> = const { Cell::new(true) };
    static SETTINGS_TILE: Cell<bool> = const { Cell::new(true) };
}

/// Launching this id opens the settings; app ids cannot contain `*`.
const POWER_ID: &str = "*power";
const SETTINGS_ID: &str = "*settings";

/// App data sent from the network thread. `Image` is not `Send`, so icons travel as paths.
#[derive(Debug, Clone)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub details: String,
    pub icon: Option<PathBuf>,
}

fn main() -> Result<()> {
    let settings = settings::Settings::load().context("reading rustv settings")?;
    let ui = Shell::new()?;
    ui.window().set_fullscreen(true);
    wallpaper::init(settings.clone());

    let clock = Timer::default();
    update_clock(&ui);
    let weak = ui.as_weak();
    clock.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        if let Some(ui) = weak.upgrade() {
            update_clock(&ui);
        }
    });

    ui.on_activity(touch);
    ui.on_return_app(|| {
        PIN_COMMANDS.with(|sender| {
            if let Some(sender) = sender.borrow().as_ref() {
                let _ = sender.send(serde_json::json!({"type":"shell_controls","open":false}).to_string());
            }
        });
    });
    ui.on_sound(|name| sounds::PLAYER.with(|player| player.borrow_mut().play_named(&name)));
    let idle = Timer::default();
    let weak = ui.as_weak();
    idle.start(TimerMode::Repeated, IDLE_CHECK, move || {
        if let Some(ui) = weak.upgrade() {
            maybe_start_screensaver(&ui);
        }
    });

    let (commands, command_rx) = mpsc::unbounded_channel::<String>();
    PIN_COMMANDS.with(|sender| *sender.borrow_mut() = Some(commands.clone()));
    let weak_pin = ui.as_weak();
    ui.on_pin_key(move |key| {
        pin_key(&key);
        if let Some(ui) = weak_pin.upgrade() {
            render_pin(&ui);
        }
    });
    let weak = ui.as_weak();
    let to_tvd = commands.clone();
    ui.on_launch(move |id| {
        if id == SETTINGS_ID || id == POWER_ID {
            if let Some(ui) = weak.upgrade() {
                open_menu(&ui, id == POWER_ID);
            }
            return;
        }
        let msg = serde_json::json!({ "type": "launch", "id": id.as_str() });
        let _ = to_tvd.send(msg.to_string());
        set_opening(&weak, id.to_string());
    });
    let weak = ui.as_weak();
    ui.on_open_settings(move || {
        if let Some(ui) = weak.upgrade() {
            open_settings(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_context(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        let apps = APPS.with(|apps| apps.borrow().clone());
        let Some(at) = apps.iter().position(|app| app.id == id.as_str()) else { return };
        let app = &apps[at];
        let label = |key| LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, key).to_string());
        let running = RUNNING_ID.with(|id| id.borrow().as_deref() == Some(app.id.as_str()));
        let mut rows = vec![
            (label("common.settings"), serde_json::json!({"type":"_settings"})),
            (
                label(if running { "remote.app.menu.close" } else { "remote.app.menu.open" }),
                if running {
                    serde_json::json!({"type":"stop"})
                } else {
                    serde_json::json!({"type":"launch","id":app.id})
                },
            ),
            (format!("{}: {}", label("remote.app.menu.info"), app.details), serde_json::Value::Null),
            (label("remote.app.menu.remove"), serde_json::json!({"type":"hide_app","id":app.id,"hidden":true})),
        ];
        for (offset, key) in [(-1isize, "tv.app.move_left"), (1, "tv.app.move_right")] {
            let next = at as isize + offset;
            if next >= 0 && next < apps.len() as isize {
                let mut ids: Vec<_> = apps.iter().map(|a| a.id.clone()).collect();
                ids.swap(at, next as usize);
                rows.push((label(key), serde_json::json!({"type":"reorder","ids":ids})));
            }
        }
        MENU.with(|menu| menu.borrow_mut().open_context(rows));
        render_menu(&ui);
        ui.set_menu_title(app.name.clone().into());
        ui.set_settings_open(true);
    });
    let to_tvd = commands.clone();
    ui.on_trial_key(move |keep| {
        let msg = serde_json::json!({ "type": if keep { "display_keep" } else { "display_revert" } });
        let _ = to_tvd.send(msg.to_string());
    });
    let weak = ui.as_weak();
    ui.on_settings_key(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        let outcome = MENU.with(|menu| menu.borrow_mut().key(&key));
        let send = |msg: &serde_json::Value| {
            let _ = commands.send(msg.to_string());
        };
        match outcome {
            menu::Outcome::Nothing => ui.invoke_sound("move".into()),
            menu::Outcome::Send(msg) => {
                send(&msg);
                if msg["type"] == "set" && msg["key"] == "theme.preset" {
                    for key in ["theme.accent", "theme.accent_2", "theme.background"] {
                        send(&serde_json::json!({"type":"set","key":key,"value":null}));
                    }
                }
                ui.invoke_sound("select".into());
            }
            menu::Outcome::Close => ui.set_settings_open(false),
            menu::Outcome::SendAndClose(msg) => {
                match msg["type"].as_str() {
                    Some("_pin") => {
                        let mode = MENU.with(|menu| if menu.borrow().has_pin() { "change" } else { "set" });
                        PIN.with(|pin| *pin.borrow_mut() = Some(dialog::Dialog::new(mode)));
                        render_pin(&ui);
                    }
                    Some("_volume") => {
                        PIN.with(|pin| *pin.borrow_mut() = Some(dialog::Dialog::new("volume")));
                    }
                    Some("_remove_pin") => {
                        PIN.with(|pin| *pin.borrow_mut() = Some(dialog::Dialog::new("remove")));
                        render_pin(&ui);
                    }
                    Some(kind) if kind.starts_with("_panel_") => {
                        open_panel(&ui, &kind[7..]);
                        return;
                    }
                    Some("_screenshot") => send(&msg),
                    Some(kind) if kind.starts_with("_color_") => {
                        let mut dialog = dialog::Dialog::new("color");
                        dialog.target = format!("theme.{}", &kind[7..]);
                        PIN.with(|pin| *pin.borrow_mut() = Some(dialog));
                    }
                    Some("_settings") => {
                        open_settings(&ui);
                        return;
                    }
                    Some("_wifi") => {
                        let ssid = msg["ssid"].as_str().unwrap_or("").to_string();
                        if msg["secure"] == true {
                            let mut dialog = dialog::Dialog::new("wifi");
                            dialog.target = ssid;
                            PIN.with(|pin| *pin.borrow_mut() = Some(dialog));
                        } else {
                            send(&serde_json::json!({"type":"wifi_connect","ssid":ssid}));
                        }
                    }
                    _ => send(&msg),
                }
                ui.set_settings_open(false);
            }
        }
        render_menu(&ui);
        render_pin(&ui);
    });

    let weak = ui.as_weak();
    std::thread::Builder::new().name("net".into()).spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
        runtime.block_on(net::run(settings, weak, command_rx));
    })?;

    ui.run()?;
    Ok(())
}

/// Input or an app change: the screensaver waits again from now.
fn touch() {
    LAST_ACTIVITY.with(|last| last.set(Instant::now()));
}

fn maybe_start_screensaver(ui: &Shell) {
    let cfg = SCREENSAVER.with(|cfg| cfg.borrow().clone());
    // While an app is open the shell is hidden behind it, so there is nothing to protect.
    if !cfg.enabled || APP_RUNNING.with(Cell::get) || ASLEEP.with(Cell::get) || ui.get_screensaver_active() {
        return;
    }
    let idle = LAST_ACTIVITY.with(|last| last.get().elapsed());
    if idle.as_secs_f32() < cfg.idle_minutes * 60.0 {
        return;
    }
    let count = SCREENSAVER_COUNT.with(|n| n.replace(n.get() + 1));
    let style = pick_style(&cfg.style, count, wallpaper::count() > 0);
    ui.set_screensaver_style(style.into());
    ui.set_screensaver_time(0.0);
    ui.set_screensaver_active(true);
    let slides = style == "images";
    if slides {
        wallpaper::start_slides(ui);
    }

    let started = Instant::now();
    let frame = if style == "clock" { CLOCK_FRAME } else { ANIMATION_FRAME };
    let weak = ui.as_weak();
    FRAME_TIMER.with(|timer| {
        timer.start(TimerMode::Repeated, frame, move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_screensaver_active() {
                ui.set_screensaver_time(started.elapsed().as_secs_f32());
                if slides {
                    wallpaper::next_slide_if_due(&ui);
                }
            } else {
                // Woken up from the UI side; nothing left to animate.
                FRAME_TIMER.with(Timer::stop);
            }
        });
    });
}

/// `random` takes the next style each time, starting at a different one on every boot.
/// Without pictures, `images` falls back to a generated style.
fn pick_style(style: &str, count: usize, photos: bool) -> &str {
    if style == "images" && !photos {
        return SCREENSAVER_STYLES[0];
    }
    if style != "random" {
        return style;
    }
    let styles = if photos { &SCREENSAVER_STYLES[..] } else { &SCREENSAVER_STYLES[..3] };
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as usize / 60)
        .unwrap_or(0);
    styles[(seed + count) % styles.len()]
}

/// The screen is off: stop animating, nobody can see it. After waking, the
/// screensaver waits its full time again.
pub fn set_asleep(ui: &Weak<Shell>, asleep: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        if ASLEEP.with(|a| a.replace(asleep)) == asleep {
            return;
        }
        if asleep {
            ui.set_screensaver_active(false);
            FRAME_TIMER.with(Timer::stop);
        } else {
            touch();
        }
    });
}

/// The pairing QR code, one pixel per module; the UI scales it up without smoothing.
pub fn set_pairing(ui: &Weak<Shell>, url: String, size: usize, modules: String) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_pairing_url(display_url(&url).into());
        if let Some(image) = qr_image(size, &modules) {
            ui.set_pairing_qr(image);
        }
    });
}

fn qr_image(size: usize, modules: &str) -> Option<Image> {
    if size == 0 || modules.len() != size * size {
        return None;
    }
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size as u32, size as u32);
    for (pixel, module) in buffer.make_mut_slice().iter_mut().zip(modules.bytes()) {
        let v = if module == b'1' { 0 } else { 255 };
        *pixel = Rgba8Pixel { r: v, g: v, b: v, a: 255 };
    }
    Some(Image::from_rgba8(buffer))
}

/// The link without its token, for people who would rather type the address.
fn display_url(url: &str) -> String {
    url.split('#').next().unwrap_or(url).trim_end_matches('/').to_string()
}

pub fn set_locked(ui: &Weak<Shell>, locked: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        if locked && !ui.get_locked() {
            ui.set_settings_open(false);
            PIN.with(|pin| *pin.borrow_mut() = Some(dialog::Dialog::new("unlock")));
        } else if !locked && ui.get_locked() {
            PIN.with(|pin| *pin.borrow_mut() = None);
        }
        ui.set_locked(locked);
        render_pin(&ui);
    });
}

/// The now playing card: title, "artist · 1:23 / 4:05", progress and cover.
pub fn set_now_playing(ui: &Weak<Shell>, now: Option<net::NowPlaying>, art: Option<std::path::PathBuf>) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_playing_open(now.is_some());
        let Some(now) = now else { return };
        let clock = |seconds: f64| {
            let s = seconds.max(0.0) as u64;
            if s >= 3600 {
                format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
            } else {
                format!("{}:{:02}", s / 60, s % 60)
            }
        };
        let place = match (now.position, now.length) {
            (Some(at), Some(length)) => format!("{} / {}", clock(at.min(length)), clock(length)),
            (None, Some(length)) => clock(length),
            _ => String::new(),
        };
        let who = if now.artist.is_empty() { now.player } else { now.artist };
        let sub: Vec<String> = [who, place].into_iter().filter(|part| !part.is_empty()).collect();
        ui.set_playing_title(now.title.into());
        ui.set_playing_sub(sub.join(" · ").into());
        let progress = match (now.position, now.length) {
            (Some(at), Some(length)) if length > 0.0 => (at / length) as f32,
            _ => 0.0,
        };
        ui.set_playing_progress(progress);
        // Decoded once per cover, not on every update.
        thread_local!(static SHOWN: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) });
        let changed = SHOWN.with(|shown| shown.replace(art.clone()) != art);
        if changed {
            let image = art.and_then(|path| slint::Image::load_from_path(&path).ok());
            ui.set_playing_has_art(image.is_some());
            ui.set_playing_art(image.unwrap_or_default());
        }
    });
}

pub fn set_show_pairing(ui: &Weak<Shell>, show: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_show_pairing(show));
}

pub fn set_screensaver(ui: &Weak<Shell>, cfg: net::ScreensaverConfig) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_screensaver_clock(cfg.clock);
        ui.set_screensaver_date(cfg.date);
        if !cfg.enabled {
            ui.set_screensaver_active(false);
        } else if cfg.style != "random" && ui.get_screensaver_active() {
            ui.set_screensaver_style(cfg.style.clone().into());
        }
        SCREENSAVER.with(|current| *current.borrow_mut() = cfg);
    });
}

fn update_clock(ui: &Shell) {
    let now = Local::now();
    LOCALE.with(|locale| {
        let locale = locale.borrow();
        ui.set_time(locale.time(&now).into());
        ui.set_date(locale.date(&now).into());
        ui.set_greeting(locale.greeting(&now).into());
    });
}

/// The palette tvd works out from the theme, as `#rrggbb` strings.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Colors {
    accent: String,
    accent_2: String,
    accent_soft: String,
    background: String,
    surface: String,
    line: String,
    // Missing from an older tvd; the shell's defaults stay then.
    #[serde(default)]
    on_accent: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    muted: String,
    #[serde(default)]
    surface_2: String,
    #[serde(default)]
    surface_3: String,
}

pub fn set_theme(ui: &Weak<Shell>, colors: Colors) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let p = ui.global::<Palette>();
        let set = |hex: &str, apply: &dyn Fn(Color)| {
            if let Some(color) = parse_hex(hex) {
                apply(color);
            }
        };
        set(&colors.accent, &|c| p.set_accent(c));
        set(&colors.accent_2, &|c| p.set_accent_2(c));
        set(&colors.accent_soft, &|c| p.set_accent_soft(c));
        set(&colors.background, &|c| p.set_background(c));
        set(&colors.surface, &|c| p.set_surface(c));
        set(&colors.surface_2, &|c| p.set_surface_2(c));
        set(&colors.surface_3, &|c| p.set_surface_3(c));
        set(&colors.line, &|c| p.set_line(c));
        set(&colors.on_accent, &|c| p.set_on_accent(c));
        set(&colors.text, &|c| p.set_text(c));
        set(&colors.muted, &|c| p.set_muted(c));
    });
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#').filter(|d| d.len() == 6)?;
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some(Color::from_rgb_u8(channel(0)?, channel(2)?, channel(4)?))
}

fn open_settings(ui: &Shell) {
    open_menu(ui, false);
}

fn open_menu(ui: &Shell, power: bool) {
    ACTIVE_PANEL.with(|active| active.borrow_mut().clear());
    MENU.with(|menu| menu.borrow_mut().open(power));
    render_menu(ui);
    ui.set_settings_open(true);
    ui.invoke_sound("select".into());
}

/// Puts the menu's rows, scroll position and hint into the UI.
fn render_menu(ui: &Shell) {
    MENU.with(|menu| {
        let mut menu = menu.borrow_mut();
        let title = LOCALE.with(|locale| {
            rustv_locales::text(
                &locale.borrow().language,
                if menu.is_power() { "common.power" } else { "common.settings" },
            )
        });
        if !menu.is_context() {
            ui.set_menu_title(title.into());
        }
        let rows: Vec<SettingRow> = menu
            .rows()
            .into_iter()
            .map(|row| SettingRow {
                header: row.header,
                label: row.label.into(),
                value: row.value.into(),
                kind: row.kind.into(),
                on: row.on,
                enabled: row.enabled,
                focused: row.focused,
                y: row.y,
                before: row.before,
                after: row.after,
            })
            .collect();
        ui.set_settings_rows(ModelRc::new(VecModel::from(rows)));
        ui.set_settings_scroll(menu.scroll());
        ui.set_settings_hint(menu.hint().into());
    });
}

/// New settings from tvd, after any change from here, the phone or the config file.
pub fn set_settings(ui: &Weak<Shell>, settings: serde_json::Value) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        MENU.with(|menu| menu.borrow_mut().set_settings(settings));
        render_menu(&ui);
    });
}

/// The "keep this resolution?" prompt, with its countdown, while tvd tries a mode.
pub fn set_trial(ui: &Weak<Shell>, trial: Option<net::Trial>) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_trial_open(trial.is_some());
        let Some(trial) = trial else { return };
        let countdown = LOCALE.with(|locale| locale.borrow().trial_countdown(trial.seconds));
        let label = match trial.mode.split_once('@') {
            Some((size, hz)) => format!("{}, {hz} Hz", size.replace('x', " × ")),
            None => trial.mode,
        };
        ui.set_trial_mode(label.into());
        ui.set_trial_countdown(countdown.into());
    });
}

pub fn set_overlay(ui: &Weak<Shell>, visible: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        MENU.with(|menu| menu.borrow_mut().set_overlay(visible));
        if ui.get_settings_open() {
            render_menu(&ui);
        }
    });
}

pub fn set_locale(ui: &Weak<Shell>, locale: locale::Locale) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let s = locale.strings();
        let g = ui.global::<Strings>();
        g.set_subtitle(s.subtitle.into());
        g.set_no_apps(s.no_apps.into());
        g.set_hint_move(s.hint_move.into());
        g.set_hint_open(s.hint_open.into());
        g.set_opening(s.opening.into());
        g.set_connecting(s.connecting.into());
        g.set_volume(s.volume.into());
        g.set_muted(s.muted.into());
        g.set_pair_title(s.pair_title.into());
        g.set_pair_body(s.pair_body.into());
        g.set_locked(s.locked.into());
        g.set_unlock_hint(s.unlock_hint.into());
        g.set_settings(s.settings.into());
        g.set_hint_change(s.hint_change.into());
        g.set_hint_select(s.hint_select.into());
        g.set_hint_close(s.hint_close.into());
        g.set_back_key(s.back_key.into());
        g.set_trial_title(s.trial_title.into());
        g.set_keep(s.keep.into());
        g.set_go_back(s.go_back.into());
        LOCALE.with(|current| *current.borrow_mut() = locale);
        update_clock(&ui);
        // The Settings tile carries a translated name.
        show_apps(&ui);
    });
}

pub fn set_connected(ui: &Weak<Shell>, connected: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_connected(connected));
}

pub fn set_apps(ui: &Weak<Shell>, apps: Vec<AppEntry>) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        APPS.with(|stored| *stored.borrow_mut() = apps);
        show_apps(&ui);
    });
}

/// The app row: the apps, then the Settings tile unless it is switched off.
fn show_apps(ui: &Shell) {
    let mut tiles: Vec<AppTile> = APPS.with(|apps| apps.borrow().iter().cloned().map(tile).collect());
    if SETTINGS_TILE.with(Cell::get) {
        let name = LOCALE.with(|locale| locale.borrow().strings().settings);
        tiles.push(AppTile { id: SETTINGS_ID.into(), name: name.into(), settings: true, ..Default::default() });
    }
    if POWER_TILE.with(Cell::get) {
        let name = LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, "common.power"));
        tiles.push(AppTile { id: POWER_ID.into(), name: name.into(), power: true, ..Default::default() });
    }
    let count = tiles.len() as i32;
    ui.set_apps(ModelRc::new(VecModel::from(tiles)));
    if ui.get_selected() >= count {
        ui.set_selected((count - 1).max(0));
    }
}

fn tile(app: AppEntry) -> AppTile {
    let icon = app.icon.as_deref().and_then(|path| Image::load_from_path(path).ok());
    AppTile {
        initial: app.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        id: app.id.into(),
        name: app.name.into(),
        settings: false,
        power: false,
    }
}

pub fn set_running_id(ui: &Weak<Shell>, id: Option<String>) {
    let _ = ui.upgrade_in_event_loop(move |_| RUNNING_ID.with(|stored| *stored.borrow_mut() = id));
}

pub fn set_running(ui: &Weak<Shell>, running: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        if running && !APP_RUNNING.with(Cell::get) {
            ui.set_opening_id(SharedString::new());
            ui.set_screensaver_active(false);
            // An app started from the phone; the menu would wait behind it otherwise.
            ui.set_settings_open(false);
        }
        if !running {
            ui.set_controls_active(false);
        }
        if APP_RUNNING.with(|r| r.replace(running)) != running {
            touch();
        }
    });
}

fn set_opening(ui: &Weak<Shell>, id: String) {
    if let Some(shell) = ui.upgrade() {
        shell.set_opening_id(id.into());
    }
    let weak = ui.clone();
    OPENING_TIMER.with(|timer| {
        timer.start(TimerMode::SingleShot, OPENING_TIMEOUT, move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_opening_id(SharedString::new());
            }
        });
    });
}

pub fn set_volume(ui: &Weak<Shell>, level: f32, muted: bool, show: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_volume(level);
        ui.set_muted(muted);
        if !show {
            return;
        }
        ui.set_osd_visible(true);
        let weak = ui.as_weak();
        OSD_TIMER.with(|timer| {
            timer.start(TimerMode::SingleShot, OSD_TIME, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_osd_visible(false);
                }
            });
        });
    });
}

pub fn set_max_volume(ui: &Weak<Shell>, max: f32) {
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_max_volume(max));
}

pub fn set_visibility(ui: &Weak<Shell>, v: net::Visibility) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_show_greeting(v.greeting);
        ui.set_show_clock(v.clock);
        ui.set_show_date(v.date);
        ui.set_show_hints(v.hints);
        ui.set_show_volume_overlay(v.volume_overlay);
        sounds::PLAYER.with(|player| player.borrow_mut().configure(v.sounds, v.sound_volume));
        ui.set_settings_enabled(v.settings);
        if SETTINGS_TILE.with(|tile| tile.replace(v.settings)) != v.settings {
            show_apps(&ui);
        }
        ui.set_power_enabled(v.power);
        if POWER_TILE.with(|tile| tile.replace(v.power)) != v.power {
            show_apps(&ui);
        }
        if MENU.with(|menu| if menu.borrow().is_power() { !v.power } else { !v.settings }) {
            ui.set_settings_open(false);
        }
    });
}

/// Show the daemon's pending power action using the existing full-screen sleep cover.
pub fn set_power(ui: &Weak<Shell>, power: Option<String>) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_power_message(
            LOCALE
                .with(|locale| {
                    power
                        .as_ref()
                        .map(|action| {
                            rustv_locales::text(&locale.borrow().language, &format!("power.going.{action}")).to_string()
                        })
                        .unwrap_or_default()
                })
                .into(),
        );
    });
}

pub fn show_controls(ui: &Weak<Shell>, open: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_controls_active(open);
        if open {
            open_settings(&ui);
        } else {
            ui.set_settings_open(false);
        }
    });
}

fn open_panel(ui: &Shell, panel: &str) {
    ACTIVE_PANEL.with(|active| *active.borrow_mut() = panel.into());
    if matches!(panel, "controls" | "timer") {
        show_panel(ui, panel, serde_json::Value::Null);
    } else {
        let text = LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, "common.checking"));
        MENU.with(|menu| menu.borrow_mut().open_context(vec![(text.into(), serde_json::Value::Null)]));
        render_menu(ui);
        ui.set_settings_open(true);
        PIN_COMMANDS.with(|sender| {
            if let Some(sender) = sender.borrow().as_ref() {
                let _ = sender.send(serde_json::json!({"type":"_get","panel":panel}).to_string());
            }
        });
    }
}

fn show_panel(ui: &Shell, panel: &str, data: serde_json::Value) {
    let rows = LOCALE.with(|locale| controls::rows(panel, &data, &locale.borrow().language));
    MENU.with(|menu| menu.borrow_mut().open_context(rows));
    render_menu(ui);
    let title_key = match panel {
        "audio" => "remote.output.title",
        "wifi" => "section.wifi",
        "bluetooth" => "section.bluetooth",
        "network" => "remote.network.title",
        "system" => "section.system",
        "apps" => "tv.controls.restore",
        "timer" => "remote.timer.title",
        _ => "tv.controls.media",
    };
    ui.set_menu_title(LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, title_key)).into());
    ui.set_settings_open(true);
}

pub fn panel_result(ui: &Weak<Shell>, panel: String, result: Result<serde_json::Value, String>) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        if ui.get_locked() || !ui.get_settings_open() || ACTIVE_PANEL.with(|active| *active.borrow() != panel) {
            return;
        }
        match result {
            Ok(data) => show_panel(&ui, &panel, data),
            Err(error) => {
                let error = LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, &error).to_string());
                MENU.with(|menu| menu.borrow_mut().open_context(vec![(error, serde_json::Value::Null)]));
                render_menu(&ui);
            }
        }
    });
}

fn pin_key(key: &str) {
    PIN.with(|pin| {
        let mut pin = pin.borrow_mut();
        let Some(dialog) = pin.as_mut() else { return };
        if key == "back" {
            if dialog.mode != "unlock" {
                *pin = None;
            } else {
                dialog.text.clear();
            }
            return;
        }
        if let Some(mut command) = dialog.key(key) {
            if command["type"] == "_cancel" {
                *pin = None;
                return;
            }
            command["_dialog_id"] = serde_json::json!(dialog.id);
            PIN_COMMANDS.with(|sender| {
                if let Some(sender) = sender.borrow().as_ref() {
                    let _ = sender.send(command.to_string());
                }
            });
        }
    });
}

fn render_pin(ui: &Shell) {
    PIN.with(|pin| {
        let pin = pin.borrow();
        ui.set_pin_open(pin.is_some());
        let Some(dialog) = pin.as_ref() else { return };
        let text = |key| LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, key).to_string());
        ui.set_pin_prompt(text(dialog.prompt()).into());
        ui.set_pin_digits(
            if matches!(dialog.mode.as_str(), "color" | "volume") {
                dialog.text.clone()
            } else {
                "●".repeat(dialog.text.chars().count())
            }
            .into(),
        );
        ui.set_pin_selected(dialog.selected as i32);
        ui.set_pin_text_mode(matches!(dialog.mode.as_str(), "wifi" | "color"));
        ui.set_pin_columns(if matches!(dialog.mode.as_str(), "wifi" | "color") { 10 } else { 3 });
        let keys = LOCALE.with(|locale| dialog.keys(&locale.borrow().language));
        ui.set_pin_keys(ModelRc::new(VecModel::from(keys.into_iter().map(SharedString::from).collect::<Vec<_>>())));
        ui.set_pin_error(
            if dialog.error.starts_with("remote.")
                || dialog.error.starts_with("tv.")
                || dialog.error.starts_with("common.")
            {
                text(&dialog.error)
            } else {
                dialog.error.clone()
            }
            .into(),
        );
        ui.set_pin_confirm(text("remote.pin.confirm").into());
        ui.set_pin_delete(text("remote.key.delete").into());
    });
}

pub fn screenshot_result(ui: &Weak<Shell>, result: Result<PathBuf, String>) {
    match result {
        Err(error) => command_error(ui, error),
        Ok(path) => {
            let _ = ui.upgrade_in_event_loop(move |ui| {
                if ui.get_locked() {
                    return;
                }
                match Image::load_from_path(&path) {
                    Ok(image) => {
                        ui.set_shot_image(image);
                        ui.set_shot_open(true);
                    }
                    Err(_) => return,
                }
                ui.set_feedback(path.display().to_string().into());
            });
        }
    }
}

pub fn command_error(ui: &Weak<Shell>, message: String) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let message = if message.starts_with("tv.") || message.starts_with("common.") {
            LOCALE.with(|locale| rustv_locales::text(&locale.borrow().language, &message).to_string())
        } else {
            message
        };
        ui.set_feedback(message.into());
        let weak = ui.as_weak();
        FEEDBACK_TIMER.with(|timer| {
            timer.start(TimerMode::SingleShot, Duration::from_secs(8), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_feedback("".into());
                }
            })
        });
    });
}

pub fn pin_result(ui: &Weak<Shell>, id: Option<u64>, error: Option<String>) {
    if id.is_none() {
        if let Some(error) = error {
            command_error(ui, error);
        }
        return;
    }
    let _ = ui.upgrade_in_event_loop(move |ui| {
        PIN.with(|pin| {
            let mut pin = pin.borrow_mut();
            if pin.as_ref().map(|dialog| dialog.id) != id {
                return;
            }
            if let Some(error) = error {
                if let Some(dialog) = pin.as_mut() {
                    dialog.failed(error);
                }
            } else {
                *pin = None;
            }
        });
        render_pin(&ui);
    });
}

pub fn lock_key(ui: &Weak<Shell>, code: String) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        if !ui.get_locked() {
            return;
        }
        let key = match code.as_str() {
            "ArrowUp" => "up",
            "ArrowDown" => "down",
            "ArrowLeft" => "left",
            "ArrowRight" => "right",
            "Enter" => "ok",
            "Escape" => "back",
            "Backspace" => "delete",
            code => code.strip_prefix("Digit").unwrap_or(""),
        };
        pin_key(key);
        render_pin(&ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_colors() {
        assert_eq!(parse_hex("#6c8cff"), Some(Color::from_rgb_u8(0x6c, 0x8c, 0xff)));
        assert_eq!(parse_hex("6c8cff"), None);
    }

    #[test]
    fn hides_the_token_in_the_shown_address() {
        assert_eq!(display_url("http://192.168.1.5:8765/#token=abc"), "http://192.168.1.5:8765");
    }

    #[test]
    fn picks_screensaver_styles() {
        assert_eq!(pick_style("orbits", 3, false), "orbits");
        let first = pick_style("random", 0, false);
        let second = pick_style("random", 1, false);
        assert!(SCREENSAVER_STYLES[..3].contains(&first) && SCREENSAVER_STYLES[..3].contains(&second));
        assert_ne!(first, second);
        // Pictures only once there are some.
        assert!((0..8).all(|n| pick_style("random", n, false) != "images"));
        assert!((0..8).any(|n| pick_style("random", n, true) == "images"));
        assert_eq!(pick_style("images", 0, false), "aurora");
        assert_eq!(pick_style("images", 0, true), "images");
    }
}
