//! Your own pictures, behind the home screen and as a screensaver style.
//!
//! The config lists files and folders; the shell asks for pictures by number
//! (`/api/v1/wallpapers/<n>`), so only those listed are ever served.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Wallpaper;

/// What the shell and the remote get: the settings and how many pictures
/// there are, but not where they live.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub home: bool,
    pub change_minutes: f32,
    pub slide_seconds: f32,
    pub shuffle: bool,
    pub count: usize,
}

pub fn info(cfg: &Wallpaper) -> Info {
    Info {
        home: cfg.home,
        change_minutes: cfg.change_minutes,
        slide_seconds: cfg.slide_seconds,
        shuffle: cfg.shuffle,
        count: images(cfg).len(),
    }
}

/// The listed pictures, folders opened (not their subfolders), in a stable order.
pub fn images(cfg: &Wallpaper) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in &cfg.images {
        let Some(path) = crate::config::expand_home(entry) else { continue };
        if path.is_dir() {
            let mut inside: Vec<PathBuf> = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| content_type(path).is_some())
                .collect();
            inside.sort();
            found.extend(inside);
        } else if content_type(&path).is_some() && path.is_file() {
            found.push(path);
        }
    }
    found
}

/// The pictures the shell can show: JPEG and PNG.
pub fn content_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_and_folders() {
        let dir = std::env::temp_dir().join(format!("tvd-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("folder/deeper")).unwrap();
        for name in ["folder/b.JPG", "folder/a.png", "folder/notes.txt", "folder/deeper/c.jpg", "single.jpeg"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let cfg = Wallpaper {
            images: vec![dir.join("single.jpeg"), dir.join("folder"), dir.join("missing.jpg")],
            ..Default::default()
        };
        let names: Vec<_> = images(&cfg).iter().map(|p| p.strip_prefix(&dir).unwrap().to_path_buf()).collect();
        assert_eq!(names, [PathBuf::from("single.jpeg"), "folder/a.png".into(), "folder/b.JPG".into()]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
