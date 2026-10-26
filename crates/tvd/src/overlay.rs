//! Debug overlay: one line of live stats drawn over everything on the TV.
//!
//! The line is drawn by swaybar (part of sway) in overlay mode, so it stays on
//! top of full screen apps and never takes input. tvd sets the bar up over sway
//! IPC and runs swaybar; swaybar runs `tvd overlay-status`, which samples the
//! system, asks tvd for the app and volume, and prints the i3bar protocol.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{Config, OverlayItem, OverlayPosition};
use crate::state::AppState;
use crate::stats::{Sample, Sampler};

const BAR_ID: &str = "rustv-overlay";
const RETRY: Duration = Duration::from_secs(5);

pub struct Overlay {
    config_path: PathBuf,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Overlay {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path, task: Mutex::new(None) }
    }

    pub fn visible(&self) -> bool {
        self.task.lock().unwrap().as_ref().is_some_and(|task| !task.is_finished())
    }

    pub fn set_visible(&self, visible: bool, state: Weak<AppState>) {
        let mut task = self.task.lock().unwrap();
        // Dropping the task kills swaybar (`kill_on_drop`), which takes the line off the screen.
        if let Some(task) = task.take() {
            task.abort();
        }
        if visible {
            *task = Some(tokio::spawn(supervise(self.config_path.clone(), state)));
        }
    }
}

/// Keeps swaybar running, also across sway restarts, until the task is aborted.
async fn supervise(config_path: PathBuf, state: Weak<AppState>) {
    let mut failing = false;
    loop {
        let Some(cfg) = state.upgrade().map(|state| state.config.overlay.clone()) else { return };
        match run_bar(&cfg, &config_path).await {
            Ok(()) => {
                failing = false;
                info!("overlay bar exited, starting it again");
            }
            Err(err) if !failing => {
                failing = true;
                warn!("debug overlay: {err:#} (retrying every {}s)", RETRY.as_secs());
            }
            Err(_) => {}
        }
        tokio::time::sleep(RETRY).await;
    }
}

async fn run_bar(cfg: &crate::config::Overlay, config_path: &Path) -> Result<()> {
    let env = crate::apps::session_env()?;
    let runtime = &env.iter().find(|(key, _)| key == "XDG_RUNTIME_DIR").context("no runtime dir")?.1;
    let sway = find_sway_socket(Path::new(runtime)).context("sway is not running")?;

    let exe = std::env::current_exe().context("locating the tvd binary")?;
    let status = format!("'{}' overlay-status --config '{}'", plain(&exe)?, plain(config_path)?);
    let position = match cfg.position {
        OverlayPosition::Top => "top",
        OverlayPosition::Bottom => "bottom",
    };
    // The first `bar` command creates the bar and makes sway start a swaybar that
    // never sees later settings, so create it invisible and stop that swaybar.
    swaymsg(&sway, &format!("bar {BAR_ID} mode invisible")).await?;
    stop_other_bars();
    // The mode goes last: it only sticks after the other settings.
    let settings = [
        format!("position {position}"),
        format!("font pango:{}", cfg.font),
        "workspace_buttons no".into(),
        "binding_mode_indicator no".into(),
        "tray_output none".into(),
        "status_padding 4".into(),
        "colors background #0b0d12b8".into(),
        "colors statusline #e6e9f0".into(),
        "colors separator #4a5168".into(),
        format!("status_command {status}"),
        "mode overlay".into(),
    ];
    for setting in settings {
        swaymsg(&sway, &format!("bar {BAR_ID} {setting}")).await?;
    }

    let mut bar = Command::new("swaybar")
        .args(["-b", BAR_ID])
        .envs(env)
        .env("SWAYSOCK", &sway)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting swaybar (part of sway)")?;
    let status = bar.wait().await?;
    if !status.success() {
        bail!("swaybar exited with {status}");
    }
    Ok(())
}

/// Stops every swaybar showing our bar except the one tvd runs. Only the one
/// started after the setup has the right font and position.
fn stop_other_bars() {
    let Ok(entries) = std::fs::read_dir("/proc") else { return };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        if is_our_bar(&cmdline) {
            // SAFETY: kill only sends a signal.
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
}

/// `swaybar -b rustv-overlay`, as a NUL-separated /proc cmdline.
fn is_our_bar(cmdline: &[u8]) -> bool {
    let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
    args.first().is_some_and(|program| program.ends_with(b"swaybar"))
        && args.windows(2).any(|pair| pair[0] == b"-b" && pair[1] == BAR_ID.as_bytes())
}

/// A path that can go inside single quotes in a sway command.
fn plain(path: &Path) -> Result<String> {
    let text = path.to_str().context("path is not UTF-8")?;
    if text.contains(['\'', ';', ',', '\n']) {
        bail!("{text}: the path must not contain quotes, ';' or ','");
    }
    Ok(text.to_string())
}

async fn swaymsg(sway: &Path, command: &str) -> Result<()> {
    let out = Command::new("swaymsg").env("SWAYSOCK", sway).arg(command).output().await.context("running swaymsg")?;
    if !out.status.success() {
        bail!("swaymsg {command:?}: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    Ok(())
}

/// The sway IPC socket of the running session. Like Wayland sockets, a crashed
/// sway leaves its socket file behind, so only one that accepts connections counts.
fn find_sway_socket(runtime: &Path) -> Option<PathBuf> {
    if let Some(socket) = std::env::var_os("SWAYSOCK").map(PathBuf::from) {
        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            return Some(socket);
        }
    }
    let mut sockets: Vec<PathBuf> = std::fs::read_dir(runtime)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("sway-ipc.")))
        .collect();
    sockets.sort();
    sockets.into_iter().rev().find(|path| std::os::unix::net::UnixStream::connect(path).is_ok())
}

// ---- `tvd overlay-status`, run by swaybar ----

/// What tvd knows that /proc does not.
#[derive(Default)]
struct Daemon {
    app: Option<String>,
    volume: Option<(f64, bool)>,
    clients: Option<u64>,
}

pub fn status_loop(cfg: &Config, token: &str) -> Result<()> {
    let overlay = &cfg.overlay;
    let port = cfg.server.bind.rsplit(':').next().unwrap_or("8765").to_string();
    let interval = Duration::from_millis(overlay.interval_ms);
    let mut sampler = Sampler::default();
    let mut display: Option<String> = None;
    let mut display_read: Option<Instant> = None;

    let mut out = std::io::stdout().lock();
    writeln!(out, "{{\"version\":1}}\n[")?;
    loop {
        let sample = sampler.sample();
        let daemon = fetch_state(&port, token).unwrap_or_default();
        if overlay.items.contains(&OverlayItem::Display)
            && display_read.map_or(true, |at| at.elapsed() > Duration::from_secs(10))
        {
            display = display_mode();
            display_read = Some(Instant::now());
        }
        let blocks: Vec<Value> =
            overlay.items.iter().filter_map(|item| block(*item, &sample, &daemon, display.as_deref())).collect();
        // A closed stdout means swaybar is gone.
        if writeln!(out, "{},", Value::Array(blocks)).and_then(|_| out.flush()).is_err() {
            return Ok(());
        }
        std::thread::sleep(interval);
    }
}

const OK: &str = "#e6e9f0";
const WARN: &str = "#ffb547";
const BAD: &str = "#ff5d73";
const DIM: &str = "#8a93a8";

fn block(item: OverlayItem, s: &Sample, d: &Daemon, display: Option<&str>) -> Option<Value> {
    let level = |value: f32, warn: f32, bad: f32| match value {
        v if v >= bad => BAD,
        v if v >= warn => WARN,
        _ => OK,
    };
    let (text, color) = match item {
        OverlayItem::App => (d.app.clone().unwrap_or_else(|| "Home".into()), OK),
        OverlayItem::Cpu => {
            let load = s.cpu?;
            let color = worse(level(load, 60.0, 90.0), s.cpu_temp.map(|t| level(t, 75.0, 90.0)));
            (format!("CPU {load:>3.0}%{}", temp(s.cpu_temp)), color)
        }
        OverlayItem::Gpu => {
            if s.gpu.is_none() && s.gpu_temp.is_none() {
                return None;
            }
            let load = s.gpu.map(|g| format!("{g:>3.0}%")).unwrap_or_default();
            let color = worse(s.gpu.map_or(OK, |g| level(g, 70.0, 95.0)), s.gpu_temp.map(|t| level(t, 75.0, 90.0)));
            (format!("GPU {load}{}", temp(s.gpu_temp)), color)
        }
        OverlayItem::Ram => {
            let (used, total) = s.memory?;
            let share = used as f32 * 100.0 / total.max(1) as f32;
            (format!("RAM {:.1}/{:.1} GB", gb(used), gb(total)), level(share, 80.0, 92.0))
        }
        OverlayItem::Net => {
            let net = s.net.as_ref()?;
            (format!("{} ↓{} ↑{}", net.interface, rate(net.rx), rate(net.tx)), OK)
        }
        OverlayItem::Wifi => {
            let dbm = s.wifi_dbm?;
            // Closer to 0 is stronger: -50 is great, -70 is fair, below -80 drops out.
            let color = if dbm <= -78.0 {
                BAD
            } else if dbm <= -68.0 {
                WARN
            } else {
                OK
            };
            (format!("Wi-Fi {dbm:.0} dBm"), color)
        }
        OverlayItem::Display => (display?.to_string(), OK),
        OverlayItem::Volume => {
            let (level, muted) = d.volume?;
            if muted {
                ("Vol muted".into(), WARN)
            } else {
                (format!("Vol {:.0}%", level * 100.0), OK)
            }
        }
        OverlayItem::Remotes => {
            let n = d.clients?;
            (format!("{n} remote{}", if n == 1 { "" } else { "s" }), if n == 0 { DIM } else { OK })
        }
        OverlayItem::Uptime => (format!("up {}", duration(s.uptime_secs?)), DIM),
        OverlayItem::Clock => {
            // Local time without a time zone crate: ask the C library.
            let mut now: libc::time_t = 0;
            // SAFETY: time and localtime_r only write to the values passed in.
            let tm = unsafe {
                libc::time(&mut now);
                let mut tm: libc::tm = std::mem::zeroed();
                libc::localtime_r(&now, &mut tm);
                tm
            };
            (format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec), OK)
        }
    };
    Some(json!({ "full_text": format!(" {text} "), "color": color, "separator_block_width": 14 }))
}

/// The more alarming of two colors.
fn worse(a: &'static str, b: Option<&'static str>) -> &'static str {
    let rank = |c: &str| [OK, WARN, BAD].iter().position(|x| *x == c).unwrap_or(0);
    match b {
        Some(b) if rank(b) > rank(a) => b,
        _ => a,
    }
}

fn temp(celsius: Option<f32>) -> String {
    celsius.map(|t| format!(" {t:.0}°C")).unwrap_or_default()
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
}

fn rate(bytes_per_sec: f64) -> String {
    match bytes_per_sec {
        b if b >= 1_048_576.0 => format!("{:.1} MB/s", b / 1_048_576.0),
        b if b >= 1024.0 => format!("{:.0} kB/s", b / 1024.0),
        b => format!("{b:.0} B/s"),
    }
}

fn duration(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

/// `GET /api/v1/state` on tvd. Plain HTTP/1.0 over the loopback, so no client library is needed.
fn fetch_state(port: &str, token: &str) -> Option<Daemon> {
    let mut stream =
        TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().ok()?, Duration::from_millis(300)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(stream, "GET /api/v1/state HTTP/1.0\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\n").ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let body: Value = serde_json::from_str(response.split_once("\r\n\r\n")?.1).ok()?;
    Some(Daemon {
        app: body["running"]["name"].as_str().map(String::from),
        volume: body["volume"]["level"].as_f64().map(|level| (level, body["volume"]["muted"] == true)),
        clients: body["clients"].as_u64(),
    })
}

/// Mode of the first active output, such as "1920×1080 60Hz".
fn display_mode() -> Option<String> {
    let out = std::process::Command::new("swaymsg").args(["-r", "-t", "get_outputs"]).output().ok()?;
    let outputs: Value = serde_json::from_slice(&out.stdout).ok()?;
    let mode = &outputs.as_array()?.iter().find(|o| o["active"] == true)?["current_mode"];
    let hz = mode["refresh"].as_f64()? / 1000.0;
    Some(format!("{}×{} {hz:.0}Hz", mode["width"].as_u64()?, mode["height"].as_u64()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_values() {
        assert_eq!(rate(512.0), "512 B/s");
        assert_eq!(rate(40.0 * 1024.0), "40 kB/s");
        assert_eq!(rate(1.5 * 1_048_576.0), "1.5 MB/s");
        assert_eq!(duration(59 * 60), "59m");
        assert_eq!(duration(3 * 3600 + 12 * 60), "3h 12m");
        assert_eq!(duration(2 * 86_400 + 5 * 3600), "2d 5h");
    }

    #[test]
    fn colors_follow_the_worst_value() {
        let sample = Sample { cpu: Some(20.0), cpu_temp: Some(95.0), ..Default::default() };
        let cpu = block(OverlayItem::Cpu, &sample, &Daemon::default(), None).unwrap();
        assert_eq!(cpu["color"], BAD);
        assert_eq!(cpu["full_text"], " CPU  20% 95°C ");
        // Items without data are left out instead of showing a placeholder.
        assert!(block(OverlayItem::Wifi, &sample, &Daemon::default(), None).is_none());
    }

    #[test]
    fn recognizes_our_swaybar() {
        assert!(is_our_bar(b"swaybar -b rustv-overlay "));
        assert!(is_our_bar(b"/usr/bin/swaybar -b rustv-overlay"));
        assert!(!is_our_bar(b"swaybar -b bar-0 "));
        assert!(!is_our_bar(b"vim -b rustv-overlay"));
    }

    #[test]
    fn rejects_paths_that_break_sway_commands() {
        assert!(plain(Path::new("/home/a/.config/rustv/config.toml")).is_ok());
        assert!(plain(Path::new("/tmp/a;exec x")).is_err());
    }
}
