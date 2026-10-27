//! `tvd`, the rustv daemon.
//!
//! Owns everything that is not pixels: the app list, launching and stopping
//! apps, virtual keyboard and pointer, volume, and the local HTTP/WebSocket API
//! used by the web remote, the TV shell and automation.

mod api;
mod apps;
mod audio;
mod cast;
mod cec;
mod config;
mod gamepad;
mod input;
mod keymap;
mod lock;
mod media;
mod network;
mod osd;
mod overlay;
mod pairing;
mod protocol;
mod settings;
mod sleep;
mod state;
mod stats;
mod theme;
mod volumes;
mod wireless;

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use tokio::signal::unix::{signal, SignalKind};
use tracing::info;
use tracing_subscriber::EnvFilter;

const HELP: &str = "\
tvd - rustv daemon

USAGE:
    tvd [OPTIONS] [COMMAND]

COMMANDS:
    serve    Run the daemon (default)
    pair     Print the pairing link for the web remote
    check    Validate the configuration and exit
    overlay-status
             Print the debug overlay line (run by swaybar, not by hand)

OPTIONS:
    -c, --config <PATH>    Config file (default: $XDG_CONFIG_HOME/rustv/config.toml)
    -h, --help             Show this help
    -V, --version          Show version
";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args().skip(1);
    let mut command = String::from("serve");
    let mut config_path: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => config_path = Some(args.next().context("--config needs a path")?.into()),
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("tvd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "serve" | "pair" | "check" | "overlay-status" => command = arg,
            other => bail!("unknown argument: {other}\n\n{HELP}"),
        }
    }

    let path = config_path.unwrap_or_else(config::default_path);
    let cfg = config::Config::load(&path)?;

    match command.as_str() {
        "check" => {
            println!("{}: OK ({} apps)", path.display(), cfg.apps.len());
            Ok(())
        }
        "pair" => {
            let token = config::load_or_create_token(&cfg)?;
            let url = pairing::url(&cfg.server.bind, &token, cfg.server.pairing_address);
            if let Some(qr) = pairing::terminal(&url) {
                println!("{qr}");
                println!("Scan with your phone's camera, or open:");
            }
            println!("{url}");
            let by_ip = pairing::ip_url(&cfg.server.bind, &token);
            if by_ip != url {
                println!("If that name does not open on your device, use the IP address:");
                println!("{by_ip}");
            }
            Ok(())
        }
        "overlay-status" => {
            let token = config::load_or_create_token(&cfg)?;
            overlay::status_loop(&cfg, &token)
        }
        _ => serve(cfg, path).await,
    }
}

async fn serve(cfg: config::Config, path: PathBuf) -> Result<()> {
    let token = config::load_or_create_token(&cfg)?;
    let input = input::Input::new().context("creating virtual input devices (is /dev/uinput writable?)")?;
    let bind: SocketAddr = cfg.server.bind.parse().context("invalid server.bind")?;

    let state = state::AppState::new(cfg, path, token, input);
    state.apps.adopt().await;
    state.enforce_volume_limit().await;
    state.after_start().await;
    let router = api::router(state);

    let listener = tokio::net::TcpListener::bind(bind).await.with_context(|| format!("binding {bind}"))?;
    info!("listening on http://{bind}");
    axum::serve(listener, router).with_graceful_shutdown(shutdown()).await?;
    info!("stopped");
    Ok(())
}

async fn shutdown() {
    let mut term = signal(SignalKind::terminate()).expect("installing SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
