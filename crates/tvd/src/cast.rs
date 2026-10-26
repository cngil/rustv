//! Screen casting: an AirPlay receiver, so iPhones, iPads and Macs can mirror
//! their screen or play video and music on the TV.
//!
//! The receiver is UxPlay, run and kept alive by tvd while `[cast] airplay` is
//! on. Its window opens full screen on top of whatever is showing and closes
//! when the sender stops. Devices find it through Avahi (mDNS).

use std::process::Stdio;
use std::sync::{Mutex, Weak};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::Cast as Config;
use crate::state::AppState;

const RETRY: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Cast {
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Cast {
    /// Starts, restarts or stops the receiver to match the config.
    pub fn apply(&self, cfg: &Config, state: Weak<AppState>) {
        let mut task = self.task.lock().unwrap();
        // Dropping the task kills UxPlay (`kill_on_drop`).
        if let Some(task) = task.take() {
            task.abort();
        }
        if cfg.airplay {
            *task = Some(tokio::spawn(supervise(cfg.clone(), state)));
        }
    }
}

async fn supervise(cfg: Config, state: Weak<AppState>) {
    let mut failing = false;
    loop {
        match run(&cfg, &state).await {
            Ok(()) => failing = false,
            Err(err) if !failing => {
                failing = true;
                warn!("AirPlay receiver: {err:#} (retrying every {}s)", RETRY.as_secs());
            }
            Err(_) => {}
        }
        tokio::time::sleep(RETRY).await;
    }
}

async fn run(cfg: &Config, state: &Weak<AppState>) -> Result<()> {
    let mut child = Command::new("uxplay")
        .args(args(cfg))
        .envs(crate::apps::session_env()?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("starting uxplay (is it installed?)")?;
    info!(name = %cfg.name, "AirPlay receiver ready");

    // A sender connecting counts as using the TV: it wakes it up and keeps it from sleeping.
    let mut lines = BufReader::new(child.stdout.take().context("no stdout")?).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if is_connection(&line) {
            info!(event = %line.trim(), "AirPlay");
            if let Some(state) = state.upgrade() {
                state.external_activity().await;
            }
        }
    }
    let status = child.wait().await?;
    if !status.success() {
        bail!("uxplay exited with {status}");
    }
    Ok(())
}

fn args(cfg: &Config) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-n".into(),
        cfg.name.clone(),
        // No "@hostname" after the name.
        "-nh".into(),
        // Fixed ports, so the firewall can allow exactly these (TCP 7000, 7001, 7100; UDP 6000, 6001, 7011).
        "-p".into(),
        // A new sender may take over from the current one.
        "-nohold".into(),
        "-vs".into(),
        "waylandsink".into(),
    ];
    args.extend(cfg.extra_args.iter().cloned());
    args
}

fn is_connection(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("connection") || line.contains("client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_uxplay_arguments() {
        let cfg =
            Config { airplay: true, name: "Living room".into(), extra_args: vec!["-vd".into(), "avdec_h264".into()] };
        let args = args(&cfg);
        assert_eq!(&args[..2], ["-n", "Living room"]);
        assert!(args.ends_with(&["-vd".to_string(), "avdec_h264".to_string()]));
    }
}
