//! HTTP routes: the web remote, the REST API and the WebSocket.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
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

type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/state", get(get_state))
        .route("/apps", get(get_apps))
        .route("/apps/:id/icon", get(get_icon))
        .route("/audio", get(get_audio))
        .route("/command", post(post_command))
        .route("/ws", get(websocket))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token))
        .route("/health", get(|| async { "ok" }));

    Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", INDEX) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", SCRIPT) }))
        .route("/style.css", get(|| async { asset("text/css; charset=utf-8", STYLE) }))
        .route("/manifest.webmanifest", get(|| async { asset("application/manifest+json", MANIFEST) }))
        .route("/icon.svg", get(|| async { asset("image/svg+xml", ICON) }))
        .nest("/api/v1", api)
        .with_state(state)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body).into_response()
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

async fn get_audio(State(state): State<Shared>) -> Json<AudioInfo> {
    Json(state.audio_info().await)
}

/// Takes the body as JSON whatever the Content-Type, so a plain `curl -d` works.
async fn post_command(State(state): State<Shared>, body: Bytes) -> Response {
    let cmd: Command = match serde_json::from_slice(&body) {
        Ok(cmd) => cmd,
        Err(err) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("bad command: {err}") }))).into_response()
        }
    };
    match state.execute(cmd).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{err:#}") }))).into_response(),
    }
}

async fn websocket(upgrade: WebSocketUpgrade, State(state): State<Shared>) -> Response {
    upgrade.on_upgrade(move |socket| client(socket, state))
}

async fn client(socket: WebSocket, state: Shared) {
    let (mut sink, mut stream) = socket.split();
    let mut events = state.events.subscribe();
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<ServerMsg>();

    let hello = [
        ServerMsg::Apps { apps: state.app_infos() },
        ServerMsg::Audio(state.audio_info().await),
        ServerMsg::Shell(state.config.shell.clone()),
        ServerMsg::State(state.snapshot().await),
    ];
    for msg in &hello {
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

async fn handle(state: &Shared, text: &str, reply: &mpsc::UnboundedSender<ServerMsg>) {
    let cmd: Command = match serde_json::from_str(text) {
        Ok(cmd) => cmd,
        Err(err) => {
            let _ = reply.send(ServerMsg::Error { message: format!("bad message: {err}") });
            return;
        }
    };
    if cmd.is_slow() {
        // Launching can take seconds; keep handling input meanwhile.
        let state = state.clone();
        let reply = reply.clone();
        tokio::spawn(async move {
            if let Err(err) = state.execute(cmd).await {
                let _ = reply.send(ServerMsg::Error { message: format!("{err:#}") });
            }
        });
    } else if let Err(err) = state.execute(cmd).await {
        let _ = reply.send(ServerMsg::Error { message: format!("{err:#}") });
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
