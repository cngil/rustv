//! What tvd shows over everything on the TV through `rustv-osd`: the volume
//! and brightness bar over apps, and the dimmer that holds sway's gamma.
//!
//! The shell draws its own volume bar on the home screen, but an open app
//! covers the shell. `rustv-osd level` draws the same bar on sway's overlay
//! layer, in the theme. `rustv-osd gamma` dims the screen for as long as it
//! runs; when it ends, sway restores full brightness, so a crash fails bright.

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

/// Dims the screen below full brightness by holding sway's gamma tables.
#[derive(Default)]
pub struct Dimmer {
    holder: Mutex<Option<Child>>,
}

impl Dimmer {
    /// 1.0 lets go (full brightness, the screen's own gamma); less dims.
    pub fn set(&self, brightness: f32) {
        let mut holder = self.holder.lock().unwrap();
        if brightness >= 0.999 {
            if let Some(child) = holder.take() {
                end(child);
            }
            return;
        }
        let line = json!({ "brightness": brightness });
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
