//! Words and formats for the home screen, from the `[locale]` section of the config.
//!
//! Day and month names are written out here instead of pulling in a locale
//! database; that keeps the shell small and adding a language is one table.

use chrono::{DateTime, Datelike, Local, Timelike};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Locale {
    /// "en" or "tr".
    pub language: String,
    /// "24h" or "12h".
    pub clock: String,
}

impl Default for Locale {
    fn default() -> Self {
        Self { language: "en".into(), clock: "24h".into() }
    }
}

/// Every piece of text the shell shows, from the shared catalog (`crates/locales`).
pub struct Strings {
    pub subtitle: &'static str,
    pub no_apps: &'static str,
    pub hint_move: &'static str,
    pub hint_open: &'static str,
    pub opening: &'static str,
    pub connecting: &'static str,
    pub volume: &'static str,
    pub muted: &'static str,
    pub pair_title: &'static str,
    pub pair_body: &'static str,
    pub locked: &'static str,
    pub unlock_hint: &'static str,
    pub settings: &'static str,
    pub hint_change: &'static str,
    pub hint_select: &'static str,
    pub hint_close: &'static str,
    pub back_key: &'static str,
    pub trial_title: &'static str,
    pub keep: &'static str,
    pub go_back: &'static str,
    greetings: [&'static str; 3],
    days: [&'static str; 7],
    months: [&'static str; 12],
    am_pm: [&'static str; 2],
}

impl Strings {
    fn new(lang: &str) -> Self {
        let text = |key: &'static str| rustv_locales::text(lang, key);
        Self {
            subtitle: text("tv.subtitle"),
            no_apps: text("tv.no_apps"),
            hint_move: text("tv.hint.move"),
            hint_open: text("tv.hint.open"),
            opening: text("tv.opening"),
            connecting: text("tv.connecting"),
            volume: text("tv.volume"),
            muted: text("tv.muted"),
            pair_title: text("tv.pair.title"),
            pair_body: text("tv.pair.body"),
            locked: text("tv.locked"),
            unlock_hint: text("tv.unlock_hint"),
            settings: text("common.settings"),
            hint_change: text("tv.hint.change"),
            hint_select: text("tv.hint.select"),
            hint_close: text("common.close"),
            back_key: text("common.back"),
            trial_title: text("remote.display.keep_title"),
            keep: text("common.keep"),
            go_back: text("common.go_back"),
            greetings: [text("tv.greeting.morning"), text("tv.greeting.afternoon"), text("tv.greeting.evening")],
            days: [
                text("tv.day.monday"),
                text("tv.day.tuesday"),
                text("tv.day.wednesday"),
                text("tv.day.thursday"),
                text("tv.day.friday"),
                text("tv.day.saturday"),
                text("tv.day.sunday"),
            ],
            months: [
                text("tv.month.january"),
                text("tv.month.february"),
                text("tv.month.march"),
                text("tv.month.april"),
                text("tv.month.may"),
                text("tv.month.june"),
                text("tv.month.july"),
                text("tv.month.august"),
                text("tv.month.september"),
                text("tv.month.october"),
                text("tv.month.november"),
                text("tv.month.december"),
            ],
            am_pm: [text("tv.am"), text("tv.pm")],
        }
    }
}

impl Locale {
    pub fn strings(&self) -> Strings {
        Strings::new(self.lang())
    }

    /// The catalog's language code; anything it does not know reads as English.
    pub fn lang(&self) -> &str {
        if rustv_locales::LANGUAGES.contains(&self.language.as_str()) {
            &self.language
        } else {
            "en"
        }
    }

    pub fn time(&self, now: &DateTime<Local>) -> String {
        if self.clock == "12h" {
            let (pm, hour) = now.hour12();
            format!("{hour}:{:02} {}", now.minute(), self.strings().am_pm[pm as usize])
        } else {
            format!("{:02}:{:02}", now.hour(), now.minute())
        }
    }

    /// "Monday, 26 October" / "Pazartesi, 26 Ekim", in the order the catalog gives.
    pub fn date(&self, now: &DateTime<Local>) -> String {
        let s = self.strings();
        let weekday = s.days[now.weekday().num_days_from_monday() as usize];
        let month = s.months[now.month0() as usize];
        rustv_locales::format(
            self.lang(),
            "tv.date",
            &[("weekday", weekday), ("day", &now.day().to_string()), ("month", month)],
        )
    }

    /// "Going back in 12 s" while a display mode is on trial.
    pub fn trial_countdown(&self, seconds: u64) -> String {
        rustv_locales::format(self.lang(), "tv.trial.countdown", &[("seconds", &seconds.to_string())])
    }

    pub fn greeting(&self, now: &DateTime<Local>) -> &'static str {
        let index = match now.hour() {
            5..=11 => 0,
            12..=17 => 1,
            _ => 2,
        };
        self.strings().greetings[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 26, h, m, 0).unwrap()
    }

    #[test]
    fn formats_in_each_language() {
        let en = Locale::default();
        let tr = Locale { language: "tr".into(), clock: "24h".into() };
        assert_eq!(en.date(&at(9, 5)), "Monday, 26 October");
        assert_eq!(tr.date(&at(9, 5)), "Pazartesi, 26 Ekim");
        assert_eq!(tr.greeting(&at(20, 0)), "İyi akşamlar");
        assert_eq!(en.time(&at(9, 5)), "09:05");
    }

    #[test]
    fn twelve_hour_clock() {
        let en = Locale { language: "en".into(), clock: "12h".into() };
        assert_eq!(en.time(&at(21, 7)), "9:07 PM");
        assert_eq!(en.time(&at(0, 30)), "12:30 AM");
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        let xx = Locale { language: "xx".into(), clock: "24h".into() };
        assert_eq!(xx.strings().subtitle, Locale::default().strings().subtitle);
    }
}
