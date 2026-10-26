#!/bin/sh
# Builds tvd and installs it for the current user.
# Needs: Rust toolchain, PipeWire (wpctl), uinput access (see packaging/udev).
set -eu

cd "$(dirname "$0")/.."

cargo build --release -p tvd
install -Dm755 target/release/tvd "$HOME/.local/bin/tvd"
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
echo "Open this on your phone or computer to pair the remote:"
"$HOME/.local/bin/tvd" pair
