//! Configuration file (`config.toml`) and the pairing token.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/etc"))
        .join("rustv")
}

pub fn default_path() -> PathBuf {
    config_dir().join("config.toml")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub browser: Browser,
    #[serde(default)]
    pub audio: Audio,
    #[serde(default)]
    pub shell: Shell,
    #[serde(default)]
    pub screensaver: Screensaver,
    #[serde(default)]
    pub sleep: Sleep,
    #[serde(default)]
    pub cast: Cast,
    #[serde(default)]
    pub overlay: Overlay,
    #[serde(default, rename = "app")]
    pub apps: Vec<App>,
}

/// What the on-screen shell shows. Sent to the shell, which has no config of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Shell {
    pub greeting: bool,
    pub clock: bool,
    pub date: bool,
    /// Key hints at the bottom of the home screen.
    pub hints: bool,
    /// Volume bar shown when the volume changes.
    pub volume_overlay: bool,
    /// QR code for pairing a phone, shown until the first remote connects and on request.
    pub pairing_qr: bool,
    /// Short sounds when moving between apps and opening one.
    pub sounds: bool,
    /// Loudness of those sounds, 0..1, on top of the TV volume.
    pub sound_volume: f32,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            greeting: true,
            clock: true,
            date: true,
            hints: true,
            volume_overlay: true,
            pairing_qr: true,
            sounds: true,
            sound_volume: 0.5,
        }
    }
}

/// Screensaver on the home screen. Sent to the shell, which runs it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Screensaver {
    pub enabled: bool,
    /// Minutes on the home screen without input before it starts.
    pub idle_minutes: f32,
    pub style: ScreensaverStyle,
    pub clock: bool,
    pub date: bool,
}

impl Default for Screensaver {
    fn default() -> Self {
        Self { enabled: true, idle_minutes: 5.0, style: ScreensaverStyle::Random, clock: true, date: true }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreensaverStyle {
    /// A different style each time.
    Random,
    /// Soft colored lights drifting across each other.
    Aurora,
    /// A slowly turning galaxy of points.
    Orbits,
    /// Only a large clock on black.
    Clock,
}

/// Sleep after a long time without use: the screen goes dark and the app closes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Sleep {
    pub enabled: bool,
    /// Minutes without remote input and without sound.
    pub after_minutes: f32,
    /// Close the open app when going to sleep, freeing its memory and CPU.
    pub close_apps: bool,
}

impl Default for Sleep {
    fn default() -> Self {
        Self { enabled: true, after_minutes: 60.0, close_apps: true }
    }
}

/// Screen casting receivers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Cast {
    /// AirPlay receiver (UxPlay) for iPhone, iPad and Mac. Needs `uxplay` and `avahi-daemon`.
    pub airplay: bool,
    /// Name shown in the sender's AirPlay / Screen Mirroring list.
    pub name: String,
    /// Extra UxPlay options, for example `["-vd", "avdec_h264"]` to force software decoding.
    pub extra_args: Vec<String>,
}

impl Default for Cast {
    fn default() -> Self {
        Self { airplay: false, name: "rustv".into(), extra_args: Vec::new() }
    }
}

/// Debug overlay: a line of live stats drawn over everything on the TV.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Overlay {
    /// Shown at start. The remote and the API can switch it at any time.
    pub enabled: bool,
    pub position: OverlayPosition,
    /// Pango font description, such as "DejaVu Sans Mono 14".
    pub font: String,
    /// How often the numbers refresh, in milliseconds.
    pub interval_ms: u64,
    /// What to show, in this order.
    pub items: Vec<OverlayItem>,
}

impl Default for Overlay {
    fn default() -> Self {
        use OverlayItem::*;
        Self {
            enabled: false,
            position: OverlayPosition::Top,
            font: "DejaVu Sans Mono 14".into(),
            interval_ms: 1000,
            items: vec![App, Cpu, Gpu, Ram, Net, Wifi, Display, Volume, Remotes, Uptime],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayItem {
    /// The app on screen.
    App,
    /// Load and temperature.
    Cpu,
    Gpu,
    Ram,
    /// Download and upload speed on the default route.
    Net,
    /// Wi-Fi signal strength.
    Wifi,
    /// Output resolution and refresh rate.
    Display,
    Volume,
    /// Connected remotes and other API clients.
    Remotes,
    Uptime,
    Clock,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Server {
    /// Address the API and web remote listen on.
    pub bind: String,
    /// Where the pairing token is stored. Created on first start.
    pub token_file: Option<PathBuf>,
}

impl Default for Server {
    fn default() -> Self {
        Self { bind: "0.0.0.0:8765".into(), token_file: None }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Browser {
    /// Command used to open `web` apps. `{url}` is replaced with the app URL.
    pub command: Vec<String>,
}

impl Default for Browser {
    fn default() -> Self {
        Self { command: ["flatpak", "run", "org.mozilla.firefox", "--kiosk", "{url}"].map(String::from).to_vec() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Audio {
    /// Volume change per step, 0.05 = 5%.
    pub step: f32,
    /// Upper limit for volume, 1.0 = 100%.
    pub max_volume: f32,
}

impl Default for Audio {
    fn default() -> Self {
        Self { step: 0.05, max_volume: 1.0 }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Icon file (SVG, PNG or JPEG). Flatpak apps fall back to their own icon.
    #[serde(default)]
    pub icon: Option<PathBuf>,
    /// Extra environment variables for this app.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(flatten)]
    pub kind: AppKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AppKind {
    Flatpak {
        app_id: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Command {
        command: Vec<String>,
    },
    Web {
        url: String,
    },
}

impl App {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            AppKind::Flatpak { .. } => "flatpak",
            AppKind::Command { .. } => "command",
            AppKind::Web { .. } => "web",
        }
    }

    /// Full command line used to start the app.
    pub fn argv(&self, browser: &Browser) -> Vec<String> {
        match &self.kind {
            AppKind::Flatpak { app_id, args } => {
                ["flatpak", "run", app_id.as_str()].into_iter().map(String::from).chain(args.iter().cloned()).collect()
            }
            AppKind::Command { command } => command.clone(),
            AppKind::Web { url } => browser.command.iter().map(|part| part.replace("{url}", url)).collect(),
        }
    }

    /// The configured icon, or the icon a Flatpak app exports.
    pub fn icon_path(&self) -> Option<PathBuf> {
        if let Some(icon) = &self.icon {
            return expand_home(icon).filter(|path| path.is_file());
        }
        let AppKind::Flatpak { app_id, .. } = &self.kind else { return None };
        let mut roots = vec![PathBuf::from("/var/lib/flatpak/exports/share/icons/hicolor")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.insert(0, PathBuf::from(home).join(".local/share/flatpak/exports/share/icons/hicolor"));
        }
        let candidates = [
            format!("scalable/apps/{app_id}.svg"),
            format!("512x512/apps/{app_id}.png"),
            format!("256x256/apps/{app_id}.png"),
            format!("128x128/apps/{app_id}.png"),
        ];
        roots.iter().flat_map(|root| candidates.iter().map(move |rel| root.join(rel))).find(|path| path.is_file())
    }
}

fn expand_home(path: &Path) -> Option<PathBuf> {
    match path.strip_prefix("~") {
        Ok(rest) => Some(PathBuf::from(std::env::var_os("HOME")?).join(rest)),
        Err(_) => Some(path.to_path_buf()),
    }
}

/// Flatpak application ID in a `flatpak run [options] <id>` command line.
pub fn flatpak_id(argv: &[String]) -> Option<String> {
    let program = Path::new(argv.first()?).file_name()?.to_str()?;
    if program != "flatpak" {
        return None;
    }
    let mut rest = argv.iter().skip(1).skip_while(|arg| arg.as_str() != "run").skip(1);
    rest.find(|arg| !arg.starts_with('-')).cloned()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| {
            format!("reading {} (copy config/config.example.toml there to get started)", path.display())
        })?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for app in &self.apps {
            let valid_id = !app.id.is_empty()
                && app.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
            if !valid_id {
                bail!("app id {:?}: use lowercase letters, digits, '-' or '_'", app.id);
            }
            if !seen.insert(app.id.as_str()) {
                bail!("duplicate app id {:?}", app.id);
            }
            if app.argv(&self.browser).is_empty() {
                bail!("app {:?} has an empty command", app.id);
            }
        }
        if !self.browser.command.iter().any(|part| part.contains("{url}")) {
            bail!("browser.command must contain {{url}}");
        }
        if !(0.0..=1.0).contains(&self.audio.step) || self.audio.max_volume <= 0.0 {
            bail!("audio.step must be within 0..1 and audio.max_volume above 0");
        }
        // The font ends up in a sway command, where these characters have meaning.
        if self.overlay.font.trim().is_empty() || self.overlay.font.contains([';', ',', '"', '\'', '\n']) {
            bail!("overlay.font must be a plain Pango font description, such as \"DejaVu Sans Mono 14\"");
        }
        if !(0.1..=1440.0).contains(&self.screensaver.idle_minutes) {
            bail!("screensaver.idle_minutes must be between 0.1 and 1440");
        }
        if !(0.0..=1.0).contains(&self.shell.sound_volume) {
            bail!("shell.sound_volume must be within 0..1");
        }
        if self.cast.name.trim().is_empty() {
            bail!("cast.name must not be empty");
        }
        if !(1.0..=1440.0).contains(&self.sleep.after_minutes) {
            bail!("sleep.after_minutes must be between 1 and 1440");
        }
        if self.overlay.interval_ms < 250 {
            bail!("overlay.interval_ms must be at least 250");
        }
        Ok(())
    }
}

pub fn load_or_create_token(cfg: &Config) -> Result<String> {
    let path = cfg.server.token_file.clone().unwrap_or_else(|| config_dir().join("token"));
    if path.exists() {
        let token = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let token = token.trim().to_string();
        if token.len() < 16 || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
            bail!("{}: token must be at least 16 letters or digits", path.display());
        }
        return Ok(token);
    }

    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).context("generating token")?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("creating {}", path.display()))?;
    writeln!(file, "{token}")?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_example_config() {
        let cfg: Config = toml::from_str(include_str!("../../../config/config.example.toml")).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.apps.is_empty());
    }

    #[test]
    fn web_app_uses_browser_command() {
        let cfg: Config =
            toml::from_str("[[app]]\nid = \"q\"\nname = \"Q\"\nkind = \"web\"\nurl = \"http://x:8080\"\n").unwrap();
        let argv = cfg.apps[0].argv(&cfg.browser);
        assert_eq!(argv.last().unwrap(), "http://x:8080");
        assert_eq!(flatpak_id(&argv).as_deref(), Some("org.mozilla.firefox"));
    }

    #[test]
    fn finds_flatpak_id() {
        assert_eq!(flatpak_id(&argv(&["flatpak", "run", "--branch=stable", "a.b.C", "--x"])).as_deref(), Some("a.b.C"));
        assert_eq!(flatpak_id(&argv(&["/usr/bin/flatpak", "run", "a.b.C"])).as_deref(), Some("a.b.C"));
        assert_eq!(flatpak_id(&argv(&["mpv", "run", "x"])), None);
    }

    #[test]
    fn rejects_duplicate_ids() {
        let text = "[[app]]\nid = \"a\"\nname = \"A\"\nkind = \"command\"\ncommand = [\"true\"]\n".repeat(2);
        let cfg: Config = toml::from_str(&text).unwrap();
        assert!(cfg.validate().is_err());
    }
}
