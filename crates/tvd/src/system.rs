//! The box itself, for the remote: a screenshot of the TV, and how the box is
//! doing (temperatures, memory, disk, uptime) with tvd's recent log lines, so
//! a problem can be looked at from the sofa.
//!
//! tvd never logs the token, typed text or passwords, so its log is safe to
//! show to a paired remote; other units' logs are not shown.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use tokio::process::Command;

use crate::stats::Sampler;

/// The TV picture as PNG, overlays and all, through `grim` in the TV session.
/// (PNG because grim is often built without JPEG; a light compression keeps it quick.)
pub async fn screenshot() -> Result<Vec<u8>> {
    let run = Command::new("grim")
        .args(["-t", "png", "-l", "3", "-"])
        .envs(crate::apps::session_env()?)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(8), run)
        .await
        .context("grim took too long")?
        .context("running grim (is it installed?)")?;
    if !out.status.success() || out.stdout.is_empty() {
        bail!("grim: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

#[derive(Debug, Serialize)]
pub struct Info {
    pub version: &'static str,
    pub host: String,
    pub kernel: String,
    pub uptime_secs: Option<u64>,
    /// 1, 5 and 15 minute load averages.
    pub load: Option<[f32; 3]>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    /// Used and total, in bytes.
    pub memory: Option<(u64, u64)>,
    pub disk: Vec<Disk>,
    /// tvd's last log lines, oldest first; empty when tvd does not run under systemd.
    pub log: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Disk {
    pub path: String,
    pub used: u64,
    pub total: u64,
}

pub async fn info() -> Info {
    let sample = Sampler::default().sample();
    let read = |path: &str| std::fs::read_to_string(path).map(|text| text.trim().to_string()).unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let mut disk: Vec<Disk> = Vec::new();
    for path in ["/", home.as_str()] {
        if let Some((used, total)) = usage(path) {
            // Home on the same filesystem as / says nothing new.
            if !disk.iter().any(|d| (d.used, d.total) == (used, total)) {
                disk.push(Disk { path: path.to_string(), used, total });
            }
        }
    }
    Info {
        version: env!("CARGO_PKG_VERSION"),
        host: read("/proc/sys/kernel/hostname"),
        kernel: read("/proc/sys/kernel/osrelease"),
        uptime_secs: sample.uptime_secs,
        load: parse_load(&read("/proc/loadavg")),
        cpu_temp: sample.cpu_temp,
        gpu_temp: sample.gpu_temp,
        memory: sample.memory,
        disk,
        log: log().await.unwrap_or_default(),
    }
}

fn parse_load(text: &str) -> Option<[f32; 3]> {
    let mut parts = text.split_whitespace().map(|part| part.parse::<f32>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

/// Used and total bytes of the filesystem `path` is on.
fn usage(path: &str) -> Option<(u64, u64)> {
    let path = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs fills the struct for a valid C string path; zeroed is a valid start.
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(path.as_ptr(), &mut stat) == 0).then_some(stat)?
    };
    let block = stat.f_frsize as u64;
    let total = stat.f_blocks as u64 * block;
    Some((total - stat.f_bfree as u64 * block, total))
}

/// The last 150 lines of tvd's log, from the user journal.
async fn log() -> Result<Vec<String>> {
    let run = Command::new("journalctl")
        .args(["--user", "--unit", "tvd", "--lines", "150", "--no-pager", "--output", "short-iso", "--quiet"])
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(5), run).await.context("journalctl took too long")??;
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_load_and_disk() {
        assert_eq!(parse_load("0.52 0.58 0.59 1/389 12345"), Some([0.52, 0.58, 0.59]));
        assert_eq!(parse_load(""), None);
        let (used, total) = usage("/").unwrap();
        assert!(total > 0 && used <= total);
    }
}
