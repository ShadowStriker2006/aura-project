use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

/// In-memory only, resets to "default" each launch — same pattern as
/// SpotifyState/DDragonCache. Exists solely to let the overlay window (a
/// genuinely separate document/webview, sharing no DOM or CSS state with the
/// dashboard) know which theme is active, both on its own startup and live
/// while both windows are open.
#[derive(Clone)]
pub struct ThemeState(pub Arc<Mutex<String>>);

impl Default for ThemeState {
    fn default() -> Self {
        ThemeState(Arc::new(Mutex::new("default".to_string())))
    }
}

fn log_err(msg: &str) {
    eprintln!("[AURA::THEME][ERR] {}", msg);
}

/// Stores the new theme AND broadcasts it — any caller just calls this once,
/// no need to separately remember to emit after invoking it.
#[tauri::command]
pub async fn set_theme(
    app: AppHandle,
    state: tauri::State<'_, ThemeState>,
    theme: String,
) -> Result<(), String> {
    *state.0.lock().await = theme.clone();
    if let Err(e) = app.emit("aura-theme-changed", &theme) {
        log_err(&format!("broadcast failed: {}", e));
    }
    Ok(())
}

#[tauri::command]
pub async fn get_theme(state: tauri::State<'_, ThemeState>) -> Result<String, String> {
    Ok(state.0.lock().await.clone())
}
