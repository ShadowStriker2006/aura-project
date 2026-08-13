//! Typed events shared by Aura's native live-client reader and the frontend.
//!
//! Keep these payloads deliberately small: `live:game-tick` is emitted at the
//! active-game tick rate, while `live:player-update` is intended for slower
//! item/build refreshes.

use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub const GAME_STATE_CHANGED_EVENT: &str = "game:state-changed";
pub const LIVE_GAME_TICK_EVENT: &str = "live:game-tick";
pub const LIVE_PLAYER_UPDATE_EVENT: &str = "live:player-update";
pub const DRAFT_UPDATE_EVENT: &str = "draft:update";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GameStatus {
    InLobby,
    ChampSelect,
    InGame,
    Ended,
}

#[derive(Default)]
pub struct LiveClientEventState {
    last_game_status: Mutex<Option<GameStatus>>,
}

impl GameStatus {
    /// Translate Riot's LCU gameflow strings to Aura's stable IPC contract.
    /// Unknown phases are ignored rather than being mislabeled.
    pub fn from_lcu_phase(phase: &str) -> Option<Self> {
        match phase.trim().to_ascii_lowercase().as_str() {
            "lobby" | "matchmaking" | "readycheck" => Some(Self::InLobby),
            "champselect" => Some(Self::ChampSelect),
            "gamestart" | "inprogress" | "reconnect" => Some(Self::InGame),
            "waitingforstats" | "preendofgame" | "endofgame" => Some(Self::Ended),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kda {
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePlayerPayload {
    pub summoner_name: String,
    pub champion_name: String,
    pub current_gold: f64,
    pub level: u32,
    pub creep_score: u32,
    pub creep_score_per_minute: f64,
    pub kill_participation_percent: f64,
    /// Current unspent gold plus the listed price of items still held.
    /// This is not lifetime gold earned and can fall after consuming/selling.
    pub observable_held_value: f64,
    pub observable_value_per_minute: f64,
    /// Exact earned GPM is not exposed by the official local endpoint.
    pub earned_gold_per_minute: f64,
    /// Riot's local Live Client Data API exposes level but not XP progress.
    pub xp_progress_percent: Option<f64>,
    pub kda: Kda,
    pub dpm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectivesPayload {
    pub dragon_type: Option<String>,
    /// Seconds remaining until the last observed dragon respawns; zero when no
    /// countdown is active.
    pub dragon_timer: f64,
    /// Seconds remaining until the last observed Baron respawns; zero when no
    /// countdown is active.
    pub baron_timer: f64,
}

/// The official local Live Client Data endpoints do not expose damage dealt to
/// champions, exact team total/earned gold, or XP progress. Required numeric
/// fields therefore use a zero sentinel until a truthful source exists, and
/// consumers must consult these flags before presenting them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricAvailabilityPayload {
    pub current_gold: bool,
    pub kda: bool,
    pub dpm: bool,
    pub team_gold_delta: bool,
    pub level: bool,
    pub creep_score: bool,
    pub creep_score_per_minute: bool,
    pub kill_participation_percent: bool,
    pub observable_held_value: bool,
    pub observable_value_per_minute: bool,
    pub earned_gold_per_minute: bool,
    pub xp_progress_percent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObservableValueSource {
    CurrentGoldPlusCurrentInventoryListedValue,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSourcesPayload {
    pub observable_held_value: Option<ObservableValueSource>,
    pub observable_value_per_minute: Option<ObservableValueSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveGameTickPayload {
    pub game_time: f64,
    pub active_player: ActivePlayerPayload,
    pub team_gold_delta: i32,
    pub objectives: ObjectivesPayload,
    pub metric_availability: MetricAvailabilityPayload,
    pub metric_sources: MetricSourcesPayload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PlayerTeam {
    Order,
    Harmony,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStatsPayload {
    pub summoner_name: String,
    pub champion_name: String,
    pub team: PlayerTeam,
    pub level: u32,
    pub creep_score: u32,
    pub items: Vec<u32>,
}

pub fn stream_game_status(app_handle: &tauri::AppHandle, status: GameStatus) -> Result<(), String> {
    let state = app_handle
        .try_state::<LiveClientEventState>()
        .ok_or_else(|| "live-client event state is not registered".to_string())?;
    let mut previous = state
        .last_game_status
        .lock()
        .map_err(|_| "live-client game status state lock failed".to_string())?;
    if *previous == Some(status) {
        return Ok(());
    }
    app_handle
        .emit(GAME_STATE_CHANGED_EVENT, status)
        .map_err(|error| format!("emit {GAME_STATE_CHANGED_EVENT} failed: {error}"))?;
    *previous = Some(status);
    Ok(())
}

pub fn stream_game_tick(
    app_handle: &tauri::AppHandle,
    payload: LiveGameTickPayload,
) -> Result<(), String> {
    app_handle
        .emit(LIVE_GAME_TICK_EVENT, payload)
        .map_err(|error| format!("emit {LIVE_GAME_TICK_EVENT} failed: {error}"))
}

pub fn stream_player_update(
    app_handle: &tauri::AppHandle,
    payload: PlayerStatsPayload,
) -> Result<(), String> {
    app_handle
        .emit(LIVE_PLAYER_UPDATE_EVENT, payload)
        .map_err(|error| format!("emit {LIVE_PLAYER_UPDATE_EVENT} failed: {error}"))
}

pub fn stream_draft_update(
    app_handle: &tauri::AppHandle,
    payload: &serde_json::Value,
) -> Result<(), String> {
    app_handle
        .emit(DRAFT_UPDATE_EVENT, payload)
        .map_err(|error| format!("emit {DRAFT_UPDATE_EVENT} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn game_status_uses_the_typescript_wire_values() {
        assert_eq!(to_value(GameStatus::InLobby).unwrap(), json!("IN_LOBBY"));
        assert_eq!(
            to_value(GameStatus::ChampSelect).unwrap(),
            json!("CHAMP_SELECT")
        );
        assert_eq!(to_value(GameStatus::InGame).unwrap(), json!("IN_GAME"));
        assert_eq!(to_value(GameStatus::Ended).unwrap(), json!("ENDED"));
    }

    #[test]
    fn maps_supported_lcu_phases_without_guessing_unknown_values() {
        assert_eq!(
            GameStatus::from_lcu_phase("ReadyCheck"),
            Some(GameStatus::InLobby)
        );
        assert_eq!(
            GameStatus::from_lcu_phase("ChampSelect"),
            Some(GameStatus::ChampSelect)
        );
        assert_eq!(
            GameStatus::from_lcu_phase("InProgress"),
            Some(GameStatus::InGame)
        );
        assert_eq!(
            GameStatus::from_lcu_phase("GameStart"),
            Some(GameStatus::InGame)
        );
        assert_eq!(
            GameStatus::from_lcu_phase("WaitingForStats"),
            Some(GameStatus::Ended)
        );
        assert_eq!(GameStatus::from_lcu_phase("None"), None);
        assert_eq!(GameStatus::from_lcu_phase("future-riot-phase"), None);
    }

    #[test]
    fn live_tick_serializes_the_complete_camel_case_contract() {
        let value = to_value(LiveGameTickPayload {
            game_time: 615.5,
            active_player: ActivePlayerPayload {
                summoner_name: "Aura Player".into(),
                champion_name: "Briar".into(),
                current_gold: 875.0,
                level: 11,
                creep_score: 132,
                creep_score_per_minute: 12.87,
                kill_participation_percent: 68.75,
                observable_held_value: 4_208.0,
                observable_value_per_minute: 410.31,
                earned_gold_per_minute: 0.0,
                xp_progress_percent: None,
                kda: Kda {
                    kills: 4,
                    deaths: 2,
                    assists: 7,
                },
                dpm: 0.0,
            },
            team_gold_delta: 0,
            objectives: ObjectivesPayload {
                dragon_type: Some("Infernal".into()),
                dragon_timer: 42.0,
                baron_timer: 0.0,
            },
            metric_availability: MetricAvailabilityPayload::default(),
            metric_sources: MetricSourcesPayload {
                observable_held_value: Some(
                    ObservableValueSource::CurrentGoldPlusCurrentInventoryListedValue,
                ),
                observable_value_per_minute: Some(
                    ObservableValueSource::CurrentGoldPlusCurrentInventoryListedValue,
                ),
            },
        })
        .unwrap();

        assert_eq!(value["gameTime"], json!(615.5));
        assert_eq!(value["activePlayer"]["summonerName"], json!("Aura Player"));
        assert_eq!(value["activePlayer"]["championName"], json!("Briar"));
        assert_eq!(value["activePlayer"]["currentGold"], json!(875.0));
        assert_eq!(value["activePlayer"]["level"], json!(11));
        assert_eq!(value["activePlayer"]["creepScore"], json!(132));
        assert_eq!(
            value["activePlayer"]["killParticipationPercent"],
            json!(68.75)
        );
        assert_eq!(value["activePlayer"]["xpProgressPercent"], json!(null));
        assert_eq!(
            value["activePlayer"]["kda"],
            json!({
                "kills": 4,
                "deaths": 2,
                "assists": 7
            })
        );
        assert_eq!(value["objectives"]["dragonType"], json!("Infernal"));
        assert_eq!(value["objectives"]["dragonTimer"], json!(42.0));
        assert_eq!(value["metricAvailability"]["dpm"], json!(false));
        assert_eq!(value["metricAvailability"]["currentGold"], json!(false));
        assert_eq!(value["metricAvailability"]["kda"], json!(false));
        assert_eq!(value["metricAvailability"]["teamGoldDelta"], json!(false));
        assert_eq!(
            value["metricSources"]["observableHeldValue"],
            json!("CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE")
        );
        assert_eq!(
            value["metricSources"]["observableValuePerMinute"],
            json!("CURRENT_GOLD_PLUS_CURRENT_INVENTORY_LISTED_VALUE")
        );
    }

    #[test]
    fn player_update_serializes_red_team_as_harmony() {
        let value = to_value(PlayerStatsPayload {
            summoner_name: "Red Player".into(),
            champion_name: "Ahri".into(),
            team: PlayerTeam::Harmony,
            level: 11,
            creep_score: 132,
            items: vec![6655, 3020],
        })
        .unwrap();

        assert_eq!(value["team"], json!("HARMONY"));
        assert_eq!(value["creepScore"], json!(132));
        assert_eq!(value["items"], json!([6655, 3020]));
    }
}
