//! Settings on the TV itself, for whoever has only the TV remote (over
//! HDMI-CEC) or a gamepad: one list, moved through with the arrows.
//!
//! Every change goes to tvd as the same `set` command the phone remote sends;
//! tvd writes the config file and sends the settings back to everyone. Wi-Fi,
//! Bluetooth, the PIN and custom colors need typing, so they stay on the phone.
//! The labels are the phone's too: `setting.<config key>.label` in the shared
//! catalog (`crates/locales`).

use serde_json::{json, Value};

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

/// How a choice reads: a catalog key, a number in a unit such as `unit.minutes`,
/// or a time of day (`HH:MM`) as the clock shows it.
#[derive(Clone, Copy)]
enum Words {
    Key(&'static str),
    Unit(&'static str, f64),
    Time(&'static str),
}

impl Words {
    fn read(self, lang: &str, twelve_hour: bool) -> String {
        match self {
            Words::Key(key) => rustv_locales::text(lang, key).to_string(),
            Words::Unit(unit, n) => rustv_locales::format(lang, unit, &[("n", &number(n))]),
            Words::Time(time) => clock_time(time, twelve_hour),
        }
    }
}

/// `22:00`, or `10 PM` with the 12-hour clock, as the remote writes it.
fn clock_time(time: &str, twelve_hour: bool) -> String {
    let (h, m) = time.split_once(':').unwrap_or((time, "00"));
    let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) else { return time.to_string() };
    if !twelve_hour {
        return time.to_string();
    }
    let minutes = if m == 0 { String::new() } else { format!(":{m:02}") };
    format!("{}{minutes} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
}

/// 30, 1.5: no trailing zeros.
fn number(n: f64) -> String {
    let text = format!("{n:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

enum Kind {
    Toggle,
    Choice(&'static [(Lit, Words)]),
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
    /// Catalog keys.
    label: &'static str,
    hint: Option<&'static str>,
    kind: Kind,
    /// Greyed out (and skipped) while this setting is off.
    needs: Option<&'static str>,
}

struct Section {
    /// A catalog key.
    title: &'static str,
    items: &'static [Item],
    /// Only shown once the config lists pictures.
    pictures: bool,
    /// Only shown once tvd reports the TV's display modes.
    display: bool,
}

const fn toggle(
    key: &'static str,
    label: &'static str,
    hint: Option<&'static str>,
    needs: Option<&'static str>,
) -> Item {
    Item { key, label, hint, kind: Kind::Toggle, needs }
}

const fn choice(
    key: &'static str,
    label: &'static str,
    options: &'static [(Lit, Words)],
    needs: Option<&'static str>,
) -> Item {
    Item { key, label, hint: None, kind: Kind::Choice(options), needs }
}

const fn section(title: &'static str, items: &'static [Item]) -> Section {
    Section { title, items, pictures: false, display: false }
}

use Words::{Key, Time, Unit};

const WARMTH: &[(Lit, Words)] = &[
    (Lit::Num(0.0), Key("option.warmth.off")),
    (Lit::Num(0.3), Key("option.warmth.low")),
    (Lit::Num(0.6), Key("option.warmth.medium")),
    (Lit::Num(1.0), Key("option.warmth.high")),
];

const MINUTES: &[(Lit, Words)] = &[
    (Lit::Num(1.0), Unit("unit.minutes", 1.0)),
    (Lit::Num(2.0), Unit("unit.minutes", 2.0)),
    (Lit::Num(5.0), Unit("unit.minutes", 5.0)),
    (Lit::Num(10.0), Unit("unit.minutes", 10.0)),
    (Lit::Num(15.0), Unit("unit.minutes", 15.0)),
    (Lit::Num(30.0), Unit("unit.minutes", 30.0)),
    (Lit::Num(60.0), Unit("unit.hours", 1.0)),
];
const STYLES: &[(Lit, Words)] = &[
    (Lit::Str("random"), Key("option.screensaver.style.random")),
    (Lit::Str("aurora"), Key("option.screensaver.style.aurora")),
    (Lit::Str("orbits"), Key("option.screensaver.style.orbits")),
    (Lit::Str("clock"), Key("option.screensaver.style.clock")),
    (Lit::Str("images"), Key("option.screensaver.style.images")),
];

const SECTIONS: &[Section] = &[
    section(
        "section.home_screen",
        &[
            toggle("shell.greeting", "setting.shell.greeting.label", None, None),
            toggle("shell.clock", "setting.shell.clock.label", None, None),
            toggle("shell.date", "setting.shell.date.label", None, None),
            toggle("shell.hints", "setting.shell.hints.label", Some("setting.shell.hints.hint"), None),
            toggle("shell.volume_overlay", "setting.shell.volume_overlay.label", None, None),
            toggle("shell.sounds", "setting.shell.sounds.label", Some("setting.shell.sounds.hint"), None),
            choice(
                "shell.sound_volume",
                "setting.shell.sound_volume.label",
                &[
                    (Lit::Num(0.25), Key("option.shell.sound_volume.quiet")),
                    (Lit::Num(0.5), Key("option.shell.sound_volume.medium")),
                    (Lit::Num(1.0), Key("option.shell.sound_volume.loud")),
                ],
                Some("shell.sounds"),
            ),
            toggle("shell.pairing_qr", "setting.shell.pairing_qr.label", Some("setting.shell.pairing_qr.hint"), None),
            Item {
                key: "",
                label: "setting.pairing.show.label",
                hint: Some("setting.pairing.show.hint"),
                kind: Kind::Action("show_pairing"),
                needs: Some("shell.pairing_qr"),
            },
        ],
    ),
    section(
        "section.look",
        &[
            choice(
                "theme.preset",
                "setting.theme.preset.label",
                &[
                    (Lit::Str("silver"), Key("option.theme.preset.silver")),
                    (Lit::Str("titanium"), Key("option.theme.preset.titanium")),
                    (Lit::Str("champagne"), Key("option.theme.preset.champagne")),
                    (Lit::Str("gunmetal"), Key("option.theme.preset.gunmetal")),
                ],
                None,
            ),
            choice(
                "locale.language",
                "setting.locale.language.label",
                &[
                    (Lit::Str("en"), Key("option.locale.language.en")),
                    (Lit::Str("tr"), Key("option.locale.language.tr")),
                ],
                None,
            ),
            choice(
                "locale.clock",
                "setting.locale.clock.label",
                &[(Lit::Str("24h"), Key("option.locale.clock.24h")), (Lit::Str("12h"), Key("option.locale.clock.12h"))],
                None,
            ),
            Item {
                key: "locale.temperature",
                label: "setting.locale.temperature.label",
                hint: Some("setting.locale.temperature.hint"),
                kind: Kind::Choice(&[
                    (Lit::Str("celsius"), Key("option.locale.temperature.celsius")),
                    (Lit::Str("fahrenheit"), Key("option.locale.temperature.fahrenheit")),
                ]),
                needs: None,
            },
        ],
    ),
    Section {
        title: "section.display",
        pictures: false,
        display: false,
        items: &[
            Item {
                key: "display.brightness",
                label: "setting.display.brightness.label",
                hint: Some("setting.display.brightness.hint"),
                kind: Kind::Choice(&[
                    (Lit::Num(0.4), Unit("unit.percent", 40.0)),
                    (Lit::Num(0.6), Unit("unit.percent", 60.0)),
                    (Lit::Num(0.8), Unit("unit.percent", 80.0)),
                    (Lit::Num(1.0), Unit("unit.percent", 100.0)),
                ]),
                needs: None,
            },
            Item {
                key: "display.warmth",
                label: "setting.display.warmth.label",
                hint: Some("setting.display.warmth.hint"),
                kind: Kind::Choice(WARMTH),
                needs: None,
            },
            Item {
                key: "",
                label: "setting.display.resolution.label",
                hint: Some("setting.display.resolution.hint"),
                kind: Kind::Resolution,
                needs: None,
            },
            Item { key: "", label: "setting.display.refresh.label", hint: None, kind: Kind::Refresh, needs: None },
        ],
    },
    section(
        "section.night",
        &[
            toggle("night.enabled", "setting.night.enabled.label", Some("setting.night.enabled.hint"), None),
            choice(
                "night.from",
                "setting.night.from.label",
                &[
                    (Lit::Str("20:00"), Time("20:00")),
                    (Lit::Str("21:00"), Time("21:00")),
                    (Lit::Str("22:00"), Time("22:00")),
                    (Lit::Str("23:00"), Time("23:00")),
                    (Lit::Str("00:00"), Time("00:00")),
                ],
                Some("night.enabled"),
            ),
            choice(
                "night.to",
                "setting.night.to.label",
                &[
                    (Lit::Str("05:00"), Time("05:00")),
                    (Lit::Str("06:00"), Time("06:00")),
                    (Lit::Str("07:00"), Time("07:00")),
                    (Lit::Str("08:00"), Time("08:00")),
                    (Lit::Str("09:00"), Time("09:00")),
                ],
                Some("night.enabled"),
            ),
            choice(
                "night.brightness",
                "setting.night.brightness.label",
                &[
                    (Lit::Num(0.4), Unit("unit.percent", 40.0)),
                    (Lit::Num(0.6), Unit("unit.percent", 60.0)),
                    (Lit::Num(0.7), Unit("unit.percent", 70.0)),
                    (Lit::Num(0.8), Unit("unit.percent", 80.0)),
                    (Lit::Num(1.0), Unit("unit.percent", 100.0)),
                ],
                Some("night.enabled"),
            ),
            choice("night.warmth", "setting.night.warmth.label", WARMTH, Some("night.enabled")),
            toggle("night.audio", "setting.night.audio.label", Some("setting.night.audio.hint"), Some("night.enabled")),
        ],
    ),
    Section {
        title: "section.wallpaper",
        pictures: true,
        display: false,
        items: &[
            toggle("wallpaper.home", "setting.wallpaper.home.label", Some("setting.wallpaper.home.hint"), None),
            choice(
                "wallpaper.change_minutes",
                "setting.wallpaper.change_minutes.label",
                &[
                    (Lit::Num(5.0), Unit("unit.minutes", 5.0)),
                    (Lit::Num(15.0), Unit("unit.minutes", 15.0)),
                    (Lit::Num(30.0), Unit("unit.minutes", 30.0)),
                    (Lit::Num(60.0), Unit("unit.hours", 1.0)),
                    (Lit::Num(240.0), Unit("unit.hours", 4.0)),
                    (Lit::Num(1440.0), Unit("unit.days", 1.0)),
                ],
                Some("wallpaper.home"),
            ),
            toggle("wallpaper.shuffle", "setting.wallpaper.shuffle.label", None, None),
        ],
    },
    section(
        "section.screensaver",
        &[
            toggle(
                "screensaver.enabled",
                "setting.screensaver.enabled.label",
                Some("setting.screensaver.enabled.hint"),
                None,
            ),
            choice(
                "screensaver.idle_minutes",
                "setting.screensaver.idle_minutes.label",
                MINUTES,
                Some("screensaver.enabled"),
            ),
            choice("screensaver.style", "setting.screensaver.style.label", STYLES, Some("screensaver.enabled")),
            toggle("screensaver.clock", "setting.screensaver.clock.label", None, Some("screensaver.enabled")),
            toggle("screensaver.date", "setting.screensaver.date.label", None, Some("screensaver.enabled")),
        ],
    ),
    section(
        "section.sleep",
        &[
            toggle("sleep.enabled", "setting.sleep.enabled.label", Some("setting.sleep.enabled.hint"), None),
            choice(
                "sleep.after_minutes",
                "setting.sleep.after_minutes.label",
                &[
                    (Lit::Num(15.0), Unit("unit.minutes", 15.0)),
                    (Lit::Num(30.0), Unit("unit.minutes", 30.0)),
                    (Lit::Num(60.0), Unit("unit.hours", 1.0)),
                    (Lit::Num(120.0), Unit("unit.hours", 2.0)),
                    (Lit::Num(240.0), Unit("unit.hours", 4.0)),
                ],
                Some("sleep.enabled"),
            ),
            toggle("sleep.close_apps", "setting.sleep.close_apps.label", Some("setting.sleep.close_apps.hint"), None),
            Item {
                key: "",
                label: "setting.sleep.now.label",
                hint: Some("setting.sleep.now.hint"),
                kind: Kind::Action("sleep"),
                needs: None,
            },
        ],
    ),
    section(
        "section.sound",
        &[
            Item {
                key: "audio.max_volume",
                label: "setting.audio.max_volume.label",
                hint: Some("setting.audio.max_volume.hint"),
                kind: Kind::Choice(&[
                    (Lit::Num(1.0), Unit("unit.percent", 100.0)),
                    (Lit::Num(1.2), Unit("unit.percent", 120.0)),
                    (Lit::Num(1.5), Unit("unit.percent", 150.0)),
                ]),
                needs: None,
            },
            choice(
                "audio.step",
                "setting.audio.step.label",
                &[
                    (Lit::Num(0.02), Unit("unit.percent", 2.0)),
                    (Lit::Num(0.05), Unit("unit.percent", 5.0)),
                    (Lit::Num(0.1), Unit("unit.percent", 10.0)),
                ],
                None,
            ),
            toggle(
                "audio.per_app_volume",
                "setting.audio.per_app_volume.label",
                Some("setting.audio.per_app_volume.hint"),
                None,
            ),
            toggle("audio.night", "setting.audio.night.label", Some("setting.audio.night.hint"), None),
        ],
    ),
    section(
        "section.keyboard",
        &[
            toggle("keyboard.enabled", "setting.keyboard.enabled.label", Some("setting.keyboard.enabled.hint"), None),
            toggle(
                "keyboard.hold_ok",
                "setting.keyboard.hold_ok.label",
                Some("setting.keyboard.hold_ok.hint"),
                Some("keyboard.enabled"),
            ),
            toggle(
                "keyboard.hold_guide",
                "setting.keyboard.hold_guide.label",
                Some("setting.keyboard.hold_guide.hint"),
                Some("keyboard.enabled"),
            ),
        ],
    ),
    section(
        "section.cec",
        &[
            toggle("cec.enabled", "setting.cec.enabled.label", Some("setting.cec.enabled.hint"), None),
            toggle(
                "cec.power_tv",
                "setting.cec.power_tv.label",
                Some("setting.cec.power_tv.hint"),
                Some("cec.enabled"),
            ),
            toggle(
                "cec.follow_tv",
                "setting.cec.follow_tv.label",
                Some("setting.cec.follow_tv.hint"),
                Some("cec.enabled"),
            ),
            toggle(
                "cec.remote_keys",
                "setting.cec.remote_keys.label",
                Some("setting.cec.remote_keys.hint"),
                Some("cec.enabled"),
            ),
        ],
    ),
    section(
        "section.gamepad_casting",
        &[
            toggle("gamepad.enabled", "setting.gamepad.enabled.label", Some("setting.gamepad.enabled.hint"), None),
            toggle(
                "gamepad.guide_closes_app",
                "setting.gamepad.guide_closes_app.label",
                Some("setting.gamepad.guide_closes_app.hint"),
                Some("gamepad.enabled"),
            ),
            toggle("cast.airplay", "setting.cast.airplay.label", Some("setting.cast.airplay.hint"), None),
        ],
    ),
    section(
        "section.overlay",
        &[
            Item {
                key: "",
                label: "setting.overlay.live.label",
                hint: Some("setting.overlay.live.hint"),
                kind: Kind::Overlay,
                needs: None,
            },
            choice(
                "overlay.position",
                "setting.overlay.position.label",
                &[
                    (Lit::Str("top"), Key("option.overlay.position.top")),
                    (Lit::Str("bottom"), Key("option.overlay.position.bottom")),
                ],
                None,
            ),
        ],
    ),
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

    /// The catalog's code for the TV's language.
    fn lang(&self) -> &'static str {
        let lang = self.settings["locale"]["language"].as_str().unwrap_or("en");
        rustv_locales::LANGUAGES.iter().find(|known| **known == lang).copied().unwrap_or("en")
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
                        let label = rustv_locales::format(self.lang(), "unit.hertz", &[("n", rate)]);
                        choices.push((format!("{other}@{rate}"), label));
                    }
                }
                let at = choices.iter().position(|(mode, _)| *mode == format!("{size}@{hz}"));
                (choices, at)
            }
            _ => (Vec::new(), None),
        }
    }

    /// Resolution and refresh rate only once tvd reports the TV's modes.
    fn shown(&self, item: &Item) -> bool {
        !matches!(item.kind, Kind::Resolution | Kind::Refresh) || !self.modes().is_empty()
    }

    fn enabled(&self, item: &Item) -> bool {
        item.needs.map_or(true, |key| self.get(key) == &Value::Bool(true))
    }

    /// The options a choice offers now: "My pictures" only once there are pictures.
    fn options(&self, item: &Item) -> Vec<(Lit, Words)> {
        let Kind::Choice(options) = item.kind else { return Vec::new() };
        let pictures = self.settings["wallpaper"]["count"].as_u64().unwrap_or(0) > 0;
        options.iter().copied().filter(|(lit, _)| pictures || !matches!(lit, Lit::Str("images"))).collect()
    }

    fn focusable(&self) -> Vec<&'static Item> {
        self.sections()
            .flat_map(|section| section.items)
            .filter(|item| self.shown(item) && self.enabled(item))
            .collect()
    }

    /// The list as the UI draws it, with the focused row kept in view.
    pub fn rows(&mut self) -> Vec<Row> {
        let lang = self.lang();
        let twelve_hour = self.settings["locale"]["clock"] == "12h";
        let focused = self.focusable().get(self.selected).map(|item| *item as *const Item);
        let mut rows = Vec::new();
        let mut y = 0.0;
        let sections: Vec<&'static Section> = self.sections().collect();
        for section in sections {
            rows.push(Row {
                header: true,
                label: rustv_locales::text(lang, section.title).into(),
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
            let items: Vec<&'static Item> = section.items.iter().filter(|item| self.shown(item)).collect();
            for item in items {
                let options = self.options(item);
                let current = self.get(item.key);
                let at = options.iter().position(|(lit, _)| lit.matches(current));
                let (choices, chosen) = self.mode_choices(&item.kind);
                let (kind, value) = match item.kind {
                    Kind::Toggle | Kind::Overlay => ("toggle", String::new()),
                    Kind::Action(_) => ("action", String::new()),
                    Kind::Choice(_) => {
                        ("choice", at.map_or_else(|| shown(current), |i| options[i].1.read(lang, twelve_hour)))
                    }
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
                    label: rustv_locales::text(lang, item.label).into(),
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
        let item = self.focusable().get(self.selected).copied();
        item.and_then(|item| item.hint).map_or_else(String::new, |hint| rustv_locales::text(self.lang(), hint).into())
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

    /// Moves down to the row with this label; fails instead of looping if there is none.
    fn go_to(menu: &mut Menu, label: &str) {
        for _ in 0..100 {
            if focused(menu).label == label {
                return;
            }
            menu.key("down");
        }
        panic!("no row {label:?}");
    }

    #[test]
    fn night_times_read_as_the_clock_shows_them() {
        let mut menu = menu();
        let mut settings = menu.settings.clone();
        settings["night"] =
            json!({ "enabled": true, "from": "22:00", "to": "07:00", "brightness": 0.7, "warmth": 0.5 });
        menu.set_settings(settings.clone());
        go_to(&mut menu, "From");
        assert_eq!(focused(&mut menu).value, "22:00");
        assert_eq!(menu.key("right"), Outcome::Send(json!({ "type": "set", "key": "night.from", "value": "23:00" })));
        settings["locale"]["clock"] = json!("12h");
        menu.set_settings(settings);
        assert_eq!(focused(&mut menu).value, "10 PM");
        assert_eq!(clock_time("00:30", true), "12:30 AM");
        assert_eq!(clock_time("12:00", true), "12 PM");
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
        go_to(&mut menu, "Start after");
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
        go_to(&mut menu, "Sounds");
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
        // Brightness is always there; resolution only with the TV's modes.
        assert!(menu.rows().iter().any(|row| row.label == "Brightness"));
        assert!(!menu.rows().iter().any(|row| row.label == "Resolution"));
        let mut settings = menu.settings.clone();
        settings["display"] = json!({
            "mode": "auto", "current": "1920x1080@60",
            "modes": ["3840x2160@60", "3840x2160@30", "1920x1080@120", "1920x1080@60", "1280x720@60"]
        });
        menu.set_settings(settings);
        go_to(&mut menu, "Resolution");
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
        go_to(&mut menu, "Put the TV to sleep now");
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
