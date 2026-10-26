//! `rustv-shell`, the on-screen home screen.
//!
//! A thin client of `tvd`: it renders the app list and sends `launch`
//! commands over the same WebSocket the web remote uses. All state lives in
//! the daemon, so the shell can crash or restart without losing anything.

mod net;
mod settings;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Local, Timelike};
use slint::{Color, ComponentHandle, Image, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};
use tokio::sync::mpsc;

slint::include_modules!();

/// How long the volume overlay stays after a change.
const OSD_TIME: Duration = Duration::from_millis(2200);
/// Clears "Opening…" if the app never shows up.
const OPENING_TIMEOUT: Duration = Duration::from_secs(15);

thread_local! {
    static OSD_TIMER: Timer = Timer::default();
    static OPENING_TIMER: Timer = Timer::default();
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
    fn colors_are_stable_and_valid() {
        assert_eq!(color_for("jellyfin"), color_for("jellyfin"));
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), (255, 0, 0));
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
    }
}
