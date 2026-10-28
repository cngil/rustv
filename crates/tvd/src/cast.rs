//! Screen casting: an AirPlay receiver, so iPhones, iPads and Macs can mirror
//! their screen or play video and music on the TV.
//!
//! The receiver is UxPlay, run and kept alive by tvd while `[cast] airplay` is
//! on. Its window opens full screen on top of whatever is showing and closes
//! when the sender stops. Devices find it through Avahi (mDNS).
//!
//! UxPlay does not always close its window: a phone that locks, walks away or
//! drops off Wi-Fi mid-stream can leave the last picture on the TV for good.
//! So tvd watches: a UxPlay window with no sender connected for a while gets
//! UxPlay restarted, which takes the window away.

use std::process::Stdio;
use std::sync::{Mutex, Weak};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::config::Cast as Config;
use crate::state::AppState;

const RETRY: Duration = Duration::from_secs(10);
/// How often the window is checked, and how many checks in a row without a
/// sender close it: longer than UxPlay's own 15 s timeout, so it gets its chance.
const CHECK: Duration = Duration::from_secs(5);
const STUCK_CHECKS: u32 = 4;
/// UxPlay's fixed ports (`-p`): a sender keeps a TCP connection to one of them.
const PORTS: [u16; 3] = [7000, 7001, 7100];

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
            // Restarted to close a window left behind: back at once.
            Ok(true) => continue,
            Ok(false) => failing = false,
            Err(err) if !failing => {
                failing = true;
                warn!("AirPlay receiver: {err:#} (retrying every {}s)", RETRY.as_secs());
            }
            Err(_) => {}
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// Runs UxPlay until it ends. `Ok(true)`: tvd ended it to close a window left without a sender.
async fn run(cfg: &Config, state: &Weak<AppState>) -> Result<bool> {
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
    let pid = child.id().context("uxplay has no process id")?;
    let mut check = tokio::time::interval(CHECK);
    let mut stuck = 0;
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    debug!(line = %line.trim(), "uxplay");
                    if is_connection(&line) {
                        info!(event = %line.trim(), "AirPlay");
                        if let Some(state) = state.upgrade() {
                            state.external_activity().await;
                        }
                    }
                }
                _ => break,
            },
            _ = check.tick() => {
                let window = crate::display::has_window(pid).await.unwrap_or(false);
                stuck = if window && !sender_connected() { stuck + 1 } else { 0 };
                if stuck >= STUCK_CHECKS {
                    info!("AirPlay window left on screen without a sender; restarting the receiver");
                    let _ = child.kill().await;
                    return Ok(true);
                }
            }
        }
    }
    let status = child.wait().await?;
    if !status.success() {
        bail!("uxplay exited with {status}");
    }
    Ok(false)
}

/// Whether a sender is connected to UxPlay's ports.
fn sender_connected() -> bool {
    ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .any(|table| connected_on(&table, &PORTS))
}

/// Reads a /proc/net/tcp table: an established connection (state 01) whose
/// local port is one of `ports`.
fn connected_on(table: &str, ports: &[u16]) -> bool {
    table.lines().skip(1).any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let port =
            fields.get(1).and_then(|local| local.rsplit(':').next()).and_then(|hex| u16::from_str_radix(hex, 16).ok());
        fields.get(3) == Some(&"01") && port.is_some_and(|port| ports.contains(&port))
    })
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
    fn sees_a_sender_by_its_connection() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue\n\
             0: 00000000:1B58 00000000:0000 0A 00000000:00000000\n\
             1: 0A00000A:1B58 0A000020:C350 01 00000000:00000000\n";
        // Listening (0A) on 7000 is no sender; established (01) on 7000 is.
        assert!(connected_on(table, &PORTS));
        assert!(!connected_on(&table.replace(" 01 ", " 06 "), &PORTS));
        assert!(!connected_on(table, &[8765]));
    }

    #[test]
    fn builds_uxplay_arguments() {
        let cfg =
            Config { airplay: true, name: "Living room".into(), extra_args: vec!["-vd".into(), "avdec_h264".into()] };
        let args = args(&cfg);
        assert_eq!(&args[..2], ["-n", "Living room"]);
        assert!(args.ends_with(&["-vd".to_string(), "avdec_h264".to_string()]));
    }
}
