use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

pub const LABEL: &str = "overlay";
pub const LAYOUT_CHANGED_EVENT: &str = "overlay:layout-changed";

const STANDBY_WIDTH: f64 = 32.0;
const STANDBY_HEIGHT: f64 = 32.0;
const COMPACT_WIDTH: f64 = 432.0;
const COMPACT_HEIGHT: f64 = 52.0;
const EXPANDED_WIDTH: f64 = 520.0;
const EXPANDED_HEIGHT: f64 = 150.0;
const OVERLAY_MARGIN: f64 = 20.0;
const OVERLAY_TOP_OFFSET: f64 = 40.0;

const SCALE_PRESETS: [u8; 3] = [75, 90, 100];
const MIN_OPACITY_PERCENT: u8 = 40;
const MAX_OPACITY_PERCENT: u8 = 100;

fn log_ok(message: &str) {
    println!("[AURA::OVERLAY][OK] {message}");
}

fn log_err(message: &str) {
    eprintln!("[AURA::OVERLAY][ERR] {message}");
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayMode {
    #[default]
    Standby,
    Compact,
    Expanded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayLayoutConfig {
    pub mode: OverlayMode,
    pub scale_percent: u8,
    pub opacity_percent: u8,
    pub locked: bool,
}

impl Default for OverlayLayoutConfig {
    fn default() -> Self {
        Self {
            mode: OverlayMode::Standby,
            scale_percent: 100,
            opacity_percent: 55,
            // The overlay must never steal a gameplay click when it first
            // appears. Users can unlock it from Aura's tray menu.
            locked: true,
        }
    }
}

impl OverlayLayoutConfig {
    fn normalized(mut self) -> Self {
        self.scale_percent = nearest_scale_preset(self.scale_percent);
        self.opacity_percent = self
            .opacity_percent
            .clamp(MIN_OPACITY_PERCENT, MAX_OPACITY_PERCENT);
        self
    }

    fn logical_size(&self) -> LogicalSize<f64> {
        let (width, height) = match self.mode {
            OverlayMode::Standby => (STANDBY_WIDTH, STANDBY_HEIGHT),
            OverlayMode::Compact => (COMPACT_WIDTH, COMPACT_HEIGHT),
            OverlayMode::Expanded => (EXPANDED_WIDTH, EXPANDED_HEIGHT),
        };
        let scale = f64::from(self.scale_percent) / 100.0;
        LogicalSize::new((width * scale).ceil(), (height * scale).ceil())
    }
}

#[derive(Clone, Debug)]
struct OverlayRuntimeState {
    layout: OverlayLayoutConfig,
    match_active: bool,
    preferred_active_mode: OverlayMode,
}

impl Default for OverlayRuntimeState {
    fn default() -> Self {
        Self {
            layout: OverlayLayoutConfig::default(),
            match_active: false,
            preferred_active_mode: OverlayMode::Compact,
        }
    }
}

/// Volatile overlay preferences and lifecycle state. Intentionally not
/// persisted: gameplay HUD settings disappear with the process and never add
/// disk I/O during a match.
pub struct OverlayState(Mutex<OverlayRuntimeState>);

impl Default for OverlayState {
    fn default() -> Self {
        Self(Mutex::new(OverlayRuntimeState::default()))
    }
}

impl OverlayState {
    fn snapshot(&self) -> Result<OverlayLayoutConfig, String> {
        self.0
            .lock()
            .map(|runtime| runtime.layout.clone())
            .map_err(|_| "overlay layout state lock was poisoned".to_string())
    }

    fn runtime_snapshot(&self) -> Result<OverlayRuntimeState, String> {
        self.0
            .lock()
            .map(|runtime| runtime.clone())
            .map_err(|_| "overlay layout state lock was poisoned".to_string())
    }

    fn restore(&self, runtime: OverlayRuntimeState) -> Result<(), String> {
        *self
            .0
            .lock()
            .map_err(|_| "overlay layout state lock was poisoned".to_string())? = runtime;
        Ok(())
    }

    fn apply_request(&self, config: OverlayLayoutConfig) -> Result<OverlayLayoutConfig, String> {
        let mut runtime = self
            .0
            .lock()
            .map_err(|_| "overlay layout state lock was poisoned".to_string())?;
        let mut config = config.normalized();
        if matches!(config.mode, OverlayMode::Compact | OverlayMode::Expanded) {
            runtime.preferred_active_mode = config.mode;
        }
        // Idle windows are always a pill. The requested active layout remains
        // in RAM and is restored by begin_match().
        if !runtime.match_active {
            config.mode = OverlayMode::Standby;
        }
        runtime.layout = config.clone();
        Ok(config)
    }

    fn begin_match(&self) -> Result<OverlayLayoutConfig, String> {
        let mut runtime = self
            .0
            .lock()
            .map_err(|_| "overlay layout state lock was poisoned".to_string())?;
        runtime.match_active = true;
        let preferred_active_mode = runtime.preferred_active_mode;
        runtime.layout.mode = preferred_active_mode;
        runtime.layout.locked = true;
        Ok(runtime.layout.clone())
    }

    fn finish_match(&self) -> Result<OverlayLayoutConfig, String> {
        let mut runtime = self
            .0
            .lock()
            .map_err(|_| "overlay layout state lock was poisoned".to_string())?;
        runtime.match_active = false;
        runtime.layout.mode = OverlayMode::Standby;
        runtime.layout.locked = true;
        Ok(runtime.layout.clone())
    }

    fn prepare_hide(&self) -> Result<OverlayLayoutConfig, String> {
        let mut runtime = self
            .0
            .lock()
            .map_err(|_| "overlay layout state lock was poisoned".to_string())?;
        runtime.layout.locked = true;
        Ok(runtime.layout.clone())
    }
}

fn nearest_scale_preset(requested: u8) -> u8 {
    *SCALE_PRESETS
        .iter()
        .min_by_key(|candidate| requested.abs_diff(**candidate))
        .expect("scale preset list must not be empty")
}

fn position_for_monitor(monitor: &Monitor, size: LogicalSize<f64>) -> LogicalPosition<f64> {
    let scale_factor = monitor.scale_factor();
    let origin = monitor.position().to_logical::<f64>(scale_factor);
    let bounds = monitor.size().to_logical::<f64>(scale_factor);
    let maximum_x = origin.x + (bounds.width - size.width).max(0.0);
    let maximum_y = origin.y + (bounds.height - size.height).max(0.0);
    LogicalPosition::new(
        (origin.x + bounds.width - size.width - OVERLAY_MARGIN).clamp(origin.x, maximum_x),
        (origin.y + OVERLAY_TOP_OFFSET).clamp(origin.y, maximum_y),
    )
}

fn target_monitor(app: &AppHandle, window: Option<&WebviewWindow>) -> Option<Monitor> {
    if let Some(window) = window {
        if let Ok(Some(monitor)) = window.current_monitor() {
            return Some(monitor);
        }
    }
    match app.primary_monitor() {
        Ok(monitor) => monitor,
        Err(error) => {
            log_err(&format!("primary monitor lookup failed: {error}"));
            None
        }
    }
}

fn apply_window_layout(
    app: &AppHandle,
    window: &WebviewWindow,
    config: &OverlayLayoutConfig,
) -> Result<(), String> {
    let size = config.logical_size();
    let monitor = target_monitor(app, Some(window));

    window
        .set_size(size)
        .map_err(|error| format!("overlay resize failed: {error}"))?;

    if let Some(monitor) = monitor {
        window
            .set_position(position_for_monitor(&monitor, size))
            .map_err(|error| format!("overlay positioning failed: {error}"))?;
    }

    // Enabling cursor pass-through before disabling focus keeps the locked
    // transition atomic from the player's perspective. Unlocking reverses the
    // order so the WebView is ready to accept its first click.
    if config.locked {
        window
            .set_ignore_cursor_events(true)
            .map_err(|error| format!("overlay click-through failed: {error}"))?;
        window
            .set_focusable(false)
            .map_err(|error| format!("overlay focus lock failed: {error}"))?;
    } else {
        window
            .set_focusable(true)
            .map_err(|error| format!("overlay focus unlock failed: {error}"))?;
        window
            .set_ignore_cursor_events(false)
            .map_err(|error| format!("overlay interaction enable failed: {error}"))?;
    }

    Ok(())
}

fn emit_layout_changed(app: &AppHandle, config: &OverlayLayoutConfig) {
    // Global emission keeps both the overlay controls and dashboard settings in
    // sync when a tray action changes the native lock state.
    if let Err(error) = app.emit(LAYOUT_CHANGED_EVENT, config.clone()) {
        log_err(&format!("layout event emission failed: {error}"));
    }
}

/// Created lazily on first use. Keeping a second WebView alive while League is
/// closed adds avoidable idle memory, so telemetry mounts this window only when
/// a match becomes active or the user explicitly toggles it from the tray.
pub fn create(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }

    let config = app.state::<OverlayState>().snapshot()?;
    let size = config.logical_size();
    let position = target_monitor(app, None)
        .map(|monitor| position_for_monitor(&monitor, size))
        .unwrap_or_else(|| {
            log_err("monitor lookup failed, defaulting overlay position to 40,40");
            LogicalPosition::new(40.0, 40.0)
        });

    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Aura Overlay")
        .inner_size(size.width, size.height)
        .position(position.x, position.y)
        .always_on_top(true)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .focusable(!config.locked)
        .focused(false)
        .transparent(true)
        .shadow(false)
        .visible(false)
        .build()
        .map_err(|error| format!("overlay WebView creation failed: {error}"))?;

    apply_window_layout(app, &window, &config)?;
    log_ok("overlay window prepared (hidden, adaptive layout applied)");
    Ok(())
}

pub fn show(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_none() {
        create(app).map_err(|error| format!("overlay creation failed: {error}"))?;
    }
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "overlay window was not created".to_string())?;
    let config = app.state::<OverlayState>().snapshot()?;
    apply_window_layout(app, &window, &config)?;
    window
        .unminimize()
        .map_err(|error| format!("overlay restore failed: {error}"))?;
    window
        .set_always_on_top(true)
        .map_err(|error| format!("overlay topmost mode failed: {error}"))?;
    window
        .show()
        .map_err(|error| format!("overlay show failed: {error}"))?;
    emit_layout_changed(app, &config);
    log_ok("overlay shown");
    Ok(())
}

/// Match-start entry point used by telemetry. It restores the last compact or
/// expanded choice from volatile RAM and always opens click-through.
pub fn show_for_match(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<OverlayState>();
    state.begin_match()?;
    // Match activity is authoritative even if Windows temporarily rejects a
    // WebView operation. A later dashboard/tray show can then retry the active
    // layout instead of incorrectly falling back to standby.
    show(app)
}

pub fn hide(app: &AppHandle) -> Result<(), String> {
    // Any hidden/closed overlay is re-armed in safe pass-through mode. This
    // prevents a HUD that was temporarily unlocked for editing from stealing
    // the first click when telemetry automatically opens it next match.
    if let Some(state) = app.try_state::<OverlayState>() {
        let config = state.prepare_hide()?;
        emit_layout_changed(app, &config);
        if let Some(window) = app.get_webview_window(LABEL) {
            // Lock before close so even a rejected close operation leaves a
            // safe click-through HUD rather than an interactive game overlay.
            apply_window_layout(app, &window, &config)?;
        }
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        window
            .close()
            .map_err(|error| format!("overlay close failed: {error}"))?;
        log_ok("overlay hidden and its WebView memory released");
    }
    // Emit once more after a successful close. The dashboard uses this event
    // to refresh visibility as well as layout; the earlier emission guarantees
    // safe lock-state feedback even if Windows rejects the close operation.
    if let Some(state) = app.try_state::<OverlayState>() {
        emit_layout_changed(app, &state.snapshot()?);
    }
    Ok(())
}

/// Match-end entry point used by telemetry. Unlike a manual hide, this marks
/// the runtime inactive so a later tray show renders only the standby pill.
pub fn end_match(app: &AppHandle) -> Result<(), String> {
    app.state::<OverlayState>().finish_match()?;
    hide(app)
}

fn update_layout(
    app: &AppHandle,
    state: &OverlayState,
    config: OverlayLayoutConfig,
) -> Result<OverlayLayoutConfig, String> {
    if !config.locked {
        let window = app
            .get_webview_window(LABEL)
            .ok_or_else(|| "overlay must be visible before controls can be unlocked".to_string())?;
        if !window
            .is_visible()
            .map_err(|error| format!("overlay visibility check failed: {error}"))?
        {
            return Err("overlay must be visible before controls can be unlocked".to_string());
        }
    }
    let previous = state.runtime_snapshot()?;
    let config = state.apply_request(config)?;

    // Roll back the volatile state if the OS rejects the window change. A
    // missing window simply records the setting for the next lazy creation.
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Err(error) = apply_window_layout(app, &window, &config) {
            state.restore(previous)?;
            return Err(error);
        }
    }
    emit_layout_changed(app, &config);
    Ok(config)
}

/// Tray/native escape hatch for a click-through overlay. This must remain
/// available because an in-window button cannot receive clicks while locked.
pub fn toggle_interaction(app: &AppHandle) -> Result<OverlayLayoutConfig, String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "overlay is not open; show it before unlocking controls".to_string())?;
    if !window
        .is_visible()
        .map_err(|error| format!("overlay visibility check failed: {error}"))?
    {
        return Err("overlay is hidden; show it before unlocking controls".to_string());
    }
    let state = app.state::<OverlayState>();
    let mut config = state.snapshot()?;
    config.locked = !config.locked;
    update_layout(app, state.inner(), config)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayStatus {
    pub visible: bool,
    pub layout: OverlayLayoutConfig,
}

#[tauri::command]
pub async fn show_overlay(app: AppHandle) -> Result<OverlayStatus, String> {
    show(&app)?;
    let state = app.state::<OverlayState>();
    overlay_status(app.clone(), state)
}

#[tauri::command]
pub async fn hide_overlay(
    app: AppHandle,
    state: State<'_, OverlayState>,
) -> Result<OverlayStatus, String> {
    hide(&app)?;
    Ok(OverlayStatus {
        visible: false,
        layout: state.snapshot()?,
    })
}

#[tauri::command]
pub fn overlay_status(
    app: AppHandle,
    state: State<'_, OverlayState>,
) -> Result<OverlayStatus, String> {
    let visible = match app.get_webview_window(LABEL) {
        Some(window) => window.is_visible().unwrap_or(false),
        None => false,
    };
    Ok(OverlayStatus {
        visible,
        layout: state.snapshot()?,
    })
}

#[tauri::command]
pub fn get_overlay_layout(state: State<'_, OverlayState>) -> Result<OverlayLayoutConfig, String> {
    state.snapshot()
}

#[tauri::command]
pub fn set_overlay_layout(
    app: AppHandle,
    state: State<'_, OverlayState>,
    config: OverlayLayoutConfig,
) -> Result<OverlayLayoutConfig, String> {
    update_layout(&app, state.inner(), config)
}

#[tauri::command]
pub fn toggle_overlay_interaction(app: AppHandle) -> Result<OverlayLayoutConfig, String> {
    toggle_interaction(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: OverlayMode, scale_percent: u8) -> OverlayLayoutConfig {
        OverlayLayoutConfig {
            mode,
            scale_percent,
            opacity_percent: 55,
            locked: true,
        }
    }

    #[test]
    fn scale_and_opacity_are_bounded_to_supported_values() {
        let below = OverlayLayoutConfig {
            scale_percent: 1,
            opacity_percent: 1,
            ..OverlayLayoutConfig::default()
        }
        .normalized();
        assert_eq!(below.scale_percent, 75);
        assert_eq!(below.opacity_percent, 40);

        let middle = OverlayLayoutConfig {
            scale_percent: 88,
            opacity_percent: 70,
            ..OverlayLayoutConfig::default()
        }
        .normalized();
        assert_eq!(middle.scale_percent, 90);
        assert_eq!(middle.opacity_percent, 70);

        let above = OverlayLayoutConfig {
            scale_percent: u8::MAX,
            opacity_percent: u8::MAX,
            ..OverlayLayoutConfig::default()
        }
        .normalized();
        assert_eq!(above.scale_percent, 100);
        assert_eq!(above.opacity_percent, 100);
    }

    #[test]
    fn native_window_sizes_match_frontend_base_geometry() {
        assert_eq!(
            config(OverlayMode::Standby, 100).logical_size(),
            LogicalSize::new(32.0, 32.0)
        );
        assert_eq!(
            config(OverlayMode::Compact, 75).logical_size(),
            LogicalSize::new(324.0, 39.0)
        );
        assert_eq!(
            config(OverlayMode::Expanded, 90).logical_size(),
            LogicalSize::new(468.0, 135.0)
        );
        assert_eq!(
            config(OverlayMode::Expanded, 100).logical_size(),
            LogicalSize::new(520.0, 150.0)
        );
    }

    #[test]
    fn nearest_scale_preset_is_deterministic_at_boundaries() {
        assert_eq!(nearest_scale_preset(82), 75);
        assert_eq!(nearest_scale_preset(83), 90);
        assert_eq!(nearest_scale_preset(95), 90);
        assert_eq!(nearest_scale_preset(96), 100);
    }

    #[test]
    fn lifecycle_distinguishes_manual_hide_from_match_end() {
        let state = OverlayState::default();
        let requested = OverlayLayoutConfig {
            mode: OverlayMode::Expanded,
            scale_percent: 90,
            opacity_percent: 60,
            locked: false,
        };

        // Idle requests remember the preference but cannot expand the pill.
        let idle = state.apply_request(requested.clone()).unwrap();
        assert_eq!(idle.mode, OverlayMode::Standby);

        let started = state.begin_match().unwrap();
        assert_eq!(started.mode, OverlayMode::Expanded);
        assert!(started.locked);

        let editing = state.apply_request(requested).unwrap();
        assert_eq!(editing.mode, OverlayMode::Expanded);
        assert!(!editing.locked);

        // Manual hide only re-locks. The active layout is still available for
        // a user-requested re-show during the same game.
        let manually_hidden = state.prepare_hide().unwrap();
        assert_eq!(manually_hidden.mode, OverlayMode::Expanded);
        assert!(manually_hidden.locked);
        assert!(state.runtime_snapshot().unwrap().match_active);

        // Match end changes the lifecycle and guarantees an idle pill.
        let ended = state.finish_match().unwrap();
        assert_eq!(ended.mode, OverlayMode::Standby);
        assert!(ended.locked);
        assert!(!state.runtime_snapshot().unwrap().match_active);

        // The next match restores only the preferred density, never the old
        // unlocked interaction state.
        let next_match = state.begin_match().unwrap();
        assert_eq!(next_match.mode, OverlayMode::Expanded);
        assert!(next_match.locked);
    }
}
