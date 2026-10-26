//! `tvd`, the rustv daemon.
//!
//! Owns everything that is not pixels: the app list, launching and stopping
//! apps, virtual keyboard and pointer, volume, and the local HTTP/WebSocket API
//! used by the web remote, the TV shell and automation.

mod api;
mod apps;
mod audio;
mod config;
mod input;
mod keymap;
mod protocol;
mod state;

use std::io::IsTerminal;
use std::net::{SocketAddr, UdpSocket};
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
            "serve" | "pair" | "check" => command = arg,
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
            println!("{}", pairing_url(&cfg.server.bind, &token));
            Ok(())
        }
        _ => serve(cfg).await,
    }
}

async fn serve(cfg: config::Config) -> Result<()> {
    let token = config::load_or_create_token(&cfg)?;
    let input = input::Input::new().context("creating virtual input devices (is /dev/uinput writable?)")?;
    let bind: SocketAddr = cfg.server.bind.parse().context("invalid server.bind")?;

    let state = state::AppState::new(cfg, token, input);
    state.apps.adopt().await;
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

fn pairing_url(bind: &str, token: &str) -> String {
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
