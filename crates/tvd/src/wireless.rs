//! Wi-Fi and Bluetooth from the remote, through NetworkManager (`nmcli`) and
//! BlueZ (`bluetoothctl`).
//!
//! Changing Wi-Fi needs a polkit rule when tvd runs as a background service
//! (see packaging/polkit). Bluetooth needs nothing extra.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

// ---------------- Wi-Fi ----------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Network {
    pub ssid: String,
    /// 0..100.
    pub signal: u8,
    pub secure: bool,
    pub connected: bool,
}

/// Networks in range, strongest first, one entry per name.
pub async fn networks(rescan: bool) -> Result<Vec<Network>> {
    let rescan = if rescan { "yes" } else { "auto" };
    let out =
        run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", rescan]).await?;
    Ok(parse_networks(&out))
}

/// Joins a network. A saved one needs no password.
pub async fn connect(ssid: &str, password: Option<&str>) -> Result<()> {
    let mut args = vec!["dev", "wifi", "connect", ssid];
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        // nmcli takes the password as an argument; on a single-user TV box only
        // root and this user could read it from the process list, for a moment.
        args.extend(["password", password]);
    }
    run("nmcli", &args).await.map(drop).map_err(|err| {
        let text = err.to_string();
        if text.contains("Secrets were required") || text.contains("802-1x") || text.contains("property is invalid") {
            crate::i18n::error("error.wifi.wrong_password", &[])
        } else if text.contains("Not authorized") || text.contains("not authorized") {
            crate::i18n::error("error.wifi.not_allowed", &[])
        } else {
            err
        }
    })
}

fn parse_networks(text: &str) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for line in text.lines() {
        let fields = split_terse(line);
        let [in_use, ssid, signal, security] = fields.as_slice() else { continue };
        if ssid.is_empty() {
            continue; // hidden network
        }
        let network = Network {
            ssid: ssid.clone(),
            signal: signal.parse().unwrap_or(0),
            secure: !security.is_empty() && security != "--",
            connected: in_use == "*",
        };
        match networks.iter_mut().find(|n| n.ssid == network.ssid) {
            Some(existing) => {
                existing.connected |= network.connected;
                existing.signal = existing.signal.max(network.signal);
            }
            None => networks.push(network),
        }
    }
    networks.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal.cmp(&a.signal)));
    networks
}

/// nmcli -t separates fields with ':' and escapes ':' and '\' inside values.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields.last_mut().unwrap().extend(chars.next()),
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

// ---------------- Bluetooth ----------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Device {
    pub address: String,
    pub name: String,
    /// BlueZ icon name, such as `audio-headset` or `input-gaming`.
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bluetooth {
    pub available: bool,
    pub powered: bool,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BluetoothAction {
    PowerOn,
    PowerOff,
    /// Look for devices for a few seconds.
    Scan,
    /// Pair, trust and connect a new device.
    Pair,
    Connect,
    Disconnect,
    /// Forget a paired device.
    Remove,
}

pub async fn bluetooth() -> Bluetooth {
    let Ok(show) = run("bluetoothctl", &["show"]).await else {
        return Bluetooth { available: false, powered: false, devices: Vec::new() };
    };
    let powered = show.lines().any(|l| l.trim() == "Powered: yes");
    let mut devices = Vec::new();
    if let Ok(list) = run("bluetoothctl", &["devices"]).await {
        for (address, name) in parse_devices(&list) {
            let info = run("bluetoothctl", &["info", &address]).await.unwrap_or_default();
            let field = |key: &str| {
                info.lines().find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string())).unwrap_or_default()
            };
            let device = Device {
                icon: field("Icon:"),
                paired: field("Paired:") == "yes",
                connected: field("Connected:") == "yes",
                address,
                name,
            };
            // Unpaired devices with no name are mostly the neighbours' beacons and gadgets.
            if device.paired || !unnamed(&device) {
                devices.push(device);
            }
        }
    }
    // Connected, then paired, then named; devices known only by address last.
    devices.sort_by_key(|d| (!d.connected, !d.paired, unnamed(d)));
    Bluetooth { available: true, powered, devices }
}

pub async fn bluetooth_action(action: BluetoothAction, address: Option<&str>) -> Result<()> {
    let address = || address.filter(|a| is_address(a)).context("which device? (address like AA:BB:CC:DD:EE:FF)");
    match action {
        BluetoothAction::PowerOn => run("bluetoothctl", &["power", "on"]).await.map(drop),
        BluetoothAction::PowerOff => run("bluetoothctl", &["power", "off"]).await.map(drop),
        BluetoothAction::Scan => run("bluetoothctl", &["--timeout", "8", "scan", "on"]).await.map(drop),
        BluetoothAction::Pair => {
            let address = address()?;
            run("bluetoothctl", &["pairable", "on"]).await?;
            // BlueZ forgets devices it only saw in a search about 30 s after the search
            // ends; look again so a slow tap on Pair still finds it.
            if run("bluetoothctl", &["info", address]).await.is_err() {
                run("bluetoothctl", &["--timeout", "8", "scan", "on"]).await?;
            }
            // The agent answers "yes" to Just Works pairing, which headphones and gamepads use.
            run("bluetoothctl", &["--agent", "NoInputNoOutput", "--timeout", "25", "pair", address]).await?;
            run("bluetoothctl", &["trust", address]).await?;
            run("bluetoothctl", &["--timeout", "15", "connect", address]).await.map(drop)
        }
        BluetoothAction::Connect => run("bluetoothctl", &["--timeout", "15", "connect", address()?]).await.map(drop),
        BluetoothAction::Disconnect => run("bluetoothctl", &["disconnect", address()?]).await.map(drop),
        BluetoothAction::Remove => run("bluetoothctl", &["remove", address()?]).await.map(drop),
    }
}

/// BlueZ names a device after its address until it says its own name.
fn unnamed(device: &Device) -> bool {
    device.name.replace('-', ":") == device.address
}

fn parse_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, rest));
            is_address(address).then(|| (address.to_string(), name.trim().to_string()))
        })
        .collect()
}

fn is_address(text: &str) -> bool {
    text.len() == 17
        && text.split(':').count() == 6
        && text.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

async fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).output().await.with_context(|| format!("running {program}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // bluetoothctl reports failures on stdout and still exits 0 in places.
    let failed = !out.status.success()
        || stdout.lines().any(|l| l.contains("Failed to") || l.starts_with("Error") || l.contains("not available"));
    if failed {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let message = stderr.lines().chain(stdout.lines()).map(str::trim).find(|l| !l.is_empty()).unwrap_or("failed");
        bail!("{program}: {message}");
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wifi_networks() {
        let text = " :OpenWrt-IoT:72:WPA2\n:OpenWrt:69:WPA3\n*:OpenWrt:64:WPA3\n :Cafe\\: free:40:\n :::\n";
        let networks = parse_networks(text);
        assert_eq!(networks[0], Network { ssid: "OpenWrt".into(), signal: 69, secure: true, connected: true });
        assert_eq!(networks[1].ssid, "OpenWrt-IoT");
        assert_eq!(networks[2], Network { ssid: "Cafe: free".into(), signal: 40, secure: false, connected: false });
        assert_eq!(networks.len(), 3);
    }

    #[test]
    fn reads_bluetooth_devices() {
        let text =
            "Device 74:77:86:69:EB:FE Arctis Nova 7\nDevice C0:11:22:33:44:55 C0-11-22-33-44-55\nController 00:00 x\n";
        let devices = parse_devices(text);
        assert_eq!(devices[0], ("74:77:86:69:EB:FE".to_string(), "Arctis Nova 7".to_string()));
        assert_eq!(devices.len(), 2);
        assert!(!is_address("74:77:86:69:EB"));
    }
}
