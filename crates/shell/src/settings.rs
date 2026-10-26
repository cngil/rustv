//! Where to find tvd. Reads only the `[server]` table of the shared rustv
//! config, so the shell needs no settings of its own.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Settings {
    /// `host:port` of tvd's API.
    pub addr: String,
    pub token_file: PathBuf,
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    server: Server,
}

#[derive(Debug, Default, Deserialize)]
struct Server {
    bind: Option<String>,
    token_file: Option<PathBuf>,
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/etc"))
        .join("rustv")
}

impl Settings {
    pub fn load() -> Result<Self> {
        let dir = config_dir();
        let path = dir.join("config.toml");
        let file: File = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?,
            Err(_) => File::default(),
        };
        Ok(Self {
            addr: local_addr(file.server.bind.as_deref().unwrap_or("0.0.0.0:8765")),
            token_file: file.server.token_file.unwrap_or_else(|| dir.join("token")),
        })
    }

    pub fn token(&self) -> Result<String> {
        let token = std::fs::read_to_string(&self.token_file)
            .with_context(|| format!("reading {} (is tvd running?)", self.token_file.display()))?;
        Ok(token.trim().to_string())
    }
}

/// A wildcard bind address is reached through loopback.
fn local_addr(bind: &str) -> String {
    match bind.rsplit_once(':') {
        Some(("0.0.0.0" | "[::]" | "::", port)) => format!("127.0.0.1:{port}"),
        _ => bind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_binds_use_loopback() {
        assert_eq!(local_addr("0.0.0.0:8765"), "127.0.0.1:8765");
        assert_eq!(local_addr("[::]:9000"), "127.0.0.1:9000");
        assert_eq!(local_addr("192.168.1.5:8765"), "192.168.1.5:8765");
    }

    #[test]
    fn ignores_other_config_sections() {
        let file: File = toml::from_str("[server]\nbind = \"0.0.0.0:1\"\n[[app]]\nid = \"x\"\n").unwrap();
        assert_eq!(file.server.bind.as_deref(), Some("0.0.0.0:1"));
    }
}
