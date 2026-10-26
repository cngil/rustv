//! Starts and stops apps. One app runs at a time: launching another one
//! closes the current app first, and when an app exits the shell is back.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::config::{self, App, Browser};

/// How long an app gets to exit after SIGTERM before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// How often a Flatpak sandbox is checked once its launcher process is gone.
const SANDBOX_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
struct Running {
    id: String,
    name: String,
    /// Process group of the launcher. `None` for apps adopted at startup.
    pgid: Option<i32>,
    flatpak_id: Option<String>,
    generation: u64,
}

pub struct AppManager {
    apps: Vec<App>,
    browser: Browser,
    running: Mutex<Option<Running>>,
    /// Serializes launch and stop so they cannot interleave.
    op: Mutex<()>,
    generation: AtomicU64,
    changed: mpsc::UnboundedSender<()>,
}

impl AppManager {
    pub fn new(apps: Vec<App>, browser: Browser, changed: mpsc::UnboundedSender<()>) -> Self {
        Self { apps, browser, running: Mutex::new(None), op: Mutex::new(()), generation: AtomicU64::new(0), changed }
    }

    pub fn list(&self) -> &[App] {
        &self.apps
    }

    /// ID and name of the running app.
    pub async fn running(&self) -> Option<(String, String)> {
        self.running.lock().await.as_ref().map(|r| (r.id.clone(), r.name.clone()))
    }

    pub async fn launch(self: &Arc<Self>, id: &str) -> Result<()> {
        let _op = self.op.lock().await;
        let app = self.apps.iter().find(|app| app.id == id).cloned().with_context(|| format!("unknown app {id:?}"))?;
        self.stop_locked().await;

        let argv = app.argv(&self.browser);
        let (program, args) = argv.split_first().context("empty command")?;
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).envs(session_env()?).envs(&app.env).stdin(Stdio::null());
        // Own process group, so stopping the app also stops its helpers.
        cmd.as_std_mut().process_group(0);
        let mut child = cmd.spawn().with_context(|| format!("starting {}", app.name))?;
        let pgid = child.id().context("app exited immediately")? as i32;

        let flatpak_id = config::flatpak_id(&argv);
        let generation = self.track(&app, Some(pgid), flatpak_id.clone()).await;
        info!(app = %app.id, pgid, "launched");

        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let status = child.wait().await;
            info!(app = %app.id, ?status, "process exited");
            // Single-instance apps hand off to an already running sandbox and
            // exit at once, so for Flatpak apps the sandbox is what counts.
            if let Some(flatpak_id) = flatpak_id {
                manager.watch_sandbox(&flatpak_id, generation).await;
            }
            manager.clear_if(generation).await;
        });
        Ok(())
    }

    /// Picks up an app that is already on screen, for example after tvd restarts.
    pub async fn adopt(self: &Arc<Self>) {
        let running = flatpak_instances().await;
        let found = self.apps.iter().find_map(|app| {
            let id = config::flatpak_id(&app.argv(&self.browser))?;
            running.contains(&id).then(|| (app.clone(), id))
        });
        let Some((app, flatpak_id)) = found else { return };
        info!(app = %app.id, "adopted running app");
        let generation = self.track(&app, None, Some(flatpak_id.clone())).await;
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager.watch_sandbox(&flatpak_id, generation).await;
            manager.clear_if(generation).await;
        });
    }

    async fn track(&self, app: &App, pgid: Option<i32>, flatpak_id: Option<String>) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.running.lock().await =
            Some(Running { id: app.id.clone(), name: app.name.clone(), pgid, flatpak_id, generation });
        let _ = self.changed.send(());
        generation
    }

    /// Returns once the sandbox is gone or another app took over.
    async fn watch_sandbox(&self, flatpak_id: &str, generation: u64) {
        while self.is_current(generation).await && flatpak_instances().await.iter().any(|id| id == flatpak_id) {
            tokio::time::sleep(SANDBOX_POLL).await;
        }
    }

    async fn is_current(&self, generation: u64) -> bool {
        self.running.lock().await.as_ref().is_some_and(|r| r.generation == generation)
    }

    pub async fn stop(&self) {
        let _op = self.op.lock().await;
        self.stop_locked().await;
    }

    async fn stop_locked(&self) {
        let Some(app) = self.running.lock().await.clone() else { return };
        info!(app = %app.id, "stopping");

        if let Some(flatpak_id) = &app.flatpak_id {
            // The sandbox can outlive the `flatpak run` process, so ask Flatpak directly too.
            let _ = tokio::process::Command::new("flatpak")
                .args(["kill", flatpak_id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
        if let Some(pgid) = app.pgid {
            signal_group(pgid, libc::SIGTERM);
            let deadline = tokio::time::Instant::now() + STOP_GRACE;
            while group_alive(pgid) && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if group_alive(pgid) {
                warn!(app = %app.id, "did not exit in time, killing");
                signal_group(pgid, libc::SIGKILL);
            }
        }
        self.clear_if(app.generation).await;
    }

    async fn clear_if(&self, generation: u64) {
        let mut running = self.running.lock().await;
        if running.as_ref().is_some_and(|r| r.generation == generation) {
            *running = None;
            drop(running);
            let _ = self.changed.send(());
        }
    }
}

/// Application IDs of running Flatpak sandboxes.
async fn flatpak_instances() -> Vec<String> {
    let output = tokio::process::Command::new("flatpak")
        .args(["ps", "--columns=application"])
        .stderr(Stdio::null())
        .output()
        .await;
    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).lines().map(|line| line.trim().to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn signal_group(pgid: i32, signal: i32) {
    // SAFETY: kill(2) with a negative pid targets the process group; no memory is involved.
    unsafe { libc::kill(-pgid, signal) };
}

fn group_alive(pgid: i32) -> bool {
    // SAFETY: as above; signal 0 only checks whether the group still exists.
    unsafe { libc::kill(-pgid, 0) == 0 }
}

/// Environment that puts an app on the TV's Wayland session, even when tvd
/// runs as a service that was started before the session.
fn session_env() -> Result<Vec<(String, String)>> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        // SAFETY: getuid has no preconditions and cannot fail.
        format!("/run/user/{}", unsafe { libc::getuid() })
    });
    let display = match std::env::var("WAYLAND_DISPLAY") {
        Ok(display) if is_live(&Path::new(&runtime).join(&display)) => display,
        _ => find_wayland_socket(&runtime).context("no Wayland session found; is the TV session running?")?,
    };
    Ok(vec![("XDG_RUNTIME_DIR".into(), runtime), ("WAYLAND_DISPLAY".into(), display)])
}

/// First Wayland socket a compositor is listening on. A compositor that
/// crashed leaves its socket file behind, so existing is not enough.
fn find_wayland_socket(runtime: &str) -> Option<String> {
    let mut sockets: Vec<String> = std::fs::read_dir(runtime)
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("wayland-") && !name.ends_with(".lock"))
        .collect();
    sockets.sort();
    sockets.into_iter().find(|name| is_live(&Path::new(runtime).join(name)))
}

fn is_live(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}
