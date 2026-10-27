//! What this box exposes to the network: every TCP port listening and every
//! UDP port bound on a non-loopback address, read from /proc/net, with the
//! owning program where Linux lets us see it (our own processes).
//!
//! This is a read-only view. Ports change in each program's own config (for
//! rustv, `server.bind`); a firewall decides who can actually reach them.

use std::collections::{BTreeMap, HashMap};
use std::fs;

use serde::Serialize;

use crate::i18n::Label;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Exposure {
    pub protocol: &'static str,
    pub port: u16,
    /// "all networks", or the one address it listens on.
    pub address: String,
    /// What it is, from the program name or the port.
    pub service: String,
    /// The two as catalog keys where rustv words them, for each screen to translate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_label: Option<Label>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_label: Option<Label>,
}

/// Addresses one (protocol, port) listens on, and who owns it.
#[derive(Default)]
struct Found {
    addresses: Vec<String>,
    program: Option<String>,
}

pub fn exposures() -> Vec<Exposure> {
    let programs = socket_programs();
    let mut found: BTreeMap<(&'static str, u16), Found> = BTreeMap::new();
    for (file, protocol, v6) in [
        ("/proc/net/tcp", "tcp", false),
        ("/proc/net/tcp6", "tcp", true),
        ("/proc/net/udp", "udp", false),
        ("/proc/net/udp6", "udp", true),
    ] {
        let Ok(text) = fs::read_to_string(file) else { continue };
        for socket in parse_table(&text, protocol == "tcp", v6) {
            let entry = found.entry((protocol, socket.port)).or_default();
            if !entry.addresses.contains(&socket.address) {
                entry.addresses.push(socket.address);
            }
            if entry.program.is_none() {
                entry.program = programs.get(&socket.inode).cloned();
            }
        }
    }
    found
        .into_iter()
        // UDP sockets on high ports with no well-known service are almost always the client
        // side of something (DNS lookups, QUIC); they accept nothing from strangers.
        .filter(|((protocol, port), _)| {
            !(*protocol == "udp" && *port >= 32768 && well_known(protocol, *port).is_none())
        })
        .map(|((protocol, port), found)| {
            let (address, address_label) = addresses(&found.addresses);
            let (service, service_label) = describe(protocol, port, found.program.as_deref());
            Exposure { protocol, port, address, service, address_label, service_label }
        })
        .collect()
}

struct Socket {
    address: String,
    port: u16,
    inode: u64,
}

fn addresses(list: &[String]) -> (String, Option<Label>) {
    let worded = |label: Label| (label.english(), Some(label));
    if list.iter().any(|a| a == "*") {
        return worded(Label::new("network.address.all", &[]));
    }
    match list {
        [one] => (one.clone(), None),
        [a, b] => (format!("{a}, {b}"), None),
        [first, rest @ ..] => {
            worded(Label::new("network.address.more", &[("first", first.clone()), ("count", rest.len().to_string())]))
        }
        [] => (String::new(), None),
    }
}

/// Rows of /proc/net/{tcp,udp}[6] that listen on something other than loopback.
fn parse_table(text: &str, tcp: bool, v6: bool) -> Vec<Socket> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (local, state, inode) = (fields.get(1)?, fields.get(3)?, fields.get(9)?);
            // TCP: LISTEN only. UDP: bound and not connected to one peer.
            if (tcp && *state != "0A") || (!tcp && *state != "07") {
                return None;
            }
            let (ip, port) = local.split_once(':')?;
            let address = address(ip, v6)?;
            Some(Socket { address, port: u16::from_str_radix(port, 16).ok()?, inode: inode.parse().ok()? })
        })
        .collect()
}

/// "*" for any address, None for loopback, otherwise the address.
fn address(hex: &str, v6: bool) -> Option<String> {
    if v6 {
        if hex.chars().all(|c| c == '0') {
            return Some("*".into());
        }
        // ::1, stored as four little-endian words.
        if hex == "00000000000000000000000001000000" {
            return None;
        }
        // IPv4-mapped (::ffff:a.b.c.d) shows as IPv4.
        if let Some(v4) = hex.strip_prefix("0000000000000000FFFF0000") {
            return address(v4, false);
        }
        let words: Vec<u32> = (0..4).filter_map(|i| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok()).collect();
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let segments: Vec<String> =
            bytes.chunks(2).map(|c| format!("{:x}", u16::from_be_bytes([c[0], c[1]]))).collect();
        return Some(segments.join(":"));
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let [a, b, c, d] = value.to_le_bytes();
    match (a, b, c, d) {
        (0, 0, 0, 0) => Some("*".into()),
        (127, ..) => None,
        _ => Some(format!("{a}.{b}.{c}.{d}")),
    }
}

/// Socket inode -> program name, for processes we may look into.
fn socket_programs() -> HashMap<u64, String> {
    let mut programs = HashMap::new();
    let Ok(procs) = fs::read_dir("/proc") else { return programs };
    for proc in procs.flatten() {
        let Ok(fds) = fs::read_dir(proc.path().join("fd")) else { continue };
        let Some(name) = fs::read_to_string(proc.path().join("comm")).ok().map(|n| n.trim().to_string()) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = fs::read_link(fd.path()) else { continue };
            let target = target.to_string_lossy();
            if let Some(inode) = target.strip_prefix("socket:[").and_then(|t| t.strip_suffix(']')) {
                if let Ok(inode) = inode.parse() {
                    programs.insert(inode, name.clone());
                }
            }
        }
    }
    programs
}

/// A name, or a catalog key (`network.service.*`) for one rustv words itself.
#[derive(Clone, Copy)]
enum Name {
    Plain(&'static str),
    Worded(&'static str),
}

fn well_known(protocol: &str, port: u16) -> Option<Name> {
    use Name::*;
    match (protocol, port) {
        ("tcp", 22) => Some(Plain("SSH")),
        ("tcp", 8765) => Some(Worded("network.service.rustv")),
        ("tcp", 7000 | 7001 | 7100) | ("udp", 6000 | 6001 | 7011) => Some(Plain("AirPlay")),
        ("udp", 5353) => Some(Worded("network.service.mdns")),
        ("udp", 68) => Some(Worded("network.service.dhcp")),
        ("udp", 546) => Some(Worded("network.service.dhcpv6")),
        ("udp", 41641) => Some(Plain("Tailscale")),
        ("tcp", 53) | ("udp", 53) => Some(Plain("DNS")),
        ("tcp", 631) | ("udp", 631) => Some(Worded("network.service.cups")),
        _ => None,
    }
}

fn describe(protocol: &str, port: u16, program: Option<&str>) -> (String, Option<Label>) {
    let name = |name: Name| match name {
        Name::Plain(text) => (text.to_string(), None),
        Name::Worded(key) => {
            let label = Label::new(key, &[]);
            (label.english(), Some(label))
        }
    };
    let by_program = program.map(|p| match p {
        "qbittorrent-nox" if port == 8080 => name(Name::Worded("network.service.qbittorrent")),
        "qbittorrent-nox" => ("qBittorrent (BitTorrent)".to_string(), None),
        "uxplay" => ("AirPlay".to_string(), None),
        "tvd" => name(Name::Worded("network.service.rustv")),
        "avahi-daemon" => name(Name::Worded("network.service.mdns")),
        other => (other.to_string(), None),
    });
    by_program.or_else(|| well_known(protocol, port).map(name)).unwrap_or_else(|| {
        let label =
            Label::new("network.service.port", &[("protocol", protocol.to_string()), ("port", port.to_string())]);
        (label.english(), Some(label))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:223D 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1 0 100 0 0 10 0
   1: 0100007F:0CEB 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 222 1 0 100 0 0 10 0
   2: EC01A8C0:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 333 1 0 100 0 0 10 0
   3: EC01A8C0:D2F0 0E01A8C0:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 444 1 0 100 0 0 10 0
";

    #[test]
    fn lists_listening_sockets_but_not_loopback_or_connections() {
        let sockets = parse_table(TCP, true, false);
        let seen: Vec<(String, u16)> = sockets.into_iter().map(|s| (s.address, s.port)).collect();
        assert_eq!(seen, [("*".to_string(), 8765), ("192.168.1.236".to_string(), 22)]);
    }

    #[test]
    fn reads_ipv6_addresses() {
        assert_eq!(address("00000000000000000000000000000000", true).as_deref(), Some("*"));
        assert_eq!(address("00000000000000000000000001000000", true), None);
        assert_eq!(address("0000000000000000FFFF0000EC01A8C0", true).as_deref(), Some("192.168.1.236"));
    }

    #[test]
    fn shortens_long_address_lists() {
        let list = ["10.0.0.1", "10.0.0.2", "10.0.0.3"].map(String::from);
        assert_eq!(addresses(&list).0, "10.0.0.1 and 2 more addresses");
        assert_eq!(addresses(&["*".to_string()]).0, "all networks");
    }

    #[test]
    fn names_services() {
        assert_eq!(describe("tcp", 8080, Some("qbittorrent-nox")).0, "qBittorrent web UI");
        assert_eq!(describe("tcp", 22, None).0, "SSH");
        assert_eq!(describe("udp", 9999, None).0, "udp port 9999");
    }
}
