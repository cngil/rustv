//! A household lock: with a PIN set, the TV can be locked so nothing works
//! until the PIN is entered on a remote. Meant for children and guests in the
//! room, not for network security; the pairing token protects the API.
//!
//! The PIN is kept as a salted, stretched hash in `~/.local/state/rustv/lock`
//! (mode 600), never in the config file. Five wrong tries block entry for a
//! minute, which matters more for a short PIN than the hash does.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

const ROUNDS: u32 = 200_000;
const MAX_TRIES: u32 = 5;
const BLOCKED_FOR: Duration = Duration::from_secs(60);

pub struct Lock {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

struct Inner {
    /// (salt, hash) of the PIN.
    pin: Option<(u64, u64)>,
    locked: bool,
    failures: u32,
    blocked_until: Option<Instant>,
}

impl Lock {
    pub fn load() -> Self {
        let path = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .map(|dir| dir.join("rustv/lock"));
        let pin = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|text| parse(&text));
        // A TV that was locked when tvd stopped stays locked.
        let locked = pin.is_some()
            && path.as_ref().is_some_and(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains("locked")));
        Self { path, inner: Mutex::new(Inner { pin, locked, failures: 0, blocked_until: None }) }
    }

    pub fn is_locked(&self) -> bool {
        self.inner.lock().unwrap().locked
    }

    pub fn has_pin(&self) -> bool {
        self.inner.lock().unwrap().pin.is_some()
    }

    pub fn lock(&self) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.pin.is_none() {
            return Err(crate::i18n::error("error.pin.not_set", &[]));
        }
        inner.locked = true;
        self.save(&inner)
    }

    pub fn unlock(&self, pin: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        check(&mut inner, pin)?;
        inner.locked = false;
        self.save(&inner)
    }

    /// Sets, changes or (with `new: None`) removes the PIN. Changing or removing
    /// an existing PIN needs the current one.
    pub fn set_pin(&self, current: Option<&str>, new: Option<&str>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.pin.is_some() {
            check(&mut inner, current.unwrap_or_default())?;
        }
        match new {
            Some(pin) => {
                if !(4..=8).contains(&pin.len()) || !pin.chars().all(|c| c.is_ascii_digit()) {
                    return Err(crate::i18n::error("error.pin.format", &[]));
                }
                let salt = salt()?;
                inner.pin = Some((salt, stretch(salt, pin)));
            }
            None => {
                inner.pin = None;
                inner.locked = false;
            }
        }
        self.save(&inner)
    }

    fn save(&self, inner: &Inner) -> Result<()> {
        let path = self.path.as_ref().context("no state directory for the lock")?;
        let Some((salt, hash)) = inner.pin else {
            let _ = std::fs::remove_file(path);
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        writeln!(file, "{salt:016x}:{hash:016x}{}", if inner.locked { " locked" } else { "" })?;
        Ok(())
    }
}

fn check(inner: &mut Inner, pin: &str) -> Result<()> {
    if let Some(until) = inner.blocked_until {
        let left = until.saturating_duration_since(Instant::now());
        if !left.is_zero() {
            return Err(crate::i18n::error("error.pin.too_many", &[("seconds", left.as_secs().max(1).to_string())]));
        }
        inner.blocked_until = None;
    }
    let Some((salt, hash)) = inner.pin else { return Ok(()) };
    if constant_time_eq(stretch(salt, pin), hash) {
        inner.failures = 0;
        return Ok(());
    }
    inner.failures += 1;
    if inner.failures >= MAX_TRIES {
        inner.failures = 0;
        inner.blocked_until = Some(Instant::now() + BLOCKED_FOR);
        return Err(crate::i18n::error("error.pin.wrong_blocked", &[("seconds", BLOCKED_FOR.as_secs().to_string())]));
    }
    Err(crate::i18n::error("error.pin.wrong", &[]))
}

fn parse(text: &str) -> Option<(u64, u64)> {
    let (salt, rest) = text.trim().split_once(':')?;
    let hash = rest.split_whitespace().next()?;
    Some((u64::from_str_radix(salt, 16).ok()?, u64::from_str_radix(hash, 16).ok()?))
}

fn salt() -> Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).context("generating a salt")?;
    Ok(u64::from_le_bytes(bytes))
}

/// FNV-1a over salt and PIN, repeated. Not a password hash for the internet;
/// enough that the file does not show the PIN at a glance, while the try limit
/// does the real work.
fn stretch(salt: u64, pin: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ salt;
    for round in 0..ROUNDS {
        for byte in salt.to_le_bytes().iter().chain(pin.as_bytes()).chain(&round.to_le_bytes()) {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

fn constant_time_eq(a: u64, b: u64) -> bool {
    (a ^ b) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> Lock {
        Lock { path: None, inner: Mutex::new(Inner { pin: None, locked: false, failures: 0, blocked_until: None }) }
    }

    #[test]
    fn locks_only_with_a_pin_and_unlocks_with_the_right_one() {
        let lock = lock();
        assert!(lock.lock().is_err());
        // Saving fails without a state directory, but the PIN is set in memory.
        let _ = lock.set_pin(None, Some("1234"));
        assert!(lock.has_pin());
        let _ = lock.lock();
        assert!(lock.is_locked());
        assert!(lock.unlock("0000").is_err());
        let _ = lock.unlock("1234");
        assert!(!lock.is_locked());
    }

    #[test]
    fn changing_the_pin_needs_the_current_one() {
        let lock = lock();
        let _ = lock.set_pin(None, Some("1234"));
        assert!(lock.set_pin(Some("9999"), Some("5678")).is_err());
        let _ = lock.set_pin(Some("1234"), None);
        assert!(!lock.has_pin());
        assert!(lock.set_pin(None, Some("12")).is_err());
        assert!(lock.set_pin(None, Some("12ab")).is_err());
    }

    #[test]
    fn blocks_after_too_many_wrong_pins() {
        let lock = lock();
        let _ = lock.set_pin(None, Some("1234"));
        for _ in 0..MAX_TRIES {
            assert!(lock.unlock("0000").is_err());
        }
        // Even the right PIN is refused while blocked.
        let err = lock.unlock("1234").unwrap_err().to_string();
        assert!(err.contains("Too many"), "{err}");
    }

    #[test]
    fn reads_the_stored_pin() {
        assert_eq!(parse("00000000000000ff:0000000000000001 locked\n"), Some((255, 1)));
        assert_eq!(parse("garbage"), None);
        assert_ne!(stretch(1, "1234"), stretch(2, "1234"));
    }
}
