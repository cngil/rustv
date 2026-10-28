//! What tvd shows over everything on the TV through `rustv-osd`: the volume
//! and brightness bar over apps, and the dimmer that holds sway's gamma.
//!
//! The shell draws its own volume bar on the home screen, but an open app
//! covers the shell. `rustv-osd level` draws the same bar on sway's overlay
//! layer, in the theme. `rustv-osd gamma` dims the screen for as long as it
//! runs; when it ends, sway restores full brightness, so a crash fails bright.
//! `rustv-osd prompt` shows the "keep this resolution?" prompt over an app.
//! `rustv-osd type` types text through the Wayland virtual keyboard, so any
//! character reaches the app, not only what a US layout has.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, warn};

use crate::theme::Palette;

/// How long the bar stays after the last change.
const SHOW_FOR: Duration = Duration::from_millis(2200);

/// `rustv-osd` from next to tvd (as installed), else from the PATH.
pub fn program() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("rustv-osd")));
    beside.filter(|path| path.is_file()).unwrap_or_else(|| "rustv-osd".into())
}

/// Starts `rustv-osd <mode>` in the TV session, with a pipe to its stdin.
pub fn spawn(mode: &str) -> Result<Child> {
    let program = program();
    Command::new(&program)
        .arg(mode)
        .envs(crate::apps::session_env()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {} {mode}", program.display()))
}

/// Ends a `rustv-osd`: closing stdin ends it, the kill is for one that hangs.
pub fn end(mut child: Child) {
    drop(child.stdin.take());
    let _ = child.kill();
    std::thread::spawn(move || child.wait());
}

fn send(child: &mut Child, line: &serde_json::Value) -> bool {
    child.stdin.as_mut().is_some_and(|stdin| writeln!(stdin, "{line}").is_ok())
}

/// A level for the bar: `label` in the TV's language, `value` 0..1 of the bar.
pub struct Level {
    pub label: String,
    pub value: f32,
    pub text: String,
    pub muted: bool,
}

/// The bar over apps.
#[derive(Default)]
pub struct Osd {
    shown: Arc<Mutex<Option<Child>>>,
    /// Counts shows, so only the last one's timer hides the bar.
    count: Arc<AtomicU64>,
}

impl Osd {
    /// Shows the bar at this level, or moves it there, and hides it a moment after the last change.
    pub fn show(&self, level: Level, palette: &Palette) {
        let line = json!({ "label": level.label, "value": level.value, "text": level.text, "muted": level.muted, "palette": palette });
        let mut shown = self.shown.lock().unwrap();
        let alive = shown.as_mut().is_some_and(|child| send(child, &line));
        if !alive {
            if let Some(old) = shown.take() {
                end(old);
            }
            match spawn("level") {
                Ok(mut child) => {
                    send(&mut child, &line);
                    *shown = Some(child);
                }
                Err(err) => {
                    debug!("bar over apps: {err:#}");
                    return;
                }
            }
        }
        drop(shown);
        let id = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        let (slot, count) = (Arc::clone(&self.shown), Arc::clone(&self.count));
        tokio::spawn(async move {
            tokio::time::sleep(SHOW_FOR).await;
            if count.load(Ordering::Relaxed) == id {
                if let Some(child) = slot.lock().unwrap().take() {
                    end(child);
                }
            }
        });
    }
}

/// A prompt over apps, shown for as long as tvd keeps sending it.
#[derive(Default)]
pub struct Prompt {
    shown: Mutex<Option<Child>>,
}

impl Prompt {
    /// Shows `line` (see `Prompt` in rustv-osd), or updates it.
    pub fn show(&self, line: serde_json::Value) {
        let mut shown = self.shown.lock().unwrap();
        if shown.as_mut().is_some_and(|child| send(child, &line)) {
            return;
        }
        if let Some(old) = shown.take() {
            end(old);
        }
        match spawn("prompt") {
            Ok(mut child) => {
                send(&mut child, &line);
                *shown = Some(child);
            }
            Err(err) => debug!("prompt over apps: {err:#}"),
        }
    }

    pub fn hide(&self) {
        if let Some(child) = self.shown.lock().unwrap().take() {
            end(child);
        }
    }
}

/// Dims and warms the screen by holding sway's gamma tables.
#[derive(Default)]
pub struct Dimmer {
    holder: Mutex<Option<Child>>,
    /// The last brightness and warmth sent, so the clock's ticks send nothing new.
    last: Mutex<Option<(f32, f32)>>,
}

impl Dimmer {
    /// Brightness 1.0 and warmth 0 let go (the screen's own gamma); otherwise
    /// the screen is dimmed and its blue and green turned down.
    pub fn set(&self, brightness: f32, warmth: f32) {
        let mut holder = self.holder.lock().unwrap();
        // Hundredths are finer than anyone sees, and keep the easing to a few steps.
        let now = ((brightness * 100.0).round() / 100.0, (warmth * 100.0).round() / 100.0);
        let mut last = self.last.lock().unwrap();
        let alive = holder.as_mut().is_some_and(|child| child.try_wait().ok().flatten().is_none());
        if *last == Some(now) && (alive || now == (1.0, 0.0)) {
            return;
        }
        *last = Some(now);
        let (brightness, warmth) = now;
        if brightness >= 1.0 && warmth <= 0.0 {
            if let Some(child) = holder.take() {
                end(child);
            }
            return;
        }
        let line = json!({ "brightness": brightness, "warmth": warmth });
        if holder.as_mut().is_some_and(|child| send(child, &line)) {
            return;
        }
        // Not running yet, or it ended: start it again.
        if let Some(old) = holder.take() {
            end(old);
        }
        match spawn("gamma") {
            Ok(mut child) => {
                send(&mut child, &line);
                *holder = Some(child);
            }
            Err(err) => warn!("dimming the screen: {err:#}"),
        }
    }
}

/// Types text into the focused app through `rustv-osd type`, kept running.
#[derive(Default)]
pub struct Typer {
    running: tokio::sync::Mutex<Option<Typing>>,
}

struct Typing {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    answers: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
}

impl Typer {
    /// Types `text` and returns once it is typed. An error means none of it was
    /// typed (the compositor has no virtual keyboard, or rustv-osd is missing),
    /// so the caller can fall back to the uinput keyboard.
    pub async fn text(&self, text: &str) -> Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let mut running = self.running.lock().await;
        if running.as_mut().is_some_and(|typing| typing.child.try_wait().ok().flatten().is_some()) {
            *running = None;
        }
        if running.is_none() {
            let program = program();
            let mut child = tokio::process::Command::new(&program)
                .arg("type")
                .envs(crate::apps::session_env()?)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .with_context(|| format!("starting {} type", program.display()))?;
            let stdin = child.stdin.take().context("no stdin")?;
            let stdout = child.stdout.take().context("no stdout")?;
            let answers = tokio::io::BufReader::new(stdout).lines();
            *running = Some(Typing { child, stdin, answers });
        }
        let typing = running.as_mut().expect("started above");
        let line = format!("{}\n", json!({ "text": text }));
        let sent = typing.stdin.write_all(line.as_bytes()).await.and(typing.stdin.flush().await);
        if let Err(err) = sent {
            *running = None;
            return Err(err).context("rustv-osd type ended");
        }
        // About 6 ms a character, plus a keymap per stretch; generous for slow boxes.
        let wait = Duration::from_secs(2) + Duration::from_millis(20 * text.chars().count() as u64);
        match tokio::time::timeout(wait, typing.answers.next_line()).await {
            Ok(Ok(Some(_))) => Ok(()),
            Ok(_) => {
                // It ended before answering: it could not type at all.
                *running = None;
                anyhow::bail!("rustv-osd type ended without typing")
            }
            Err(_) => {
                // Still typing, or stuck: start fresh next time, but don't type it twice.
                warn!("typing took longer than {wait:?}");
                *running = None;
                Ok(())
            }
        }
    }
}
