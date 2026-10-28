//! HTTP routes: the web remote, the REST API and the WebSocket.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::protocol::{AppInfo, AudioInfo, Command, ServerMsg, StateSnapshot};
use crate::state::AppState;

const INDEX: &str = include_str!("../web/index.html");
const SCRIPT: &str = include_str!("../web/app.js");
const STYLE: &str = include_str!("../web/style.css");
const MANIFEST: &str = include_str!("../web/manifest.webmanifest");
const ICON: &str = include_str!("../web/icon.svg");
// iOS ignores SVG home screen icons, and Android wants fixed sizes.
const TOUCH_ICON: &[u8] = include_bytes!("../web/apple-touch-icon.png");
const ICON_192: &[u8] = include_bytes!("../web/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../web/icon-512.png");
// Hanken Grotesk (SIL Open Font License, see web/fonts/OFL.txt), served from the box:
// the remote loads nothing from the internet.
const HANKEN_400: &[u8] = include_bytes!("../web/fonts/HankenGrotesk-Regular.woff2");
const HANKEN_500: &[u8] = include_bytes!("../web/fonts/HankenGrotesk-Medium.woff2");
const HANKEN_600: &[u8] = include_bytes!("../web/fonts/HankenGrotesk-SemiBold.woff2");

type Shared = Arc<AppState>;

/// Changes whenever the script or the styles change, so they can be cached for a
/// year under a URL that carries it, while the page itself is revalidated cheaply.
/// Every language's texts for the remote, as a script that runs before app.js.
static LOCALES: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| format!("window.LOCALES = {};\n", rustv_locales::catalogs_json()));

static VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let hash = [SCRIPT, STYLE, INDEX, LOCALES.as_str()]
        .iter()
        .flat_map(|s| s.bytes())
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}")
});

static PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    INDEX
        .replace("\"/app.js\"", &format!("\"/app.js?v={}\"", *VERSION))
        .replace("\"/locales.js\"", &format!("\"/locales.js?v={}\"", *VERSION))
        .replace("\"/style.css\"", &format!("\"/style.css?v={}\"", *VERSION))
});

/// The page: no-cache, but an unchanged one is answered with an empty 304.
async fn page(headers: axum::http::HeaderMap) -> Response {
    let etag = format!("\"{}\"", *VERSION);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::ETAG, etag),
        ],
        PAGE.as_str(),
    )
        .into_response()
}

/// Script and styles, under versioned URLs: cached for good.
fn immutable(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], body)
        .into_response()
}

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/state", get(get_state))
        .route("/apps", get(get_apps))
        .route("/apps/:id/icon", get(get_icon))
        .route("/wallpapers/:n", get(get_wallpaper))
        .route("/audio", get(get_audio))
        .route("/network", get(get_network))
        .route("/wifi", get(get_wifi))
        .route("/bluetooth", get(get_bluetooth))
        .route("/command", post(post_command))
        .route("/ws", get(websocket))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token))
        .route("/health", get(|| async { "ok" }));

    Router::new()
        .route("/", get(page))
        .route("/app.js", get(|| async { immutable("text/javascript; charset=utf-8", SCRIPT) }))
        .route("/style.css", get(|| async { immutable("text/css; charset=utf-8", STYLE) }))
        .route("/locales.js", get(|| async { immutable("text/javascript; charset=utf-8", LOCALES.as_str()) }))
        .route("/manifest.webmanifest", get(|| async { asset("application/manifest+json", MANIFEST) }))
        .route("/icon.svg", get(|| async { asset("image/svg+xml", ICON) }))
        .route("/apple-touch-icon.png", get(|| async { image(TOUCH_ICON) }))
        .route("/icon-192.png", get(|| async { image(ICON_192) }))
        .route("/icon-512.png", get(|| async { image(ICON_512) }))
        .route("/fonts/hanken-3.013-400.woff2", get(|| async { font(HANKEN_400) }))
        .route("/fonts/hanken-3.013-500.woff2", get(|| async { font(HANKEN_500) }))
        .route("/fonts/hanken-3.013-600.woff2", get(|| async { font(HANKEN_600) }))
        .nest("/api/v1", api)
        .with_state(state)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

/// `{"error": "English text", "key": ..., "args": ...}`: the key and values let the
/// remote word a known error in its own language.
fn error_body(err: &anyhow::Error) -> serde_json::Value {
    let mut body = crate::i18n::describe(err);
    let message = body["message"].take();
    let object = body.as_object_mut().expect("describe returns an object");
    object.remove("message");
    object.insert("error".into(), message);
    body
}

/// Font files never change under a URL that carries their version.
fn font(woff2: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, "font/woff2"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], woff2)
        .into_response()
}

fn image(png: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "max-age=86400")], png).into_response()
}

/// Accepts `Authorization: Bearer <token>`, or `?token=` for WebSocket clients.
async fn require_token(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let from_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let from_query = req.uri().query().and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("token=")));
    match from_header.or(from_query) {
        Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => next.run(req).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn get_state(State(state): State<Shared>) -> Json<StateSnapshot> {
    Json(state.snapshot().await)
}

async fn get_apps(State(state): State<Shared>) -> Json<Vec<AppInfo>> {
    Json(state.app_infos())
}

async fn get_icon(State(state): State<Shared>, Path(id): Path<String>) -> Response {
    let Some(path) = state.icon_path(&id) else { return StatusCode::NOT_FOUND.into_response() };
    let content_type = match path.extension().and_then(|ext| ext.to_str()) {
        Some("svg") => "image/svg+xml",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "image/png",
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "private, max-age=3600")], bytes)
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Picture `n` of `[wallpaper] images`; nothing outside that list.
async fn get_wallpaper(State(state): State<Shared>, Path(n): Path<usize>) -> Response {
    let cfg = state.config().wallpaper.clone();
    let Some(path) =
        tokio::task::spawn_blocking(move || crate::wallpaper::images(&cfg).into_iter().nth(n)).await.ok().flatten()
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = crate::wallpaper::content_type(&path).unwrap_or("application/octet-stream");
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_audio(State(state): State<Shared>) -> Json<AudioInfo> {
    Json(state.audio_info().await)
}

/// Takes the body as JSON whatever the Content-Type, so a plain `curl -d` works.
async fn get_network() -> Json<Vec<crate::network::Exposure>> {
    Json(tokio::task::spawn_blocking(crate::network::exposures).await.unwrap_or_default())
}

#[derive(serde::Deserialize)]
struct WifiQuery {
    #[serde(default)]
    rescan: bool,
}

async fn get_wifi(Query(query): Query<WifiQuery>) -> Response {
    match crate::wireless::networks(query.rescan).await {
        Ok(networks) => Json(networks).into_response(),
        Err(err) => (StatusCode::SERVICE_UNAVAILABLE, Json(error_body(&err))).into_response(),
    }
}

async fn get_bluetooth() -> Json<crate::wireless::Bluetooth> {
    Json(crate::wireless::bluetooth().await)
}

async fn post_command(State(state): State<Shared>, body: Bytes) -> Response {
    let cmd: Command = match serde_json::from_slice(&body) {
        Ok(cmd) => cmd,
        Err(err) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("bad command: {err}") }))).into_response()
        }
    };
    match state.execute(cmd).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, Json(error_body(&err))).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct WsQuery {
    /// `shell` for the TV's own home screen; anything else is a remote.
    #[serde(default)]
    client: String,
}

async fn websocket(upgrade: WebSocketUpgrade, State(state): State<Shared>, Query(query): Query<WsQuery>) -> Response {
    let is_shell = query.client == "shell";
    upgrade.on_upgrade(move |socket| client(socket, state, is_shell))
}

async fn client(socket: WebSocket, state: Shared, is_shell: bool) {
    if !is_shell {
        state.remote_connected();
    }
    let _connected = Connected::new(state.clone(), is_shell);
    let (mut sink, mut stream) = socket.split();
    let mut events = state.events.subscribe();
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<ServerMsg>();

    // Read the config before awaiting anything: its lock must not be held across an await.
    let (shell, screensaver, wallpaper, locale, palette) = {
        let cfg = state.config();
        (
            cfg.shell.clone(),
            cfg.screensaver.clone(),
            crate::wallpaper::info(&cfg.wallpaper),
            cfg.locale.clone(),
            crate::theme::resolve(&cfg.theme),
        )
    };
    let pairing = is_shell.then(|| {
        let url = state.pairing_url();
        let qr = crate::pairing::matrix(&url);
        ServerMsg::Pairing {
            size: qr.as_ref().map_or(0, |qr| qr.size),
            modules: qr.map(|qr| qr.dark.iter().map(|&d| if d { '1' } else { '0' }).collect()).unwrap_or_default(),
            url,
        }
    });
    let hello = [
        ServerMsg::Apps { apps: state.app_infos() },
        ServerMsg::Audio(state.audio_info().await),
        ServerMsg::Shell(shell),
        ServerMsg::Screensaver(screensaver),
        ServerMsg::Wallpaper(wallpaper),
        ServerMsg::Locale(locale),
        ServerMsg::Theme(palette),
        ServerMsg::Settings(Box::new(state.settings_info())),
        ServerMsg::State(state.snapshot().await),
    ];
    for msg in hello.iter().chain(&pairing) {
        if send(&mut sink, msg).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => handle(&state, &text, &reply_tx).await,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            event = events.recv() => match event {
                Ok(msg) => if send(&mut sink, &msg).await.is_err() { break },
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
            Some(reply) = reply_rx.recv() => if send(&mut sink, &reply).await.is_err() { break },
        }
    }
}

/// Counts a WebSocket client for as long as it is connected.
struct Connected(Shared, bool);

impl Connected {
    fn new(state: Shared, is_shell: bool) -> Self {
        state.clients.fetch_add(1, Ordering::Relaxed);
        if !is_shell {
            state.remotes.fetch_add(1, Ordering::Relaxed);
        }
        notify_soon(&state);
        Self(state, is_shell)
    }
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::Relaxed);
        if !self.1 {
            self.0.remotes.fetch_sub(1, Ordering::Relaxed);
        }
        notify_soon(&self.0);
    }
}

fn notify_soon(state: &Shared) {
    let state = state.clone();
    tokio::spawn(async move { state.notify().await });
}

async fn handle(state: &Shared, text: &str, reply: &mpsc::UnboundedSender<ServerMsg>) {
    let cmd: Command = match serde_json::from_str(text) {
        Ok(cmd) => cmd,
        Err(err) => {
            let _ = reply.send(ServerMsg::Error { message: format!("bad message: {err}"), key: None, args: None });
            return;
        }
    };
    if cmd.is_slow() {
        // Launching can take seconds; keep handling input meanwhile.
        let state = state.clone();
        let reply = reply.clone();
        tokio::spawn(async move {
            if let Err(err) = state.execute(cmd).await {
                let _ = reply.send(ServerMsg::error(&err));
            }
        });
    } else if let Err(err) = state.execute(cmd).await {
        let _ = reply.send(ServerMsg::error(&err));
    }
}

async fn send(sink: &mut SplitSink<WebSocket, Message>, msg: &ServerMsg) -> Result<(), axum::Error> {
    let text = serde_json::to_string(msg).expect("server messages always serialize");
    sink.send(Message::Text(text)).await
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn compares_tokens() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
