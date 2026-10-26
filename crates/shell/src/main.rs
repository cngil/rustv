//! `rustv-shell`, the on-screen home screen.
//!
//! A thin client of `tvd`: it renders the app list and sends `launch`
//! commands over the same WebSocket the web remote uses. All state lives in
//! the daemon, so the shell can crash or restart without losing anything.

mod net;
mod settings;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{Local, Timelike};
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
/// Styles `random` cycles through.
const SCREENSAVER_STYLES: [&str; 3] = ["aurora", "orbits", "clock"];
/// Screensaver frame rates. Slow motion looks the same at 30 fps and costs
/// half of 60; the plain clock only drifts, so 10 fps is plenty.
const ANIMATION_FRAME: Duration = Duration::from_millis(33);
const CLOCK_FRAME: Duration = Duration::from_millis(100);

thread_local! {
    static OSD_TIMER: Timer = Timer::default();
    static OPENING_TIMER: Timer = Timer::default();
    static LAST_ACTIVITY: Cell<Instant> = Cell::new(Instant::now());
    static APP_RUNNING: Cell<bool> = const { Cell::new(false) };
    static ASLEEP: Cell<bool> = const { Cell::new(false) };
    static SCREENSAVER: RefCell<net::ScreensaverConfig> = RefCell::new(net::ScreensaverConfig::default());
    static SCREENSAVER_COUNT: Cell<usize> = const { Cell::new(0) };
    static FRAME_TIMER: Timer = Timer::default();
}

/// App data sent from the network thread. `Image` is not `Send`, so icons travel as paths.
#[derive(Debug, Clone)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
}

fn main() -> Result<()> {
    let settings = settings::Settings::load().context("reading rustv settings")?;
    let ui = Shell::new()?;
    ui.window().set_fullscreen(true);

    let clock = Timer::default();
    update_clock(&ui);
    let weak = ui.as_weak();
    clock.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        if let Some(ui) = weak.upgrade() {
            update_clock(&ui);
        }
    });

    ui.on_activity(touch);
    let idle = Timer::default();
    let weak = ui.as_weak();
    idle.start(TimerMode::Repeated, IDLE_CHECK, move || {
        if let Some(ui) = weak.upgrade() {
            maybe_start_screensaver(&ui);
        }
    });

    let (commands, command_rx) = mpsc::unbounded_channel::<String>();
    let weak = ui.as_weak();
    ui.on_launch(move |id| {
        let msg = serde_json::json!({ "type": "launch", "id": id.as_str() });
        let _ = commands.send(msg.to_string());
        set_opening(&weak, id.to_string());
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
    let style = pick_style(&cfg.style, count);
    ui.set_screensaver_style(style.into());
    ui.set_screensaver_time(0.0);
    ui.set_screensaver_active(true);

    let started = Instant::now();
    let frame = if style == "clock" { CLOCK_FRAME } else { ANIMATION_FRAME };
    let weak = ui.as_weak();
    FRAME_TIMER.with(|timer| {
        timer.start(TimerMode::Repeated, frame, move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_screensaver_active() {
                ui.set_screensaver_time(started.elapsed().as_secs_f32());
            } else {
                // Woken up from the UI side; nothing left to animate.
                FRAME_TIMER.with(Timer::stop);
            }
        });
    });
}

/// `random` takes the next style each time, starting at a different one on every boot.
fn pick_style(style: &str, count: usize) -> &str {
    if style != "random" {
        return style;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as usize / 60)
        .unwrap_or(0);
    SCREENSAVER_STYLES[(seed + count) % SCREENSAVER_STYLES.len()]
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
    ui.set_time(now.format("%H:%M").to_string().into());
    ui.set_date(now.format("%A, %-d %B").to_string().into());
    let greeting = match now.hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    ui.set_greeting(greeting.into());
}

pub fn set_connected(ui: &Weak<Shell>, connected: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_connected(connected));
}

pub fn set_apps(ui: &Weak<Shell>, apps: Vec<AppEntry>) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let tiles: Vec<AppTile> = apps.into_iter().map(tile).collect();
        let count = tiles.len() as i32;
        ui.set_apps(ModelRc::new(VecModel::from(tiles)));
        if ui.get_selected() >= count {
            ui.set_selected((count - 1).max(0));
        }
    });
}

fn tile(app: AppEntry) -> AppTile {
    let icon = app.icon.as_deref().and_then(|path| Image::load_from_path(path).ok());
    AppTile {
        initial: app.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
        color: color_for(&app.id),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        id: app.id.into(),
        name: app.name.into(),
    }
}

/// Stable color per app, same idea as the web remote.
fn color_for(id: &str) -> Color {
    let hue = id.chars().fold(0u32, |h, c| (h * 31 + c as u32) % 360) as f32;
    let (r, g, b) = hsl_to_rgb(hue, 0.65, 0.55);
    Color::from_rgb_u8(r, g, b)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r), to_u8(g), to_u8(b))
}

pub fn set_running(ui: &Weak<Shell>, running: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        if running {
            ui.set_opening_id(SharedString::new());
            ui.set_screensaver_active(false);
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
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_the_token_in_the_shown_address() {
        assert_eq!(display_url("http://192.168.1.5:8765/#token=abc"), "http://192.168.1.5:8765");
    }

    #[test]
    fn picks_screensaver_styles() {
        assert_eq!(pick_style("orbits", 3), "orbits");
        let first = pick_style("random", 0);
        let second = pick_style("random", 1);
        assert!(SCREENSAVER_STYLES.contains(&first) && SCREENSAVER_STYLES.contains(&second));
        assert_ne!(first, second);
    }

    #[test]
    fn colors_are_stable_and_valid() {
        assert_eq!(color_for("jellyfin"), color_for("jellyfin"));
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), (255, 0, 0));
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
    }
}
