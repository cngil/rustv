#!/bin/sh
# Runs tvd and the shell from this checkout on a desktop, without touching an
# installed rustv: config, token and state live in target/dev.
#
#   scripts/dev.sh          the shell in a window on the current Wayland desktop
#   scripts/dev.sh --sway   a nested sway kiosk in a window, closest to the TV
#
# Edit target/dev/rustv/config.toml to change apps and settings; delete it to
# start again from the example config. The web remote is at
# http://<this machine>:8765 (pair with the QR code on screen).
# Remote key presses go to the focused window, so keep the shell focused.
# Volume presses still change the desktop's volume.

set -eu

# Always protect the host, including nested sway and release-build experiments.
export TVD_POWER_DRY_RUN=1

cd "$(dirname "$0")/.."
root=$(pwd)

sway=false
for arg in "$@"; do
  case "$arg" in
    --sway) sway=true ;;
    *) echo "usage: $0 [--sway]" >&2; exit 2 ;;
  esac
done

if [ -z "${WAYLAND_DISPLAY:-}" ]; then
  echo "error: run this inside a Wayland session" >&2
  exit 1
fi
if [ "$sway" = true ] && ! command -v sway >/dev/null; then
  echo "error: --sway needs sway installed" >&2
  exit 1
fi
if [ ! -w /dev/uinput ]; then
  echo "error: /dev/uinput is not writable; tvd cannot start (see the README)" >&2
  exit 1
fi

dev="$root/target/dev"
bin="$root/target/debug"
config="$dev/rustv/config.toml"
mkdir -p "$dev/rustv" "$dev/state"
if [ ! -f "$config" ]; then
  # The example config, made safe for a desktop: the token stays in target/dev
  # (tvd keeps it in the real config dir otherwise), a gamepad stays the
  # desktop's, nothing closes apps after an idle hour, and volume presses do not
  # rewrite every app's stream volume.
  awk -v token="$dev/rustv/token" '
    /^\[/ { section = $0 }
    /^# token_file = / { $0 = "token_file = \"" token "\"" }
    section == "[audio]" && /^per_app_volume = true/ { sub(/true/, "false") }
    (section == "[gamepad]" || section == "[sleep]") && /^enabled = true/ { sub(/true/, "false") }
    { print }
  ' config/config.example.toml >"$config"
  echo "Created $config (gamepad, sleep and per-app volume off)."
fi

cargo build -p tvd -p rustv-shell

# tvd gets the dev config through --config; the shell finds it through
# XDG_CONFIG_HOME. Launched apps keep the real config dir.
tvd="env XDG_STATE_HOME=$dev/state $bin/tvd --config $config"
shell="env XDG_CONFIG_HOME=$dev $bin/rustv-shell"

stop_tvd() { pkill -f "^$bin/tvd --config $config" 2>/dev/null || true; }
trap stop_tvd EXIT INT TERM
stop_tvd

if [ "$sway" = true ]; then
  # packaging/sway/rustv.conf without the TV output settings, plus a way out.
  cat >"$dev/sway.conf" <<EOF
xwayland disable
default_border none
default_floating_border none
focus_follows_mouse no
focus_on_window_activation focus
seat * hide_cursor 3000
for_window [app_id=".*"] fullscreen enable
# Not on the TV: Alt+Shift+Q closes the dev session.
bindsym Mod1+Shift+q exit
exec $tvd
exec sh -c 'while true; do $shell; sleep 1; done'
EOF
  sway -c "$dev/sway.conf"
else
  $tvd &
  # The shell reads the token tvd writes at its first start.
  tries=0
  until [ -f "$dev/rustv/token" ] && [ -n "$(cat "$dev/rustv/token")" ]; do
    tries=$((tries + 1))
    if [ "$tries" -gt 50 ]; then
      echo "error: tvd did not start" >&2
      exit 1
    fi
    sleep 0.2
  done
  $shell
fi
