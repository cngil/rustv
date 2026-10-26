//! Volume bar over full screen apps.
//!
//! The shell draws its own volume bar, but an open app covers the shell. This
//! one is drawn by wob, a tiny overlay bar for sway, in the theme's colors.
//! Optional: without wob installed, volume changes inside apps just show no bar.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::state::AppState;
use crate::theme::Palette;

const RETRY: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Osd {
    inner: Mutex<Option<(JoinHandle<()>, mpsc::UnboundedSender<u32>)>>,
}

impl Osd {
    /// (Re)starts wob with these colors and this many steps for a full bar.
    pub fn start(&self, palette: Palette, max: u32, state: Weak<AppState>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(supervise(palette, max, rx, state));
        if let Some((old, _)) = self.inner.lock().unwrap().replace((task, tx)) {
            old.abort();
        }
    }

    /// Shows the bar at `percent`.
    pub fn show(&self, percent: u32) {
        if let Some((_, tx)) = self.inner.lock().unwrap().as_ref() {
            let _ = tx.send(percent);
        }
    }
}

async fn supervise(palette: Palette, max: u32, mut rx: mpsc::UnboundedReceiver<u32>, state: Weak<AppState>) {
    let mut warned = false;
    loop {
        if state.strong_count() == 0 {
            return;
        }
        if let Err(err) = run(&palette, max, &mut rx).await {
            if !warned {
                warned = true;
                warn!("volume bar over apps: {err:#} (install wob to get it)");
            }
        }
        tokio::time::sleep(RETRY).await;
    }
}

async fn run(palette: &Palette, max: u32, rx: &mut mpsc::UnboundedReceiver<u32>) -> Result<()> {
    let config = write_config(palette, max)?;
    let mut child = Command::new("wob")
        .arg("-c")
        .arg(&config)
        .envs(crate::apps::session_env()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("starting wob")?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    loop {
        tokio::select! {
            value = rx.recv() => match value {
                Some(value) => stdin.write_all(format!("{value}\n").as_bytes()).await?,
                None => return Ok(()),
            },
            status = child.wait() => {
                anyhow::bail!("wob exited with {}", status?);
            }
        }
    }
}

fn write_config(palette: &Palette, max: u32) -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let path = runtime.join("rustv-wob.ini");
    std::fs::write(&path, config(palette, max)).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// wob wants colors as RRGGBB[AA], without the #.
fn config(palette: &Palette, max: u32) -> String {
    let c = |hex: &str| hex.trim_start_matches('#').to_string();
    format!(
        "timeout = 1500\nmax = {max}\nwidth = 560\nheight = 36\nborder_offset = 4\nborder_size = 2\n\
         bar_padding = 4\nanchor = bottom center\nmargin = 120\noutput_mode = focused\noverflow_mode = nowrap\n\
         border_color = {}\nbackground_color = {}E6\nbar_color = {}\n",
        c(&palette.line),
        c(&palette.surface),
        c(&palette.accent),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_theme_colors_without_hashes() {
        let palette = crate::theme::resolve(&crate::config::Theme::default());
        let ini = config(&palette, 100);
        assert!(ini.contains("max = 100\n"));
        assert!(ini.contains("bar_color = 6c8cff\n"));
        assert!(!ini.contains('#'));
    }
}
