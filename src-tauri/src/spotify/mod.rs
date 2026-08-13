pub mod embedded;
pub mod oauth;
pub mod pkce;
pub mod player;

pub use embedded::EmbeddedSpotifyState;
pub use oauth::SpotifyState;

#[tauri::command]
pub async fn spotify_login(state: tauri::State<'_, SpotifyState>) -> Result<(), String> {
    oauth::login(state.inner().clone())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spotify_play(
    state: tauri::State<'_, SpotifyState>,
    device_id: Option<String>,
) -> Result<(), String> {
    player::play(state.inner(), device_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spotify_pause(
    state: tauri::State<'_, SpotifyState>,
    device_id: Option<String>,
) -> Result<(), String> {
    player::pause(state.inner(), device_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spotify_previous(
    state: tauri::State<'_, SpotifyState>,
    device_id: Option<String>,
) -> Result<(), String> {
    player::previous(state.inner(), device_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spotify_skip(
    state: tauri::State<'_, SpotifyState>,
    device_id: Option<String>,
) -> Result<(), String> {
    player::skip_next(state.inner(), device_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn spotify_playback_status(
    state: tauri::State<'_, SpotifyState>,
) -> Result<player::PlaybackState, String> {
    player::playback_state(state.inner())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn spotify_devices(
    state: tauri::State<'_, SpotifyState>,
    embedded_state: tauri::State<'_, EmbeddedSpotifyState>,
) -> Result<Vec<player::SpotifyDevice>, String> {
    let mut devices = player::devices(state.inner())
        .await
        .map_err(|error| error.to_string())?;
    if let Some(embedded_device) = embedded::device(embedded_state.inner()).await {
        let embedded_id = embedded_device.id.as_deref();
        if !devices
            .iter()
            .any(|device| device.id.as_deref() == embedded_id)
        {
            devices.insert(0, embedded_device);
        }
    }
    Ok(devices)
}

#[tauri::command]
pub async fn spotify_transfer(
    state: tauri::State<'_, SpotifyState>,
    device_id: String,
) -> Result<(), String> {
    player::transfer(state.inner(), &device_id, true)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn spotify_open_web_player() -> Result<(), String> {
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", "https://open.spotify.com/"])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open Spotify Web Player: {error}"))
}
