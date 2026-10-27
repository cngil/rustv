//! Resolution and refresh rate of the TV output, through sway.
//!
//! A new mode applies at once and goes back by itself unless someone keeps it:
//! a mode the TV cannot show leaves a black screen, and whoever has only the
//! TV remote must still get their picture back. Only a kept mode is written to
//! the config, so a restart never lands on an untried one.

use std::fmt;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use tokio::process::Command;

/// How long a new mode stays without being kept.
pub const TRIAL_SECONDS: u64 = 15;
/// Smaller modes are for old monitors, not TVs.
const MIN_WIDTH: u32 = 1280;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    /// Millihertz, as sway reports it: 59940 is 59.94 Hz.
    pub refresh: u32,
}

impl Mode {
    /// `1920x1080@60` or `1920x1080@59.94`.
    pub fn parse(text: &str) -> Result<Self> {
        let (size, hz) = text.split_once('@').context("expected WIDTHxHEIGHT@HZ, such as 1920x1080@60")?;
        let (width, height) = size.split_once('x').context("expected WIDTHxHEIGHT@HZ, such as 1920x1080@60")?;
        let hz: f64 = hz.trim_end_matches("Hz").parse().context("refresh rate must be a number")?;
        if !(1.0..=1000.0).contains(&hz) {
            bail!("refresh rate must be between 1 and 1000 Hz");
        }
        Ok(Self { width: width.parse()?, height: height.parse()?, refresh: (hz * 1000.0).round() as u32 })
    }

    /// What sway's `output … mode` takes.
    fn sway(&self) -> String {
        format!("{}x{}@{:.3}Hz", self.width, self.height, f64::from(self.refresh) / 1000.0)
    }
}

impl fmt::Display for Mode {
    /// `1920x1080@60`, `1920x1080@59.94`: the form the config and the API use.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hz = format!("{:.3}", f64::from(self.refresh) / 1000.0);
        write!(f, "{}x{}@{}", self.width, self.height, hz.trim_end_matches('0').trim_end_matches('.'))
    }
}

/// The TV output as the remotes and the shell see it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Info {
    /// What the config asks for: `auto` (leave it to sway) or a mode.
    pub mode: String,
    pub output: String,
    /// The mode on screen; tvd's last one while the screen is off.
    pub current: Option<String>,
    /// Every mode the TV offers, largest and fastest first.
    pub modes: Vec<String>,
}

/// The active output, its mode and the modes it offers, from `swaymsg -t get_outputs`.
pub fn parse_outputs(outputs: &Value) -> Option<(String, Option<Mode>, Vec<Mode>)> {
    let output = outputs.as_array()?.iter().find(|o| o["active"] == true)?;
    let mode = |m: &Value| {
        Some(Mode {
            width: u32::try_from(m["width"].as_u64()?).ok()?,
            height: u32::try_from(m["height"].as_u64()?).ok()?,
            refresh: u32::try_from(m["refresh"].as_u64()?).ok()?,
        })
    };
    // A powered-off output reports 0x0.
    let current = mode(&output["current_mode"]).filter(|m| m.width > 0);
    let mut modes: Vec<Mode> =
        output["modes"].as_array()?.iter().filter_map(mode).filter(|m| m.width >= MIN_WIDTH && m.refresh > 0).collect();
    modes.sort_by_key(|m| std::cmp::Reverse((m.width * m.height, m.refresh)));
    modes.dedup();
    Some((output["name"].as_str()?.to_string(), current, modes))
}

fn sway_socket() -> Result<std::path::PathBuf> {
    let env = crate::apps::session_env()?;
    let runtime = &env.iter().find(|(key, _)| key == "XDG_RUNTIME_DIR").context("no runtime dir")?.1;
    crate::overlay::find_sway_socket(Path::new(runtime)).context("sway is not running")
}

/// Asks sway for the active output.
pub async fn read() -> Result<(String, Option<Mode>, Vec<Mode>)> {
    let out = Command::new("swaymsg")
        .env("SWAYSOCK", sway_socket()?)
        .args(["-r", "-t", "get_outputs"])
        .output()
        .await
        .context("running swaymsg")?;
    let outputs: Value = serde_json::from_slice(&out.stdout).context("reading sway's outputs")?;
    parse_outputs(&outputs).context("no active output")
}

pub async fn apply(output: &str, mode: Mode) -> Result<()> {
    let command = format!("output {output} mode {}", mode.sway());
    let out = Command::new("swaymsg").env("SWAYSOCK", sway_socket()?).arg(&command).output().await?;
    if !out.status.success() {
        bail!("swaymsg {command:?}: {}", String::from_utf8_lossy(&out.stdout).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn modes_read_and_write() {
        let mode = Mode::parse("1920x1080@59.94").unwrap();
        assert_eq!(mode, Mode { width: 1920, height: 1080, refresh: 59940 });
        assert_eq!(mode.to_string(), "1920x1080@59.94");
        assert_eq!(mode.sway(), "1920x1080@59.940Hz");
        assert_eq!(Mode::parse("3840x2160@60").unwrap().to_string(), "3840x2160@60");
        assert_eq!(Mode::parse("1920x1080@60Hz").unwrap().refresh, 60000);
        assert!(Mode::parse("1920x1080").is_err());
        assert!(Mode::parse("fullhd@60").is_err());
    }

    #[test]
    fn lists_tv_modes_largest_first() {
        let outputs = json!([
            { "name": "eDP-1", "active": false, "current_mode": {}, "modes": [] },
            { "name": "HDMI-A-1", "active": true,
              "current_mode": { "width": 0, "height": 0, "refresh": 0 },
              "modes": [
                { "width": 1920, "height": 1080, "refresh": 60000 },
                { "width": 640, "height": 480, "refresh": 60000 },
                { "width": 3840, "height": 2160, "refresh": 30000 },
                { "width": 1920, "height": 1080, "refresh": 120000 },
                { "width": 1920, "height": 1080, "refresh": 60000 }
              ] }
        ]);
        let (name, current, modes) = parse_outputs(&outputs).unwrap();
        assert_eq!(name, "HDMI-A-1");
        // The screen is off.
        assert_eq!(current, None);
        let modes: Vec<String> = modes.iter().map(Mode::to_string).collect();
        assert_eq!(modes, ["3840x2160@30", "1920x1080@120", "1920x1080@60"]);
    }
}
