//! Controller-operated text entry. Secrets never appear in rendered text.
use serde_json::{json, Value};

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub struct Dialog {
    pub id: u64,
    pub mode: String,
    pub target: String,
    page: usize,
    pub selected: usize,
    pub text: String,
    current: String,
    first: String,
    step: u8,
    pub error: String,
    pub waiting: bool,
}
impl Dialog {
    pub fn new(mode: &str) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            mode: mode.into(),
            target: String::new(),
            page: 0,
            selected: 0,
            text: String::new(),
            current: String::new(),
            first: String::new(),
            step: 0,
            error: String::new(),
            waiting: false,
        }
    }
    pub fn prompt(&self) -> &'static str {
        match (self.mode.as_str(), self.step) {
            ("volume", _) => "tv.controls.volume_value",
            ("color", _) => "tv.controls.color_hint",
            ("wifi", _) => "tv.controls.wifi_password",
            ("unlock", _) => "remote.pin.enter",
            ("change" | "remove", 0) => "remote.pin.enter_current",
            ("set", 1) | ("change", 2) => "remote.pin.again",
            _ => "remote.pin.choose",
        }
    }
    pub fn keys(&self, lang: &str) -> Vec<String> {
        let t = |key| rustv_locales::text(lang, key).to_string();
        if matches!(self.mode.as_str(), "wifi" | "color") {
            let chars: Vec<char> = (32u8..=126).map(char::from).collect();
            let mut keys: Vec<String> =
                (0..36).map(|i| chars[(self.page * 36 + i) % chars.len()].to_string()).collect();
            keys.extend([
                t("remote.key.delete"),
                t("tv.controls.more_keys"),
                t("remote.pin.confirm"),
                t("common.cancel"),
            ]);
            keys
        } else {
            let mut keys: Vec<String> = (1..=9).map(|i| i.to_string()).collect();
            keys.extend([t("remote.key.delete"), "0".into(), t("remote.pin.confirm")]);
            keys
        }
    }
    pub fn key(&mut self, key: &str) -> Option<Value> {
        if self.waiting {
            return None;
        }
        if matches!(self.mode.as_str(), "wifi" | "color") {
            match key {
                "left" => self.selected = self.selected.saturating_sub(1),
                "right" => self.selected = (self.selected + 1).min(39),
                "up" => self.selected = self.selected.saturating_sub(10),
                "down" => self.selected = (self.selected + 10).min(39),
                "delete" => {
                    self.text.pop();
                }
                "ok" => match self.selected {
                    36 => {
                        self.text.pop();
                    }
                    37 => self.page = (self.page + 1) % 3,
                    38 => return self.submit(),
                    39 => return Some(json!({"type":"_cancel"})),
                    i => {
                        let c = char::from(32 + ((self.page * 36 + i) % 95) as u8);
                        if self.text.len() < 256 {
                            self.text.push(c);
                        }
                    }
                },
                "submit" => return self.submit(),
                key if key.chars().count() == 1 && !key.chars().any(char::is_control) && self.text.len() < 256 => {
                    self.text.push_str(key);
                    self.selected = 38;
                }
                _ => {}
            }
            return None;
        }
        match key {
            "left" => self.selected = self.selected.saturating_sub(1),
            "right" => self.selected = (self.selected + 1).min(11),
            "up" => self.selected = self.selected.saturating_sub(3),
            "down" => self.selected = (self.selected + 3).min(11),
            "delete" => {
                self.text.pop();
            }
            "ok" => match self.selected {
                9 => {
                    self.text.pop();
                }
                11 => return self.submit(),
                i => self.digit(if i == 10 { '0' } else { char::from(b'1' + i as u8) }),
            },
            "submit" => return self.submit(),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => self.digit(k.chars().next().unwrap()),
            _ => {}
        }
        None
    }
    fn digit(&mut self, digit: char) {
        if self.text.len() < 8 {
            self.text.push(digit);
        }
    }
    fn submit(&mut self) -> Option<Value> {
        self.error.clear();
        if self.mode == "volume" {
            let Ok(percent) = self.text.parse::<u16>() else {
                self.error = "tv.controls.volume_value".into();
                return None;
            };
            if percent > 150 {
                self.error = "tv.controls.volume_value".into();
                return None;
            }
            self.text.clear();
            self.waiting = true;
            return Some(json!({"type":"volume","action":"set","level":f64::from(percent)/100.0}));
        }
        if self.mode == "color" {
            let text = self.text.trim();
            if !text.is_empty()
                && !(text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit()))
            {
                self.error = "tv.controls.color_hint".into();
                return None;
            }
            let value = if text.is_empty() { Value::Null } else { json!(text) };
            self.text.clear();
            self.waiting = true;
            return Some(json!({"type":"set","key":self.target,"value":value}));
        }
        if self.mode == "wifi" {
            self.waiting = true;
            return Some(json!({"type":"wifi_connect","ssid":self.target,"password":std::mem::take(&mut self.text)}));
        }
        if self.text.len() < 4 {
            self.error = "remote.pin.too_short".into();
            return None;
        }
        let digits = std::mem::take(&mut self.text);
        let command = match (self.mode.as_str(), self.step) {
            ("unlock", _) => json!({"type":"unlock","pin":digits}),
            ("remove", _) => json!({"type":"set_pin","current":digits}),
            ("change", 0) => {
                self.current = digits;
                self.step = 1;
                return None;
            }
            ("set", 0) | ("change", 1) => {
                self.first = digits;
                self.step += 1;
                return None;
            }
            _ if digits != self.first => {
                self.first.clear();
                self.step = if self.mode == "set" { 0 } else { 1 };
                self.error = "remote.pin.mismatch".into();
                return None;
            }
            _ => {
                json!({"type":"set_pin","current":if self.current.is_empty() { None } else { Some(&self.current) },"pin":digits})
            }
        };
        self.current.clear();
        self.first.clear();
        self.waiting = true;
        Some(command)
    }
    pub fn failed(&mut self, message: String) {
        let target = self.target.clone();
        let mode = self.mode.clone();
        *self = Self::new(&mode);
        self.target = target;
        self.error = message;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn enter(d: &mut Dialog, pin: &str) -> Option<Value> {
        for c in pin.chars() {
            d.key(&c.to_string());
        }
        d.key("submit")
    }
    #[test]
    fn requires_confirmation_and_clears_secrets() {
        let mut d = Dialog::new("set");
        assert_eq!(enter(&mut d, "1234"), None);
        assert_eq!(enter(&mut d, "4321"), None);
        assert_eq!(d.error, "remote.pin.mismatch");
        assert_eq!(enter(&mut d, "1234"), None);
        assert_eq!(enter(&mut d, "1234"), Some(json!({"type":"set_pin","current":null,"pin":"1234"})));
        assert!(d.text.is_empty() && d.first.is_empty() && d.current.is_empty());
        assert_eq!(enter(&mut d, "1234"), None);
    }
    #[test]
    fn controller_entry_covers_wifi_characters_and_empty_saved_network() {
        let mut d = Dialog::new("wifi");
        d.target = "Test Wi-Fi".into();
        let mut chars = std::collections::BTreeSet::new();
        for _ in 0..3 {
            chars.extend(d.keys("en").into_iter().take(36));
            d.selected = 37;
            d.key("ok");
        }
        assert_eq!(chars.len(), 95);
        assert_eq!(d.key("submit"), Some(json!({"type":"wifi_connect","ssid":"Test Wi-Fi","password":""})));
        d.failed("retry".into());
        assert_eq!(d.target, "Test Wi-Fi");
        assert!(!d.waiting);
    }

    #[test]
    fn validates_custom_colors_and_allows_reset() {
        let mut d = Dialog::new("color");
        d.target = "theme.accent".into();
        assert_eq!(enter(&mut d, "invalid"), None);
        assert!(!d.waiting);
        d.text = "#123aBc".into();
        assert_eq!(d.key("submit"), Some(json!({"type":"set","key":"theme.accent","value":"#123aBc"})));
        let mut d = Dialog::new("color");
        d.target = "theme.accent".into();
        assert_eq!(d.key("submit"), Some(json!({"type":"set","key":"theme.accent","value":null})));
    }

    #[test]
    fn change_remove_unlock_and_retry() {
        let mut d = Dialog::new("change");
        enter(&mut d, "1234");
        enter(&mut d, "5678");
        assert_eq!(enter(&mut d, "5678"), Some(json!({"type":"set_pin","current":"1234","pin":"5678"})));
        d.failed("incorrect".into());
        assert_eq!(d.prompt(), "remote.pin.enter_current");
        assert!(!d.waiting);
        assert_eq!(enter(&mut Dialog::new("remove"), "1234"), Some(json!({"type":"set_pin","current":"1234"})));
        assert_eq!(enter(&mut Dialog::new("unlock"), "1234"), Some(json!({"type":"unlock","pin":"1234"})));
    }
}
