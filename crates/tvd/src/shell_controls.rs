//! Temporarily focus the home screen without stopping the running app.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::process::Command;
pub static POINTER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CONTROL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RETURN_TO: AtomicI64 = AtomicI64::new(0);
pub fn active() -> bool {
    RETURN_TO.load(Ordering::Relaxed) != 0
}
fn focused(tree: &Value) -> Option<i64> {
    if tree["focused"] == true {
        return tree["id"].as_i64();
    }
    ["nodes", "floating_nodes"].iter().flat_map(|key| tree[key].as_array().into_iter().flatten()).find_map(focused)
}
fn shell(tree: &Value) -> Option<i64> {
    if let Some(pid) = tree["pid"].as_u64() {
        if std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|name| name.trim() == "rustv-shell") {
            return tree["id"].as_i64();
        }
    }
    ["nodes", "floating_nodes"].iter().flat_map(|key| tree[key].as_array().into_iter().flatten()).find_map(shell)
}
async fn sway(args: &[&str]) -> Result<Value> {
    let output = Command::new("swaymsg")
        .env("SWAYSOCK", crate::display::sway_socket()?)
        .args(args)
        .output()
        .await
        .context("contacting the TV session")?;
    if !output.status.success() {
        bail!("Could not focus the TV controls");
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    if value.as_array().is_some_and(|rows| rows.iter().any(|row| row["success"] == false)) {
        bail!("Could not focus the TV controls");
    }
    Ok(value)
}
pub async fn show(open: bool) -> Result<()> {
    let _guard = CONTROL_LOCK.lock().await;
    if open {
        if active() {
            return Ok(());
        }
        let tree = sway(&["-t", "get_tree"]).await?;
        let id = focused(&tree).context("No focused TV window")?;
        let shell = shell(&tree).context("The home screen is not running")?;
        sway(&[&format!("[con_id={shell}] focus")]).await?;
        RETURN_TO.store(id, Ordering::Relaxed);
    } else {
        let id = RETURN_TO.swap(0, Ordering::Relaxed);
        if id != 0 {
            sway(&[&format!("[con_id={id}] focus")]).await?;
        }
    }
    Ok(())
}
pub fn clear() {
    RETURN_TO.store(0, Ordering::Relaxed);
    POINTER.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_focused_floating_and_tiled_windows() {
        let tree = serde_json::json!({"nodes":[{"nodes":[]}],"floating_nodes":[{"id":7,"focused":true}]});
        assert_eq!(focused(&tree), Some(7));
        assert_eq!(focused(&serde_json::json!({})), None);
    }
}
