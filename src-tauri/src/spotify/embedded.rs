use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use tokio::time::{timeout, Duration};
use url::Url;

use super::oauth::{ensure_fresh, SpotifyState};
use super::player;

const PLAYER_WINDOW_LABEL: &str = "spotify-player";
const MAX_REQUEST_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerMode {
    EmbeddedWebview,
    CompatibleBrowser,
}

impl PlayerMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::EmbeddedWebview => "embedded_webview",
            Self::CompatibleBrowser => "compatible_browser",
        }
    }
}

#[derive(Default)]
struct EmbeddedRuntime {
    shutdown: Option<oneshot::Sender<()>>,
    session_id: Option<String>,
    mode: Option<PlayerMode>,
    device_id: Option<String>,
    activated: bool,
    last_error: Option<String>,
    fallback_available: bool,
}

#[derive(Clone, Default)]
pub struct EmbeddedSpotifyState {
    runtime: Arc<Mutex<EmbeddedRuntime>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddedPlayerStatus {
    pub running: bool,
    pub ready: bool,
    pub activated: bool,
    pub error: bool,
    pub fallback_available: bool,
    pub mode: Option<String>,
    pub device_id: Option<String>,
    pub message: String,
}

impl EmbeddedSpotifyState {
    async fn status(&self) -> EmbeddedPlayerStatus {
        let runtime = self.runtime.lock().await;
        let has_error = runtime.last_error.is_some();
        let message = if let Some(error) = &runtime.last_error {
            error.clone()
        } else if runtime.activated {
            "Aura on this PC is the selected Spotify device.".into()
        } else if runtime.device_id.is_some() {
            match runtime.mode {
                Some(PlayerMode::CompatibleBrowser) => {
                    "Aura Player is ready in your browser. Press Activate Aura Playback there once."
                        .into()
                }
                _ => "Aura Player is ready. Press Activate in its small window once.".into(),
            }
        } else if runtime.shutdown.is_some() {
            match runtime.mode {
                Some(PlayerMode::CompatibleBrowser) => {
                    "Aura Player opened in your browser. Keep that small tab open while listening."
                        .into()
                }
                _ => "Aura Player is starting. Keep its small activation window open.".into(),
            }
        } else {
            "Aura Player is stopped.".into()
        };
        EmbeddedPlayerStatus {
            running: runtime.shutdown.is_some(),
            ready: runtime.device_id.is_some(),
            activated: runtime.activated,
            error: has_error,
            fallback_available: runtime.fallback_available,
            mode: runtime.mode.map(|mode| mode.as_str().to_string()),
            device_id: runtime.device_id.clone(),
            message,
        }
    }

    async fn install_runtime(
        &self,
        shutdown: oneshot::Sender<()>,
        session_id: String,
        mode: PlayerMode,
    ) {
        let mut runtime = self.runtime.lock().await;
        runtime.shutdown = Some(shutdown);
        runtime.session_id = Some(session_id);
        runtime.mode = Some(mode);
        runtime.device_id = None;
        runtime.activated = false;
        runtime.last_error = None;
        runtime.fallback_available = false;
    }

    async fn stop_runtime(&self) {
        let shutdown = {
            let mut runtime = self.runtime.lock().await;
            runtime.session_id = None;
            runtime.mode = None;
            runtime.device_id = None;
            runtime.activated = false;
            runtime.last_error = None;
            runtime.fallback_available = false;
            runtime.shutdown.take()
        };
        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(());
        }
    }

    async fn set_device(&self, session_id: &str, device_id: String) {
        let mut runtime = self.runtime.lock().await;
        if runtime.session_id.as_deref() != Some(session_id) {
            return;
        }
        runtime.device_id = Some(device_id);
        runtime.activated = false;
        runtime.last_error = None;
        runtime.fallback_available = false;
    }

    async fn set_activated(&self, session_id: &str, activated: bool) {
        let mut runtime = self.runtime.lock().await;
        if runtime.session_id.as_deref() != Some(session_id) {
            return;
        }
        runtime.activated = activated;
        if activated {
            runtime.last_error = None;
        }
    }

    async fn clear_device(&self, session_id: &str) {
        let mut runtime = self.runtime.lock().await;
        if runtime.session_id.as_deref() != Some(session_id) {
            return;
        }
        runtime.device_id = None;
        runtime.activated = false;
    }

    async fn set_error(&self, session_id: &str, error: String) {
        let mut runtime = self.runtime.lock().await;
        if runtime.session_id.as_deref() != Some(session_id) {
            return;
        }
        runtime.last_error = Some(error.chars().take(300).collect());
    }

    async fn set_sdk_error(&self, session_id: &str, kind: &str, message: &str) {
        let mut runtime = self.runtime.lock().await;
        if runtime.session_id.as_deref() != Some(session_id) {
            return;
        }
        let is_initialization = kind.eq_ignore_ascii_case("initialization");
        runtime.fallback_available =
            is_initialization && runtime.mode == Some(PlayerMode::EmbeddedWebview);
        runtime.last_error = Some(friendly_sdk_error(kind, message, runtime.mode));
    }

    async fn stop_embedded_window_session(&self, session_id: &str) {
        let shutdown = {
            let mut runtime = self.runtime.lock().await;
            if runtime.session_id.as_deref() != Some(session_id)
                || runtime.mode != Some(PlayerMode::EmbeddedWebview)
            {
                return;
            }
            runtime.session_id = None;
            runtime.mode = None;
            runtime.device_id = None;
            runtime.activated = false;
            runtime.last_error = None;
            runtime.fallback_available = false;
            runtime.shutdown.take()
        };
        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(());
        }
    }

    async fn finish_session(&self, session_id: &str) {
        let mut runtime = self.runtime.lock().await;
        if runtime.session_id.as_deref() != Some(session_id) {
            return;
        }
        runtime.shutdown = None;
        runtime.session_id = None;
        runtime.mode = None;
        runtime.device_id = None;
        runtime.activated = false;
        runtime.fallback_available = false;
    }
}

fn friendly_sdk_error(kind: &str, message: &str, mode: Option<PlayerMode>) -> String {
    if kind.eq_ignore_ascii_case("initialization") {
        return match mode {
            Some(PlayerMode::EmbeddedWebview) =>
                "Spotify protected audio could not initialize in Aura's WebView. Choose Open Browser Player in Aura; Spotify Desktop and open.spotify.com are not required.".into(),
            _ =>
                "Spotify protected audio could not start in this browser. Use an up-to-date Microsoft Edge or Chrome, allow protected content/Widevine, and keep the Aura Player tab open.".into(),
        };
    }

    format!(
        "Spotify {kind}: {}",
        message.chars().take(240).collect::<String>()
    )
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_device_id(value: &str) -> bool {
    (8..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn query_value(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn has_session(url: &Url, secret: &str) -> bool {
    query_value(url, "session").is_some_and(|value| value == secret)
}

async fn emit_status(app: &AppHandle, state: &EmbeddedSpotifyState) {
    let status = state.status().await;
    if let Err(error) = app.emit("spotify-embedded-status", status) {
        eprintln!("[AURA::SPOTIFY::EMBEDDED][ERR] status emit failed: {error}");
    }
}

fn open_in_default_browser(url: &Url) -> Result<(), String> {
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url.as_str()])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Aura Player could not open your default browser: {error}"))
}

async fn start_local_player_server(
    app: &AppHandle,
    spotify: &SpotifyState,
    embedded: &EmbeddedSpotifyState,
    mode: PlayerMode,
) -> Result<(Url, String), String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("Aura Player could not bind its loopback server: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Aura Player could not read its loopback port: {error}"))?
        .port();
    let secret = random_secret();
    let url = Url::parse(&format!("http://127.0.0.1:{port}/#{secret}"))
        .map_err(|error| format!("Aura Player URL could not be created: {error}"))?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    embedded
        .install_runtime(shutdown_tx, secret.clone(), mode)
        .await;

    let server_app = app.clone();
    let server_spotify = spotify.clone();
    let server_state = embedded.clone();
    let server_secret = secret.clone();
    tauri::async_runtime::spawn(async move {
        run_server(
            listener,
            shutdown_rx,
            server_app,
            server_spotify,
            server_state,
            server_secret,
        )
        .await;
    });

    Ok((url, secret))
}

#[tauri::command]
pub async fn spotify_start_browser_player(
    app: AppHandle,
    spotify: tauri::State<'_, SpotifyState>,
    embedded: tauri::State<'_, EmbeddedSpotifyState>,
) -> Result<EmbeddedPlayerStatus, String> {
    // The Web Playback SDK requires a fresh token with the streaming scopes.
    ensure_fresh(spotify.inner())
        .await
        .map_err(|error| error.to_string())?;

    // Always create a fresh browser generation. Reusing a URL would let two tabs
    // compete for one Connect device and one in-memory session nonce.
    embedded.stop_runtime().await;
    if let Some(window) = app.get_webview_window(PLAYER_WINDOW_LABEL) {
        let _ = window.close();
    }

    let (url, _) = start_local_player_server(
        &app,
        spotify.inner(),
        embedded.inner(),
        PlayerMode::CompatibleBrowser,
    )
    .await?;
    if let Err(error) = open_in_default_browser(&url) {
        embedded.stop_runtime().await;
        return Err(error);
    }

    println!("[AURA::SPOTIFY::PLAYER][OK] compatible browser player started");
    emit_status(&app, embedded.inner()).await;
    Ok(embedded.status().await)
}

#[tauri::command]
pub async fn spotify_start_embedded_player(
    app: AppHandle,
    spotify: tauri::State<'_, SpotifyState>,
    embedded: tauri::State<'_, EmbeddedSpotifyState>,
) -> Result<EmbeddedPlayerStatus, String> {
    // Fail before opening a window if the user has not completed OAuth.
    ensure_fresh(spotify.inner())
        .await
        .map_err(|error| error.to_string())?;

    if let Some(window) = app.get_webview_window(PLAYER_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(embedded.status().await);
    }

    embedded.stop_runtime().await;
    let (url, session_id) = start_local_player_server(
        &app,
        spotify.inner(),
        embedded.inner(),
        PlayerMode::EmbeddedWebview,
    )
    .await?;
    let window =
        match WebviewWindowBuilder::new(&app, PLAYER_WINDOW_LABEL, WebviewUrl::External(url))
            .title("Aura Player")
            .inner_size(460.0, 235.0)
            .min_inner_size(420.0, 210.0)
            .resizable(false)
            .build()
        {
            Ok(window) => window,
            Err(error) => {
                embedded.stop_runtime().await;
                return Err(format!("Aura Player window could not be created: {error}"));
            }
        };

    let close_state = embedded.inner().clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let close_state = close_state.clone();
            let session_id = session_id.clone();
            tauri::async_runtime::spawn(async move {
                close_state.stop_embedded_window_session(&session_id).await;
            });
        }
    });

    println!("[AURA::SPOTIFY::EMBEDDED][OK] isolated Aura Player window started");
    emit_status(&app, embedded.inner()).await;
    Ok(embedded.status().await)
}

#[tauri::command]
pub async fn spotify_stop_embedded_player(
    app: AppHandle,
    embedded: tauri::State<'_, EmbeddedSpotifyState>,
) -> Result<EmbeddedPlayerStatus, String> {
    embedded.stop_runtime().await;
    if let Some(window) = app.get_webview_window(PLAYER_WINDOW_LABEL) {
        window
            .close()
            .map_err(|error| format!("Aura Player window could not close: {error}"))?;
    }
    emit_status(&app, embedded.inner()).await;
    Ok(embedded.status().await)
}

#[tauri::command]
pub async fn spotify_embedded_status(
    embedded: tauri::State<'_, EmbeddedSpotifyState>,
) -> Result<EmbeddedPlayerStatus, String> {
    Ok(embedded.status().await)
}

pub async fn device(state: &EmbeddedSpotifyState) -> Option<super::player::SpotifyDevice> {
    let status = state.status().await;
    status
        .device_id
        .map(|device_id| super::player::SpotifyDevice {
            id: Some(device_id),
            device_type: "computer".into(),
            name: "Aura on this PC".into(),
            is_active: status.activated,
            is_restricted: false,
            volume_percent: None,
        })
}

async fn run_server(
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
    app: AppHandle,
    spotify: SpotifyState,
    embedded: EmbeddedSpotifyState,
    secret: String,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => {
                    if let Err(error) = handle_request(
                        socket,
                        &app,
                        &spotify,
                        &embedded,
                        &secret,
                    ).await {
                        eprintln!("[AURA::SPOTIFY::EMBEDDED][ERR] local player request failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("[AURA::SPOTIFY::EMBEDDED][ERR] accept failed: {error}");
                    break;
                }
            }
        }
    }
    embedded.finish_session(&secret).await;
    emit_status(&app, &embedded).await;
    println!("[AURA::SPOTIFY::EMBEDDED][OK] local player server stopped");
}

async fn read_request(socket: &mut TcpStream) -> Result<String, String> {
    let mut request = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    loop {
        let read = timeout(Duration::from_secs(3), socket.read(&mut chunk))
            .await
            .map_err(|_| "request read timed out".to_string())?
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() >= MAX_REQUEST_BYTES {
            return Err("request exceeded local player limit".into());
        }
    }
    String::from_utf8(request).map_err(|_| "request was not UTF-8".into())
}

async fn respond(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    extra_headers: &str,
) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n{extra_headers}Connection: close\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

async fn handle_request(
    mut socket: TcpStream,
    app: &AppHandle,
    spotify: &SpotifyState,
    embedded: &EmbeddedSpotifyState,
    secret: &str,
) -> Result<(), String> {
    let request = read_request(&mut socket).await?;
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "request target was invalid".to_string())?;

    match (method, url.path()) {
        ("GET", "/") => {
            respond(
                &mut socket,
                "200 OK",
                "text/html; charset=utf-8",
                PLAYER_HTML,
                PLAYER_SECURITY_HEADERS,
            )
            .await
        }
        ("GET", "/player.js") => {
            respond(
                &mut socket,
                "200 OK",
                "text/javascript; charset=utf-8",
                PLAYER_JS,
                "",
            )
            .await
        }
        ("GET", "/mode") if has_session(&url, secret) => {
            let mode = embedded
                .status()
                .await
                .mode
                .unwrap_or_else(|| "stopped".into());
            let body = serde_json::json!({ "mode": mode }).to_string();
            respond(&mut socket, "200 OK", "application/json", &body, "").await
        }
        ("GET", "/token") if has_session(&url, secret) => match ensure_fresh(spotify).await {
            Ok(token) => {
                let body = serde_json::json!({ "access_token": token }).to_string();
                respond(&mut socket, "200 OK", "application/json", &body, "").await
            }
            Err(error) => {
                let body = serde_json::json!({ "error": error.to_string() }).to_string();
                respond(
                    &mut socket,
                    "401 Unauthorized",
                    "application/json",
                    &body,
                    "",
                )
                .await
            }
        },
        ("POST", "/ready") if has_session(&url, secret) => {
            let Some(device_id) = query_value(&url, "device_id").filter(|id| valid_device_id(id))
            else {
                return respond(
                    &mut socket,
                    "400 Bad Request",
                    "text/plain",
                    "invalid device",
                    "",
                )
                .await;
            };
            embedded.set_device(secret, device_id).await;
            emit_status(app, embedded).await;
            respond(&mut socket, "204 No Content", "text/plain", "", "").await
        }
        ("POST", "/activate") if has_session(&url, secret) => {
            let device_id = embedded.status().await.device_id;
            let result = match device_id {
                Some(device_id) => player::transfer(spotify, &device_id, true).await,
                None => Err(player::PlayerError::NoActiveDevice),
            };
            match result {
                Ok(()) => {
                    embedded.set_activated(secret, true).await;
                    emit_status(app, embedded).await;
                    respond(&mut socket, "204 No Content", "text/plain", "", "").await
                }
                Err(error) => {
                    embedded.set_error(secret, error.to_string()).await;
                    emit_status(app, embedded).await;
                    let body = serde_json::json!({ "error": error.to_string() }).to_string();
                    respond(&mut socket, "409 Conflict", "application/json", &body, "").await
                }
            }
        }
        ("POST", "/not-ready") if has_session(&url, secret) => {
            embedded.clear_device(secret).await;
            emit_status(app, embedded).await;
            respond(&mut socket, "204 No Content", "text/plain", "", "").await
        }
        ("POST", "/sdk-error") if has_session(&url, secret) => {
            let kind = query_value(&url, "kind").unwrap_or_else(|| "SDK".into());
            let message = query_value(&url, "message").unwrap_or_else(|| "unknown error".into());
            embedded.set_sdk_error(secret, &kind, &message).await;
            emit_status(app, embedded).await;
            respond(&mut socket, "204 No Content", "text/plain", "", "").await
        }
        _ => respond(&mut socket, "404 Not Found", "text/plain", "not found", "").await,
    }
}

const PLAYER_SECURITY_HEADERS: &str = concat!(
    "Content-Security-Policy: default-src 'none'; ",
    "script-src 'self' https://sdk.scdn.co; ",
    "connect-src 'self' https://api.spotify.com https://*.spotify.com https://*.scdn.co wss://*.spotify.com; ",
    "media-src blob: https://*.spotify.com https://*.scdn.co; worker-src blob:; ",
    "frame-src https://sdk.scdn.co https://*.spotify.com; frame-ancestors 'none'; ",
    "style-src 'unsafe-inline'; img-src data: https://i.scdn.co https://*.scdn.co;\r\n",
    // Spotify's SDK creates a cross-origin protected-media iframe. CSP still limits
    // the permitted frame/connect origins, while Permissions-Policy must delegate
    // autoplay and EME to that Spotify-owned frame.
    "Permissions-Policy: autoplay=*, encrypted-media=*\r\n",
    "Cross-Origin-Resource-Policy: same-origin\r\n",
);

const PLAYER_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Aura Player</title><style>
html,body{height:100%;margin:0;background:#090d16;color:#edf8ff;font:14px system-ui,sans-serif}main{padding:22px;border:1px solid #22d3ee;min-height:100%;box-sizing:border-box}small{display:block;color:#91a0b7;margin:8px 0 18px;line-height:1.45}button{background:#22d3ee;color:#051018;border:0;border-radius:8px;padding:11px 16px;font-weight:800;cursor:pointer;margin:0 8px 8px 0}button:disabled{opacity:.45;cursor:wait}.error{color:#fb7185}.ok{color:#67e8f9}
</style></head><body><main><strong>Aura on this PC</strong><small id="status">Connecting to Spotify's protected player…</small><button id="activate" disabled>Activate Aura Playback</button></main><script src="/player.js"></script><script src="https://sdk.scdn.co/spotify-player.js"></script></body></html>"#;

const PLAYER_JS: &str = r#"(() => {
  'use strict';
  const session = location.hash.slice(1);
  history.replaceState(null, '', '/');
  const status = document.getElementById('status');
  const activate = document.getElementById('activate');
  let player = null;
  let deviceId = '';
  let playerMode = 'unknown';
  const show = (message, mode = '') => { status.textContent = message; status.className = mode; };
  const endpoint = (path, params = {}) => {
    const query = new URLSearchParams({ session, ...params });
    return `${path}?${query}`;
  };
  const reportError = (kind, message) => {
    if (kind === 'Initialization' && playerMode === 'embedded_webview') {
      show('Protected audio is unavailable here. Return to Aura and choose Open Browser Player.', 'error');
    } else if (kind === 'Initialization') {
      show('Protected audio could not start. Use current Edge or Chrome and allow protected content/Widevine.', 'error');
    } else {
      show(`${kind}: ${message}`, 'error');
    }
    fetch(endpoint('/sdk-error', { kind, message }), { method: 'POST' }).catch(() => {});
  };
  const fetchToken = async (callback) => {
    try {
      const response = await fetch(endpoint('/token'), { cache: 'no-store' });
      const data = await response.json();
      if (!response.ok || !data.access_token) throw new Error(data.error || 'token unavailable');
      callback(data.access_token);
    } catch (error) { reportError('Authentication', String(error)); }
  };
  const modeReady = fetch(endpoint('/mode'), { cache: 'no-store' })
    .then((response) => response.ok ? response.json() : Promise.reject(new Error(`HTTP ${response.status}`)))
    .then((data) => { playerMode = data.mode || 'unknown'; })
    .catch(() => {});
  window.onSpotifyWebPlaybackSDKReady = async () => {
    await modeReady;
    player = new Spotify.Player({
      name: 'Aura on this PC',
      getOAuthToken: fetchToken,
      volume: 0.55,
      enableMediaSession: true,
    });
    player.addListener('ready', ({ device_id }) => {
      deviceId = device_id;
      activate.disabled = false;
      show('Ready. Press Activate once to make Aura the playback device.', 'ok');
      fetch(endpoint('/ready', { device_id }), { method: 'POST' }).catch(() => {});
    });
    player.addListener('not_ready', () => {
      deviceId = '';
      activate.disabled = true;
      show('Aura Player went offline. Reopen it from Aura.', 'error');
      fetch(endpoint('/not-ready'), { method: 'POST' }).catch(() => {});
    });
    player.addListener('initialization_error', ({ message }) => reportError('Initialization', message));
    player.addListener('authentication_error', ({ message }) => reportError('Authentication', message));
    player.addListener('account_error', ({ message }) => reportError('Account', message));
    player.addListener('playback_error', ({ message }) => reportError('Playback', message));
    player.addListener('autoplay_failed', () => show('Press Activate Aura Playback once to allow audio.', 'error'));
    player.connect().then((connected) => {
      if (!connected) reportError('Connection', 'Spotify did not accept the embedded player.');
    });
  };
  activate.addEventListener('click', async () => {
    if (!player || !deviceId) return;
    activate.disabled = true;
    try {
      await player.activateElement();
      const response = await fetch(endpoint('/activate'), { method: 'POST' });
      if (!response.ok) {
        const data = await response.json().catch(() => ({}));
        throw new Error(data.error || `HTTP ${response.status}`);
      }
      show('Aura is active. Use the controls in the main Aura window.', 'ok');
    } catch (error) {
      show(String(error), 'error');
      activate.disabled = false;
    }
  });
  addEventListener('beforeunload', () => { if (player) player.disconnect(); });
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_and_session_queries_are_validated() {
        assert!(valid_device_id("abcdef0123456789"));
        assert!(!valid_device_id("short"));
        assert!(!valid_device_id("device id with spaces"));

        let url = Url::parse("http://127.0.0.1/token?session=abc").unwrap();
        assert!(has_session(&url, "abc"));
        assert!(!has_session(&url, "abd"));
    }

    #[test]
    fn player_shell_does_not_embed_credentials() {
        assert!(!PLAYER_HTML.contains("access_token"));
        assert!(!PLAYER_JS.contains("client_secret"));
        assert!(PLAYER_SECURITY_HEADERS.contains("sdk.scdn.co"));
        assert!(PLAYER_SECURITY_HEADERS.contains("https://*.scdn.co"));
        assert!(PLAYER_SECURITY_HEADERS.contains("default-src 'none'"));
        assert!(PLAYER_SECURITY_HEADERS.contains("frame-ancestors 'none'"));
        assert!(PLAYER_SECURITY_HEADERS.contains("encrypted-media=*"));
        assert!(PLAYER_SECURITY_HEADERS.contains("autoplay=*"));
        assert!(!PLAYER_SECURITY_HEADERS.contains("encrypted-media=(self)"));
        assert!(!PLAYER_SECURITY_HEADERS.contains("autoplay=(self)"));
    }

    #[test]
    fn initialization_errors_have_mode_specific_recovery() {
        let embedded = friendly_sdk_error(
            "Initialization",
            "Failed to initialize player",
            Some(PlayerMode::EmbeddedWebview),
        );
        assert!(embedded.contains("Open Browser Player"));
        assert!(embedded.contains("Spotify Desktop"));

        let browser = friendly_sdk_error(
            "Initialization",
            "Failed to initialize player",
            Some(PlayerMode::CompatibleBrowser),
        );
        assert!(browser.contains("Edge or Chrome"));
        assert!(browser.contains("Widevine"));
    }

    #[tokio::test]
    async fn stale_session_updates_cannot_replace_the_current_device() {
        let state = EmbeddedSpotifyState::default();
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        state
            .install_runtime(
                shutdown_tx,
                "current-session".into(),
                PlayerMode::EmbeddedWebview,
            )
            .await;

        state
            .set_device("old-session", "old-device-12345".into())
            .await;
        state
            .set_sdk_error("old-session", "Initialization", "late error")
            .await;
        let untouched = state.status().await;
        assert!(untouched.device_id.is_none());
        assert!(!untouched.error);

        state
            .set_device("current-session", "current-device-12345".into())
            .await;
        assert_eq!(
            state.status().await.device_id.as_deref(),
            Some("current-device-12345")
        );
        state.stop_runtime().await;
    }
}
