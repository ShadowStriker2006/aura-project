use futures_util::{SinkExt, StreamExt};
use native_tls::TlsConnector;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::{connect_async_tls_with_config, Connector};

use super::lockfile::{build_auth_header, LcuCredentials};

#[derive(Debug)]
pub enum WatcherError {
    ConnectorBuildFailed(String),
    RequestBuildFailed(String),
    ConnectFailed(String),
    SubscribeFailed(String),
    StreamClosed,
}
impl std::fmt::Display for WatcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatcherError::ConnectorBuildFailed(s) => write!(f, "TLS connector build failed: {}", s),
            WatcherError::RequestBuildFailed(s) => write!(f, "request build failed: {}", s),
            WatcherError::ConnectFailed(s) => write!(f, "ws connect failed: {}", s),
            WatcherError::SubscribeFailed(s) => write!(f, "subscribe send failed: {}", s),
            WatcherError::StreamClosed => write!(f, "ws stream closed"),
        }
    }
}
impl std::error::Error for WatcherError {}

fn log_ok(msg: &str) {
    println!("[AURA::LCU::WATCHER][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::LCU::WATCHER][ERR] {}", msg);
}

/// One connection attempt: connect, subscribe, read until closed, return.
/// No internal retry loop — retry lives in the supervisor, which re-reads the
/// lockfile first (stale credentials after a client restart would otherwise
/// retry forever against a dead port/password).
pub async fn run_once(app: AppHandle, creds: &LcuCredentials) -> Result<(), WatcherError> {
    let tls_connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true) // cert CN won't match "127.0.0.1"
        .build()
        .map_err(|e| WatcherError::ConnectorBuildFailed(e.to_string()))?;

    let ws_config = WebSocketConfig {
        max_message_size: Some(64 * 1024),
        ..Default::default()
    };

    let mut request = format!("wss://127.0.0.1:{}/", creds.port)
        .into_client_request()
        .map_err(|e| WatcherError::RequestBuildFailed(e.to_string()))?;
    request.headers_mut().insert(
        "Authorization",
        build_auth_header(creds)
            .parse()
            .map_err(|e: http::header::InvalidHeaderValue| {
                WatcherError::RequestBuildFailed(e.to_string())
            })?,
    );

    log_ok(&format!("connecting to port {}", creds.port));
    let (mut ws_stream, _resp) = connect_async_tls_with_config(
        request,
        Some(ws_config),
        false,
        Some(Connector::NativeTls(tls_connector)),
    )
    .await
    .map_err(|e| WatcherError::ConnectFailed(e.to_string()))?;

    log_ok("connected, subscribing to OnJsonApiEvent");
    ws_stream
        .send(Message::Text(
            serde_json::json!([5, "OnJsonApiEvent"]).to_string(),
        ))
        .await
        .map_err(|e| WatcherError::SubscribeFailed(e.to_string()))?;

    while let Some(msg) = ws_stream.next().await {
        match msg {
            Ok(Message::Text(txt)) => {
                if txt.is_empty() {
                    continue;
                } // LCU keepalive frames
                match serde_json::from_str::<Value>(&txt) {
                    Ok(parsed) => forward_event(&app, &parsed),
                    Err(e) => log_err(&format!("frame parse failed: {}", e)),
                }
            }
            Ok(Message::Close(_)) => {
                log_ok("LCU sent close frame — client shutting down");
                break;
            }
            Err(e) => {
                log_err(&format!("ws read error: {}", e));
                break;
            }
            _ => {}
        }
    }

    Err(WatcherError::StreamClosed)
}

/// LCU frames are `[opcode, event_name, data]`. Parses structurally and
/// forwards the payload to the frontend.
fn forward_event(app: &AppHandle, parsed: &Value) {
    let Some(arr) = parsed.as_array() else { return };
    if arr.len() != 3 {
        return;
    }
    let uri = arr[2]
        .get("uri")
        .and_then(|u| u.as_str())
        .unwrap_or("unknown");
    log_ok(&format!("event: {}", uri));
    if let Err(e) = app.emit("lcu-event", &arr[2]) {
        log_err(&format!("emit failed for {}: {}", uri, e));
    }
    if uri == "/lol-gameflow/v1/gameflow-phase" {
        if let Some(status) = arr[2]
            .get("data")
            .and_then(Value::as_str)
            .and_then(crate::live_client::GameStatus::from_lcu_phase)
        {
            if let Err(error) = crate::live_client::stream_game_status(app, status) {
                log_err(&error);
            }
        }
    }
    if uri == "/lol-champ-select/v1/session" {
        if let Some(payload) = arr[2].get("data") {
            if let Err(error) = crate::live_client::stream_draft_update(app, payload) {
                log_err(&error);
            }
        }
    }
}
