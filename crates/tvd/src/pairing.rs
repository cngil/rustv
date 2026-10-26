//! Pairing a phone: the link with the token, as text and as a QR code.
//!
//! The TV shows the code until the first remote connects, and again for a
//! couple of minutes whenever a paired remote asks for it. Being able to see
//! the TV is the proof that you belong in the room.

use std::net::UdpSocket;
use std::path::PathBuf;

use qrcode::render::unicode::Dense1x2;
use qrcode::{Color, QrCode};

/// `http://<lan ip>:<port>/#token=…`. The token sits in the fragment, which
/// browsers never send to the server or put in logs.
pub fn url(bind: &str, token: &str) -> String {
    let port = bind.rsplit(':').next().unwrap_or("8765");
    let ip = lan_ip().unwrap_or_else(|| "<device-ip>".into());
    format!("http://{ip}:{port}/#token={token}")
}

/// Primary LAN address. Connecting a UDP socket sends no packets; it only
/// makes the kernel pick the outgoing interface.
fn lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// QR modules, row by row, `true` for dark.
pub struct Matrix {
    pub size: usize,
    pub dark: Vec<bool>,
}

pub fn matrix(text: &str) -> Option<Matrix> {
    let code = QrCode::new(text.as_bytes()).ok()?;
    Some(Matrix { size: code.width(), dark: code.to_colors().into_iter().map(|c| c == Color::Dark).collect() })
}

/// The code drawn with half blocks, light on dark, for `tvd pair` in a terminal.
pub fn terminal(text: &str) -> Option<String> {
    let code = QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).quiet_zone(true).build())
}

/// Remembers that a remote connected with the current token, so the TV stops
/// offering the code on its own. A new token (delete the token file) brings it back.
pub struct Marker {
    path: Option<PathBuf>,
    fingerprint: String,
}

impl Marker {
    pub fn new(token: &str) -> Self {
        let path = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
            .map(|dir| dir.join("rustv/paired"));
        Self { path, fingerprint: fingerprint(token) }
    }

    pub fn is_paired(&self) -> bool {
        let Some(path) = &self.path else { return false };
        std::fs::read_to_string(path).is_ok_and(|text| text.trim() == self.fingerprint)
    }

    pub fn set_paired(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, format!("{}\n", self.fingerprint));
    }
}

/// FNV-1a: stable across Rust versions, and the file never holds the token itself.
fn fingerprint(token: &str) -> String {
    let hash = token.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_pairing_links() {
        let link = "http://192.168.1.236:8765/#token=ae9e7f8f61d65e5c90e60e5abb58914c";
        let qr = matrix(link).unwrap();
        assert_eq!(qr.dark.len(), qr.size * qr.size);
        // Finder pattern: the top-left corner module is always dark.
        assert!(qr.dark[0]);
        assert!(terminal(link).unwrap().lines().count() > 10);
    }

    #[test]
    fn fingerprints_are_stable() {
        assert_eq!(fingerprint(""), "cbf29ce484222325");
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }
}
