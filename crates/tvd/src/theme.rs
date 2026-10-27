//! Colors: a metal finish on graphite, optionally with your own colors on top.
//!
//! tvd works out the whole palette (surfaces, text, the ink on a lit plate)
//! from three colors and sends it to the shell and the remotes, so the TV and
//! the phone always match. The accent is the "lit plate": the focused tile,
//! the OK button, a switch that is on.

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::config::Theme;

/// name, accent (the plate), second accent (a plate being pressed), background.
pub const PRESETS: &[(&str, &str, &str, &str)] = &[
    ("silver", "#c9ced4", "#eceff2", "#0e1013"),
    ("titanium", "#a8b3be", "#d4dce4", "#0c0f12"),
    ("champagne", "#d5c4a1", "#efe5cf", "#100f0d"),
    ("gunmetal", "#8e959d", "#bcc2c8", "#0b0c0d"),
];

/// Presets from before the metal finishes, so older configs keep working.
const RENAMED: &[(&str, &str)] = &[
    ("indigo", "silver"),
    ("ocean", "silver"),
    ("sunset", "silver"),
    ("forest", "silver"),
    ("rose", "silver"),
    ("graphite", "gunmetal"),
];

/// The finish a configured preset name stands for.
fn canonical(name: &str) -> &str {
    RENAMED.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Palette {
    pub preset: String,
    pub accent: String,
    pub accent_2: String,
    /// Lighter accent, for focus rings and highlights.
    pub accent_soft: String,
    /// Text and icons on the accent: dark on a light metal, light on a dark custom color.
    pub on_accent: String,
    /// Body text, and quieter text for hints and labels.
    pub text: String,
    pub muted: String,
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
    if !PRESETS.iter().any(|p| p.0 == canonical(&theme.preset)) {
        let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
        bail!("theme.preset must be one of {}", names.join(", "));
    }
    for (key, value) in [("accent", &theme.accent), ("accent_2", &theme.accent_2), ("background", &theme.background)] {
        if let Some(value) = value {
            rgb(value).with_context(|| format!("theme.{key} must look like #c9ced4"))?;
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
    let name = canonical(&theme.preset);
    let preset = PRESETS.iter().find(|p| p.0 == name).unwrap_or(&PRESETS[0]);
    let pick = |own: &Option<String>, fallback: &str| {
        own.as_deref().and_then(|c| rgb(c).ok()).unwrap_or_else(|| rgb(fallback).expect("presets are valid"))
    };
    let accent = pick(&theme.accent, preset.1);
    let accent_2 = pick(&theme.accent_2, preset.2);
    let bg = pick(&theme.background, preset.3);
    const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
    // Graphite surfaces carry a trace of the metal, so champagne reads warm and titanium cool.
    let surface = |lift: f32| mix(mix(bg, WHITE, lift), accent, 0.025);
    // Light metals take dark ink; a dark custom accent takes light ink.
    let on_accent = if luminance(accent) > 0.25 { mix(bg, BLACK, 0.2) } else { WHITE };
    Palette {
        preset: preset.0.to_string(),
        accent: hex(accent),
        accent_2: hex(accent_2),
        accent_soft: hex(mix(accent, WHITE, 0.45)),
        on_accent: hex(on_accent),
        text: hex(mix(mix(WHITE, accent, 0.06), bg, 0.04)),
        muted: hex(mix(mix(bg, WHITE, 0.58), accent, 0.06)),
        background: hex(bg),
        // The background is flat now; these stay for clients that still draw a gradient.
        background_top: hex(bg),
        background_mid: hex(bg),
        surface: hex(surface(0.045)),
        surface_2: hex(surface(0.075)),
        surface_3: hex(surface(0.115)),
        line: hex(surface(0.14)),
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

const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

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
        let mut t = theme("titanium");
        t.accent = Some("#ff0000".into());
        let palette = resolve(&t);
        assert_eq!(palette.accent, "#ff0000");
        assert_eq!(palette.accent_2, "#d4dce4");
        assert_eq!(palette.preset, "titanium");
    }

    #[test]
    fn old_preset_names_still_load() {
        for (old, new) in RENAMED {
            let t = theme(old);
            validate(&t).unwrap();
            assert_eq!(resolve(&t).preset, *new);
        }
    }

    /// WCAG contrast ratio between two colors.
    fn contrast(a: &str, b: &str) -> f32 {
        let (a, b) = (luminance(rgb(a).unwrap()), luminance(rgb(b).unwrap()));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_is_readable_on_every_finish() {
        for (name, ..) in PRESETS {
            let p = resolve(&theme(name));
            for ground in [&p.background, &p.surface, &p.surface_2, &p.surface_3] {
                assert!(contrast(&p.text, ground) >= 7.0, "{name}: text on {ground}");
                assert!(contrast(&p.muted, ground) >= 4.5, "{name}: muted on {ground}");
            }
            assert!(contrast(&p.on_accent, &p.accent) >= 4.5, "{name}: ink on the plate");
            assert!(contrast(&p.on_accent, &p.accent_2) >= 4.5, "{name}: ink on a pressed plate");
        }
        // A dark custom accent gets light ink.
        let mut t = theme("silver");
        t.accent = Some("#2f3a8f".into());
        assert_eq!(resolve(&t).on_accent, "#ffffff");
    }

    #[test]
    fn rejects_bad_colors_and_bright_backgrounds() {
        let mut t = theme("silver");
        t.accent = Some("red".into());
        assert!(validate(&t).is_err());
        let mut t = theme("silver");
        t.background = Some("#f0f0f0".into());
        assert!(validate(&t).is_err());
        assert!(validate(&theme("neon")).is_err());
    }
}
