//! Volume control for the default PipeWire sink, through `wpctl`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::config::Audio;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Volume {
    /// 1.0 = 100%.
    pub level: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeAction {
    Up,
    Down,
    Mute,
    Set,
}

pub async fn get() -> Result<Volume> {
    parse(&wpctl(&["get-volume".into(), SINK.into()]).await?)
}

pub async fn apply(action: VolumeAction, level: Option<f32>, cfg: &Audio) -> Result<()> {
    let step = format!("{}%", (cfg.step * 100.0).round() as u32);
    let max = format!("{:.2}", cfg.max_volume);
    let args: Vec<String> = match action {
        VolumeAction::Up => vec!["set-volume".into(), "-l".into(), max, SINK.into(), format!("{step}+")],
        VolumeAction::Down => vec!["set-volume".into(), SINK.into(), format!("{step}-")],
        VolumeAction::Mute => vec!["set-mute".into(), SINK.into(), "toggle".into()],
        VolumeAction::Set => {
            let level = level.context("volume \"set\" needs a level")?.clamp(0.0, cfg.max_volume);
            vec!["set-volume".into(), SINK.into(), format!("{level:.2}")]
        }
    };
    wpctl(&args).await?;
    Ok(())
}

async fn wpctl(args: &[String]) -> Result<String> {
    let out = Command::new("wpctl").args(args).output().await.context("running wpctl (is PipeWire running?)")?;
    if !out.status.success() {
        bail!("wpctl {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parses `Volume: 0.45` or `Volume: 0.45 [MUTED]`.
fn parse(output: &str) -> Result<Volume> {
    let rest = output.trim().strip_prefix("Volume:").context("unexpected wpctl output")?;
    let level = rest.split_whitespace().next().context("missing volume level")?.parse()?;
    Ok(Volume { level, muted: rest.contains("[MUTED]") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_output() {
        assert_eq!(parse("Volume: 0.45\n").unwrap(), Volume { level: 0.45, muted: false });
        assert_eq!(parse("Volume: 1.00 [MUTED]").unwrap(), Volume { level: 1.0, muted: true });
        assert!(parse("garbage").is_err());
    }
}
