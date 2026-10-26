<p align="center">
  <img src="crates/tvd/web/icon.svg" width="96" alt="rustv">
</p>

<h1 align="center">rustv</h1>

<p align="center">
  A private, open-source TV shell for Linux.<br>
  Your living room without ads, tracking or lock-in.
</p>

---

Smart TV platforms like Tizen and Android TV ship ads on the home screen,
watch what you play, push updates you did not ask for and get slower every
year. rustv turns any Linux box (an old laptop, a mini PC) into a TV front
end that you own: it shows the apps you choose, does what you configure, and
talks to nobody but you.

> **Status:** early development. The daemon, the web remote and a first
> on-screen shell work today. See the [roadmap](#roadmap).

## Principles

- **Private by design.** No telemetry, no accounts, no cloud, no analytics.
  rustv never connects to the internet; only the apps you launch do.
  The API listens on your LAN, protected by a token.
- **Yours to shape.** Every app, button and limit lives in one plain TOML
  file. Nothing is hardcoded, nothing is promoted, nothing is hidden.
- **Light.** Built for low-end hardware. The daemon is a ~3 MB binary that
  uses about 5 MB of RAM and no CPU while idle.
- **Open.** GPL-3.0. Read it, change it, fork it.
- **Scriptable.** One small JSON API for the remote, the shell, your scripts
  and, later, AI agents and voice control.

## What works today

- **On-screen shell** (`rustv-shell`, Rust + Slint): a home screen with big
  app cards, real app icons, a clock and a volume overlay, each switchable in
  the config. Arrow keys and OK from the remote are all you need; no pointer,
  no cursor.
- **Screensaver**: after a few idle minutes on the home screen, the time
  drifts over generated animation (soft drifting lights, a turning galaxy of
  points, or just a large clock), different each time. Nothing to set up, no
  images, no network; any key wakes it.
- **`tvd` daemon**: launches and closes apps, one at a time; when an app
  exits you are back home.
- **Three kinds of apps**: Flatpak apps, any command, and web apps (a URL
  opened full screen in a kiosk browser, handy for qBittorrent or router
  panels).
- **Web remote** for phone and computer, nothing to install:
  - D-pad with press-and-hold, Back, Home, Play
  - Touchpad with acceleration, tap to click, two-finger scroll and right click
  - On a computer: take over the TV pointer with your mouse (pointer lock)
    and forward your keyboard
  - App dock with icons, text input
  - One volume control with mute, and an output picker (your TV by name,
    headphones, Bluetooth) right under the D-pad
  - Shows progress and the result of every action (opening an app,
    switching the output, losing the connection)
  - Installable as an app (PWA)
- **Debug overlay**: one line of live stats on top of everything, even full
  screen video: CPU and GPU load and temperature, RAM, network speed, Wi-Fi
  signal, display mode, volume, connected remotes, uptime. Items turn yellow
  or red when something runs hot. Pick the items in the config, switch it
  from the remote or the API.
- **Virtual keyboard and mouse** through `uinput`, so control works in every
  app without app support.
- **Audio** through PipeWire: volume and output selection.
- **App icons**: Flatpak apps bring their own; any app can set one.

## How it works

```
 phone / computer / scripts
            │  WebSocket + REST (token)
            ▼
 ┌────────────────────────┐        ┌──────────────────────┐
 │ tvd (user service)     │ starts │ apps on the TV       │
 │  app manager           ├───────►│ Jellyfin, Stremio,   │
 │  uinput keyboard/mouse │        │ Moonlight, browser…  │
 │  PipeWire volume       │        └──────────────────────┘
 │  web remote, API       │                  ▲
 └────────────────────────┘                  │ Wayland
                                    ┌────────┴─────────┐
                                    │ sway (kiosk)     │
                                    │ + rustv-shell    │
                                    └──────────────────┘
```

`tvd` holds all logic. The shell is a thin client of the same API as the
remote, so a crashed screen never takes the remote down.

## Requirements

- Linux with [sway](https://swaywm.org) on the TV output, used as a kiosk
  (no bars, no decorations, no keybindings; see [`packaging/sway/rustv.conf`](packaging/sway/rustv.conf)).
  sway picks the output mode, which matters on 4K TVs that only do 30 Hz over HDMI 1.4.
  The debug overlay uses swaybar, which ships with sway.
- PipeWire (`wpctl`, `pw-dump`) for audio
- Write access to `/dev/uinput` (see below)
- Flatpak for Flatpak apps, Firefox (Flatpak) for web apps by default
- Rust 1.80+ and, for the shell, fontconfig and xkbcommon headers
  (`sudo apt install libfontconfig1-dev libxkbcommon-dev` on Debian/Ubuntu)

## Install

```sh
git clone https://github.com/cngil/rustv && cd rustv

# allow virtual input devices for your user
sudo install -m644 packaging/udev/60-rustv-uinput.rules /etc/udev/rules.d/
echo uinput | sudo tee /etc/modules-load.d/uinput.conf
sudo usermod -aG input "$USER"   # log in again afterwards

./scripts/install.sh             # builds, installs the user service, prints the pairing link
loginctl enable-linger "$USER"   # start at boot without logging in

# show the shell on the TV (edit YOUR_USER and the uid in the unit first)
install -Dm644 packaging/sway/rustv.conf ~/.config/rustv/sway.conf
install -Dm644 packaging/xdg-desktop-portal/portals.conf ~/.config/xdg-desktop-portal/portals.conf
sudo install -m644 packaging/systemd/rustv-session.service /etc/systemd/system/
sudo systemctl enable --now rustv-session
```

The `portals.conf` matters: without it, a box with no desktop environment
makes every Flatpak app wait ~25 seconds for a portal backend that never
starts.

Open the printed link (`http://<box>:8765/#token=…`) on your phone and add
it to the home screen. Run `tvd pair` any time to see it again.

## Configure

`~/.config/rustv/config.toml`, fully documented in
[`config/config.example.toml`](config/config.example.toml):

```toml
[[app]]
id = "jellyfin"
name = "Jellyfin"
kind = "flatpak"
app_id = "org.jellyfin.JellyfinDesktop"
args = ["--tv", "--fullscreen"]

[[app]]
id = "qbittorrent"
name = "qBittorrent"
kind = "web"
url = "http://localhost:8080"

[[app]]
id = "live-tv"
name = "Live TV"
kind = "command"
command = ["mpv", "--fs", "/home/you/iptv.m3u"]
```

Everything the shell shows can be switched off:

```toml
[shell]
greeting = true
clock = true
date = false
hints = false
volume_overlay = true
```

Check it with `tvd check`, then `systemctl --user restart tvd`.

## API

All endpoints except `/api/v1/health` need `Authorization: Bearer <token>`
(WebSocket clients may pass `?token=`). The token is in
`~/.config/rustv/token`.

| Method | Path | |
| --- | --- | --- |
| `GET` | `/api/v1/state` | Running app, volume, seconds since last input, overlay, connected clients |
| `GET` | `/api/v1/apps` | Configured apps |
| `GET` | `/api/v1/apps/{id}/icon` | App icon (SVG, PNG or JPEG) |
| `GET` | `/api/v1/audio` | Audio outputs and volume limits |
| `POST` | `/api/v1/command` | Run one command (below) |
| `GET` | `/api/v1/ws` | WebSocket: send commands, receive `apps`, `audio` and `state` updates |

Commands are JSON objects tagged by `type`:

```json
{"type": "launch", "id": "jellyfin"}
{"type": "stop"}
{"type": "key", "code": "ArrowDown"}
{"type": "key", "code": "KeyF", "ctrl": true}
{"type": "text", "text": "the expanse"}
{"type": "move", "dx": 40, "dy": -12}
{"type": "scroll", "dy": -2}
{"type": "button", "button": "left", "action": "click"}
{"type": "volume", "action": "up"}
{"type": "volume", "action": "set", "level": 0.6}
{"type": "audio_output", "id": 52}
{"type": "overlay", "visible": true}
```

Key codes follow the browser's
[`KeyboardEvent.code`](https://developer.mozilla.org/docs/Web/API/UI_Events/Keyboard_event_code_values).

```sh
curl -H "Authorization: Bearer $(cat ~/.config/rustv/token)" \
     -d '{"type":"launch","id":"jellyfin"}' http://localhost:8765/api/v1/command
```

## Security

The token gives full control of the TV's keyboard and mouse. Keep port 8765
on your LAN or a VPN such as Tailscale, and never forward it to the internet.
Delete `~/.config/rustv/token` and restart `tvd` to revoke all paired devices.

## Roadmap

- [x] Daemon: app launching, virtual input, volume, API
- [x] Web remote for phone and computer, with app dock and audio controls
- [x] On-screen shell (Rust + Slint): home screen, D-pad navigation, no cursor needed
- [x] App icons
- [x] Screensaver: generated animation with a drifting clock
- [ ] Now playing card, Spotify Connect through `spotifyd`
- [ ] Themes and shell settings in the config
- [ ] QR code pairing, mDNS discovery
- [ ] Agent integration (MCP) and voice commands
- [ ] Switching between running apps

### Backlog

- [ ] System info on screen, each item switchable on or off in user
  preferences: clock, date, weather, power consumption, battery, component
  temperatures, disk space, RAM, Wi-Fi/Ethernet status, network usage, and
  any other data the system exposes
- [ ] UI audio feedback (navigation and selection sounds)
- [ ] Manage Bluetooth, Wi-Fi and audio from the shell
- [ ] Lock and unlock, set a password, configure remotely exposed ports
- [ ] Reorder apps
- [ ] Localization options (UI language, date and time format, units)

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
RUST_LOG=debug cargo run -p tvd -- --config config/config.example.toml
```

## License

[GPL-3.0-or-later](LICENSE). The shell bundles the [Inter](https://rsms.me/inter/)
typeface, licensed under the [SIL Open Font License](crates/shell/ui/fonts/OFL.txt).
