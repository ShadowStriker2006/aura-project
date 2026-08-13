use reqwest::header::CONTENT_LENGTH;
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::OnceLock;
use std::time::Duration;

use super::oauth::{ensure_fresh, OAuthError, SpotifyState};

const PLAYER_URL: &str = "https://api.spotify.com/v1/me/player";

#[derive(Debug)]
pub enum PlayerError {
    NotAuthenticated(String),
    RequestFailed(String),
    NoActiveDevice,
    SpotifyResponse(u16, String),
}

impl std::fmt::Display for PlayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerError::NotAuthenticated(reason) => write!(f, "{reason}"),
            PlayerError::RequestFailed(reason) => write!(f, "Spotify request failed: {reason}"),
            PlayerError::NoActiveDevice => write!(
                f,
                "No active Spotify device. Start Aura Player for playback on this PC, or activate an official Spotify client and refresh devices."
            ),
            PlayerError::SpotifyResponse(403, message) => write!(
                f,
                "Spotify denied playback control. A Spotify Premium account is required. {message}"
            ),
            PlayerError::SpotifyResponse(411, _) => write!(
                f,
                "Spotify rejected an empty playback request. Aura sent a corrected request; reconnect Spotify once if this message persists."
            ),
            PlayerError::SpotifyResponse(status, message) => {
                write!(f, "Spotify returned HTTP {status}: {message}")
            }
        }
    }
}

impl std::error::Error for PlayerError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpotifyDevice {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub device_type: String,
    pub name: String,
    pub is_active: bool,
    pub is_restricted: bool,
    pub volume_percent: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct DevicesResponse {
    devices: Vec<SpotifyDevice>,
}

#[derive(Debug, Deserialize)]
struct SpotifyArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct SpotifyTrack {
    name: String,
    artists: Vec<SpotifyArtist>,
}

#[derive(Debug, Deserialize)]
struct RawPlaybackState {
    is_playing: bool,
    progress_ms: Option<u64>,
    item: Option<SpotifyTrack>,
    device: SpotifyDevice,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackState {
    pub is_playing: bool,
    pub progress_ms: Option<u64>,
    pub track_name: Option<String>,
    pub artists: Vec<String>,
    pub device: Option<SpotifyDevice>,
}

fn log_ok(message: &str) {
    println!("[AURA::SPOTIFY::PLAYER][OK] {message}");
}

fn log_err(message: &str) {
    eprintln!("[AURA::SPOTIFY::PLAYER][ERR] {message}");
}

static CLIENT: OnceLock<Client> = OnceLock::new();

fn client() -> Result<&'static Client, PlayerError> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let built = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;
    Ok(CLIENT.get_or_init(|| built))
}

async fn token(state: &SpotifyState) -> Result<String, PlayerError> {
    ensure_fresh(state)
        .await
        .map_err(|error: OAuthError| PlayerError::NotAuthenticated(error.to_string()))
}

async fn error_from_response(response: reqwest::Response) -> PlayerError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "playback request was not accepted".into());
    PlayerError::SpotifyResponse(status, message)
}

async fn simple_player_call(
    state: &SpotifyState,
    method: Method,
    endpoint: &str,
    device_id: Option<&str>,
) -> Result<(), PlayerError> {
    let token = token(state).await?;
    let mut url = reqwest::Url::parse(&format!("{PLAYER_URL}/{endpoint}"))
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;
    if let Some(device_id) = device_id.filter(|value| !value.trim().is_empty()) {
        url.query_pairs_mut().append_pair("device_id", device_id);
    }
    // Spotify's player endpoints accept an empty body, but some of its edge
    // servers respond with HTTP 411 unless Content-Length is explicit.
    let response = client()?
        .request(method.clone(), url)
        .bearer_auth(token)
        .header(CONTENT_LENGTH, 0)
        .body(Vec::new())
        .send()
        .await
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;

    match response.status() {
        StatusCode::NO_CONTENT | StatusCode::OK => {
            log_ok(&format!("{method} {endpoint} completed"));
            Ok(())
        }
        StatusCode::NOT_FOUND => Err(PlayerError::NoActiveDevice),
        _ => {
            let error = error_from_response(response).await;
            log_err(&format!("{method} {endpoint} failed: {error}"));
            Err(error)
        }
    }
}

pub async fn devices(state: &SpotifyState) -> Result<Vec<SpotifyDevice>, PlayerError> {
    let token = token(state).await?;
    let response = client()?
        .get(format!("{PLAYER_URL}/devices"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;

    if !response.status().is_success() {
        return Err(error_from_response(response).await);
    }
    let parsed: DevicesResponse = response
        .json()
        .await
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;
    Ok(parsed.devices)
}

pub async fn playback_state(state: &SpotifyState) -> Result<PlaybackState, PlayerError> {
    let token = token(state).await?;
    let response = client()?
        .get(PLAYER_URL)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;

    if response.status() == StatusCode::NO_CONTENT {
        return Ok(PlaybackState {
            is_playing: false,
            progress_ms: None,
            track_name: None,
            artists: Vec::new(),
            device: None,
        });
    }
    if !response.status().is_success() {
        return Err(error_from_response(response).await);
    }
    let parsed: RawPlaybackState = response
        .json()
        .await
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;
    Ok(PlaybackState {
        is_playing: parsed.is_playing,
        progress_ms: parsed.progress_ms,
        track_name: parsed.item.as_ref().map(|track| track.name.clone()),
        artists: parsed
            .item
            .map(|track| {
                track
                    .artists
                    .into_iter()
                    .map(|artist| artist.name)
                    .collect()
            })
            .unwrap_or_default(),
        device: Some(parsed.device),
    })
}

pub async fn transfer(
    state: &SpotifyState,
    device_id: &str,
    start_playback: bool,
) -> Result<(), PlayerError> {
    if device_id.trim().is_empty() {
        return Err(PlayerError::NoActiveDevice);
    }
    let token = token(state).await?;
    let response = client()?
        .put(PLAYER_URL)
        .bearer_auth(token)
        .json(&json!({ "device_ids": [device_id], "play": start_playback }))
        .send()
        .await
        .map_err(|error| PlayerError::RequestFailed(error.to_string()))?;

    if response.status().is_success() || response.status() == StatusCode::NO_CONTENT {
        log_ok("playback transferred to selected device");
        Ok(())
    } else {
        Err(error_from_response(response).await)
    }
}

pub async fn play(state: &SpotifyState, device_id: Option<&str>) -> Result<(), PlayerError> {
    match simple_player_call(state, Method::PUT, "play", device_id).await {
        Ok(()) => Ok(()),
        Err(PlayerError::NoActiveDevice) => {
            let available = devices(state).await?;
            let candidate = available
                .iter()
                .find(|device| !device.is_restricted)
                .and_then(|device| device.id.as_deref());
            match candidate {
                Some(device_id) => transfer(state, device_id, true).await,
                None => Err(PlayerError::NoActiveDevice),
            }
        }
        Err(error) => Err(error),
    }
}

pub async fn pause(state: &SpotifyState, device_id: Option<&str>) -> Result<(), PlayerError> {
    simple_player_call(state, Method::PUT, "pause", device_id).await
}

pub async fn previous(state: &SpotifyState, device_id: Option<&str>) -> Result<(), PlayerError> {
    simple_player_call(state, Method::POST, "previous", device_id).await
}

pub async fn skip_next(state: &SpotifyState, device_id: Option<&str>) -> Result<(), PlayerError> {
    simple_player_call(state, Method::POST, "next", device_id).await
}
