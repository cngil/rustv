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

/// Every piece of text the shell shows.
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
    greetings: [&'static str; 3],
    days: [&'static str; 7],
    months: [&'static str; 12],
    am_pm: [&'static str; 2],
}

const EN: Strings = Strings {
    subtitle: "What would you like to watch?",
    no_apps: "No apps yet. Add some to ~/.config/rustv/config.toml",
    hint_move: "Move",
    hint_open: "Open",
    opening: "Opening…",
    connecting: "Connecting to tvd…",
    volume: "Volume",
    muted: "Muted",
    pair_title: "Use your phone as the remote",
    pair_body: "Scan the code with the camera, then add the page to your home screen.",
    locked: "Locked",
    unlock_hint: "Enter the PIN on your phone to unlock.",
    greetings: ["Good morning", "Good afternoon", "Good evening"],
    days: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    months: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    am_pm: ["AM", "PM"],
};

const TR: Strings = Strings {
    subtitle: "Ne izlemek istersin?",
    no_apps: "Henüz uygulama yok. ~/.config/rustv/config.toml dosyasına ekleyebilirsin",
    hint_move: "Gez",
    hint_open: "Aç",
    opening: "Açılıyor…",
    connecting: "tvd'ye bağlanılıyor…",
    volume: "Ses",
    muted: "Sessiz",
    pair_title: "Telefonunu kumanda yap",
    pair_body: "Kodu kamerayla tara, sonra sayfayı ana ekranına ekle.",
    locked: "Kilitli",
    unlock_hint: "Kilidi açmak için PIN'i telefonundan gir.",
    greetings: ["Günaydın", "İyi günler", "İyi akşamlar"],
    days: ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"],
    months: [
        "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık",
    ],
    am_pm: ["ÖÖ", "ÖS"],
};

impl Locale {
    pub fn strings(&self) -> &'static Strings {
        match self.language.as_str() {
            "tr" => &TR,
            _ => &EN,
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

    /// "Monday, 26 October" / "Pazartesi, 26 Ekim".
    pub fn date(&self, now: &DateTime<Local>) -> String {
        let s = self.strings();
        let day = s.days[now.weekday().num_days_from_monday() as usize];
        format!("{day}, {} {}", now.day(), s.months[now.month0() as usize])
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
        assert_eq!(xx.strings().subtitle, EN.subtitle);
    }
}
