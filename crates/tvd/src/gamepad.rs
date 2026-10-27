//! Gamepads as remotes: the D-pad, the left stick or the hat move around the
//! home screen, A opens, B goes back, the shoulder buttons change the volume.
//!
//! Inside an app the gamepad belongs to the app (games, Moonlight), so nothing
//! is forwarded then, except the Guide button, which closes the app. tvd only
//! listens and never grabs the device, so apps still see every press.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use evdev::{AbsoluteAxisType, Device, InputEventKind, Key};
use tokio::time::Instant;
use tracing::{debug, info};

use crate::audio::VolumeAction;
use crate::protocol::Command;
use crate::state::AppState;

const SCAN_EVERY: Duration = Duration::from_secs(5);
/// Holding a direction repeats it, like holding an arrow key.
const REPEAT_AFTER: Duration = Duration::from_millis(400);
const REPEAT_EVERY: Duration = Duration::from_millis(140);
/// Stick travel (0..1) that counts as a direction, and where it lets go again.
const STICK_ON: f32 = 0.6;
const STICK_OFF: f32 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn key(self) -> &'static str {
        match self {
            Dir::Up => "ArrowUp",
            Dir::Down => "ArrowDown",
            Dir::Left => "ArrowLeft",
            Dir::Right => "ArrowRight",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Move(Dir),
    Open,
    Back,
    VolumeUp,
    VolumeDown,
    Guide,
}

/// Looks for gamepads every few seconds, for as long as tvd runs.
pub fn watch(state: Weak<AppState>) {
    let open: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    tokio::spawn(async move {
        loop {
            if state.strong_count() == 0 {
                return;
            }
            let found = tokio::task::spawn_blocking(evdev::enumerate).await.map(Iterator::collect::<Vec<_>>);
            for (path, device) in found.unwrap_or_default() {
                if !is_gamepad(&device) || !open.lock().unwrap().insert(path.clone()) {
                    continue;
                }
                info!(name = device.name().unwrap_or("?"), "gamepad connected");
                let (state, open) = (state.clone(), open.clone());
                tokio::spawn(async move {
                    read(device, &state).await;
                    info!(path = %path.display(), "gamepad gone");
                    open.lock().unwrap().remove(&path);
                });
            }
            tokio::time::sleep(SCAN_EVERY).await;
        }
    });
}

fn is_gamepad(device: &Device) -> bool {
    // BTN_SOUTH (A) is what makes a gamepad; the kernel also calls it BTN_GAMEPAD.
    device.supported_keys().is_some_and(|keys| keys.contains(Key::BTN_SOUTH))
}

/// The two ends of a stick axis, to turn raw values into -1..1.
#[derive(Clone, Copy)]
struct Range {
    center: f32,
    half: f32,
}

async fn read(device: Device, state: &Weak<AppState>) {
    let ranges = device.get_abs_state().ok().map(|abs| {
        let range = |axis: AbsoluteAxisType| {
            let info = abs[axis.0 as usize];
            let (min, max) = (info.minimum as f32, info.maximum as f32);
            Range { center: (min + max) / 2.0, half: ((max - min) / 2.0).max(1.0) }
        };
        (range(AbsoluteAxisType::ABS_X), range(AbsoluteAxisType::ABS_Y))
    });
    let Ok(mut events) = device.into_event_stream() else { return };

    let mut held: Option<Dir> = None;
    let mut next_repeat = Instant::now();
    let mut stick: (Option<Dir>, Option<Dir>) = (None, None);
    loop {
        let event = tokio::select! {
            event = events.next_event() => match event {
                Ok(event) => event,
                Err(_) => return,
            },
            _ = tokio::time::sleep_until(next_repeat), if held.is_some() => {
                next_repeat = Instant::now() + REPEAT_EVERY;
                if let Some(dir) = held {
                    act(state, Action::Move(dir)).await;
                }
                continue;
            }
        };
        let value = event.value();
        let mut press = |dir: Dir, down: bool, held: &mut Option<Dir>| {
            if down {
                *held = Some(dir);
                next_repeat = Instant::now() + REPEAT_AFTER;
                Some(Action::Move(dir))
            } else {
                if *held == Some(dir) {
                    *held = None;
                }
                None
            }
        };
        let action = match event.kind() {
            InputEventKind::Key(key) => match (key, value) {
                (Key::BTN_DPAD_UP, v) => press(Dir::Up, v == 1, &mut held),
                (Key::BTN_DPAD_DOWN, v) => press(Dir::Down, v == 1, &mut held),
                (Key::BTN_DPAD_LEFT, v) => press(Dir::Left, v == 1, &mut held),
                (Key::BTN_DPAD_RIGHT, v) => press(Dir::Right, v == 1, &mut held),
                (Key::BTN_SOUTH, 1) => Some(Action::Open),
                (Key::BTN_EAST, 1) => Some(Action::Back),
                (Key::BTN_TR, 1) => Some(Action::VolumeUp),
                (Key::BTN_TL, 1) => Some(Action::VolumeDown),
                (Key::BTN_MODE, 1) => Some(Action::Guide),
                _ => None,
            },
            InputEventKind::AbsAxis(axis)
                if axis == AbsoluteAxisType::ABS_HAT0X || axis == AbsoluteAxisType::ABS_HAT0Y =>
            {
                let (neg, pos) =
                    if axis == AbsoluteAxisType::ABS_HAT0X { (Dir::Left, Dir::Right) } else { (Dir::Up, Dir::Down) };
                match value.signum() {
                    -1 => press(neg, true, &mut held),
                    1 => press(pos, true, &mut held),
                    _ => {
                        press(neg, false, &mut held);
                        press(pos, false, &mut held)
                    }
                }
            }
            InputEventKind::AbsAxis(axis) if axis == AbsoluteAxisType::ABS_X || axis == AbsoluteAxisType::ABS_Y => {
                let Some((rx, ry)) = ranges else { continue };
                let x_axis = axis == AbsoluteAxisType::ABS_X;
                let range = if x_axis { rx } else { ry };
                let (neg, pos) = if x_axis { (Dir::Left, Dir::Right) } else { (Dir::Up, Dir::Down) };
                let slot = if x_axis { &mut stick.0 } else { &mut stick.1 };
                match stick_dir((value as f32 - range.center) / range.half, *slot, neg, pos) {
                    StickChange::Press(dir) => {
                        *slot = Some(dir);
                        press(dir, true, &mut held)
                    }
                    StickChange::Release(dir) => {
                        *slot = None;
                        press(dir, false, &mut held)
                    }
                    StickChange::None => None,
                }
            }
            _ => None,
        };
        if let Some(action) = action {
            act(state, action).await;
        }
    }
}

#[derive(Debug, PartialEq)]
enum StickChange {
    Press(Dir),
    Release(Dir),
    None,
}

/// A stick only counts once pushed well over (`STICK_ON`), and lets go once
/// mostly back (`STICK_OFF`), so it does not flicker around the threshold.
fn stick_dir(travel: f32, current: Option<Dir>, neg: Dir, pos: Dir) -> StickChange {
    match current {
        Some(dir) if travel.abs() < STICK_OFF => StickChange::Release(dir),
        None if travel <= -STICK_ON => StickChange::Press(neg),
        None if travel >= STICK_ON => StickChange::Press(pos),
        _ => StickChange::None,
    }
}

async fn act(state: &Weak<AppState>, action: Action) {
    let Some(state) = state.upgrade() else { return };
    let cfg = state.config().gamepad.clone();
    if !cfg.enabled {
        return;
    }
    let in_app = state.apps.running().await.is_some();
    // An asleep TV wakes on any button; tvd swallows that first key itself.
    let command = if state.is_asleep() {
        Some(key("ArrowLeft"))
    } else {
        match action {
            Action::Guide if in_app && cfg.guide_closes_app => Some(Command::Stop),
            _ if in_app => None,
            Action::Move(dir) => Some(key(dir.key())),
            Action::Open => Some(key("Enter")),
            Action::Back => Some(key("Escape")),
            Action::VolumeUp => Some(Command::Volume { action: VolumeAction::Up, level: None }),
            Action::VolumeDown => Some(Command::Volume { action: VolumeAction::Down, level: None }),
            Action::Guide => None,
        }
    };
    let Some(command) = command else { return };
    debug!(?action, "gamepad");
    if let Err(err) = state.execute(command).await {
        debug!("gamepad command: {err:#}");
    }
}

fn key(code: &str) -> Command {
    Command::Key { code: code.to_string(), shift: false, ctrl: false, alt: false, meta: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_need_a_real_push_and_let_go_with_slack() {
        assert_eq!(stick_dir(0.5, None, Dir::Left, Dir::Right), StickChange::None);
        assert_eq!(stick_dir(0.7, None, Dir::Left, Dir::Right), StickChange::Press(Dir::Right));
        assert_eq!(stick_dir(-0.9, None, Dir::Up, Dir::Down), StickChange::Press(Dir::Up));
        // Between the two thresholds it stays held.
        assert_eq!(stick_dir(0.45, Some(Dir::Right), Dir::Left, Dir::Right), StickChange::None);
        assert_eq!(stick_dir(0.1, Some(Dir::Right), Dir::Left, Dir::Right), StickChange::Release(Dir::Right));
    }
}
