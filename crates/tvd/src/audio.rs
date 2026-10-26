//! Volume and output selection for PipeWire, through `wpctl` and `pw-dump`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// An audio output (PipeWire sink), such as HDMI or the headphone jack.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Output {
    pub id: u32,
    /// Short, human name: the TV's own name over HDMI, the device name for Bluetooth.
    pub name: String,
    /// `hdmi`, `analog`, `bluetooth`, `usb` or `other`, for picking an icon.
    pub kind: &'static str,
    /// The full PipeWire description.
    pub detail: String,
    pub default: bool,
}

pub async fn outputs() -> Result<Vec<Output>> {
    let out = Command::new("pw-dump").output().await.context("running pw-dump (is PipeWire running?)")?;
    if !out.status.success() {
        bail!("pw-dump: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse_outputs(&out.stdout)
}

pub async fn set_output(id: u32) -> Result<()> {
    wpctl(&["set-default".into(), id.to_string()]).await?;
    Ok(())
}

fn parse_outputs(json: &[u8]) -> Result<Vec<Output>> {
    // pw-dump prints a second array when objects change while it runs (for example right
    // after switching outputs); later entries win.
    let mut objects: Vec<Value> = Vec::new();
    for batch in serde_json::Deserializer::from_slice(json).into_iter::<Vec<Value>>() {
        for object in batch.context("parsing pw-dump output")? {
            match objects.iter_mut().find(|o| o["id"].is_u64() && o["id"] == object["id"]) {
                Some(existing) if object["info"].is_null() && object.get("metadata").is_none() => {
                    // A removal ({"id": N, "info": null}).
                    *existing = Value::Null;
                }
                Some(existing) => *existing = object,
                None => objects.push(object),
            }
        }
    }

    // The default sink is stored by node name in the "default" metadata object.
    let default_name = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Metadata")
        .flat_map(|o| o["metadata"].as_array().into_iter().flatten())
        .find(|entry| entry["key"] == "default.audio.sink")
        .and_then(|entry| match &entry["value"] {
            Value::String(text) => serde_json::from_str::<Value>(text).ok()?["name"].as_str().map(String::from),
            value => value["name"].as_str().map(String::from),
        });

    let outputs = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Node" && o["info"]["props"]["media.class"] == "Audio/Sink")
        .filter_map(|o| {
            let props = &o["info"]["props"];
            let text = |key: &str| props[key].as_str().unwrap_or_default();
            let node_name = text("node.name");
            let detail = [text("node.description"), text("node.nick"), node_name]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            let (kind, name) = describe(node_name, text("device.profile.name"), text("node.nick"), detail);
            Some(Output {
                id: o["id"].as_u64()?.try_into().ok()?,
                name,
                kind,
                detail: detail.to_string(),
                default: default_name.as_deref() == Some(node_name),
            })
        })
        .collect();
    Ok(outputs)
}

/// ALSA descriptions name the chip ("Raven/Raven2/Fenghuang HDMI/DP Audio Controller"),
/// which means nothing on a remote. HDMI sinks carry the TV's name in `node.nick`.
fn describe(node_name: &str, profile: &str, nick: &str, detail: &str) -> (&'static str, String) {
    if node_name.starts_with("bluez_output") {
        ("bluetooth", detail.to_string())
    } else if profile.contains("hdmi") || node_name.contains("hdmi") {
        let generic = nick.is_empty() || nick.to_ascii_lowercase().contains("hdmi");
        ("hdmi", if generic { "TV (HDMI)".to_string() } else { format!("{nick} TV") })
    } else if node_name.contains(".usb-") {
        ("usb", detail.to_string())
    } else if profile.contains("analog") || node_name.contains("analog") {
        ("analog", "Speakers and headphones".to_string())
    } else {
        ("other", detail.to_string())
    }
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

    #[test]
    fn parses_pw_dump_outputs() {
        let json = br#"[
            {"id": 30, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
             "metadata": [{"subject": 0, "key": "default.audio.sink", "type": "Spa:String:JSON",
                           "value": {"name": "alsa_output.hdmi"}}]},
            {"id": 52, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Sink", "node.name": "alsa_output.hdmi", "node.description": "Raven HDMI/DP Audio Controller",
                "node.nick": "SAMSUNG", "device.profile.name": "hdmi-stereo"}}},
            {"id": 58, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Sink", "node.name": "alsa_output.analog", "node.description": "Headphones"}}},
            {"id": 60, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Source", "node.name": "mic"}}}
        ]"#;
        assert_eq!(
            parse_outputs(json).unwrap(),
            vec![
                Output {
                    id: 52,
                    name: "SAMSUNG TV".into(),
                    kind: "hdmi",
                    detail: "Raven HDMI/DP Audio Controller".into(),
                    default: true
                },
                Output {
                    id: 58,
                    name: "Speakers and headphones".into(),
                    kind: "analog",
                    detail: "Headphones".into(),
                    default: false
                },
            ]
        );
    }

    #[test]
    fn merges_updates_printed_as_extra_arrays() {
        let json = br#"[
            {"id": 30, "type": "PipeWire:Interface:Metadata", "metadata": [{"key": "default.audio.sink", "value": {"name": "a"}}]},
            {"id": 1, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "a"}}},
            {"id": 2, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "b"}}}
        ]
        [
            {"id": 30, "type": "PipeWire:Interface:Metadata", "metadata": [{"key": "default.audio.sink", "value": {"name": "b"}}]},
            {"id": 1, "info": null}
        ]"#;
        assert_eq!(
            parse_outputs(json).unwrap(),
            vec![Output { id: 2, name: "b".into(), kind: "other", detail: "b".into(), default: true }]
        );
    }

    #[test]
    fn reads_default_sink_stored_as_json_text() {
        let json = br#"[
            {"type": "PipeWire:Interface:Metadata", "metadata": [{"key": "default.audio.sink", "value": "{\"name\":\"a\"}"}]},
            {"id": 1, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "a"}}}
        ]"#;
        assert!(parse_outputs(json).unwrap()[0].default);
    }
}
