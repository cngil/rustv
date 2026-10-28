//! Starts and stops apps. One app runs at a time: launching another one
//! closes the current app first, and when an app exits the shell is back.

use std::collections::BTreeSet;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
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
    /// In the order the user arranged them; see [`arrange`].
    apps: RwLock<Vec<App>>,
    order_file: Option<PathBuf>,
    /// Apps taken off the home screen (and the remote's dock); they stay in the config.
    hidden: RwLock<BTreeSet<String>>,
    hidden_file: Option<PathBuf>,
    browser: Browser,
    running: Mutex<Option<Running>>,
    /// Serializes launch and stop so they cannot interleave.
    op: Mutex<()>,
    generation: AtomicU64,
    changed: mpsc::UnboundedSender<()>,
}

impl AppManager {
    pub fn new(apps: Vec<App>, browser: Browser, changed: mpsc::UnboundedSender<()>) -> Self {
        let order_file = state_file("app-order");
        let hidden_file = state_file("hidden-apps");
        let saved = read_ids(order_file.as_deref());
        Self {
            apps: RwLock::new(arrange(apps, &saved)),
            order_file,
            hidden: RwLock::new(read_ids(hidden_file.as_deref()).into_iter().collect()),
            hidden_file,
            browser,
            running: Mutex::new(None),
            op: Mutex::new(()),
            generation: AtomicU64::new(0),
            changed,
        }
    }

    pub fn list(&self) -> Vec<App> {
        self.apps.read().unwrap().clone()
    }

    /// Puts the apps in the given order and remembers it across restarts.
    pub fn reorder(&self, ids: &[String]) -> Result<()> {
        let order: Vec<String> = {
            let mut apps = self.apps.write().unwrap();
            *apps = arrange(std::mem::take(&mut *apps), ids);
            apps.iter().map(|app| app.id.clone()).collect()
        };
        write_ids(self.order_file.as_deref(), &order)?;
        info!(order = ?order, "apps reordered");
        Ok(())
    }

    /// Whether the app is off the home screen.
    pub fn is_hidden(&self, id: &str) -> bool {
        self.hidden.read().unwrap().contains(id)
    }

    /// Takes an app off the home screen or puts it back, and remembers it across restarts.
    /// The app stays in the config, so it can still be opened from the API.
    pub fn set_hidden(&self, id: &str, hidden: bool) -> Result<()> {
        if !self.list().iter().any(|app| app.id == id) {
            anyhow::bail!("unknown app {id:?}");
        }
        let ids: Vec<String> = {
            let mut set = self.hidden.write().unwrap();
            if hidden {
                set.insert(id.to_string());
            } else {
                set.remove(id);
            }
            set.iter().cloned().collect()
        };
        write_ids(self.hidden_file.as_deref(), &ids)?;
        info!(app = %id, hidden, "app visibility changed");
        Ok(())
    }

    /// ID and name of the running app.
    pub async fn running(&self) -> Option<(String, String)> {
        self.running.lock().await.as_ref().map(|r| (r.id.clone(), r.name.clone()))
    }

    pub async fn launch(self: &Arc<Self>, id: &str) -> Result<()> {
        let _op = self.op.lock().await;
        let app = self.list().into_iter().find(|app| app.id == id).with_context(|| format!("unknown app {id:?}"))?;
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
        let found = self.list().into_iter().find_map(|app| {
            let id = config::flatpak_id(&app.argv(&self.browser))?;
            running.contains(&id).then_some((app, id))
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
            // Right after launch the instance may not know its pid yet, and `flatpak kill`
            // then signals pid 0, its own process group. Give it a group of its own so
            // that can never reach tvd.
            let mut kill = tokio::process::Command::new("flatpak");
            kill.as_std_mut().process_group(0);
            let _ = kill.args(["kill", flatpak_id]).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
            // Helper sandboxes (such as QtWebEngine zygotes) get their own scope and are not
            // part of the instance `flatpak kill` stops.
            let _ = tokio::process::Command::new("systemctl")
                .args(["--user", "stop", &format!("app-flatpak-{flatpak_id}-*.scope")])
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

/// Where the order from the remote is kept. The config says which apps exist;
/// this file only says where they go, so editing the config never fights it.
/// A file in rustv's state directory.
fn state_file(name: &str) -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(state.join("rustv").join(name))
}

/// App ids, one per line; none when the file is missing.
fn read_ids(path: Option<&Path>) -> Vec<String> {
    let text = path.and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

fn write_ids(path: Option<&Path>, ids: &[String]) -> Result<()> {
    let path = path.context("no state directory to save this in")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = if ids.is_empty() { String::new() } else { ids.join("\n") + "\n" };
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Apps named in `order` first, in that order; the rest keep their config order
/// at the end, so apps added to the config later still show up.
fn arrange(mut apps: Vec<App>, order: &[String]) -> Vec<App> {
    let mut arranged = Vec::with_capacity(apps.len());
    for id in order {
        if let Some(pos) = apps.iter().position(|app| &app.id == id) {
            arranged.push(apps.remove(pos));
        }
    }
    arranged.extend(apps);
    arranged
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
pub(crate) fn session_env() -> Result<Vec<(String, String)>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> App {
        toml::from_str(&format!("id = \"{id}\"\nname = \"{id}\"\nkind = \"command\"\ncommand = [\"true\"]\n")).unwrap()
    }

    fn ids(apps: &[App]) -> Vec<&str> {
        apps.iter().map(|app| app.id.as_str()).collect()
    }

    #[test]
    fn arranges_apps_in_saved_order() {
        let apps = vec![app("a"), app("b"), app("c"), app("d")];
        let order = ["c", "gone", "a"].map(String::from);
        // Unknown ids are ignored, apps missing from the order keep their place at the end.
        assert_eq!(ids(&arrange(apps, &order)), ["c", "a", "b", "d"]);
    }
}
