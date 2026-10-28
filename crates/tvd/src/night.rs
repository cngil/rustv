//! Night mode: from `night.from` to `night.to` the screen is dimmer and warmer,
//! eased in and out over half an hour so nobody sees it jump. Outside those
//! hours, `display.brightness` and `display.warmth` (the all-day night light)
//! apply. Both end up in sway's gamma tables through the dimmer.

use crate::config::Config;

/// How long night mode takes to come on after `from`, and to go after `to`.
const EASE_MINUTES: f32 = 30.0;

/// `HH:MM` as minutes after midnight.
pub fn parse(time: &str) -> Option<u32> {
    let (h, m) = time.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// The minutes after local midnight now, in the box's time zone.
pub fn local_minutes() -> u32 {
    // SAFETY: time with a null pointer and localtime_r with valid pointers have no other preconditions.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 12 * 60;
        }
        (tm.tm_hour * 60 + tm.tm_min) as u32
    }
}

/// How much of night mode applies at `minute` (after midnight): 0 by day, 1 at
/// night, in between while it eases in after `from` and out after `to`.
pub fn amount(minute: u32, from: u32, to: u32) -> f32 {
    const DAY: u32 = 24 * 60;
    if from == to {
        return 0.0;
    }
    // Minutes since `from`, and how long the night is, both across midnight.
    let since = (minute + DAY - from) % DAY;
    let length = (to + DAY - from) % DAY;
    if since < length {
        (since as f32 / EASE_MINUTES).min(1.0)
    } else {
        // After `to`: easing out, unless the next night is already close.
        let after = (since - length) as f32;
        (1.0 - after / EASE_MINUTES).max(0.0)
    }
}

/// Whether night mode is on at `minute`, easing included.
pub fn active(cfg: &Config, minute: u32) -> bool {
    match (cfg.night.enabled, parse(&cfg.night.from), parse(&cfg.night.to)) {
        (true, Some(from), Some(to)) => amount(minute, from, to) > 0.0,
        _ => false,
    }
}

/// The brightness and warmth the screen should have at `minute`.
pub fn screen(cfg: &Config, minute: u32) -> (f32, f32) {
    let day = (cfg.display.brightness, cfg.display.warmth);
    if !cfg.night.enabled {
        return day;
    }
    let (Some(from), Some(to)) = (parse(&cfg.night.from), parse(&cfg.night.to)) else { return day };
    let t = amount(minute, from, to);
    // Night mode never makes the screen brighter or cooler than by day.
    let night = (cfg.night.brightness.min(day.0), cfg.night.warmth.max(day.1));
    (day.0 + (night.0 - day.0) * t, day.1 + (night.1 - day.1) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(h: u32, m: u32) -> u32 {
        h * 60 + m
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse("22:00"), Some(hm(22, 0)));
        assert_eq!(parse("7:30"), Some(hm(7, 30)));
        assert_eq!(parse("00:00"), Some(0));
        for bad in ["24:00", "12:60", "12", "12:5", ":30", "ab:cd", "123:00"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn eases_in_after_from_and_out_after_to_across_midnight() {
        let (from, to) = (hm(22, 0), hm(7, 0));
        assert_eq!(amount(hm(21, 59), from, to), 0.0);
        assert_eq!(amount(hm(22, 0), from, to), 0.0);
        assert_eq!(amount(hm(22, 15), from, to), 0.5);
        assert_eq!(amount(hm(22, 30), from, to), 1.0);
        assert_eq!(amount(hm(3, 0), from, to), 1.0);
        assert_eq!(amount(hm(6, 59), from, to), 1.0);
        assert_eq!(amount(hm(7, 15), from, to), 0.5);
        assert_eq!(amount(hm(7, 30), from, to), 0.0);
        assert_eq!(amount(hm(12, 0), from, to), 0.0);
    }

    #[test]
    fn a_night_within_one_day_and_an_empty_one() {
        assert_eq!(amount(hm(14, 0), hm(13, 0), hm(15, 0)), 1.0);
        assert_eq!(amount(hm(16, 0), hm(13, 0), hm(15, 0)), 0.0);
        assert_eq!(amount(hm(14, 0), hm(13, 0), hm(13, 0)), 0.0);
    }

    #[test]
    fn night_only_dims_and_warms() {
        let mut cfg = Config::parse("").unwrap();
        cfg.display.brightness = 0.5;
        cfg.display.warmth = 0.2;
        cfg.night.enabled = true;
        cfg.night.brightness = 0.8;
        cfg.night.warmth = 0.6;
        assert_eq!(screen(&cfg, hm(12, 0)), (0.5, 0.2));
        // Night brightness above the day's leaves the day's.
        assert_eq!(screen(&cfg, hm(1, 0)), (0.5, 0.6));
        cfg.night.enabled = false;
        assert_eq!(screen(&cfg, hm(1, 0)), (0.5, 0.2));
    }
}
