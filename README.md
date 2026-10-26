<p align="center">
  <img src="crates/tvd/web/icon.svg" width="96" alt="rustv">
</p>

<h1 align="center">rustv</h1>

<p align="center">
  <a href="https://github.com/cngil/rustv/actions/workflows/ci.yml"><img src="https://github.com/cngil/rustv/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="GPL-3.0"></a>
</p>

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

**No new device needed.** rustv is just two small programs: `tvd`, a daemon
that runs your apps and serves the remote, and `rustv-shell`, the home
screen. Add them to the Linux machine you already have, plug it into the TV,
and your phone becomes the remote.

> **rustv is a shell, not a streaming service.** It plays nothing by itself
> and comes with no content, accounts or app store. It is the home screen,
> remote and glue around the Linux apps *you* choose to put on it. See
> [what people put on it](#what-to-put-on-it).

<p align="center">
  <img src="docs/screenshots/tv-home.png" alt="The rustv home screen on a TV, with app cards and the pairing QR code" width="900">
</p>

<table>
  <tr>
    <td width="33%"><img src="docs/screenshots/remote-phone.png" alt="The web remote on a phone: D-pad, volume, app dock"></td>
    <td width="33%"><img src="docs/screenshots/remote-settings.png" alt="The remote's settings panel"></td>
    <td width="33%">
      <img src="docs/screenshots/tv-screensaver-aurora.jpg" alt="Aurora screensaver with a drifting clock"><br><br>
      <img src="docs/screenshots/tv-screensaver-orbits.png" alt="Orbits screensaver"><br><br>
      <img src="docs/screenshots/tv-overlay.png" alt="Debug overlay with live system stats over the home screen">
    </td>
  </tr>
  <tr>
    <td align="center">Phone remote</td>
    <td align="center">Settings, saved to your config</td>
    <td align="center">Screensavers and the debug overlay</td>
  </tr>
</table>

<details>
<summary>The remote on a computer</summary>
<img src="docs/screenshots/remote-desktop.png" alt="The web remote in a desktop browser: D-pad, touchpad, keyboard and app dock side by side">
</details>

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
- **Lock with a PIN**: lock the TV from the remote (or automatically when it
  sleeps); until the PIN is entered on a remote, nothing works. For children
  and guests, not a network defense: the pairing token is that.
- **Network overview** in the remote's settings: every port the box listens
  on, on which address, and which program owns it.
- **Themes**: six color presets (indigo, ocean, sunset, forest, rose,
  graphite) or your own accent and background, picked from the remote; the
  TV and the remote change together.
- **Media keys** in the remote: play/pause, previous, next and ±10 s, sent
  to the player over MPRIS (Firefox, Jellyfin, VLC…) or as keys otherwise.
- **Sleep timer** from the remote: 15 minutes to 2 hours.
- **Volume per app**: each app and the home screen keep their own level, and
  a volume bar shows over apps too (with [wob](https://github.com/francma/wob)
  installed).
- **Languages**: English and Turkish on the TV, the remote and the overlay,
  with a 24 or 12 hour clock and °C or °F, switchable from the remote.
- **UI sounds**: soft, generated clicks when moving between apps and opening
  one (no audio files shipped). Level or off in the settings.
- **QR pairing**: the home screen shows a QR code until the first phone
  connects; scan it and the phone is a remote. To add another device, tap
  "Show code" in the remote's settings. `tvd pair` prints the same code in
  the terminal.
- **Screensaver**: after a few idle minutes on the home screen, the time
  drifts over generated animation (soft drifting lights, a turning galaxy of
  points, or just a large clock), different each time. Nothing to set up, no
  images, no network; any key wakes it.
- **AirPlay**: mirror an iPhone, iPad or Mac, or play its videos and music
  on the TV, through [UxPlay](https://github.com/FDH2/UxPlay). Off by
  default; turn it on in the config or the remote's settings.
- **Sleep**: after an hour with no remote input and no sound, the screen
  turns off (most TVs then go to standby) and the open app closes, freeing
  its memory and CPU. Any button wakes it. A film playing never counts as
  unused. The box itself stays on, so the remote can always wake it.
- **Reorder apps** from the remote: hold an app in the dock, drag it, tap
  Done. The TV follows at once, and the order survives restarts.
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
  - Settings panel (gear button): home screen items, screensaver, volume
    limit and step, debug overlay. Changes apply at once and are written
    into your `config.toml`, comments and all
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

## What to put on it

Anything that runs on Linux can be a tile: a Flatpak, a command or a web
page. Some setups that work well:

| You want | Put on it | Kind |
| --- | --- | --- |
| Your own film and series library | [Jellyfin](https://jellyfin.org) Desktop (`org.jellyfin.JellyfinDesktop`) with a Jellyfin server at home | flatpak |
| PC games on the TV | [Moonlight](https://moonlight-stream.org) (`com.moonlight_stream.Moonlight`) streaming from your gaming PC with Sunshine | flatpak |
| Local files, network shares, IPTV lists | [VLC](https://www.videolan.org/vlc/) (`org.videolan.VLC`) or [mpv](https://mpv.io) (`io.mpv.Mpv`) | flatpak, or a command such as `mpv --fs list.m3u` |
| YouTube without ads or tracking | [FreeTube](https://freetubeapp.io) (`io.freetubeapp.FreeTube`) | flatpak |
| Steam games from another PC | [Steam Link](https://store.steampowered.com/remoteplay) (`com.valvesoftware.SteamLink`) | flatpak |
| Plex or Kodi, if that is your library | Plex HTPC (`tv.plex.PlexHTPC`), Kodi (`tv.kodi.Kodi`) | flatpak |
| Retro games | [RetroArch](https://www.retroarch.com) (`org.libretro.RetroArch`) or small games such as KDE's Kapman and KBlocks | flatpak |
| Dashboards and web UIs | Home Assistant, qBittorrent, your router, any website | web |
| Phone and Mac screens | AirPlay, built in (`[cast] airplay = true`) | built in |

The author's own box runs Jellyfin for films and series and Moonlight for
games, plus qBittorrent's web UI and a few retro games. Start with the one
or two apps you actually use; every tile is three lines in the config.

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
- For a volume bar over apps (optional): `wob`
- For AirPlay (optional): `uxplay` and `avahi-daemon`
  (`sudo apt install uxplay avahi-daemon gstreamer1.0-plugins-bad gstreamer1.0-libav`),
  and, behind a firewall, TCP 7000, 7001, 7100 and UDP 5353, 6000, 6001, 7011 open to your LAN
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

Optional, recommended: with Avahi the remote opens at
`http://<hostname>.local:8765` and pairing keeps working if the box gets a
new IP. The service file also announces it to apps that browse the network:

```sh
sudo apt install avahi-daemon
sudo install -m644 packaging/avahi/rustv.service /etc/avahi/services/
```

The `portals.conf` matters: without it, a box with no desktop environment
makes every Flatpak app wait ~25 seconds for a portal backend that never
starts.

Scan the QR code on the TV (or the one `tvd pair` prints) with your phone,
or open the printed link (`http://<box>:8765/#token=…`), and add the page to
the home screen. Run `tvd pair` any time to see it again.

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

The order you arrange apps in from the remote is kept in
`~/.local/state/rustv/app-order`, separate from the config: the config says
which apps exist, that file only where they go. Apps it does not list appear
at the end in config order. Delete it to go back to the config order.

The remote's settings panel edits the same file. It can only change the
home screen, screensaver, audio and overlay sections; apps, the server and
the browser stay in your hands. A change is saved only if the whole file
still passes `tvd check`.

The volume stops at 100% unless you raise `audio.max_volume`; above 100%
PipeWire boosts the signal in software, which can distort.

Web apps are ordinary websites, often made for a desk, not a couch. To make
them all larger, add this to the kiosk browser's `user.js` (for Flatpak
Firefox, in its profile under `~/.var/app/org.mozilla.firefox/`):

```js
user_pref("layout.css.devPixelsPerPx", "1.5");
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
| `GET` | `/api/v1/network` | Ports the box listens on, with address and program |
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
{"type": "sleep"}
{"type": "sleep_timer", "minutes": 30}
{"type": "media", "action": "play_pause"}
{"type": "media", "action": "seek", "seconds": -10}
{"type": "show_pairing"}
{"type": "set_pin", "pin": "2468"}
{"type": "lock"}
{"type": "unlock", "pin": "2468"}
{"type": "set", "key": "cast.airplay", "value": true}
{"type": "set", "key": "theme.preset", "value": "ocean"}
{"type": "set", "key": "theme.accent", "value": null}
{"type": "set", "key": "screensaver.idle_minutes", "value": 10}
{"type": "reorder", "ids": ["stremio", "jellyfin"]}
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
- [x] Themes: six presets or your own colors, on the TV and the remote
- [x] QR code pairing
- [x] mDNS: the remote at `http://<hostname>.local:8765`, announced on the network
- [ ] Agent integration (MCP) and voice commands
- [ ] Switching between running apps

### Backlog

- [ ] System info on screen, each item switchable on or off in user
  preferences: clock, date, weather, power consumption, battery, component
  temperatures, disk space, RAM, Wi-Fi/Ethernet status, network usage, and
  any other data the system exposes
- [x] UI audio feedback (navigation and selection sounds)
- [ ] Manage Bluetooth, Wi-Fi and audio from the shell
- [x] Lock and unlock with a PIN; see the ports the box exposes
- [ ] Change exposed ports and addresses from the remote (today: in each
  program's config)
- [x] Reorder apps (from the remote)
- [x] Localization: English and Turkish, 24 or 12 hour clock, °C or °F
- [ ] More languages (one table each in the shell, the remote and the overlay)
- [x] Screen casting with AirPlay (iPhone, iPad, Mac)
- [ ] Screen casting from Android and Windows (Google Cast, Miracast); there
  is no dependable open-source receiver yet

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
