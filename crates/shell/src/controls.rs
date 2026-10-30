//! Controller-friendly views of the same data and actions used by the web remote.
use serde_json::{json, Value};
pub type Rows = Vec<(String, Value)>;
pub fn rows(panel: &str, data: &Value, lang: &str) -> Rows {
    let t = |key| rustv_locales::text(lang, key).to_string();
    let mut rows = Vec::new();
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    match panel {
        "audio" => {
            for output in list(&data["outputs"]) {
                rows.push((
                    format!(
                        "{}{}",
                        output["name"].as_str().unwrap_or(""),
                        if output["default"] == true { " ✓" } else { "" }
                    ),
                    json!({"type":"audio_output","id":output["id"]}),
                ));
            }
            if rows.is_empty() {
                rows.push((t("remote.output.empty"), Value::Null));
            }
        }
        "apps" => {
            for app in list(data) {
                if app["hidden"] == true {
                    rows.push((
                        app["name"].as_str().unwrap_or("").into(),
                        json!({"type":"hide_app","id":app["id"],"hidden":false}),
                    ));
                }
            }
            if rows.is_empty() {
                rows.push((t("tv.controls.no_hidden"), Value::Null));
            }
        }
        "wifi" => {
            for network in list(data) {
                rows.push((
                    format!(
                        "{} · {}%{}",
                        network["ssid"].as_str().unwrap_or(""),
                        network["signal"],
                        if network["connected"] == true { " ✓" } else { "" }
                    ),
                    json!({"type":"_wifi","ssid":network["ssid"],"secure":network["secure"]}),
                ));
            }
            if rows.is_empty() {
                rows.push((t("remote.wifi.unavailable"), Value::Null));
            }
        }
        "bluetooth" => {
            if data["available"] != true {
                rows.push((t("remote.bluetooth.no_adapter"), Value::Null));
            } else {
                rows.push((
                    t(if data["powered"] == true { "tv.controls.bluetooth_off" } else { "tv.controls.bluetooth_on" }),
                    json!({"type":"bluetooth","action":if data["powered"] == true {"power_off"} else {"power_on"}}),
                ));
                rows.push((t("remote.bluetooth.search"), json!({"type":"bluetooth","action":"scan"})));
                for device in list(&data["devices"]) {
                    for (key, action) in if device["connected"] == true {
                        vec![("remote.bluetooth.disconnect", "disconnect"), ("remote.bluetooth.forget", "remove")]
                    } else if device["paired"] == true {
                        vec![("common.connected", "connect"), ("remote.bluetooth.forget", "remove")]
                    } else {
                        vec![("remote.bluetooth.pair", "pair")]
                    } {
                        rows.push((
                            format!("{} · {}", device["name"].as_str().unwrap_or(""), t(key)),
                            json!({"type":"bluetooth","action":action,"address":device["address"]}),
                        ));
                    }
                }
            }
        }
        "network" => {
            for port in list(data) {
                rows.push((
                    format!(
                        "{} {} · {} · {}",
                        port["protocol"].as_str().unwrap_or(""),
                        port["port"],
                        port["address"].as_str().unwrap_or(""),
                        port["service"].as_str().unwrap_or("")
                    ),
                    Value::Null,
                ));
            }
        }
        "system" => {
            for key in ["host", "version", "kernel"] {
                rows.push((data[key].as_str().unwrap_or("").into(), Value::Null));
            }
            for (label, key, unit) in [
                ("remote.system.uptime", "uptime_secs", "s"),
                ("remote.system.temperature", "cpu_temp", "°C"),
                ("remote.system.temperature", "gpu_temp", "°C"),
            ] {
                if !data[key].is_null() {
                    rows.push((format!("{}: {} {unit}", t(label), data[key]), Value::Null));
                }
            }
            if let Some(memory) = data["memory"].as_array() {
                if memory.len() == 2 {
                    rows.push((
                        format!(
                            "{}: {} / {} MiB",
                            t("remote.system.memory"),
                            memory[0].as_u64().unwrap_or(0) / 1048576,
                            memory[1].as_u64().unwrap_or(0) / 1048576
                        ),
                        Value::Null,
                    ));
                }
            }
            for disk in list(&data["disk"]) {
                rows.push((
                    format!(
                        "{} {}: {} / {} GiB",
                        t("remote.system.disk"),
                        disk["path"].as_str().unwrap_or(""),
                        disk["used"].as_u64().unwrap_or(0) / 1073741824,
                        disk["total"].as_u64().unwrap_or(0) / 1073741824
                    ),
                    Value::Null,
                ));
            }
            for line in list(&data["log"]) {
                rows.push((line.as_str().unwrap_or("").into(), Value::Null));
            }
        }
        "timer" => {
            for minutes in [0, 15, 30, 45, 60, 90, 120] {
                rows.push((
                    if minutes == 0 { t("remote.timer.off_button") } else { format!("{minutes} min") },
                    json!({"type":"sleep_timer","minutes":minutes}),
                ));
            }
        }
        "controls" => {
            for (key, command) in [
                ("tv.controls.volume_value", json!({"type":"_volume"})),
                ("remote.volume.mute", json!({"type":"volume","action":"mute"})),
                ("tv.controls.volume_up", json!({"type":"volume","action":"up"})),
                ("tv.controls.volume_down", json!({"type":"volume","action":"down"})),
                ("remote.media.play", json!({"type":"media","action":"play_pause"})),
                ("remote.media.previous", json!({"type":"media","action":"previous"})),
                ("remote.media.next", json!({"type":"media","action":"next"})),
                ("remote.media.back_10", json!({"type":"media","action":"seek","seconds":-10})),
                ("remote.media.forward_10", json!({"type":"media","action":"seek","seconds":10})),
                ("section.keyboard", json!({"type":"keyboard","open":true})),
                ("tv.controls.pointer", json!({"type":"pointer_mode","enabled":true})),
                ("remote.close_app", json!({"type":"stop"})),
            ] {
                rows.push((t(key), command));
            }
        }
        _ => {}
    }
    if !matches!(panel, "timer" | "controls") {
        rows.push((t("remote.system.refresh"), json!({"type":format!("_panel_{panel}")})));
    }
    rows
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_only_hidden_apps_and_keep_stable_ids() {
        let rows = rows("apps", &json!([{"id":"x","name":"X","hidden":true},{"id":"y","hidden":false}]), "en");
        assert_eq!(rows[0].1, json!({"type":"hide_app","id":"x","hidden":false}));
        assert_eq!(rows.len(), 2);
    }
    #[test]
    fn wifi_defers_password_entry_and_bluetooth_uses_device_address() {
        assert_eq!(rows("wifi", &json!([{"ssid":"test","signal":75,"secure":true}]), "en")[0].1["type"], "_wifi");
        let rows = rows(
            "bluetooth",
            &json!({"available":true,"powered":true,"devices":[{"name":"Pad","address":"AA","paired":false}]}),
            "en",
        );
        assert_eq!(rows[2].1, json!({"type":"bluetooth","action":"pair","address":"AA"}));
    }
}
