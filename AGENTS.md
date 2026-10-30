# AGENTS.md

Guidance for AI coding agents working on rustv. Humans should read
[CONTRIBUTING.md](CONTRIBUTING.md) and the [README](README.md).

## Project

rustv is a private, open-source TV shell for Linux. The `tvd` daemon holds all
logic (app launching, virtual input, audio, JSON API, web remote). The
on-screen shell (`rustv-shell`) is a thin client of the same API.

## Layout

- `crates/tvd/src/`: daemon
  - `main.rs` entry point and CLI (`tvd check`, `tvd pair`)
  - `config.rs` TOML config loading and validation
  - `apps.rs` app manager (flatpak, command, web)
  - `api.rs` REST and WebSocket server
  - `protocol.rs` command and message types
  - `state.rs` shared daemon state
  - `input.rs`, `keymap.rs` uinput virtual keyboard and mouse
  - `audio.rs` PipeWire volume (`wpctl`) and outputs (`pw-dump`)
- `crates/tvd/web/`: web remote (plain HTML, CSS, JS, PWA), no build step
- `crates/shell/`: on-screen shell
  - `ui/shell.slint` home screen (Slint markup), `ui/fonts/` bundled Hanken Grotesk
  - `src/main.rs` window, clock, UI updates; `src/net.rs` WebSocket client;
    `src/settings.rs` finds tvd from the shared config
- `crates/locales/`: every text on screen, one catalog per language
  (`en.toml`, `tr.toml`), shared by tvd, the shell and the remote; see
  CONTRIBUTING.md before adding or changing user-facing text
- `config/config.example.toml`: documented example config
- `packaging/`: systemd units (tvd user service, TV session), sway kiosk
  config, udev rule, xdg-desktop-portal config
- `scripts/install.sh`: build and install; `scripts/get.sh`: one-line install
  (system setup, then `install.sh`), shellcheck-clean POSIX sh

## Commands

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
RUST_LOG=debug cargo run -p tvd -- --config config/config.example.toml
```

Run test, clippy and fmt before declaring a task done. `scripts/check.sh`
runs all of CI locally; `scripts/check.sh --remote` adds the browser tests.

## Rules

- No network access from `tvd` or the shell. No telemetry, analytics or
  accounts. Only launched apps may use the internet.
- Keep dependencies minimal. Prefer the standard library and existing crates.
- Everything user-facing is configurable in TOML. Do not hardcode apps,
  limits or promoted content.
- New displayed information or features must be user-toggleable.
- Keep the web remote and TV shell functionally equivalent. A phone must not be
  required: provide TV-remote/gamepad navigation and local text/PIN entry.
  Keep unavailable actions visible but disabled with an explanation.
- Every API endpoint except `/api/v1/health` requires the bearer token. Do
  not weaken auth or log the token.
- The web remote has no build step. Do not introduce a bundler or framework.
- Match surrounding code style and comment density. rustfmt uses
  `max_width = 120`.
- When behavior, config keys or endpoints change, update the README and
  `config/config.example.toml` in the same change.
- Code, comments, commits and docs are in English.

## Commits and PRs

Conventional Commits (`feat`, `fix`, `docs`, `refactor`, `test`, `chore`) with
a scope when useful. One focused change per commit. Do not commit secrets,
tokens or local config.

## Scope

Check the roadmap and backlog in the README before starting. Ask before
large architectural changes, new crates or new runtime dependencies.
