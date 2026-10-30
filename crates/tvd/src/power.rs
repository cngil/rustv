//! Getting out of trouble from the sofa, mildest first: restart the open app,
//! the home screen, the TV session (sway), or the whole box; or shut it down.
//!
//! Debug builds always simulate shell/session/system power actions.
//! Each action can be left out with `system.power` in the config. With
//! `TVD_POWER_DRY_RUN` set (the dev runner and the browser tests set it), the
//! home screen, session and box actions are only logged, so trying them on a
//! desktop never takes that desktop down.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::info;

/// How long the shell gets to exit after SIGTERM before it is killed.
const SHELL_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Close the open app and open it again.
    RestartApp,
    /// Restart `rustv-shell`; the sway config starts it again.
    RestartShell,
    /// Leave sway; the `rustv-session` service starts it again.
    RestartSession,
    Reboot,
    Shutdown,
}

impl Action {
    pub const ALL: [Action; 5] =
        [Action::RestartApp, Action::RestartShell, Action::RestartSession, Action::Reboot, Action::Shutdown];

    pub fn name(self) -> &'static str {
        match self {
            Action::RestartApp => "restart_app",
            Action::RestartShell => "restart_shell",
            Action::RestartSession => "restart_session",
            Action::Reboot => "reboot",
            Action::Shutdown => "shutdown",
        }
    }
}

/// Debug builds always simulate disruptive actions, even outside the dev runner.
/// Release builds can also opt into simulation with TVD_POWER_DRY_RUN.
pub fn dry_run() -> bool {
    simulate_power(cfg!(debug_assertions), std::env::var_os("TVD_POWER_DRY_RUN").is_some_and(|value| !value.is_empty()))
}

fn simulate_power(debug: bool, requested: bool) -> bool {
    debug || requested
}

/// Ends every `rustv-shell` of this user; the loop in the sway config starts it again.
pub async fn restart_shell() -> Result<()> {
    let pids = shell_pids();
    if pids.is_empty() {
        return Err(crate::i18n::error("error.power.no_shell", &[]));
    }
    if dry_run() {
        info!(?pids, "dry run: would restart the shell");
        return Ok(());
    }
    info!(?pids, "restarting the shell");
    signal(&pids, libc::SIGTERM);
    let deadline = tokio::time::Instant::now() + SHELL_GRACE;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if pids.iter().all(|pid| !alive(*pid)) {
            return Ok(());
        }
    }
    // A shell that hangs does not answer SIGTERM either.
    signal(&pids, libc::SIGKILL);
    Ok(())
}

/// Leaves sway, which takes every window with it.
pub async fn end_session() -> Result<()> {
    if dry_run() {
        info!("dry run: would leave sway");
        return Ok(());
    }
    let sway = crate::display::sway_socket()?;
    let out = Command::new("swaymsg").env("SWAYSOCK", sway).arg("exit").output().await.context("running swaymsg")?;
    if !out.status.success() {
        bail!("swaymsg exit: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    Ok(())
}

/// Reboots or powers off through logind. Never asks for a password: without
/// permission it fails, and the remote says how to allow it.
pub async fn system(action: Action) -> Result<()> {
    let verb = match action {
        Action::Reboot => "reboot",
        Action::Shutdown => "poweroff",
        _ => bail!("{} is not a system action", action.name()),
    };
    if dry_run() {
        info!("dry run: would run systemctl {verb}");
        return Ok(());
    }
    info!("systemctl {verb}");
    let out =
        Command::new("systemctl").args(["--no-ask-password", verb]).output().await.context("running systemctl")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("Access denied") || stderr.contains("authentication") {
            return Err(crate::i18n::error("error.power.not_allowed", &[]));
        }
        bail!("systemctl {verb}: {}", stderr.trim());
    }
    Ok(())
}

/// This user's processes running the shell.
fn shell_pids() -> Vec<i32> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: i32 = entry.file_name().to_str()?.parse().ok()?;
            let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
            let owner = std::os::unix::fs::MetadataExt::uid(&entry.metadata().ok()?);
            (is_shell(&comm) && owner == uid).then_some(pid)
        })
        .collect()
}

fn is_shell(comm: &str) -> bool {
    comm.trim_end() == "rustv-shell"
}

fn signal(pids: &[i32], signal: i32) {
    for pid in pids {
        // SAFETY: kill(2) only sends a signal; no memory is involved.
        unsafe { libc::kill(*pid, signal) };
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: as above; signal 0 only checks whether the process still exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_cannot_enable_host_power_actions() {
        assert!(simulate_power(true, false));
        assert!(simulate_power(true, true));
        assert!(simulate_power(false, true));
        assert!(!simulate_power(false, false));
    }

    // These exercise the actual dispatch without changing process environment.
    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn debug_power_dispatch_is_safe_without_environment_setup() {
        assert!(dry_run());
        system(Action::Shutdown).await.unwrap();
        system(Action::Reboot).await.unwrap();
        end_session().await.unwrap();
    }

    #[test]
    fn names_match_the_config() {
        for action in Action::ALL {
            let json = serde_json::to_value(action).unwrap();
            assert_eq!(json, action.name());
            assert_eq!(serde_json::from_value::<Action>(json).unwrap(), action);
        }
        assert!(serde_json::from_str::<Action>(r#""format_disk""#).is_err());
    }

    #[test]
    fn finds_the_shell_by_name() {
        assert!(is_shell("rustv-shell\n"));
        assert!(!is_shell("rustv-osd\n"));
        assert!(!is_shell("rustv-shell-old\n"));
    }
}
