//! Navigation and selection sounds.
//!
//! The sounds are synthesized at startup (short sine blips with a soft
//! envelope) and written to the runtime directory, so the repository ships no
//! audio files. They play through PipeWire with `pw-play`, one short process
//! per sound.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;
/// Holding an arrow repeats every ~110 ms; closer than this, sounds would smear.
const MIN_GAP: Duration = Duration::from_millis(45);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sound {
    /// Focus moved to another card.
    Move,
    /// An app was opened.
    Select,
    /// Nothing there: already at the first or last card.
    Edge,
}

impl Sound {
    fn file(self) -> &'static str {
        match self {
            Sound::Move => "move.wav",
            Sound::Select => "select.wav",
            Sound::Edge => "edge.wav",
        }
    }

    /// (frequency in Hz, duration in ms, loudness 0..1) for each tone, played back to back.
    fn tones(self) -> &'static [(f32, u32, f32)] {
        match self {
            Sound::Move => &[(1320.0, 26, 0.35)],
            Sound::Select => &[(880.0, 45, 0.45), (1320.0, 80, 0.45)],
            Sound::Edge => &[(330.0, 60, 0.4)],
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "move" => Some(Sound::Move),
            "select" => Some(Sound::Select),
            "edge" => Some(Sound::Edge),
            _ => None,
        }
    }
}

pub struct Player {
    dir: PathBuf,
    enabled: bool,
    volume: f32,
    last: Option<Instant>,
}

thread_local! {
    pub static PLAYER: RefCell<Player> = RefCell::new(Player::new());
}

impl Player {
    fn new() -> Self {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("rustv-shell");
        let ready = std::fs::create_dir_all(&dir).is_ok()
            && [Sound::Move, Sound::Select, Sound::Edge]
                .iter()
                .all(|s| write_wav(&dir.join(s.file()), s.tones()).is_ok());
        if !ready {
            eprintln!("rustv-shell: could not prepare UI sounds in {}", dir.display());
        }
        Self { dir, enabled: ready, volume: 0.5, last: None }
    }

    pub fn configure(&mut self, enabled: bool, volume: f32) {
        self.enabled = enabled;
        self.volume = volume.clamp(0.0, 1.0);
    }

    pub fn play_named(&mut self, name: &str) {
        if let Some(sound) = Sound::from_name(name) {
            self.play(sound);
        }
    }

    pub fn play(&mut self, sound: Sound) {
        if !self.enabled || self.volume <= 0.0 {
            return;
        }
        let now = Instant::now();
        if sound == Sound::Move && self.last.is_some_and(|last| now - last < MIN_GAP) {
            return;
        }
        self.last = Some(now);
        let child = Command::new("pw-play")
            .arg("--volume")
            .arg(format!("{:.2}", self.volume))
            .arg(self.dir.join(sound.file()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            // Reap it when it ends, so finished players never linger as zombies.
            std::thread::spawn(move || child.wait());
        }
    }
}

/// 16-bit mono WAV of the given tones, each with a 4 ms fade in and an exponential fade out.
fn write_wav(path: &Path, tones: &[(f32, u32, f32)]) -> std::io::Result<()> {
    let samples = synthesize(tones);
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::File::create(path)?.write_all(&out)
}

fn synthesize(tones: &[(f32, u32, f32)]) -> Vec<i16> {
    let mut samples = Vec::new();
    for &(freq, ms, loudness) in tones {
        let n = (RATE * ms / 1000) as usize;
        let attack = (RATE as usize * 4 / 1000).max(1);
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let fade_in = (i as f32 / attack as f32).min(1.0);
            let fade_out = (-5.0 * i as f32 / n as f32).exp();
            let value = (2.0 * std::f32::consts::PI * freq * t).sin() * loudness * fade_in * fade_out;
            samples.push((value * i16::MAX as f32) as i16);
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesizes_short_quiet_tones() {
        let samples = synthesize(Sound::Select.tones());
        assert_eq!(samples.len(), (RATE * 125 / 1000) as usize);
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 1000 && peak < (i16::MAX as f32 * 0.5) as u16, "peak {peak}");
        // Starts silent, so there is no click.
        assert!(samples[0].unsigned_abs() < 100);
    }

    #[test]
    fn writes_a_valid_wav_header() {
        let path = std::env::temp_dir().join(format!("rustv-sound-{}.wav", std::process::id()));
        write_wav(&path, Sound::Move.tones()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + (RATE * 26 / 1000) as usize * 2);
        std::fs::remove_file(path).unwrap();
    }
}
