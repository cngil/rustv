//! Configuration file (`config.toml`) and the pairing token.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

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
    #[serde(default, rename = "app")]
    pub apps: Vec<App>,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Audio {
    /// Volume change per step, 0.05 = 5%.
    pub step: f32,
    /// Upper limit for volume, 1.0 = 100%.
    pub max_volume: f32,
}

impl Default for Audio {
    fn default() -> Self {
        Self { step: 0.05, max_volume: 1.5 }
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
        let cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
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
