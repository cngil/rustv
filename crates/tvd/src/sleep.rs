//! Sleep: after a long time without use the screen goes dark and the open app
//! closes, which frees its memory and CPU and lets the TV fall into standby.
//! Any remote input wakes it.
//!
//! "Without use" means no remote input *and* no sound playing, so a two hour
//! film never counts as idle. This is not a system suspend: tvd keeps running,
//! so the remote can always wake the TV. A suspended box would drop off the
//! network, remote and all.

use std::path::Path;
use std::sync::Weak;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use crate::state::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(30);

/// Checks every 30 s whether the TV should go to sleep, for as long as tvd runs.
pub fn watch(state: Weak<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CHECK_EVERY).await;
            let Some(state) = state.upgrade() else { return };
            state.check_sleep().await;
        }
    });
}

/// Whether any sound card is playing. PipeWire closes idle devices after a few
/// seconds, so a paused film reads as silent.
pub fn sound_playing() -> bool {
    let Ok(cards) = std::fs::read_dir("/proc/asound") else { return false };
    cards
        .flatten()
        .filter(|card| card.file_name().to_string_lossy().starts_with("card"))
        .flat_map(|card| std::fs::read_dir(card.path()).into_iter().flatten().flatten())
        .filter(|pcm| pcm.file_name().to_string_lossy().ends_with('p'))
        .flat_map(|pcm| std::fs::read_dir(pcm.path()).into_iter().flatten().flatten())
        .any(|sub| playing(&sub.path().join("status")))
}

fn playing(status: &Path) -> bool {
    std::fs::read_to_string(status).is_ok_and(|text| text.lines().any(|line| line.trim() == "state: RUNNING"))
}

/// Turns the TV outputs on or off through sway. Off, most TVs show "no signal"
/// and drop into standby on their own.
pub async fn display_power(on: bool) -> Result<()> {
    let env = crate::apps::session_env()?;
    let runtime = &env.iter().find(|(key, _)| key == "XDG_RUNTIME_DIR").context("no runtime dir")?.1;
    let sway = crate::overlay::find_sway_socket(Path::new(runtime)).context("sway is not running")?;
    let command = format!("output * power {}", if on { "on" } else { "off" });
    let out = Command::new("swaymsg").env("SWAYSOCK", sway).arg(&command).output().await.context("running swaymsg")?;
    if !out.status.success() {
        bail!("swaymsg {command:?}: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_alsa_status() {
        let dir = std::env::temp_dir().join(format!("tvd-sleep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let status = dir.join("status");
        std::fs::write(&status, "state: RUNNING\nowner_pid   : 1234\n").unwrap();
        assert!(playing(&status));
        std::fs::write(&status, "closed\n").unwrap();
        assert!(!playing(&status));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
