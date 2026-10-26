//! Colors: a preset, optionally with your own accent and background on top.
//!
//! tvd works out the whole palette (surfaces, lines, the lighter accent for
//! focus rings) from three colors and sends it to the shell and the remotes,
//! so the TV and the phone always match.

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::config::Theme;

/// name, accent, second accent (gradients), background.
pub const PRESETS: &[(&str, &str, &str, &str)] = &[
    ("indigo", "#6c8cff", "#9b6cff", "#0b0d12"),
    ("ocean", "#22b8cf", "#3b82f6", "#071216"),
    ("sunset", "#ff8a4c", "#ff4f8b", "#130c0c"),
    ("forest", "#4ade80", "#14b8a6", "#0a110d"),
    ("rose", "#f472b6", "#a855f7", "#130b12"),
    ("graphite", "#a1a1aa", "#e4e4e7", "#0c0c0e"),
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Palette {
    pub preset: String,
    pub accent: String,
    pub accent_2: String,
    /// Lighter accent, for focus rings and highlights.
    pub accent_soft: String,
    pub background: String,
    /// Top and middle of the background gradient.
    pub background_top: String,
    pub background_mid: String,
    /// Cards, buttons and panels, from darkest to lightest.
    pub surface: String,
    pub surface_2: String,
    pub surface_3: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub name: &'static str,
    pub accent: &'static str,
    pub accent_2: &'static str,
    pub background: &'static str,
}

pub fn presets() -> Vec<Preset> {
    PRESETS.iter().map(|&(name, accent, accent_2, background)| Preset { name, accent, accent_2, background }).collect()
}

/// Checks the config: a known preset, colors as `#rrggbb`, a dark background
/// (all text is light, so a bright background would make it unreadable).
pub fn validate(theme: &Theme) -> Result<()> {
    if !PRESETS.iter().any(|p| p.0 == theme.preset) {
        let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
        bail!("theme.preset must be one of {}", names.join(", "));
    }
    for (key, value) in [("accent", &theme.accent), ("accent_2", &theme.accent_2), ("background", &theme.background)] {
        if let Some(value) = value {
            rgb(value).with_context(|| format!("theme.{key} must look like #6c8cff"))?;
        }
    }
    if let Some(background) = &theme.background {
        if luminance(rgb(background)?) > 0.06 {
            bail!("theme.background must be a dark color; text on the TV is light");
        }
    }
    Ok(())
}

pub fn resolve(theme: &Theme) -> Palette {
    let preset = PRESETS.iter().find(|p| p.0 == theme.preset).unwrap_or(&PRESETS[0]);
    let pick = |own: &Option<String>, fallback: &str| {
        own.as_deref().and_then(|c| rgb(c).ok()).unwrap_or_else(|| rgb(fallback).expect("presets are valid"))
    };
    let accent = pick(&theme.accent, preset.1);
    let accent_2 = pick(&theme.accent_2, preset.2);
    let bg = pick(&theme.background, preset.3);
    const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
    let surface = |lift: f32, tint: f32| mix(mix(bg, WHITE, lift), accent, tint);
    Palette {
        preset: preset.0.to_string(),
        accent: hex(accent),
        accent_2: hex(accent_2),
        accent_soft: hex(mix(accent, WHITE, 0.45)),
        background: hex(bg),
        background_top: hex(mix(bg, accent, 0.2)),
        background_mid: hex(mix(bg, accent, 0.07)),
        surface: hex(surface(0.045, 0.03)),
        surface_2: hex(surface(0.085, 0.05)),
        surface_3: hex(surface(0.12, 0.06)),
        line: hex(surface(0.12, 0.06)),
    }
}

fn rgb(text: &str) -> Result<[f32; 3]> {
    let digits = text.strip_prefix('#').filter(|d| d.len() == 6).context("expected #rrggbb")?;
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).map(f32::from);
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

fn hex([r, g, b]: [f32; 3]) -> String {
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// `a` moved `t` of the way towards `b`.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Relative luminance (WCAG), 0 for black to 1 for white.
fn luminance(color: [f32; 3]) -> f32 {
    let linear = |v: f32| {
        let v = v / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(preset: &str) -> Theme {
        Theme { preset: preset.into(), accent: None, accent_2: None, background: None }
    }

    #[test]
    fn presets_resolve_and_validate() {
        for (name, accent, ..) in PRESETS {
            let t = theme(name);
            validate(&t).unwrap();
            let palette = resolve(&t);
            assert_eq!(&palette.accent, accent);
            // Surfaces sit between the background and white.
            assert!(luminance(rgb(&palette.surface).unwrap()) > luminance(rgb(&palette.background).unwrap()));
        }
    }

    #[test]
    fn own_colors_override_the_preset() {
        let mut t = theme("ocean");
        t.accent = Some("#ff0000".into());
        let palette = resolve(&t);
        assert_eq!(palette.accent, "#ff0000");
        assert_eq!(palette.accent_2, "#3b82f6");
        assert_eq!(palette.preset, "ocean");
    }

    #[test]
    fn rejects_bad_colors_and_bright_backgrounds() {
        let mut t = theme("indigo");
        t.accent = Some("red".into());
        assert!(validate(&t).is_err());
        let mut t = theme("indigo");
        t.background = Some("#f0f0f0".into());
        assert!(validate(&t).is_err());
        assert!(validate(&theme("neon")).is_err());
    }
}
