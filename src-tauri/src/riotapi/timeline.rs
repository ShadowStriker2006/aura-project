use super::{get_json_as, log_ok, match_timeline_url, MatchDetail, RiotApiError, RiotApiState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const REPLAY_SCHEMA_VERSION: u8 = 2;
const MAX_CACHED_TIMELINES: usize = 3;
const MAX_REPLAY_DURATION_MS: u64 = 21_600_000;
const MAX_TIMELINE_FRAMES: usize = 512;
const MAX_EVENTS: usize = 4_096;
const MAX_PLAYERS_PER_FRAME: usize = 20;
const SAFE_RAW_MIN_COORDINATE: i32 = -50_000;
const SAFE_RAW_MAX_COORDINATE: i32 = 50_000;
const SUMMONERS_RIFT_MAP_ID: u32 = 11;
const HOWLING_ABYSS_MAP_ID: u32 = 12;
const SPAWN_FRAME_MAX_TIMESTAMP_MS: i64 = 30_000;
const MIN_TEAM_SPAWN_SAMPLES: usize = 3;

#[derive(Deserialize)]
struct TimelineResponse {
    metadata: RawTimelineMetadata,
    info: RawTimelineInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTimelineMetadata {
    match_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTimelineInfo {
    frame_interval: i64,
    frames: Vec<RawTimelineFrame>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTimelineFrame {
    #[serde(default)]
    timestamp: i64,
    #[serde(default)]
    participant_frames: HashMap<String, RawParticipantFrame>,
    #[serde(default)]
    events: Vec<RawTimelineEvent>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawParticipantFrame {
    #[serde(default)]
    participant_id: u8,
    #[serde(default)]
    position: Option<RawPosition>,
    #[serde(default)]
    total_gold: u32,
    #[serde(default)]
    level: u8,
    #[serde(default)]
    minions_killed: u32,
    #[serde(default)]
    jungle_minions_killed: u32,
}

#[derive(Clone, Copy, Default, Deserialize)]
struct RawPosition {
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
}

/// Riot's formal Timeline DTO documents only the common event fields. Parse
/// observed event-specific fields as optional so additions do not make the
/// entire replay fail to decode.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTimelineEvent {
    #[serde(rename = "type", default)]
    event_type: String,
    #[serde(default)]
    timestamp: i64,
    #[serde(default)]
    position: Option<RawPosition>,
    #[serde(default)]
    killer_id: Option<u8>,
    #[serde(default)]
    victim_id: Option<u8>,
    #[serde(default)]
    assisting_participant_ids: Vec<u8>,
    #[serde(default)]
    killer_team_id: Option<u16>,
    #[serde(default)]
    team_id: Option<u16>,
    #[serde(default)]
    building_type: Option<String>,
    #[serde(default)]
    tower_type: Option<String>,
    #[serde(default)]
    lane_type: Option<String>,
    #[serde(default)]
    monster_type: Option<String>,
    #[serde(default)]
    monster_sub_type: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ReplayCoordinates {
    // IDs and provenance describe Aura's projection, not a Riot-guaranteed
    // coordinate contract. Numeric bounds remain explicit for the renderer.
    pub model_id: String,
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
    pub invert_x_for_canvas: bool,
    pub invert_y_for_canvas: bool,
    pub swap_axes_for_canvas: bool,
    pub provenance: String,
}

#[derive(Clone, Serialize)]
pub struct ReplayPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Serialize)]
pub struct ReplayControlModel {
    pub id: String,
    pub topology: String,
    pub blue_base: ReplayPoint,
    pub red_base: ReplayPoint,
    pub anchor_provenance: String,
}

#[derive(Clone, Serialize)]
pub struct ReplayParticipant {
    pub participant_id: u8,
    pub team_id: u16,
    pub champion_id: u32,
    pub champion_name: String,
}

#[derive(Clone, Serialize)]
pub struct ReplayPlayerFrame {
    pub participant_id: u8,
    pub x: i32,
    pub y: i32,
    pub level: u8,
    pub total_gold: u32,
    pub cs: u32,
}

#[derive(Clone, Serialize)]
pub struct ReplayTeamFrame {
    pub team_id: u16,
    pub kills: u32,
    pub gold: u64,
    pub turrets: u32,
}

#[derive(Clone, Serialize)]
pub struct ReplayFrame {
    pub timestamp_ms: u64,
    pub players: Vec<ReplayPlayerFrame>,
    pub teams: Vec<ReplayTeamFrame>,
}

#[derive(Clone, Serialize)]
pub struct ReplayEvent {
    pub id: String,
    pub timestamp_ms: u64,
    pub kind: String,
    pub raw_type: String,
    pub team_id: Option<u16>,
    pub killer_participant_id: Option<u8>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub detail: String,
}

#[derive(Clone, Serialize)]
pub struct ReplayAvailability {
    // Schema v2 deliberately separates drawable movement from the optional
    // derived pressure model. A map can therefore replay positions, events,
    // and team stats even when no honest control estimate is available.
    pub positions: bool,
    pub positions_reason: Option<String>,
    pub control_estimate: bool,
    pub control_reason: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct MatchTimelineReplay {
    pub schema_version: u8,
    pub match_id: String,
    pub map_id: u32,
    pub game_version: String,
    pub frame_interval_ms: u64,
    pub duration_ms: u64,
    pub coordinates: ReplayCoordinates,
    pub control_model: Option<ReplayControlModel>,
    pub participants: Vec<ReplayParticipant>,
    pub frames: Vec<ReplayFrame>,
    pub events: Vec<ReplayEvent>,
    pub availability: ReplayAvailability,
}

#[derive(Clone)]
struct NormalizedEvent {
    timestamp_ms: u64,
    kind: String,
    raw_type: String,
    team_id: Option<u16>,
    killer_participant_id: Option<u8>,
    x: Option<i32>,
    y: Option<i32>,
    detail: String,
}

#[derive(Clone, Copy)]
struct CoordinateBounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl CoordinateBounds {
    fn contains(self, position: RawPosition) -> bool {
        (self.min_x..=self.max_x).contains(&position.x)
            && (self.min_y..=self.max_y).contains(&position.y)
    }

    fn clamp(self, position: RawPosition) -> RawPosition {
        RawPosition {
            x: position.x.clamp(self.min_x, self.max_x),
            y: position.y.clamp(self.min_y, self.max_y),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CalibratedMap {
    SummonersRift,
    HowlingAbyss,
}

struct CoordinateSelection {
    bounds: CoordinateBounds,
    accepted_bounds: CoordinateBounds,
    calibrated_map: Option<CalibratedMap>,
    replay: ReplayCoordinates,
}

/// Lazily fetches the Match-V5 timeline for a match already present in Aura's
/// current profile cache. This avoids doubling the request count when the
/// Profile page initially loads 10-20 match summaries.
#[tauri::command]
pub async fn get_match_timeline(
    state: tauri::State<'_, RiotApiState>,
    match_id: String,
) -> Result<MatchTimelineReplay, String> {
    let match_id = match_id.trim();
    if match_id.is_empty() || match_id.len() > 96 {
        return Err("invalid match id".into());
    }

    let detail = state
        .match_details
        .lock()
        .await
        .get(match_id)
        .cloned()
        .ok_or_else(|| {
            "match detail is not in the current RAM cache; refresh Profile".to_string()
        })?;
    if let Some(replay) = state.match_timelines.lock().await.get(match_id) {
        log_ok("served match timeline from volatile RAM cache");
        return Ok(replay);
    }

    // One timeline request at a time is sufficient for a user-driven replay
    // panel and prevents duplicate clicks from consuming Riot rate-limit
    // budget. Recheck the cache after acquiring the single-flight guard.
    let _fetch_guard = state.match_timeline_fetch.lock().await;
    if !state.match_details.lock().await.contains_key(match_id) {
        return Err(
            "Riot profile changed before the timeline request started; reopen the match".into(),
        );
    }
    if let Some(replay) = state.match_timelines.lock().await.get(match_id) {
        return Ok(replay);
    }
    let identity = state
        .identity
        .lock()
        .await
        .clone()
        .ok_or(RiotApiError::NotIdentified)
        .map_err(|error| error.to_string())?;
    let url =
        match_timeline_url(identity.region_group, match_id).map_err(|error| error.to_string())?;
    let timeline: TimelineResponse = get_json_as(state.inner(), "match-timeline lookup", &url)
        .await
        .map_err(|error| error.to_string())?;
    if timeline.metadata.match_id != match_id {
        return Err(RiotApiError::DecodeFailed(
            "timeline match id did not match the request".into(),
        )
        .to_string());
    }
    if state.identity.lock().await.as_ref() != Some(&identity)
        || !state.match_details.lock().await.contains_key(match_id)
    {
        return Err("Riot profile changed while the timeline was loading; reopen the match".into());
    }
    let replay = build_timeline_replay(match_id.to_string(), &detail, timeline.info);

    state.match_timelines.lock().await.insert(
        match_id.to_string(),
        replay.clone(),
        MAX_CACHED_TIMELINES,
    );
    log_ok("fetched and normalized a match timeline");
    Ok(replay)
}

fn build_timeline_replay(
    match_id: String,
    detail: &MatchDetail,
    mut timeline: RawTimelineInfo,
) -> MatchTimelineReplay {
    timeline.frames.sort_by_key(|frame| frame.timestamp);
    timeline.frames.truncate(MAX_TIMELINE_FRAMES);
    let match_duration_ms = (detail.game_duration_secs.max(0) as u64)
        .saturating_mul(1_000)
        .min(MAX_REPLAY_DURATION_MS);
    let timestamp_cap = if match_duration_ms > 0 {
        match_duration_ms
    } else {
        MAX_REPLAY_DURATION_MS
    };
    let participant_teams: HashMap<u8, u16> = detail
        .participants
        .iter()
        .map(|participant| (participant.participant_id, participant.team_id))
        .collect();
    let coordinate_selection = select_coordinates(detail.map_id, &timeline);
    let aram_spawn_anchors = (coordinate_selection.calibrated_map
        == Some(CalibratedMap::HowlingAbyss))
    .then(|| {
        infer_howling_abyss_spawn_anchors(
            &timeline,
            &participant_teams,
            coordinate_selection.bounds,
        )
    })
    .flatten();
    let participants = detail
        .participants
        .iter()
        .take(MAX_PLAYERS_PER_FRAME)
        .map(|participant| ReplayParticipant {
            participant_id: participant.participant_id,
            team_id: participant.team_id,
            champion_id: participant.champion_id,
            champion_name: bounded_text(&participant.champion_name, 64),
        })
        .collect::<Vec<_>>();

    let mut normalized_events = timeline
        .frames
        .iter()
        .flat_map(|frame| frame.events.iter())
        .filter_map(|event| {
            normalize_event(
                event,
                &participant_teams,
                timestamp_cap,
                coordinate_selection.bounds,
            )
        })
        .take(MAX_EVENTS)
        .collect::<Vec<_>>();
    normalized_events.sort_by_key(|event| event.timestamp_ms);

    let mut frames = Vec::with_capacity(timeline.frames.len());
    for raw_frame in timeline.frames {
        let timestamp_ms = bounded_millis(raw_frame.timestamp, timestamp_cap);
        let mut raw_players = raw_frame
            .participant_frames
            .into_iter()
            .filter_map(|(key, mut player)| {
                if player.participant_id == 0 {
                    player.participant_id = key.parse::<u8>().unwrap_or_default();
                }
                participant_teams
                    .contains_key(&player.participant_id)
                    .then_some(player)
            })
            .collect::<Vec<_>>();
        raw_players.sort_by_key(|player| player.participant_id);
        raw_players.dedup_by_key(|player| player.participant_id);
        raw_players.truncate(MAX_PLAYERS_PER_FRAME);
        let mut blue_gold = 0_u64;
        let mut red_gold = 0_u64;
        for player in &raw_players {
            match participant_teams.get(&player.participant_id).copied() {
                Some(100) => blue_gold = blue_gold.saturating_add(player.total_gold as u64),
                Some(200) => red_gold = red_gold.saturating_add(player.total_gold as u64),
                _ => {}
            }
        }
        let mut players = raw_players
            .into_iter()
            .filter_map(|player| normalize_player_frame(player, coordinate_selection.bounds))
            .collect::<Vec<_>>();
        players.sort_by_key(|player| player.participant_id);
        let (blue_kills, blue_turrets, red_kills, red_turrets) =
            cumulative_team_events(&normalized_events, timestamp_ms);
        frames.push(ReplayFrame {
            timestamp_ms,
            players,
            teams: vec![
                ReplayTeamFrame {
                    team_id: 100,
                    kills: blue_kills,
                    gold: blue_gold,
                    turrets: blue_turrets,
                },
                ReplayTeamFrame {
                    team_id: 200,
                    kills: red_kills,
                    gold: red_gold,
                    turrets: red_turrets,
                },
            ],
        });
    }

    frames.sort_by_key(|frame| frame.timestamp_ms);
    frames.dedup_by_key(|frame| frame.timestamp_ms);
    let events = normalized_events
        .iter()
        .enumerate()
        .map(|(index, event)| ReplayEvent {
            id: format!("e{index}"),
            timestamp_ms: event.timestamp_ms,
            kind: event.kind.clone(),
            raw_type: event.raw_type.clone(),
            team_id: event.team_id,
            killer_participant_id: event.killer_participant_id,
            x: event.x,
            y: event.y,
            detail: event.detail.clone(),
        })
        .collect::<Vec<_>>();

    let duration_ms = if match_duration_ms > 0 {
        match_duration_ms
    } else {
        frames
            .last()
            .map(|frame| frame.timestamp_ms)
            .into_iter()
            .chain(events.last().map(|event| event.timestamp_ms))
            .max()
            .unwrap_or_default()
            .min(MAX_REPLAY_DURATION_MS)
    };
    let positions = frames.iter().any(|frame| !frame.players.is_empty());
    let both_teams_observed = frames.iter().any(|frame| {
        let mut blue = false;
        let mut red = false;
        for player in &frame.players {
            match participant_teams.get(&player.participant_id) {
                Some(100) => blue = true,
                Some(200) => red = true,
                _ => {}
            }
        }
        blue && red
    });
    let positions_reason = (!positions)
        .then(|| "Riot returned no usable participant positions for this match.".to_string());
    let (control_model, control_reason) = if !positions {
        (None, None)
    } else if !both_teams_observed {
        (
            None,
            Some("Riot did not return same-frame positions for both teams.".to_string()),
        )
    } else {
        match coordinate_selection.calibrated_map {
            Some(CalibratedMap::SummonersRift) => (
                Some(ReplayControlModel {
                    id: "summoners_rift_radial_v1".to_string(),
                    topology: "two_dimensional".to_string(),
                    blue_base: ReplayPoint { x: 900, y: 900 },
                    red_base: ReplayPoint {
                        x: 14_100,
                        y: 14_100,
                    },
                    anchor_provenance: "aura_static_calibration".to_string(),
                }),
                None,
            ),
            Some(CalibratedMap::HowlingAbyss) => match aram_spawn_anchors {
                Some((blue_base, red_base)) => (
                    Some(ReplayControlModel {
                        id: "howling_abyss_linear_v1".to_string(),
                        topology: "linear_lane".to_string(),
                        blue_base,
                        red_base,
                        anchor_provenance: "timeline_spawn_clusters".to_string(),
                    }),
                    None,
                ),
                None => (
                    None,
                    Some(
                        "Reliable Howling Abyss spawn anchors could not be established for this match."
                            .to_string(),
                    ),
                ),
            },
            None => (
                None,
                Some(format!(
                    "Dynamic map control has no calibrated model for map {}.",
                    detail.map_id
                )),
            ),
        }
    };
    let control_estimate = control_model.is_some();
    let frame_interval_ms = normalized_frame_interval(timeline.frame_interval, &frames);

    MatchTimelineReplay {
        schema_version: REPLAY_SCHEMA_VERSION,
        match_id,
        map_id: detail.map_id,
        game_version: bounded_text(&detail.game_version, 64),
        frame_interval_ms,
        duration_ms,
        coordinates: coordinate_selection.replay,
        control_model,
        participants,
        frames,
        events,
        availability: ReplayAvailability {
            positions,
            positions_reason,
            control_estimate,
            control_reason,
        },
    }
}

fn normalize_player_frame(
    frame: RawParticipantFrame,
    bounds: CoordinateBounds,
) -> Option<ReplayPlayerFrame> {
    if frame.participant_id == 0 {
        return None;
    }
    let position = frame.position?;
    if !safe_raw_position(position) {
        return None;
    }
    let position = bounds.clamp(position);
    Some(ReplayPlayerFrame {
        participant_id: frame.participant_id,
        x: position.x,
        y: position.y,
        level: frame.level,
        total_gold: frame.total_gold,
        cs: frame
            .minions_killed
            .saturating_add(frame.jungle_minions_killed),
    })
}

fn normalize_event(
    event: &RawTimelineEvent,
    participant_teams: &HashMap<u8, u16>,
    timestamp_cap: u64,
    bounds: CoordinateBounds,
) -> Option<NormalizedEvent> {
    let raw_type = bounded_uppercase(Some(&event.event_type), 64);
    let building_type = bounded_uppercase(event.building_type.as_deref(), 64);
    let monster_type = bounded_uppercase(event.monster_type.as_deref(), 64);
    let kind = match raw_type.as_str() {
        "CHAMPION_KILL" => "champion_kill",
        "ELITE_MONSTER_KILL" => match monster_type.as_str() {
            "DRAGON" => "dragon",
            "BARON_NASHOR" => "baron",
            "RIFTHERALD" => "rift_herald",
            "HORDE" => "void_grubs",
            "ATAKHAN" => "atakhan",
            _ => "objective",
        },
        "BUILDING_KILL" if building_type.contains("TOWER") => "turret",
        "BUILDING_KILL" if building_type.contains("INHIBITOR") => "inhibitor",
        "BUILDING_KILL" => "building",
        _ => return None,
    }
    .to_string();
    let killer_participant_id = event.killer_id.filter(|id| *id > 0);
    let participant_team = killer_participant_id
        .and_then(|participant_id| participant_teams.get(&participant_id).copied());
    let assisting_team = event
        .assisting_participant_ids
        .iter()
        .take(MAX_PLAYERS_PER_FRAME)
        .find_map(|participant_id| participant_teams.get(participant_id).copied());
    let victim_opponent = event
        .victim_id
        .and_then(|participant_id| participant_teams.get(&participant_id).copied())
        .map(opposing_team);
    let team_id = valid_team(event.killer_team_id)
        .or(participant_team)
        .or(assisting_team)
        .or_else(|| {
            if raw_type == "CHAMPION_KILL" {
                victim_opponent
            } else {
                None
            }
        })
        .or_else(|| {
            let event_team = valid_team(event.team_id)?;
            if raw_type == "BUILDING_KILL" {
                Some(opposing_team(event_team))
            } else {
                Some(event_team)
            }
        });
    let position = event
        .position
        .filter(|position| safe_raw_position(*position))
        .map(|position| bounds.clamp(position));
    let detail = match raw_type.as_str() {
        "ELITE_MONSTER_KILL" => bounded_text(
            event
                .monster_sub_type
                .as_deref()
                .filter(|value| !value.is_empty())
                .or(event.monster_type.as_deref())
                .unwrap_or("Objective"),
            80,
        ),
        "BUILDING_KILL" => {
            let joined = [event.lane_type.as_deref(), event.tower_type.as_deref()]
                .into_iter()
                .flatten()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            bounded_text(&joined, 80)
        }
        _ => String::new(),
    };
    Some(NormalizedEvent {
        timestamp_ms: bounded_millis(event.timestamp, timestamp_cap),
        kind,
        raw_type,
        team_id,
        killer_participant_id,
        x: position.map(|value| value.x),
        y: position.map(|value| value.y),
        detail,
    })
}

fn cumulative_team_events(events: &[NormalizedEvent], timestamp_ms: u64) -> (u32, u32, u32, u32) {
    let mut blue_kills = 0_u32;
    let mut blue_turrets = 0_u32;
    let mut red_kills = 0_u32;
    let mut red_turrets = 0_u32;
    for event in events
        .iter()
        .take_while(|event| event.timestamp_ms <= timestamp_ms)
    {
        match (event.team_id, event.kind.as_str()) {
            (Some(100), "champion_kill") => blue_kills = blue_kills.saturating_add(1),
            (Some(200), "champion_kill") => red_kills = red_kills.saturating_add(1),
            (Some(100), "turret") => blue_turrets = blue_turrets.saturating_add(1),
            (Some(200), "turret") => red_turrets = red_turrets.saturating_add(1),
            _ => {}
        }
    }
    (blue_kills, blue_turrets, red_kills, red_turrets)
}

fn normalized_frame_interval(raw: i64, frames: &[ReplayFrame]) -> u64 {
    if raw > 0 {
        return (raw as u64).clamp(1_000, 300_000);
    }
    let mut intervals = frames
        .windows(2)
        .filter_map(|pair| pair[1].timestamp_ms.checked_sub(pair[0].timestamp_ms))
        .filter(|interval| *interval > 0)
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    intervals
        .get(intervals.len() / 2)
        .copied()
        .unwrap_or(60_000)
        .clamp(1_000, 300_000)
}

fn bounded_millis(value: i64, maximum: u64) -> u64 {
    (value.max(0) as u64).min(maximum)
}

fn bounded_uppercase(value: Option<&str>, maximum_chars: usize) -> String {
    bounded_text(value.unwrap_or_default().trim(), maximum_chars).to_ascii_uppercase()
}

fn bounded_text(value: &str, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}

fn select_coordinates(map_id: u32, timeline: &RawTimelineInfo) -> CoordinateSelection {
    let calibrated = calibrated_coordinates(map_id);
    if let Some(selection) = calibrated {
        let observed = observed_player_positions(timeline);
        let outside_calibration = observed
            .iter()
            .filter(|position| !selection.accepted_bounds.contains(**position))
            .count();
        // A small number of edge samples are safely clamped to the calibrated
        // projection. If a substantial share no longer fits, retain movement
        // with an observed extent but fail map-control calibration closed.
        if observed.len() >= 5 && outside_calibration.saturating_mul(5) > observed.len() {
            return observed_coordinate_selection(timeline);
        }
        return selection;
    }
    observed_coordinate_selection(timeline)
}

fn calibrated_coordinates(map_id: u32) -> Option<CoordinateSelection> {
    let (bounds, accepted_bounds, calibrated_map, model_id, provenance) = match map_id {
        SUMMONERS_RIFT_MAP_ID => (
            CoordinateBounds {
                min_x: 0,
                max_x: 15_000,
                min_y: 0,
                max_y: 15_000,
            },
            CoordinateBounds {
                min_x: -512,
                max_x: 15_512,
                min_y: -512,
                max_y: 15_512,
            },
            CalibratedMap::SummonersRift,
            "summoners_rift_rect_v1",
            "aura_static_calibration",
        ),
        HOWLING_ABYSS_MAP_ID => (
            CoordinateBounds {
                min_x: 0,
                max_x: 12_800,
                min_y: 0,
                max_y: 12_800,
            },
            CoordinateBounds {
                min_x: -512,
                max_x: 13_312,
                min_y: -512,
                max_y: 13_312,
            },
            CalibratedMap::HowlingAbyss,
            "howling_abyss_rect_v1",
            "aura_match_v5_calibration_16_15_v1",
        ),
        _ => return None,
    };
    Some(CoordinateSelection {
        bounds,
        accepted_bounds,
        calibrated_map: Some(calibrated_map),
        replay: ReplayCoordinates {
            model_id: model_id.to_string(),
            min_x: bounds.min_x,
            max_x: bounds.max_x,
            min_y: bounds.min_y,
            max_y: bounds.max_y,
            invert_x_for_canvas: false,
            invert_y_for_canvas: true,
            swap_axes_for_canvas: false,
            provenance: provenance.to_string(),
        },
    })
}

fn observed_player_positions(timeline: &RawTimelineInfo) -> Vec<RawPosition> {
    timeline
        .frames
        .iter()
        .flat_map(|frame| frame.participant_frames.values())
        .filter_map(|frame| frame.position)
        .filter(|position| safe_raw_position(*position))
        .collect()
}

fn observed_coordinate_selection(timeline: &RawTimelineInfo) -> CoordinateSelection {
    let mut observed = observed_player_positions(timeline);
    if observed.is_empty() {
        observed.extend(
            timeline
                .frames
                .iter()
                .flat_map(|frame| frame.events.iter())
                .filter_map(|event| event.position)
                .filter(|position| safe_raw_position(*position)),
        );
    }
    let bounds = observed_bounds(&observed).unwrap_or(CoordinateBounds {
        min_x: 0,
        max_x: 1,
        min_y: 0,
        max_y: 1,
    });
    CoordinateSelection {
        bounds,
        accepted_bounds: CoordinateBounds {
            min_x: SAFE_RAW_MIN_COORDINATE,
            max_x: SAFE_RAW_MAX_COORDINATE,
            min_y: SAFE_RAW_MIN_COORDINATE,
            max_y: SAFE_RAW_MAX_COORDINATE,
        },
        calibrated_map: None,
        replay: ReplayCoordinates {
            model_id: "observed_extent_v1".to_string(),
            min_x: bounds.min_x,
            max_x: bounds.max_x,
            min_y: bounds.min_y,
            max_y: bounds.max_y,
            invert_x_for_canvas: false,
            invert_y_for_canvas: true,
            swap_axes_for_canvas: false,
            provenance: "timeline_observed_extent".to_string(),
        },
    }
}

fn observed_bounds(positions: &[RawPosition]) -> Option<CoordinateBounds> {
    let min_x = positions.iter().map(|position| position.x).min()?;
    let max_x = positions.iter().map(|position| position.x).max()?;
    let min_y = positions.iter().map(|position| position.y).min()?;
    let max_y = positions.iter().map(|position| position.y).max()?;
    let (min_x, max_x) = padded_extent(min_x, max_x);
    let (min_y, max_y) = padded_extent(min_y, max_y);
    Some(CoordinateBounds {
        min_x,
        max_x,
        min_y,
        max_y,
    })
}

fn padded_extent(minimum: i32, maximum: i32) -> (i32, i32) {
    let span = maximum.saturating_sub(minimum).max(1_000);
    let padding = (span / 20).clamp(100, 2_000);
    let mut lower = minimum.saturating_sub(padding).max(SAFE_RAW_MIN_COORDINATE);
    let mut upper = maximum.saturating_add(padding).min(SAFE_RAW_MAX_COORDINATE);
    if upper <= lower {
        lower = minimum.saturating_sub(500).max(SAFE_RAW_MIN_COORDINATE);
        upper = maximum.saturating_add(500).min(SAFE_RAW_MAX_COORDINATE);
    }
    (lower, upper.max(lower.saturating_add(1)))
}

fn infer_howling_abyss_spawn_anchors(
    timeline: &RawTimelineInfo,
    participant_teams: &HashMap<u8, u16>,
    bounds: CoordinateBounds,
) -> Option<(ReplayPoint, ReplayPoint)> {
    for frame in timeline
        .frames
        .iter()
        .filter(|frame| (0..=SPAWN_FRAME_MAX_TIMESTAMP_MS).contains(&frame.timestamp))
    {
        let mut blue = Vec::new();
        let mut red = Vec::new();
        for (key, participant) in &frame.participant_frames {
            let participant_id = if participant.participant_id == 0 {
                key.parse::<u8>().unwrap_or_default()
            } else {
                participant.participant_id
            };
            let Some(position) = participant
                .position
                .filter(|value| safe_raw_position(*value))
            else {
                continue;
            };
            let position = bounds.clamp(position);
            match participant_teams.get(&participant_id) {
                Some(100) => blue.push(position),
                Some(200) => red.push(position),
                _ => {}
            }
        }
        if blue.len() < MIN_TEAM_SPAWN_SAMPLES || red.len() < MIN_TEAM_SPAWN_SAMPLES {
            continue;
        }
        let blue_base = median_point(&blue);
        let red_base = median_point(&red);
        if reliable_howling_abyss_anchors(&blue, &red, blue_base, red_base, bounds) {
            return Some((
                ReplayPoint {
                    x: blue_base.x,
                    y: blue_base.y,
                },
                ReplayPoint {
                    x: red_base.x,
                    y: red_base.y,
                },
            ));
        }
    }
    None
}

fn median_point(positions: &[RawPosition]) -> RawPosition {
    let mut xs = positions
        .iter()
        .map(|position| position.x)
        .collect::<Vec<_>>();
    let mut ys = positions
        .iter()
        .map(|position| position.y)
        .collect::<Vec<_>>();
    xs.sort_unstable();
    ys.sort_unstable();
    RawPosition {
        x: xs[xs.len() / 2],
        y: ys[ys.len() / 2],
    }
}

fn reliable_howling_abyss_anchors(
    blue: &[RawPosition],
    red: &[RawPosition],
    blue_base: RawPosition,
    red_base: RawPosition,
    bounds: CoordinateBounds,
) -> bool {
    let blue_normalized = normalized_position(blue_base, bounds);
    let red_normalized = normalized_position(red_base, bounds);
    let separated = normalized_distance(blue_normalized, red_normalized) >= 0.85;
    let expected_base_zones = blue_normalized.0 <= 0.18
        && blue_normalized.1 <= 0.18
        && red_normalized.0 >= 0.82
        && red_normalized.1 >= 0.82;
    let blue_compact = blue.iter().all(|position| {
        normalized_distance(normalized_position(*position, bounds), blue_normalized) <= 0.18
    });
    let red_compact = red.iter().all(|position| {
        normalized_distance(normalized_position(*position, bounds), red_normalized) <= 0.18
    });
    separated && expected_base_zones && blue_compact && red_compact
}

fn normalized_position(position: RawPosition, bounds: CoordinateBounds) -> (f64, f64) {
    let width = (bounds.max_x - bounds.min_x).max(1) as f64;
    let height = (bounds.max_y - bounds.min_y).max(1) as f64;
    (
        (position.x - bounds.min_x) as f64 / width,
        (position.y - bounds.min_y) as f64 / height,
    )
}

fn normalized_distance(left: (f64, f64), right: (f64, f64)) -> f64 {
    ((left.0 - right.0).powi(2) + (left.1 - right.1).powi(2)).sqrt()
}

fn valid_team(team_id: Option<u16>) -> Option<u16> {
    team_id.filter(|team_id| matches!(team_id, 100 | 200))
}

fn safe_raw_position(position: RawPosition) -> bool {
    (SAFE_RAW_MIN_COORDINATE..=SAFE_RAW_MAX_COORDINATE).contains(&position.x)
        && (SAFE_RAW_MIN_COORDINATE..=SAFE_RAW_MAX_COORDINATE).contains(&position.y)
}

fn opposing_team(team_id: u16) -> u16 {
    if team_id == 100 {
        200
    } else {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riotapi::{build_match_payload, MatchInfo, RegionGroup};

    fn representative_detail() -> MatchDetail {
        let info: MatchInfo = serde_json::from_value(serde_json::json!({
            "gameDuration": 120,
            "gameVersion": "16.15.1.1234",
            "mapId": 11,
            "participants": [
                {"participantId": 1, "teamId": 100, "puuid": "player-puuid", "championId": 103, "championName": "Ahri"},
                {"participantId": 6, "teamId": 200, "puuid": "enemy-puuid", "championId": 238, "championName": "Zed"}
            ]
        }))
        .expect("minimal Match-V5 match should decode");
        build_match_payload("EUN1_123".into(), info, "player-puuid")
            .expect("identified participant should produce detail")
            .1
    }

    fn representative_timeline() -> RawTimelineInfo {
        serde_json::from_value(serde_json::json!({
            "frameInterval": 60000,
            "frames": [
                {
                    "timestamp": 0,
                    "participantFrames": {
                        "1": {"participantId": 1, "position": {"x": 1000, "y": 1000}, "level": 1, "totalGold": 500, "minionsKilled": 0},
                        "6": {"participantId": 6, "position": {"x": 14000, "y": 14000}, "level": 1, "totalGold": 500}
                    },
                    "events": []
                },
                {
                    "timestamp": 60000,
                    "participantFrames": {
                        "1": {"participantId": 1, "position": {"x": 6500, "y": 6500}, "level": 5, "totalGold": 3000, "minionsKilled": 40, "jungleMinionsKilled": 2},
                        "6": {"participantId": 6, "position": {"x": 9000, "y": 9000}, "level": 5, "totalGold": 2700}
                    },
                    "events": [
                        {"type": "CHAMPION_KILL", "timestamp": 55000, "killerId": 1},
                        {"type": "ELITE_MONSTER_KILL", "timestamp": 58000, "killerTeamId": 100, "monsterType": "DRAGON", "monsterSubType": "AIR_DRAGON", "position": {"x": 9850, "y": 4400}}
                    ]
                },
                {
                    "timestamp": 120000,
                    "participantFrames": {
                        "1": {"participantId": 1, "position": {"x": 9000, "y": 9000}, "level": 8, "totalGold": 5500},
                        "6": {"participantId": 6, "position": {"x": 11000, "y": 11000}, "level": 7, "totalGold": 4800}
                    },
                    "events": [
                        {"type": "BUILDING_KILL", "timestamp": 110000, "killerId": 1, "teamId": 200, "buildingType": "TOWER_BUILDING", "towerType": "OUTER_TURRET", "laneType": "MID_LANE"},
                        {"type": "ELITE_MONSTER_KILL", "timestamp": 115000, "killerTeamId": 200, "monsterType": "HORDE"}
                    ]
                }
            ]
        }))
        .expect("representative Timeline-V5 response should decode")
    }

    fn sanitized_aram_detail() -> MatchDetail {
        let participants = (1..=10)
            .map(|participant_id| {
                serde_json::json!({
                    "participantId": participant_id,
                    "teamId": if participant_id <= 5 { 100 } else { 200 },
                    "puuid": if participant_id == 1 {
                        "player-puuid".to_string()
                    } else {
                        format!("fixture-{participant_id}")
                    },
                    "championId": 100 + participant_id,
                    "championName": format!("Champion{participant_id}")
                })
            })
            .collect::<Vec<_>>();
        let info: MatchInfo = serde_json::from_value(serde_json::json!({
            "gameDuration": 180,
            "gameMode": "ARAM",
            "gameVersion": "16.15.1.4321",
            "mapId": 12,
            "queueId": 450,
            "participants": participants
        }))
        .expect("sanitized ten-player ARAM match should decode");
        build_match_payload("EUN1_ARAM".into(), info, "player-puuid")
            .expect("identified ARAM participant should produce detail")
            .1
    }

    fn sanitized_aram_timeline() -> RawTimelineInfo {
        serde_json::from_value(serde_json::json!({
            "frameInterval": 60000,
            "frames": [
                {
                    "timestamp": 0,
                    "participantFrames": {
                        "1": {"participantId": 1, "position": {"x": 846, "y": 860}, "totalGold": 500},
                        "2": {"participantId": 2, "position": {"x": 980, "y": 950}, "totalGold": 500},
                        "3": {"participantId": 3, "position": {"x": 1041, "y": 985}, "totalGold": 500},
                        "4": {"participantId": 4, "position": {"x": 1090, "y": 1020}, "totalGold": 500},
                        "5": {"participantId": 5, "position": {"x": 1150, "y": 1070}, "totalGold": 500},
                        "6": {"participantId": 6, "position": {"x": 11800, "y": 11250}, "totalGold": 500},
                        "7": {"participantId": 7, "position": {"x": 11920, "y": 11480}, "totalGold": 500},
                        "8": {"participantId": 8, "position": {"x": 11965, "y": 11521}, "totalGold": 500},
                        "9": {"participantId": 9, "position": {"x": 12050, "y": 11610}, "totalGold": 500},
                        "10": {"participantId": 10, "position": {"x": 12321, "y": 11716}, "totalGold": 500}
                    }
                },
                {
                    "timestamp": 60000,
                    "participantFrames": {
                        "1": {"participantId": 1, "position": {"x": 3300, "y": 3100}, "totalGold": 1800},
                        "2": {"participantId": 2, "position": {"x": 3500, "y": 3350}, "totalGold": 1750},
                        "3": {"participantId": 3, "position": {"x": 3700, "y": 3500}, "totalGold": 1700},
                        "4": {"participantId": 4, "position": {"x": 3900, "y": 3650}, "totalGold": 1650},
                        "5": {"participantId": 5, "position": {"x": 4100, "y": 3800}, "totalGold": 1600},
                        "6": {"participantId": 6, "position": {"x": 9000, "y": 8700}, "totalGold": 1700},
                        "7": {"participantId": 7, "position": {"x": 9200, "y": 8900}, "totalGold": 1750},
                        "8": {"participantId": 8, "position": {"x": 9400, "y": 9100}, "totalGold": 1800},
                        "9": {"participantId": 9, "position": {"x": 9600, "y": 9300}, "totalGold": 1650},
                        "10": {"participantId": 10, "position": {"x": 9800, "y": 9500}, "totalGold": 1600}
                    },
                    "events": [{
                        "type": "BUILDING_KILL",
                        "timestamp": 55000,
                        "killerId": 1,
                        "teamId": 200,
                        "buildingType": "TOWER_BUILDING",
                        "position": {"x": 10000, "y": 9700}
                    }]
                }
            ]
        }))
        .expect("sanitized map-12 Timeline-V5 fixture should decode")
    }

    #[test]
    fn timeline_url_uses_regional_match_route() {
        let url = match_timeline_url(RegionGroup::Europe, "EUN1_123").expect("URL should build");
        assert_eq!(
            url,
            "https://europe.api.riotgames.com/lol/match/v5/matches/EUN1_123/timeline"
        );
    }

    #[test]
    fn timeline_is_joined_and_compacted_for_canvas_replay() {
        let detail = representative_detail();
        let replay = build_timeline_replay("EUN1_123".into(), &detail, representative_timeline());

        assert_eq!(replay.schema_version, 2);
        assert_eq!(replay.map_id, 11);
        assert_eq!(replay.game_version, "16.15.1.1234");
        assert_eq!(replay.frame_interval_ms, 60_000);
        assert_eq!(replay.duration_ms, 120_000);
        assert_eq!(replay.participants.len(), 2);
        assert_eq!(replay.participants[0].champion_name, "Ahri");
        assert_eq!(replay.frames[1].players[0].cs, 42);
        assert_eq!(replay.frames[1].teams[0].kills, 1);
        assert_eq!(replay.frames[2].teams[0].turrets, 1);
        assert_eq!(replay.frames[2].teams[0].gold, 5_500);
        assert_eq!(replay.events[1].kind, "dragon");
        assert_eq!(replay.events[2].kind, "turret");
        assert_eq!(replay.events[3].kind, "void_grubs");
        assert!(replay.availability.control_estimate);
        assert_eq!(
            replay.control_model.as_ref().map(|model| model.id.as_str()),
            Some("summoners_rift_radial_v1")
        );
    }

    #[test]
    fn unknown_objectives_and_missing_positions_do_not_break_the_replay() {
        let detail = representative_detail();
        let timeline: RawTimelineInfo = serde_json::from_value(serde_json::json!({
            "frameInterval": 60000,
            "frames": [{
                "timestamp": -1,
                "participantFrames": {"1": {"participantId": 1}},
                "events": [{"type": "ELITE_MONSTER_KILL", "timestamp": 10, "monsterType": "FUTURE_MONSTER"}]
            }]
        }))
        .expect("open event object should decode");
        let replay = build_timeline_replay("EUN1_future".into(), &detail, timeline);

        assert_eq!(replay.events[0].kind, "objective");
        assert!(!replay.availability.positions);
        assert!(!replay.availability.control_estimate);
        assert!(replay.availability.positions_reason.is_some());
        assert!(replay.availability.control_reason.is_none());
    }

    #[test]
    fn malformed_timeline_success_payload_is_rejected() {
        assert!(serde_json::from_value::<TimelineResponse>(serde_json::json!({})).is_err());
        assert!(
            serde_json::from_value::<TimelineResponse>(serde_json::json!({
                "metadata": {"matchId": "EUN1_123"},
                "info": {"frameInterval": 60000}
            }))
            .is_err()
        );
    }

    #[test]
    fn bounds_outliers_and_keeps_gold_without_a_drawable_position() {
        let detail = representative_detail();
        let timeline: RawTimelineInfo = serde_json::from_value(serde_json::json!({
            "frameInterval": 99999999,
            "frames": [{
                "timestamp": i64::MAX,
                "participantFrames": {
                    "1": {"participantId": 1, "totalGold": 1234},
                    "6": {"participantId": 6, "position": {"x": 14000, "y": 14000}, "totalGold": 987}
                },
                "events": [{"type": "CHAMPION_KILL", "timestamp": i64::MAX, "victimId": 6}]
            }]
        }))
        .expect("outlier timeline should still decode defensively");
        let replay = build_timeline_replay("EUN1_bounds".into(), &detail, timeline);

        assert_eq!(replay.frame_interval_ms, 300_000);
        assert_eq!(replay.duration_ms, 120_000);
        assert_eq!(replay.frames[0].teams[0].gold, 1_234);
        assert_eq!(replay.frames[0].teams[1].gold, 987);
        assert_eq!(replay.events[0].timestamp_ms, 120_000);
        assert_eq!(replay.events[0].team_id, Some(100));
    }

    #[test]
    fn map_12_uses_verified_projection_and_linear_control_model() {
        let detail = sanitized_aram_detail();
        let replay = build_timeline_replay("EUN1_ARAM".into(), &detail, sanitized_aram_timeline());

        assert_eq!(replay.schema_version, 2);
        assert_eq!(replay.map_id, 12);
        assert_eq!(replay.game_version, "16.15.1.4321");
        assert_eq!(replay.coordinates.model_id, "howling_abyss_rect_v1");
        assert_eq!(replay.coordinates.min_x, 0);
        assert_eq!(replay.coordinates.max_x, 12_800);
        assert_eq!(
            replay.coordinates.provenance,
            "aura_match_v5_calibration_16_15_v1"
        );
        assert!(replay.availability.positions);
        assert!(replay.availability.positions_reason.is_none());
        assert!(replay.availability.control_estimate);
        assert!(replay.availability.control_reason.is_none());
        let model = replay
            .control_model
            .expect("reliable map-12 spawn clusters should enable a linear model");
        assert_eq!(model.id, "howling_abyss_linear_v1");
        assert_eq!(model.topology, "linear_lane");
        assert_eq!(model.anchor_provenance, "timeline_spawn_clusters");
        assert_eq!((model.blue_base.x, model.blue_base.y), (1041, 985));
        assert_eq!((model.red_base.x, model.red_base.y), (11965, 11521));
    }

    #[test]
    fn map_12_clamps_safe_edge_samples_instead_of_dropping_them() {
        let detail = sanitized_aram_detail();
        let timeline: RawTimelineInfo = serde_json::from_value(serde_json::json!({
            "frameInterval": 60000,
            "frames": [{
                "timestamp": 60000,
                "participantFrames": {
                    "1": {"participantId": 1, "position": {"x": -28, "y": -19}},
                    "6": {"participantId": 6, "position": {"x": 12849, "y": 12858}}
                },
                "events": [{
                    "type": "BUILDING_KILL",
                    "timestamp": 50000,
                    "killerId": 1,
                    "buildingType": "TOWER_BUILDING",
                    "position": {"x": 12900, "y": -100}
                }]
            }]
        }))
        .expect("safe edge-coordinate fixture should decode");
        let replay = build_timeline_replay("EUN1_ARAM_EDGE".into(), &detail, timeline);

        assert_eq!(replay.coordinates.model_id, "howling_abyss_rect_v1");
        assert_eq!(
            (replay.frames[0].players[0].x, replay.frames[0].players[0].y),
            (0, 0)
        );
        assert_eq!(
            (replay.frames[0].players[1].x, replay.frames[0].players[1].y),
            (12_800, 12_800)
        );
        assert_eq!(
            (replay.events[0].x, replay.events[0].y),
            (Some(12_800), Some(0))
        );
        assert!(replay.availability.positions);
        assert!(!replay.availability.control_estimate);
    }

    #[test]
    fn map_12_without_reliable_spawn_clusters_keeps_movement() {
        let detail = sanitized_aram_detail();
        let timeline: RawTimelineInfo = serde_json::from_value(serde_json::json!({
            "frameInterval": 60000,
            "frames": [{
                "timestamp": 60000,
                "participantFrames": {
                    "1": {"participantId": 1, "position": {"x": 4300, "y": 4100}},
                    "2": {"participantId": 2, "position": {"x": 4500, "y": 4200}},
                    "3": {"participantId": 3, "position": {"x": 4700, "y": 4400}},
                    "6": {"participantId": 6, "position": {"x": 8500, "y": 8200}},
                    "7": {"participantId": 7, "position": {"x": 8700, "y": 8400}},
                    "8": {"participantId": 8, "position": {"x": 8900, "y": 8600}}
                }
            }]
        }))
        .expect("late ARAM frame should decode");
        let replay = build_timeline_replay("EUN1_ARAM_LATE".into(), &detail, timeline);

        assert!(replay.availability.positions);
        assert!(replay.availability.positions_reason.is_none());
        assert!(!replay.availability.control_estimate);
        assert!(replay.control_model.is_none());
        assert!(replay
            .availability
            .control_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("spawn anchors")));
    }

    #[test]
    fn unsupported_map_retains_safe_movement_with_observed_extent() {
        let mut detail = representative_detail();
        detail.map_id = 35;
        let timeline: RawTimelineInfo = serde_json::from_value(serde_json::json!({
            "frameInterval": 60000,
            "frames": [{
                "timestamp": 0,
                "participantFrames": {
                    "1": {"participantId": 1, "position": {"x": -2000, "y": 7000}},
                    "6": {"participantId": 6, "position": {"x": 22000, "y": 9000}}
                }
            }]
        }))
        .expect("unknown-map positions should decode");
        let replay = build_timeline_replay("EUN1_UNKNOWN_MAP".into(), &detail, timeline);

        assert_eq!(replay.coordinates.model_id, "observed_extent_v1");
        assert_eq!(replay.coordinates.provenance, "timeline_observed_extent");
        assert!(replay.coordinates.min_x < -2000);
        assert!(replay.coordinates.max_x > 22000);
        assert_eq!(replay.frames[0].players.len(), 2);
        assert_eq!(replay.frames[0].players[0].x, -2000);
        assert_eq!(replay.frames[0].players[1].x, 22000);
        assert!(replay.availability.positions);
        assert!(!replay.availability.control_estimate);
        assert!(replay
            .availability
            .control_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("map 35")));
    }

    #[test]
    fn serialized_contract_stays_compact_and_snake_case() {
        let detail = representative_detail();
        let replay = build_timeline_replay("EUN1_123".into(), &detail, representative_timeline());
        let value = serde_json::to_value(replay).expect("replay should serialize");

        assert!(value.get("frame_interval_ms").is_some());
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["game_version"], "16.15.1.1234");
        assert_eq!(value["coordinates"]["model_id"], "summoners_rift_rect_v1");
        assert_eq!(value["control_model"]["id"], "summoners_rift_radial_v1");
        assert!(value["availability"].get("positions_reason").is_some());
        assert!(value["availability"].get("control_reason").is_some());
        assert!(value["availability"].get("reason").is_none());
        assert!(value["frames"][0].get("players").is_some());
        assert!(value["frames"][0].get("participantFrames").is_none());
        assert!(value.to_string().len() < 10_000);
    }
}
