#!/bin/sh
# Builds rustv and installs it for the current user.
# Needs: Rust toolchain, PipeWire (wpctl, pw-dump), uinput access (see packaging/udev),
# and for the shell: sway plus libfontconfig/libxkbcommon development headers.
# Optional: avahi-daemon, so the remote opens at http://<hostname>.local:8765.
set -eu

cd "$(dirname "$0")/.."

cargo build --release
install -Dm755 target/release/tvd "$HOME/.local/bin/tvd"
install -Dm755 target/release/rustv-shell "$HOME/.local/bin/rustv-shell"
install -Dm644 packaging/systemd/tvd.service "$HOME/.config/systemd/user/tvd.service"

config="${XDG_CONFIG_HOME:-$HOME/.config}/rustv/config.toml"
if [ ! -f "$config" ]; then
    install -Dm644 config/config.example.toml "$config"
    echo "Created $config, edit it to list your apps."
fi

"$HOME/.local/bin/tvd" check
systemctl --user daemon-reload
systemctl --user enable --now tvd
systemctl --user restart tvd

echo
echo "To show the shell on the TV, install packaging/systemd/rustv-session.service (see the comments inside)."
echo
echo "Open this on your phone or computer to pair the remote:"
"$HOME/.local/bin/tvd" pair
