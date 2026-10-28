//! Changing settings from the remote.
//!
//! A change is written into `config.toml` in place, so comments and layout
//! survive, and only once the whole file still passes `tvd check`. The file
//! stays the single source of truth; the remote is just another way to edit it.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use toml_edit::DocumentMut;

use crate::config::Config;

/// What the remote may change. Apps, the server and the browser stay
/// config-file only: they decide what runs on the box and who can connect.
pub const EDITABLE: &[&str] = &[
    "shell.greeting",
    "shell.clock",
    "shell.date",
    "shell.hints",
    "shell.volume_overlay",
    "shell.pairing_qr",
    "shell.sounds",
    "shell.sound_volume",
    "shell.settings",
    "shell.now_playing",
    "screensaver.enabled",
    "screensaver.idle_minutes",
    "screensaver.style",
    "screensaver.clock",
    "screensaver.date",
    "audio.max_volume",
    "audio.step",
    "audio.per_app_volume",
    "audio.night",
    "cast.airplay",
    "lock.on_sleep",
    "gamepad.enabled",
    "gamepad.guide_closes_app",
    "theme.preset",
    "theme.accent",
    "theme.accent_2",
    "theme.background",
    "locale.language",
    "locale.clock",
    "locale.temperature",
    "sleep.enabled",
    "sleep.after_minutes",
    "sleep.close_apps",
    "overlay.enabled",
    "overlay.position",
    "wallpaper.home",
    "wallpaper.change_minutes",
    "wallpaper.slide_seconds",
    "wallpaper.shuffle",
    "display.brightness",
    "display.warmth",
    "night.enabled",
    "night.from",
    "night.to",
    "night.brightness",
    "night.warmth",
    "night.audio",
    "keyboard.enabled",
    "keyboard.hold_ok",
    "keyboard.hold_guide",
    "cec.enabled",
    "cec.power_tv",
    "cec.follow_tv",
    "cec.remote_keys",
];

/// Sets `key` (such as `screensaver.idle_minutes`) in the config file and
/// returns the config as it is now.
pub fn apply(path: &Path, key: &str, value: &Value) -> Result<Config> {
    if !EDITABLE.contains(&key) {
        bail!("{key} cannot be changed from the remote");
    }
    write(path, key, value)
}

/// The same for settings tvd changes itself after its own checks, such as a
/// display mode once someone kept it.
pub fn apply_kept(path: &Path, key: &str, value: &Value) -> Result<Config> {
    write(path, key, value)
}

fn write(path: &Path, key: &str, value: &Value) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let updated = set(&text, key, value)?;
    let config = Config::parse(&updated).with_context(|| format!("{key}: value rejected"))?;
    // Write next to the file and rename, so a crash never leaves half a config.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &updated).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(config)
}

fn set(text: &str, key: &str, value: &Value) -> Result<String> {
    let (table, field) = key.split_once('.').context("key must look like section.name")?;
    let mut new = match value {
        Value::Bool(b) => toml_edit::value(*b),
        // Whole numbers stay integers (`idle_minutes = 10`), except volume levels, which read better as 1.0.
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e9 && table != "audio" => toml_edit::value(f as i64),
            Some(f) => toml_edit::value(f),
            None => bail!("{key}: not a number"),
        },
        Value::String(s) => toml_edit::value(s.as_str()),
        // `null` removes the key, so its default applies again (for example a preset color).
        Value::Null => toml_edit::Item::None,
        _ => bail!("{key}: expected true/false, a number, text or null"),
    };

    let mut doc: DocumentMut = text.parse().context("parsing the config")?;
    let section = doc
        .entry(table)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .with_context(|| format!("[{table}] must be a normal table to be edited from the remote"))?;
    if let Some(old) = section.get(field).and_then(|item| item.as_value()) {
        // Keep the comment after the old value.
        if let Some(value) = new.as_value_mut() {
            *value.decor_mut() = old.decor().clone();
        }
    }
    if new.is_none() {
        section.remove(field);
    } else {
        section[field] = new;
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CONFIG: &str = "# my TV\n[screensaver]\nidle_minutes = 5   # minutes\nstyle = \"random\"\n";

    #[test]
    fn keeps_comments_and_layout() {
        let out = set(CONFIG, "screensaver.idle_minutes", &json!(10)).unwrap();
        assert_eq!(out, "# my TV\n[screensaver]\nidle_minutes = 10   # minutes\nstyle = \"random\"\n");
        Config::parse(&out).unwrap();
    }

    #[test]
    fn adds_missing_sections() {
        let out = set(CONFIG, "audio.max_volume", &json!(1)).unwrap();
        assert!(out.contains("[audio]\nmax_volume = 1.0\n"), "{out}");
        assert_eq!(Config::parse(&out).unwrap().audio.max_volume, 1.0);
    }

    #[test]
    fn null_removes_a_key() {
        let out = set(CONFIG, "screensaver.style", &serde_json::Value::Null).unwrap();
        assert!(!out.contains("style"), "{out}");
        assert_eq!(Config::parse(&out).unwrap().screensaver.idle_minutes, 5.0);
    }

    #[test]
    fn refuses_other_keys_and_bad_values() {
        let path = std::env::temp_dir().join(format!("tvd-settings-{}.toml", std::process::id()));
        std::fs::write(&path, CONFIG).unwrap();
        assert!(apply(&path, "server.bind", &json!("0.0.0.0:1")).is_err());
        // Only after tvd's own checks.
        assert!(apply(&path, "display.mode", &json!("1920x1080@60")).is_err());
        assert_eq!(apply_kept(&path, "display.mode", &json!("1920x1080@60")).unwrap().display.mode, "1920x1080@60");
        std::fs::remove_file(path).unwrap();
        assert!(set(CONFIG, "screensaver.idle_minutes", &json!([1])).is_err());
        // Parses as TOML but fails validation.
        let out = set(CONFIG, "screensaver.style", &json!("neon")).unwrap();
        assert!(Config::parse(&out).is_err());
    }
}
