#!/bin/sh
# One-line install of rustv:
#
#   curl -fsSL https://raw.githubusercontent.com/cngil/rustv/main/scripts/get.sh | sh
#
# Installs the system packages (apt, dnf or pacman), Rust if it is missing or
# too old, allows virtual input devices, builds and installs rustv for the
# current user (scripts/install.sh) and sets up the TV session. It asks before
# each system change; run it again any time to update. Options go after
# `sh -s --`, for example `curl -fsSL … | sh -s -- --yes --no-session`:
#
#   --yes           answer yes to every question
#   --no-packages   do not install system packages
#   --no-session    do not set up the TV session (sway on tty1)
#   --dir DIR       where the source lives (default ~/.local/src/rustv)
#   --ref REF       branch or tag to install (default main)
#   --dry-run       print what would run, change nothing
set -eu

repo="https://github.com/cngil/rustv"
dir="${RUSTV_DIR:-$HOME/.local/src/rustv}"
ref="${RUSTV_REF:-main}"
yes=0
packages=1
session=1
dry=0
relogin=0

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() {
    printf '\033[31merror:\033[0m %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# Runs a command, or only prints it with --dry-run.
run() {
    if [ "$dry" = 1 ]; then
        echo "+ $*"
    else
        "$@"
    fi
}

as_root() {
    if [ "$(id -u)" = 0 ]; then
        run "$@"
    else
        run sudo "$@"
    fi
}

# Asks a yes/no question on the terminal (stdin is the script under `curl | sh`).
ask() {
    if [ "$yes" = 1 ]; then
        return 0
    fi
    if ! { : </dev/tty; } 2>/dev/null; then
        warn "no terminal to ask \"$1\", skipping (use --yes to accept)"
        return 1
    fi
    printf '%s [Y/n] ' "$1" >/dev/tty
    read -r answer </dev/tty || answer=n
    case "$answer" in
    "" | y | Y | yes | Yes) return 0 ;;
    *) return 1 ;;
    esac
}

# Copies a file into a root-owned path, or prints it with --dry-run.
install_root() {
    as_root install -Dm644 "$1" "$2"
}

packages_install() {
    if have apt-get; then
        set -- git curl ca-certificates build-essential pkg-config libfontconfig1-dev libxkbcommon-dev \
            sway pipewire-bin wireplumber flatpak avahi-daemon
        as_root apt-get update
        as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@"
    elif have dnf; then
        as_root dnf install -y git curl gcc pkgconf-pkg-config fontconfig-devel libxkbcommon-devel \
            sway pipewire-utils wireplumber flatpak avahi
    elif have pacman; then
        as_root pacman -S --needed --noconfirm git curl base-devel fontconfig libxkbcommon \
            sway pipewire wireplumber flatpak avahi
    else
        warn "unknown package manager; install git, a C compiler, pkg-config, fontconfig and"
        warn "xkbcommon headers, sway, PipeWire (wpctl, pw-dump) and flatpak yourself"
        return 0
    fi
    if have flatpak; then
        as_root flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
        if ! flatpak info org.mozilla.firefox >/dev/null 2>&1 && ask "Install Firefox (Flatpak), which web apps open in?"; then
            as_root flatpak install -y --noninteractive flathub org.mozilla.firefox
        fi
    fi
}

# rustc 1.80 or newer, from rustup when the system one is missing or older.
rust_ensure() {
    if [ -f "$HOME/.cargo/env" ]; then
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi
    if have cargo && have rustc; then
        minor=$(rustc --version | sed -n 's/^rustc 1\.\([0-9]*\).*/\1/p')
        if [ "${minor:-0}" -ge 80 ]; then
            return 0
        fi
        if have rustup; then
            say "Updating Rust"
            run rustup update stable
            return 0
        fi
    fi
    ask "Rust 1.80+ is needed. Install it with rustup into ~/.cargo?" || die "Rust 1.80+ is needed"
    if [ "$dry" = 1 ]; then
        echo "+ curl https://sh.rustup.rs | sh -s -- -y --profile minimal"
        return 0
    fi
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
}

source_fetch() {
    if [ -d "$dir/.git" ]; then
        say "Updating $dir"
        run git -C "$dir" fetch --tags origin
        run git -C "$dir" checkout -q "$ref"
        if [ "$dry" = 1 ] || git -C "$dir" symbolic-ref -q HEAD >/dev/null; then
            run git -C "$dir" merge -q --ff-only "origin/$ref"
        fi
    else
        say "Downloading rustv into $dir"
        run git clone -q --branch "$ref" "$repo" "$dir"
    fi
}

# tvd creates a virtual keyboard and mouse through /dev/uinput.
uinput_allow() {
    ask "Allow virtual input devices for $(id -un) (udev rule, uinput module, input group)?" || {
        warn "skipped; tvd cannot type or move the pointer until /dev/uinput is writable"
        return 0
    }
    install_root "$dir/packaging/udev/60-rustv-uinput.rules" /etc/udev/rules.d/60-rustv-uinput.rules
    if [ "$dry" = 1 ]; then
        echo "+ echo uinput > /etc/modules-load.d/uinput.conf"
    else
        echo uinput | as_root tee /etc/modules-load.d/uinput.conf >/dev/null
    fi
    as_root modprobe uinput || warn "could not load the uinput module now; it loads at the next boot"
    as_root udevadm control --reload || true
    as_root udevadm trigger --name-match=uinput || true
    getent group input >/dev/null || as_root groupadd --system input
    if ! id -nG | tr ' ' '\n' | grep -qx input; then
        as_root usermod -aG input "$(id -un)"
        relogin=1
    fi
}

# Wi-Fi from the remote: NetworkManager allows members of netdev through polkit.
wifi_allow() {
    have nmcli && [ -d /etc/polkit-1 ] || return 0
    ask "Let the remote change Wi-Fi (polkit rule, netdev group)?" || return 0
    install_root "$dir/packaging/polkit/50-rustv-network.rules" /etc/polkit-1/rules.d/50-rustv-network.rules
    getent group netdev >/dev/null || as_root groupadd --system netdev
    if ! id -nG | tr ' ' '\n' | grep -qx netdev; then
        as_root usermod -aG netdev "$(id -un)"
        relogin=1
    fi
}

avahi_announce() {
    [ -d /etc/avahi ] || return 0
    ask "Announce the remote as http://$(hostname).local:8765 (Avahi)?" || return 0
    install_root "$dir/packaging/avahi/rustv.service" /etc/avahi/services/rustv.service
    as_root systemctl enable --now avahi-daemon || warn "could not start avahi-daemon"
}

# sway as a kiosk on tty1, started at boot by a system unit.
session_setup() {
    config="${XDG_CONFIG_HOME:-$HOME/.config}"
    if systemctl is-active -q display-manager 2>/dev/null; then
        warn "a display manager (desktop login screen) is running; the TV session replaces it on tty1"
        ask "Set up the TV session anyway?" || return 0
    else
        ask "Show rustv on the TV at boot (sway on tty1)?" || return 0
    fi
    have sway || warn "sway is not installed; the TV session needs it"
    if [ ! -f "$config/rustv/sway.conf" ]; then
        run install -Dm644 "$dir/packaging/sway/rustv.conf" "$config/rustv/sway.conf"
        [ "$dry" = 1 ] || echo "Created $config/rustv/sway.conf, adjust the output section to your TV."
    fi
    if [ ! -f "$config/xdg-desktop-portal/portals.conf" ]; then
        run install -Dm644 "$dir/packaging/xdg-desktop-portal/portals.conf" "$config/xdg-desktop-portal/portals.conf"
    fi
    if [ "$dry" = 1 ]; then
        echo "+ install rustv-session.service for $(id -un) into /etc/systemd/system/"
        echo "+ systemctl enable rustv-session (and start it unless a reboot is due)"
        return 0
    fi
    unit=$(mktemp)
    sed -e "s|YOUR_USER|$(id -un)|g" -e "s|/run/user/1000|/run/user/$(id -u)|g" -e "s|/home/[^/]*/|$HOME/|g" \
        "$dir/packaging/systemd/rustv-session.service" >"$unit"
    install_root "$unit" /etc/systemd/system/rustv-session.service
    rm -f "$unit"
    as_root systemctl daemon-reload
    # Starting it now would end a login on tty1, so then it waits for the reboot.
    if [ "$(tty 2>/dev/null)" = /dev/tty1 ] || [ "$relogin" = 1 ]; then
        as_root systemctl enable rustv-session
        relogin=1
    else
        as_root systemctl enable --now rustv-session
    fi
}

main() {
    while [ $# -gt 0 ]; do
        case "$1" in
        -y | --yes) yes=1 ;;
        --no-packages) packages=0 ;;
        --no-session) session=0 ;;
        --dry-run) dry=1 ;;
        --dir)
            dir="${2:?--dir needs a path}"
            shift
            ;;
        --ref)
            ref="${2:?--ref needs a branch or tag}"
            shift
            ;;
        -h | --help)
            sed -n '2,17s/^# \{0,1\}//p' "$0" 2>/dev/null || echo "see $repo/blob/main/scripts/get.sh"
            exit 0
            ;;
        *) die "unknown option: $1" ;;
        esac
        shift
    done

    [ "$(uname -s)" = Linux ] || die "rustv runs on Linux"
    [ "$(id -u)" != 0 ] || die "run this as the user who will watch TV, not root (it asks for sudo when needed)"
    have systemctl || die "rustv needs systemd"
    have sudo || die "sudo is needed for the system setup"

    if [ "$packages" = 1 ] && ask "Install the system packages rustv needs (sudo)?"; then
        say "Installing system packages"
        packages_install
    fi
    have git || [ "$dry" = 1 ] || die "git is needed"
    rust_ensure
    source_fetch
    uinput_allow
    wifi_allow
    avahi_announce

    say "Building and installing rustv (takes a few minutes on a small box)"
    if [ "$dry" = 1 ]; then
        echo "+ $dir/scripts/install.sh"
        echo "+ loginctl enable-linger $(id -un)"
    else
        "$dir/scripts/install.sh" </dev/null
        # tvd runs at boot without anyone logging in.
        loginctl enable-linger "$(id -un)" || warn "could not enable lingering; tvd starts at login only"
    fi

    if [ "$session" = 1 ]; then
        session_setup
    fi

    echo
    say "rustv is installed. Run this again to update."
    if [ "$relogin" = 1 ]; then
        say "Reboot once to finish: new group memberships and the TV session take effect then."
    fi
}

# Everything runs from here, so `curl | sh` has read the whole script first.
main "$@"
