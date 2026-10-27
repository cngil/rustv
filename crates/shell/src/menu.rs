//! Settings on the TV itself, for whoever has only the TV remote (over
//! HDMI-CEC) or a gamepad: one list, moved through with the arrows.
//!
//! Every change goes to tvd as the same `set` command the phone remote sends;
//! tvd writes the config file and sends the settings back to everyone. Wi-Fi,
//! Bluetooth, the PIN and custom colors need typing, so they stay on the phone.

use serde_json::{json, Value};

/// English and Turkish, in that order.
type Text = [&'static str; 2];

#[derive(Clone, Copy)]
enum Lit {
    Num(f64),
    Str(&'static str),
}

impl Lit {
    fn value(self) -> Value {
        match self {
            Lit::Num(n) => json!(n),
            Lit::Str(s) => json!(s),
        }
    }

    fn matches(self, value: &Value) -> bool {
        match self {
            Lit::Num(n) => value.as_f64().is_some_and(|v| (v - n).abs() < 1e-6),
            Lit::Str(s) => value.as_str() == Some(s),
        }
    }
}

enum Kind {
    Toggle,
    Choice(&'static [(Lit, Text)]),
    /// Sends this command, then closes the menu so its result shows.
    Action(&'static str),
    /// Shows or hides the debug overlay now; not saved.
    Overlay,
    /// The TV output's resolution and refresh rate, from the modes tvd reports.
    /// A change is tried first and kept on the TV's own prompt.
    Resolution,
    Refresh,
}

struct Item {
    /// `section.name` in the config; unused for actions.
    key: &'static str,
    label: Text,
    hint: Text,
    kind: Kind,
    /// Greyed out (and skipped) while this setting is off.
    needs: Option<&'static str>,
}

struct Section {
    title: Text,
    items: &'static [Item],
    /// Only shown once the config lists pictures.
    pictures: bool,
    /// Only shown once tvd reports the TV's display modes.
    display: bool,
}

const fn toggle(key: &'static str, label: Text, hint: Text, needs: Option<&'static str>) -> Item {
    Item { key, label, hint, kind: Kind::Toggle, needs }
}

const fn choice(key: &'static str, label: Text, options: &'static [(Lit, Text)], needs: Option<&'static str>) -> Item {
    Item { key, label, hint: ["", ""], kind: Kind::Choice(options), needs }
}

const NO_HINT: Text = ["", ""];
const MINUTES: &[(Lit, Text)] = &[
    (Lit::Num(1.0), ["1 min", "1 dk"]),
    (Lit::Num(2.0), ["2 min", "2 dk"]),
    (Lit::Num(5.0), ["5 min", "5 dk"]),
    (Lit::Num(10.0), ["10 min", "10 dk"]),
    (Lit::Num(15.0), ["15 min", "15 dk"]),
    (Lit::Num(30.0), ["30 min", "30 dk"]),
    (Lit::Num(60.0), ["1 h", "1 sa"]),
];
const STYLES: &[(Lit, Text)] = &[
    (Lit::Str("random"), ["Mix", "Karışık"]),
    (Lit::Str("aurora"), ["Aurora", "Aurora"]),
    (Lit::Str("orbits"), ["Orbits", "Yörünge"]),
    (Lit::Str("clock"), ["Clock", "Saat"]),
    (Lit::Str("images"), ["My pictures", "Resimlerim"]),
];

const SECTIONS: &[Section] = &[
    Section {
        title: ["Home screen", "Ana ekran"],
        pictures: false,
        display: false,
        items: &[
            toggle("shell.greeting", ["Greeting", "Karşılama"], NO_HINT, None),
            toggle("shell.clock", ["Clock", "Saat"], NO_HINT, None),
            toggle("shell.date", ["Date", "Tarih"], NO_HINT, None),
            toggle(
                "shell.hints",
                ["Key hints", "Tuş ipuçları"],
                ["The keys at the bottom of the screen.", "Ekranın altındaki tuşlar."],
                None,
            ),
            toggle(
                "shell.volume_overlay",
                ["Volume bar", "Ses çubuğu"],
                ["Shown when the volume changes.", "Ses değişince görünür."],
                None,
            ),
            toggle(
                "shell.sounds",
                ["Sounds", "Sesler"],
                [
                    "Soft clicks when moving between apps and opening one.",
                    "Uygulamalar arasında gezerken ve açarken hafif tıklamalar.",
                ],
                None,
            ),
            choice(
                "shell.sound_volume",
                ["Sound level", "Ses düzeyi"],
                &[
                    (Lit::Num(0.25), ["Quiet", "Sessiz"]),
                    (Lit::Num(0.5), ["Medium", "Orta"]),
                    (Lit::Num(1.0), ["Loud", "Yüksek"]),
                ],
                Some("shell.sounds"),
            ),
            toggle(
                "shell.pairing_qr",
                ["Pairing code", "Eşleştirme kodu"],
                [
                    "QR code on the TV until a phone pairs, and when you ask for it.",
                    "Bir telefon eşleşene kadar ve istediğinde TV'deki QR kod.",
                ],
                None,
            ),
            Item {
                key: "",
                label: ["Pair a phone", "Telefon eşleştir"],
                hint: ["Shows the QR code for two minutes.", "QR kodu iki dakika gösterir."],
                kind: Kind::Action("show_pairing"),
                needs: Some("shell.pairing_qr"),
            },
        ],
    },
    Section {
        title: ["Look and language", "Görünüm ve dil"],
        pictures: false,
        display: false,
        items: &[
            choice(
                "theme.preset",
                ["Finish", "Kaplama"],
                &[
                    (Lit::Str("silver"), ["Silver", "Gümüş"]),
                    (Lit::Str("titanium"), ["Titanium", "Titanyum"]),
                    (Lit::Str("champagne"), ["Champagne", "Şampanya"]),
                    (Lit::Str("gunmetal"), ["Gunmetal", "Füme"]),
                ],
                None,
            ),
            choice(
                "locale.language",
                ["Language", "Dil"],
                &[(Lit::Str("en"), ["English", "English"]), (Lit::Str("tr"), ["Türkçe", "Türkçe"])],
                None,
            ),
            choice(
                "locale.clock",
                ["Time format", "Saat biçimi"],
                &[(Lit::Str("24h"), ["24-hour", "24 saat"]), (Lit::Str("12h"), ["12-hour", "12 saat"])],
                None,
            ),
            choice(
                "locale.temperature",
                ["Temperature", "Sıcaklık"],
                &[(Lit::Str("celsius"), ["°C", "°C"]), (Lit::Str("fahrenheit"), ["°F", "°F"])],
                None,
            ),
        ],
    },
    Section {
        title: ["Display", "Ekran"],
        pictures: false,
        display: true,
        items: &[
            Item {
                key: "",
                label: ["Resolution", "Çözünürlük"],
                hint: [
                    "A new mode goes back by itself after 15 seconds unless you keep it.",
                    "Yeni bir mod, korumazsan 15 saniye sonra kendiliğinden geri döner.",
                ],
                kind: Kind::Resolution,
                needs: None,
            },
            Item {
                key: "", label: ["Refresh rate", "Yenileme hızı"], hint: NO_HINT, kind: Kind::Refresh, needs: None
            },
        ],
    },
    Section {
        title: ["Wallpaper", "Duvar kağıdı"],
        pictures: true,
        display: false,
        items: &[
            toggle(
                "wallpaper.home",
                ["Behind the home screen", "Ana ekranın arkasında"],
                ["Your pictures instead of the plain background.", "Düz arka plan yerine kendi resimlerin."],
                None,
            ),
            choice(
                "wallpaper.change_minutes",
                ["Change every", "Değiştirme sıklığı"],
                &[
                    (Lit::Num(5.0), ["5 min", "5 dk"]),
                    (Lit::Num(15.0), ["15 min", "15 dk"]),
                    (Lit::Num(30.0), ["30 min", "30 dk"]),
                    (Lit::Num(60.0), ["1 h", "1 sa"]),
                    (Lit::Num(240.0), ["4 h", "4 sa"]),
                    (Lit::Num(1440.0), ["1 day", "1 gün"]),
                ],
                Some("wallpaper.home"),
            ),
            toggle("wallpaper.shuffle", ["Random order", "Rastgele sıra"], NO_HINT, None),
        ],
    },
    Section {
        title: ["Screensaver", "Ekran koruyucu"],
        pictures: false,
        display: false,
        items: &[
            toggle(
                "screensaver.enabled",
                ["Screensaver", "Ekran koruyucu"],
                [
                    "Starts after a while on the home screen without input.",
                    "Ana ekranda bir süre tuşa basılmazsa başlar.",
                ],
                None,
            ),
            choice("screensaver.idle_minutes", ["Start after", "Başlama süresi"], MINUTES, Some("screensaver.enabled")),
            choice("screensaver.style", ["Style", "Stil"], STYLES, Some("screensaver.enabled")),
            toggle("screensaver.clock", ["Show the time", "Saati göster"], NO_HINT, Some("screensaver.enabled")),
            toggle("screensaver.date", ["Show the date", "Tarihi göster"], NO_HINT, Some("screensaver.enabled")),
        ],
    },
    Section {
        title: ["Sleep", "Uyku"],
        pictures: false,
        display: false,
        items: &[
            toggle(
                "sleep.enabled",
                ["Sleep when unused", "Kullanılmayınca uyu"],
                [
                    "Screen off after no remote input and no sound. Any button wakes it.",
                    "Tuşa basılmaz ve ses çalmazsa ekran kapanır. Herhangi bir tuş uyandırır.",
                ],
                None,
            ),
            choice(
                "sleep.after_minutes",
                ["After", "Süre"],
                &[
                    (Lit::Num(15.0), ["15 min", "15 dk"]),
                    (Lit::Num(30.0), ["30 min", "30 dk"]),
                    (Lit::Num(60.0), ["1 h", "1 sa"]),
                    (Lit::Num(120.0), ["2 h", "2 sa"]),
                    (Lit::Num(240.0), ["4 h", "4 sa"]),
                ],
                Some("sleep.enabled"),
            ),
            toggle(
                "sleep.close_apps",
                ["Close the open app", "Açık uygulamayı kapat"],
                ["Frees memory and CPU.", "Bellek ve işlemciyi boşaltır."],
                None,
            ),
            Item {
                key: "",
                label: ["Sleep now", "Şimdi uyut"],
                hint: ["Any button wakes the TV.", "Herhangi bir tuş TV'yi uyandırır."],
                kind: Kind::Action("sleep"),
                needs: None,
            },
        ],
    },
    Section {
        title: ["Sound", "Ses"],
        pictures: false,
        display: false,
        items: &[
            choice(
                "audio.max_volume",
                ["Volume limit", "Ses sınırı"],
                &[
                    (Lit::Num(1.0), ["100%", "%100"]),
                    (Lit::Num(1.2), ["120%", "%120"]),
                    (Lit::Num(1.5), ["150%", "%150"]),
                ],
                None,
            ),
            choice(
                "audio.step",
                ["Volume buttons change by", "Ses tuşları adımı"],
                &[(Lit::Num(0.02), ["2%", "%2"]), (Lit::Num(0.05), ["5%", "%5"]), (Lit::Num(0.1), ["10%", "%10"])],
                None,
            ),
            toggle(
                "audio.per_app_volume",
                ["Volume per app", "Uygulamaya göre ses"],
                [
                    "Each app and the home screen keep their own volume.",
                    "Her uygulama ve ana ekran kendi ses düzeyini korur.",
                ],
                None,
            ),
        ],
    },
    Section {
        title: ["On-screen keyboard", "Ekran klavyesi"],
        pictures: false,
        display: false,
        items: &[
            toggle(
                "keyboard.enabled",
                ["Keyboard on the TV", "TV'de klavye"],
                [
                    "For typing with the TV remote or a gamepad: arrows move, OK types, Back closes.",
                    "TV kumandası ya da oyun koluyla yazmak için: yön tuşları gezer, OK yazar, Geri kapatır.",
                ],
                None,
            ),
            toggle(
                "keyboard.hold_ok",
                ["Hold OK to open it", "Açmak için OK'a basılı tut"],
                ["On the TV remote, over HDMI-CEC.", "TV kumandasında, HDMI-CEC üzerinden."],
                Some("keyboard.enabled"),
            ),
            toggle(
                "keyboard.hold_guide",
                ["Hold Guide on a gamepad", "Oyun kolunda ana menü tuşuna basılı tut"],
                ["A short press still goes home.", "Kısa basış yine ana ekrana döner."],
                Some("keyboard.enabled"),
            ),
        ],
    },
    Section {
        title: ["TV control (HDMI-CEC)", "TV kontrolü (HDMI-CEC)"],
        pictures: false,
        display: false,
        items: &[
            toggle(
                "cec.enabled",
                ["Control the TV over HDMI", "TV'yi HDMI üzerinden kontrol et"],
                ["Needs a CEC adapter on the box (/dev/cec*).", "Kutuda bir CEC bağdaştırıcısı gerekir (/dev/cec*)."],
                None,
            ),
            toggle(
                "cec.power_tv",
                ["Turn the TV on and off", "TV'yi aç ve kapat"],
                [
                    "Waking turns the TV on; sleep puts it in standby, unless it shows another input.",
                    "Uyanınca TV açılır; uykuda bekleme moduna geçer, başka bir giriş açıksa dokunulmaz.",
                ],
                Some("cec.enabled"),
            ),
            toggle(
                "cec.follow_tv",
                ["Follow the TV", "TV'yi takip et"],
                [
                    "Turning the TV off puts rustv to sleep; switching to rustv wakes it.",
                    "TV kapanınca rustv uyur; rustv'ye geçince uyanır.",
                ],
                Some("cec.enabled"),
            ),
            toggle(
                "cec.remote_keys",
                ["Use the TV remote", "TV kumandasını kullan"],
                [
                    "Arrows, OK, Back, play and volume buttons control rustv.",
                    "Yön, OK, Geri, oynat ve ses tuşları rustv'yi kontrol eder.",
                ],
                Some("cec.enabled"),
            ),
        ],
    },
    Section {
        title: ["Gamepad and casting", "Oyun kolu ve yansıtma"],
        pictures: false,
        display: false,
        items: &[
            toggle(
                "gamepad.enabled",
                ["Use a gamepad as the remote", "Oyun kolunu kumanda olarak kullan"],
                ["D-pad or stick to move, A to open, B back.", "Yön tuşları ya da çubukla gez, A aç, B geri."],
                None,
            ),
            toggle(
                "gamepad.guide_closes_app",
                ["Guide button goes home", "Ana menü tuşu ana ekrana döner"],
                NO_HINT,
                Some("gamepad.enabled"),
            ),
            toggle(
                "cast.airplay",
                ["AirPlay", "AirPlay"],
                [
                    "Mirror an iPhone, iPad or Mac. Needs uxplay on the box.",
                    "iPhone, iPad ya da Mac ekranını yansıt. Kutuda uxplay gerekir.",
                ],
                None,
            ),
        ],
    },
    Section {
        title: ["Debug overlay", "Hata ayıklama katmanı"],
        pictures: false,
        display: false,
        items: &[
            Item {
                key: "",
                label: ["Show on the TV now", "Şimdi TV'de göster"],
                hint: [
                    "CPU, GPU, RAM, network and more, over everything.",
                    "İşlemci, ekran kartı, bellek, ağ ve dahası, her şeyin üstünde.",
                ],
                kind: Kind::Overlay,
                needs: None,
            },
            choice(
                "overlay.position",
                ["Position", "Konum"],
                &[(Lit::Str("top"), ["Top", "Üstte"]), (Lit::Str("bottom"), ["Bottom", "Altta"])],
                None,
            ),
        ],
    },
];

/// Heights in the 1920×1080 layout the shell scales from.
const HEADER: f32 = 92.0;
const ROW: f32 = 84.0;
/// How much of the list fits on screen.
pub const VIEW: f32 = 680.0;

/// One line of the list, ready for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub header: bool,
    pub label: String,
    /// The choice shown on the right; empty for toggles and actions.
    pub value: String,
    /// "toggle", "choice", "action" or "header".
    pub kind: &'static str,
    pub on: bool,
    pub enabled: bool,
    pub focused: bool,
    /// Top of the row in the list, before scrolling.
    pub y: f32,
    /// More choices to the left / right.
    pub before: bool,
    pub after: bool,
}

/// What a key press asks of the shell.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// Send this command to tvd.
    Send(Value),
    Close,
    SendAndClose(Value),
}

#[derive(Default)]
pub struct Menu {
    /// The last `settings` message from tvd.
    settings: Value,
    overlay: bool,
    /// Index into the items that can take focus.
    selected: usize,
    scroll: f32,
}

impl Menu {
    pub fn set_settings(&mut self, settings: Value) {
        self.settings = settings;
        self.selected = self.selected.min(self.focusable().len().saturating_sub(1));
    }

    pub fn set_overlay(&mut self, visible: bool) {
        self.overlay = visible;
    }

    /// Opening starts at the top.
    pub fn reset(&mut self) {
        self.selected = 0;
        self.scroll = 0.0;
    }

    fn lang(&self) -> usize {
        usize::from(self.settings["locale"]["language"] == "tr")
    }

    fn get(&self, key: &str) -> &Value {
        let (section, name) = key.split_once('.').unwrap_or((key, ""));
        &self.settings[section][name]
    }

    fn sections(&self) -> impl Iterator<Item = &'static Section> + '_ {
        let pictures = self.settings["wallpaper"]["count"].as_u64().unwrap_or(0) > 0;
        let display = !self.modes().is_empty();
        SECTIONS.iter().filter(move |section| (pictures || !section.pictures) && (display || !section.display))
    }

    /// The TV's modes as `(size, hz)`, such as `("1920x1080", "59.94")`.
    fn modes(&self) -> Vec<(String, String)> {
        let modes = self.settings["display"]["modes"].as_array().cloned().unwrap_or_default();
        modes
            .iter()
            .filter_map(|mode| mode.as_str()?.split_once('@').map(|(size, hz)| (size.to_string(), hz.to_string())))
            .collect()
    }

    /// The mode on screen, or the configured one while the screen is off.
    fn current_mode(&self) -> Option<(String, String)> {
        let display = &self.settings["display"];
        let mode = display["current"].as_str().or(display["mode"].as_str().filter(|mode| *mode != "auto"))?;
        mode.split_once('@').map(|(size, hz)| (size.to_string(), hz.to_string()))
    }

    /// For a resolution or refresh row: the full modes it can switch to (with
    /// their labels) and which one is on now.
    fn mode_choices(&self, kind: &Kind) -> (Vec<(String, String)>, Option<usize>) {
        let modes = self.modes();
        let Some((size, hz)) = self.current_mode().or_else(|| modes.first().cloned()) else {
            return (Vec::new(), None);
        };
        let mut choices: Vec<(String, String)> = Vec::new();
        match kind {
            Kind::Resolution => {
                for (other, _) in &modes {
                    if choices.iter().any(|(mode, _)| mode.starts_with(&format!("{other}@"))) {
                        continue;
                    }
                    // Keep the refresh rate where the new size offers it, else the fastest up to 60 Hz.
                    let rates: Vec<&String> = modes.iter().filter(|(s, _)| s == other).map(|(_, h)| h).collect();
                    let rate = rates
                        .iter()
                        .find(|h| **h == &hz)
                        .or_else(|| rates.iter().find(|h| h.parse::<f64>().is_ok_and(|v| v <= 60.0)))
                        .or(rates.first())
                        .map_or(hz.clone(), |h| h.to_string());
                    choices.push((format!("{other}@{rate}"), other.replace('x', " × ")));
                }
                let at = choices.iter().position(|(mode, _)| mode.starts_with(&format!("{size}@")));
                (choices, at)
            }
            Kind::Refresh => {
                for (other, rate) in &modes {
                    if *other == size {
                        choices.push((format!("{other}@{rate}"), format!("{rate} Hz")));
                    }
                }
                let at = choices.iter().position(|(mode, _)| *mode == format!("{size}@{hz}"));
                (choices, at)
            }
            _ => (Vec::new(), None),
        }
    }

    fn enabled(&self, item: &Item) -> bool {
        item.needs.map_or(true, |key| self.get(key) == &Value::Bool(true))
    }

    /// The options a choice offers now: "My pictures" only once there are pictures.
    fn options(&self, item: &Item) -> Vec<(Lit, Text)> {
        let Kind::Choice(options) = item.kind else { return Vec::new() };
        let pictures = self.settings["wallpaper"]["count"].as_u64().unwrap_or(0) > 0;
        options.iter().copied().filter(|(lit, _)| pictures || !matches!(lit, Lit::Str("images"))).collect()
    }

    fn focusable(&self) -> Vec<&'static Item> {
        self.sections().flat_map(|section| section.items).filter(|item| self.enabled(item)).collect()
    }

    /// The list as the UI draws it, with the focused row kept in view.
    pub fn rows(&mut self) -> Vec<Row> {
        let lang = self.lang();
        let focused = self.focusable().get(self.selected).map(|item| *item as *const Item);
        let mut rows = Vec::new();
        let mut y = 0.0;
        let sections: Vec<&'static Section> = self.sections().collect();
        for section in sections {
            rows.push(Row {
                header: true,
                label: section.title[lang].into(),
                value: String::new(),
                kind: "header",
                on: false,
                enabled: true,
                focused: false,
                y,
                before: false,
                after: false,
            });
            y += HEADER;
            for item in section.items {
                let options = self.options(item);
                let current = self.get(item.key);
                let at = options.iter().position(|(lit, _)| lit.matches(current));
                let (choices, chosen) = self.mode_choices(&item.kind);
                let (kind, value) = match item.kind {
                    Kind::Toggle | Kind::Overlay => ("toggle", String::new()),
                    Kind::Action(_) => ("action", String::new()),
                    Kind::Choice(_) => ("choice", at.map_or_else(|| shown(current), |i| options[i].1[lang].into())),
                    Kind::Resolution | Kind::Refresh => {
                        ("choice", chosen.map(|i| choices[i].1.clone()).unwrap_or_default())
                    }
                };
                let (at, count) = match item.kind {
                    Kind::Resolution | Kind::Refresh => (chosen, choices.len()),
                    _ => (at, options.len()),
                };
                let on = match item.kind {
                    Kind::Overlay => self.overlay,
                    _ => current == &Value::Bool(true),
                };
                let is_focused = focused == Some(item as *const Item);
                if is_focused {
                    // Keep a row of room above and below the focus where there is more list.
                    let top = (y - ROW).max(0.0);
                    if top < self.scroll {
                        self.scroll = top;
                    } else if y + 2.0 * ROW > self.scroll + VIEW {
                        self.scroll = y + 2.0 * ROW - VIEW;
                    }
                }
                rows.push(Row {
                    header: false,
                    label: item.label[lang].into(),
                    value,
                    kind,
                    on,
                    enabled: self.enabled(item),
                    focused: is_focused,
                    y,
                    before: at.is_some_and(|i| i > 0),
                    after: at.map_or(count > 0, |i| i + 1 < count),
                });
                y += ROW;
            }
        }
        // Never scroll past the end, nor before the start.
        self.scroll = self.scroll.min((y - VIEW).max(0.0)).max(0.0);
        rows
    }

    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// The explanation for the focused row, if it has one.
    pub fn hint(&self) -> String {
        self.focusable().get(self.selected).map_or_else(String::new, |item| item.hint[self.lang()].into())
    }

    /// "up", "down", "left", "right", "ok" or "back".
    pub fn key(&mut self, key: &str) -> Outcome {
        let items = self.focusable();
        let Some(item) = items.get(self.selected).copied() else {
            return if key == "back" { Outcome::Close } else { Outcome::Nothing };
        };
        match key {
            "up" => self.selected = self.selected.saturating_sub(1),
            "down" => self.selected = (self.selected + 1).min(items.len() - 1),
            "back" => return Outcome::Close,
            "left" | "right" | "ok" => return self.change(item, key),
            _ => {}
        }
        Outcome::Nothing
    }

    fn change(&mut self, item: &Item, key: &str) -> Outcome {
        match item.kind {
            Kind::Toggle if key == "ok" => {
                let value = json!(self.get(item.key) != &Value::Bool(true));
                self.set(item.key, value.clone());
                Outcome::Send(json!({ "type": "set", "key": item.key, "value": value }))
            }
            Kind::Overlay if key == "ok" => {
                self.overlay = !self.overlay;
                Outcome::Send(json!({ "type": "overlay", "visible": self.overlay }))
            }
            Kind::Action(command) if key == "ok" => Outcome::SendAndClose(json!({ "type": command })),
            Kind::Resolution | Kind::Refresh => {
                let (choices, at) = self.mode_choices(&item.kind);
                if choices.is_empty() {
                    return Outcome::Nothing;
                }
                let next = match (key, at) {
                    (_, None) => 0,
                    ("left", Some(i)) => i.saturating_sub(1),
                    ("right", Some(i)) => (i + 1).min(choices.len() - 1),
                    (_, Some(i)) => (i + 1) % choices.len(),
                };
                if at == Some(next) {
                    return Outcome::Nothing;
                }
                // tvd tries it and the TV asks whether to keep it.
                Outcome::Send(json!({ "type": "display_mode", "mode": choices[next].0 }))
            }
            Kind::Choice(_) => {
                let options = self.options(item);
                let at = options.iter().position(|(lit, _)| lit.matches(self.get(item.key)));
                let next = match (key, at) {
                    // A value from the config file that is no preset: any move lands on the first preset.
                    (_, None) => 0,
                    ("left", Some(i)) => i.saturating_sub(1),
                    ("right", Some(i)) => (i + 1).min(options.len() - 1),
                    // OK cycles round.
                    (_, Some(i)) => (i + 1) % options.len(),
                };
                if at == Some(next) {
                    return Outcome::Nothing;
                }
                let value = options[next].0.value();
                self.set(item.key, value.clone());
                Outcome::Send(json!({ "type": "set", "key": item.key, "value": value }))
            }
            _ => Outcome::Nothing,
        }
    }

    /// Shows a change at once; tvd's `settings` message confirms it (or puts it back).
    fn set(&mut self, key: &str, value: Value) {
        let (section, name) = key.split_once('.').unwrap_or((key, ""));
        if let Some(section) = self.settings.get_mut(section).and_then(Value::as_object_mut) {
            section.insert(name.into(), value);
        }
        // Turning a setting off can grey out the row after it; keep the focus in range.
        self.selected = self.selected.min(self.focusable().len().saturating_sub(1));
    }
}

/// A config value that matches no preset, as text.
fn shown(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Menu {
        let mut menu = Menu::default();
        menu.set_settings(json!({
            "type": "settings",
            "shell": { "greeting": true, "clock": true, "date": false, "hints": true, "volume_overlay": true,
                       "sounds": false, "sound_volume": 0.5, "pairing_qr": true },
            "screensaver": { "enabled": true, "idle_minutes": 5.0, "style": "random", "clock": true, "date": true },
            "theme": { "preset": "silver" },
            "locale": { "language": "en", "clock": "24h", "temperature": "celsius" },
            "wallpaper": { "home": true, "change_minutes": 30.0, "shuffle": true, "count": 0 },
            "sleep": { "enabled": true, "after_minutes": 60.0, "close_apps": true },
            "audio": { "max_volume": 1.0, "step": 0.05, "per_app_volume": true },
            "cec": { "enabled": false },
            "gamepad": { "enabled": true, "guide_closes_app": true },
            "cast": { "airplay": false },
            "overlay": { "position": "top" }
        }));
        menu
    }

    fn focused(menu: &mut Menu) -> Row {
        menu.rows().into_iter().find(|row| row.focused).unwrap()
    }

    #[test]
    fn toggles_send_the_same_command_as_the_phone() {
        let mut menu = menu();
        assert_eq!(focused(&mut menu).label, "Greeting");
        assert_eq!(menu.key("ok"), Outcome::Send(json!({ "type": "set", "key": "shell.greeting", "value": false })));
        assert!(!focused(&mut menu).on);
        // Left and right do nothing on a switch.
        assert_eq!(menu.key("right"), Outcome::Nothing);
    }

    #[test]
    fn choices_step_and_stop_at_the_ends() {
        let mut menu = menu();
        while focused(&mut menu).label != "Start after" {
            menu.key("down");
        }
        assert_eq!(focused(&mut menu).value, "5 min");
        assert_eq!(
            menu.key("right"),
            Outcome::Send(json!({ "type": "set", "key": "screensaver.idle_minutes", "value": 10.0 }))
        );
        for _ in 0..10 {
            menu.key("right");
        }
        let row = focused(&mut menu);
        assert_eq!((row.value.as_str(), row.after), ("1 h", false));
        assert_eq!(menu.key("right"), Outcome::Nothing);
        // OK cycles round.
        assert_eq!(
            menu.key("ok"),
            Outcome::Send(json!({ "type": "set", "key": "screensaver.idle_minutes", "value": 1.0 }))
        );
    }

    #[test]
    fn rows_that_need_a_switch_are_skipped_while_it_is_off() {
        let mut menu = menu();
        // Sounds is off, so Sound level is grey and the focus jumps from Sounds to Pairing code.
        while focused(&mut menu).label != "Sounds" {
            menu.key("down");
        }
        menu.key("down");
        assert_eq!(focused(&mut menu).label, "Pairing code");
        let rows = menu.rows();
        assert!(!rows.iter().find(|row| row.label == "Sound level").unwrap().enabled);
        // The CEC rows below the master switch are grey too.
        assert!(!rows.iter().find(|row| row.label == "Follow the TV").unwrap().enabled);
    }

    #[test]
    fn pictures_only_once_there_are_some() {
        let mut menu = menu();
        assert!(!menu.rows().iter().any(|row| row.label == "Wallpaper"));
        let mut settings = menu.settings.clone();
        settings["wallpaper"]["count"] = json!(3);
        menu.set_settings(settings);
        assert!(menu.rows().iter().any(|row| row.label == "Wallpaper"));
    }

    #[test]
    fn display_modes_are_tried_not_set() {
        let mut menu = menu();
        assert!(!menu.rows().iter().any(|row| row.label == "Resolution"));
        let mut settings = menu.settings.clone();
        settings["display"] = json!({
            "mode": "auto", "current": "1920x1080@60",
            "modes": ["3840x2160@60", "3840x2160@30", "1920x1080@120", "1920x1080@60", "1280x720@60"]
        });
        menu.set_settings(settings);
        while focused(&mut menu).label != "Resolution" {
            menu.key("down");
        }
        assert_eq!(focused(&mut menu).value, "1920 × 1080");
        // A larger size keeps 60 Hz, which it offers.
        assert_eq!(menu.key("left"), Outcome::Send(json!({ "type": "display_mode", "mode": "3840x2160@60" })));
        menu.key("down");
        let row = focused(&mut menu);
        assert_eq!((row.label.as_str(), row.value.as_str()), ("Refresh rate", "60 Hz"));
        assert_eq!(menu.key("left"), Outcome::Send(json!({ "type": "display_mode", "mode": "1920x1080@120" })));
    }

    #[test]
    fn actions_close_the_menu_and_back_closes_it() {
        let mut menu = menu();
        while focused(&mut menu).label != "Sleep now" {
            menu.key("down");
        }
        assert_eq!(menu.key("ok"), Outcome::SendAndClose(json!({ "type": "sleep" })));
        assert_eq!(menu.key("back"), Outcome::Close);
    }

    #[test]
    fn the_focus_stays_in_view_and_turkish_works() {
        let mut menu = menu();
        for _ in 0..40 {
            menu.key("down");
        }
        let row = focused(&mut menu);
        assert!(row.y >= menu.scroll() && row.y + ROW <= menu.scroll() + VIEW, "{} in {}", row.y, menu.scroll());
        let mut settings = menu.settings.clone();
        settings["locale"]["language"] = json!("tr");
        menu.set_settings(settings);
        menu.reset();
        assert_eq!(focused(&mut menu).label, "Karşılama");
        assert_eq!(menu.scroll(), 0.0);
    }
}
