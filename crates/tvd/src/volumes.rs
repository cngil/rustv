//! Remembers the volume for each app and for the home screen, so a loud game
//! and a quiet film each come back at their own level.
//!
//! Kept in `~/.local/state/rustv/volumes`, one `id level` per line.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub const HOME: &str = "home";

pub struct Volumes {
    path: Option<PathBuf>,
    levels: BTreeMap<String, f32>,
    /// The app on screen ("home" for the shell).
    pub current: String,
}

impl Volumes {
    pub fn load() -> Self {
        let path = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .map(|dir| dir.join("rustv/volumes"));
        let text = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        Self { path, levels: parse(&text), current: HOME.into() }
    }

    pub fn get(&self, id: &str) -> Option<f32> {
        self.levels.get(id).copied()
    }

    pub fn set(&mut self, id: &str, level: f32) {
        if self.levels.get(id).is_some_and(|old| (old - level).abs() < 0.001) {
            return;
        }
        self.levels.insert(id.to_string(), level);
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let text: String = self.levels.iter().map(|(id, level)| format!("{id} {level:.2}\n")).collect();
            let _ = std::fs::write(path, text);
        }
    }
}

fn parse(text: &str) -> BTreeMap<String, f32> {
    text.lines()
        .filter_map(|line| {
            let (id, level) = line.split_once(' ')?;
            Some((id.to_string(), level.trim().parse().ok()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_saved_levels() {
        let levels = parse("home 0.40\njellyfin 0.65\nbroken\nkapman x\n");
        assert_eq!(levels.len(), 2);
        assert_eq!(levels["jellyfin"], 0.65);
    }

    #[test]
    fn remembers_in_memory_without_a_file() {
        let mut v = Volumes { path: None, levels: BTreeMap::new(), current: HOME.into() };
        v.set("moonlight", 0.8);
        assert_eq!(v.get("moonlight"), Some(0.8));
        assert_eq!(v.get("jellyfin"), None);
    }
}
