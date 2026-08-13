use crate::live_client::{
    ActivePlayerPayload, GameStatus, Kda, LiveGameTickPayload, MetricAvailabilityPayload,
    MetricSourcesPayload, ObjectivesPayload, ObservableValueSource, PlayerStatsPayload, PlayerTeam,
};
use crate::perf::process_guard;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::time::sleep;

// Local Live Client Data endpoints. These are available only while a match is
// active and do not use the developer API key or write to disk.
const GAMESTATS_URL: &str = "https://127.0.0.1:2999/liveclientdata/gamestats";
const EVENTDATA_URL: &str = "https://127.0.0.1:2999/liveclientdata/eventdata";
const ACTIVE_PLAYER_URL: &str = "https://127.0.0.1:2999/liveclientdata/activeplayer";
const PLAYER_LIST_URL: &str = "https://127.0.0.1:2999/liveclientdata/playerlist";

const ACTIVE_TICK_INTERVAL: Duration = Duration::from_secs(1);
const EVENT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const PLAYER_UPDATE_INTERVAL: Duration = Duration::from_secs(5);
const ACTIVE_END_FAILURE_THRESHOLD: u8 = 3;
const MAX_LOCAL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

// Confirmed against Riot's documented objective rules. Elder Dragon has a
// different rule after soul and is intentionally not guessed here.
const DRAGON_RESPAWN_SECS: f64 = 300.0;
const BARON_RESPAWN_SECS: f64 = 360.0;

#[derive(Deserialize, Debug, Clone)]
pub struct GameStats {
    #[serde(rename = "gameTime")]
    pub game_time: f64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct EventFeed {
    #[serde(rename = "Events")]
    pub events: Vec<GameEvent>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GameEvent {
    #[serde(rename = "EventName")]
    pub event_name: String,
    #[serde(rename = "EventTime")]
    pub event_time: f64,
    #[serde(rename = "DragonType", default)]
    pub dragon_type: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
struct ActivePlayerData {
    #[serde(rename = "summonerName", default)]
    summoner_name: String,
    #[serde(rename = "riotId", default)]
    riot_id: String,
    #[serde(rename = "riotIdGameName", default)]
    riot_id_game_name: String,
    #[serde(rename = "riotIdTagLine", default)]
    riot_id_tag_line: String,
    #[serde(rename = "currentGold", default)]
    current_gold: Option<f64>,
}

#[derive(Deserialize, Debug, Clone, Default)]
struct PlayerScores {
    #[serde(default)]
    kills: Option<u32>,
    #[serde(default)]
    deaths: Option<u32>,
    #[serde(default)]
    assists: Option<u32>,
    #[serde(rename = "creepScore", default)]
    creep_score: Option<u32>,
}

#[derive(Deserialize, Debug, Clone)]
struct PlayerItem {
    #[serde(rename = "itemID", alias = "itemId", default)]
    item_id: u32,
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Deserialize, Debug, Clone)]
struct PlayerData {
    #[serde(rename = "summonerName", default)]
    summoner_name: String,
    #[serde(rename = "riotId", default)]
    riot_id: String,
    #[serde(rename = "riotIdGameName", default)]
    riot_id_game_name: String,
    #[serde(rename = "riotIdTagLine", default)]
    riot_id_tag_line: String,
    #[serde(rename = "championName", default)]
    champion_name: String,
    #[serde(default)]
    team: String,
    #[serde(default)]
    level: Option<u32>,
    #[serde(default)]
    scores: PlayerScores,
    #[serde(default)]
    items: Option<Vec<PlayerItem>>,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct ObjectiveTimers {
    pub game_time: f64,
    pub dragon_respawn_at: Option<f64>,
    pub baron_respawn_at: Option<f64>,
    pub last_dragon_type: Option<String>,
}

fn log_ok(msg: &str) {
    println!("[AURA::GAME][OK] {msg}");
}

fn log_err(msg: &str) {
    eprintln!("[AURA::GAME][ERR] {msg}");
}

static CLIENT: OnceLock<Client> = OnceLock::new();

fn client() -> Result<&'static Client, String> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let built = Client::builder()
        .danger_accept_invalid_certs(true) // the local game client uses a self-signed cert
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(|error| error.to_string())?;
    Ok(CLIENT.get_or_init(|| built))
}

/// One supervisor owns all Live Client Data requests. During a match it emits
/// the HUD tick once per second, refreshes objective events and player/item
/// diffs at five-second intervals, and publishes one canonical typed tick.
pub async fn run_telemetry_loop(app: AppHandle) {
    log_ok("telemetry supervisor started");
    let mut match_active = false;
    let mut cached_events = Vec::new();
    let mut last_event_refresh: Option<Instant> = None;
    let mut last_player_update: Option<Instant> = None;
    let mut last_detail_error: Option<String> = None;
    let mut last_stats_error: Option<String> = None;
    let mut last_event_error: Option<String> = None;
    let mut consecutive_stats_failures = 0_u8;

    loop {
        // Process-table snapshots are useful for cheap idle discovery, but are
        // intentionally skipped during a match. The local endpoint failure
        // threshold detects a closed/crashed game without a 1 Hz OS scan.
        if should_scan_process_table(match_active)
            && process_guard::find_pid_by_name("League of Legends.exe").is_err()
        {
            sleep(Duration::from_secs(10)).await;
            continue;
        }

        // These three endpoints are independent and fetched concurrently; the
        // loop never requests the same resource twice for a single tick.
        let (stats_result, active_result, players_result) = tokio::join!(
            fetch_json::<GameStats>(GAMESTATS_URL),
            fetch_json::<ActivePlayerData>(ACTIVE_PLAYER_URL),
            fetch_json::<Vec<PlayerData>>(PLAYER_LIST_URL),
        );

        let stats = match stats_result {
            Ok(stats) => stats,
            Err(error) => {
                // Loading screen/champ select: process exists but port 2999 is
                // not serving game data yet. This is normal.
                if match_active {
                    consecutive_stats_failures = consecutive_stats_failures.saturating_add(1);
                    log_detail_error_once(
                        &mut last_stats_error,
                        format!("gamestats refresh failed: {}", bounded_error(&error)),
                    );
                    if consecutive_stats_failures >= ACTIVE_END_FAILURE_THRESHOLD {
                        end_match(&app);
                        match_active = false;
                        cached_events.clear();
                        last_event_refresh = None;
                        last_player_update = None;
                        last_detail_error = None;
                        last_stats_error = None;
                        last_event_error = None;
                        consecutive_stats_failures = 0;
                    }
                }
                sleep(if match_active {
                    ACTIVE_TICK_INTERVAL
                } else {
                    Duration::from_secs(5)
                })
                .await;
                continue;
            }
        };

        consecutive_stats_failures = 0;
        if last_stats_error.take().is_some() {
            log_ok("gamestats telemetry recovered");
        }

        if !match_active {
            if let Err(error) = crate::overlay::show_for_match(&app) {
                log_err(&error);
            }
            if let Err(error) = crate::live_client::stream_game_status(&app, GameStatus::InGame) {
                log_err(&error);
            }
            match_active = true;
        }

        let now = Instant::now();
        if last_event_refresh
            .map(|last| now.duration_since(last) >= EVENT_REFRESH_INTERVAL)
            .unwrap_or(true)
        {
            match fetch_json::<EventFeed>(EVENTDATA_URL).await {
                Ok(feed) => {
                    cached_events = feed.events;
                    if last_event_error.take().is_some() {
                        log_ok("eventdata telemetry recovered");
                    }
                }
                Err(error) => log_detail_error_once(
                    &mut last_event_error,
                    format!("eventdata refresh failed: {}", bounded_error(&error)),
                ),
            }
            last_event_refresh = Some(now);
        }

        let timers = compute_objective_timers(stats.game_time, &cached_events);

        match (active_result, players_result) {
            (Ok(active), Ok(players)) => {
                let detail_resolved = if let Some(payload) =
                    build_live_game_tick(&stats, &active, &players, &timers)
                {
                    if let Err(error) = crate::live_client::stream_game_tick(&app, payload) {
                        log_err(&error);
                    }
                    true
                } else {
                    log_detail_error_once(
                        &mut last_detail_error,
                        "active player was not present in playerlist".to_string(),
                    );
                    false
                };

                if last_player_update
                    .map(|last| now.duration_since(last) >= PLAYER_UPDATE_INTERVAL)
                    .unwrap_or(true)
                {
                    for player in &players {
                        if let Some(payload) = build_player_update(player) {
                            if let Err(error) =
                                crate::live_client::stream_player_update(&app, payload)
                            {
                                log_err(&error);
                            }
                        }
                    }
                    last_player_update = Some(now);
                }

                if detail_resolved && last_detail_error.take().is_some() {
                    log_ok("live player telemetry recovered");
                }
            }
            (active, players) => {
                let error = format!(
                    "live detail refresh incomplete: activeplayer={}, playerlist={}",
                    result_description(&active),
                    result_description(&players)
                );
                log_detail_error_once(&mut last_detail_error, error);
            }
        }

        sleep(ACTIVE_TICK_INTERVAL).await;
    }
}

fn should_scan_process_table(match_active: bool) -> bool {
    !match_active
}

fn end_match(app: &AppHandle) {
    if let Err(error) = crate::overlay::end_match(app) {
        log_err(&error);
    }
    if let Err(error) = crate::live_client::stream_game_status(app, GameStatus::Ended) {
        log_err(&error);
    }
}

fn log_detail_error_once(previous: &mut Option<String>, error: String) {
    if previous.as_deref() != Some(error.as_str()) {
        log_err(&error);
        *previous = Some(error);
    }
}

fn result_description<T>(result: &Result<T, String>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(error) => bounded_error(error),
    }
}

fn bounded_error(error: &str) -> String {
    const LIMIT: usize = 180;
    let mut value = error.trim().chars().take(LIMIT).collect::<String>();
    if error.trim().chars().count() > LIMIT {
        value.push_str("...");
    }
    value
}

async fn fetch_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_LOCAL_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "local response exceeded {MAX_LOCAL_RESPONSE_BYTES} bytes"
        ));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        let next_length = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "local response size overflow".to_string())?;
        if next_length > MAX_LOCAL_RESPONSE_BYTES {
            return Err(format!(
                "local response exceeded {MAX_LOCAL_RESPONSE_BYTES} bytes"
            ));
        }
        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice::<T>(&body).map_err(|error| format!("decode failed: {error}"))
}

fn build_live_game_tick(
    stats: &GameStats,
    active: &ActivePlayerData,
    players: &[PlayerData],
    timers: &ObjectiveTimers,
) -> Option<LiveGameTickPayload> {
    let active_name = preferred_riot_name(
        &active.summoner_name,
        &active.riot_id,
        &active.riot_id_game_name,
        &active.riot_id_tag_line,
    );
    let player = find_active_player(&active_name, players)?;
    let player_name = player_display_name(player);
    let game_time = finite_non_negative(stats.game_time);
    let current_gold = finite_non_negative_option(active.current_gold);
    let kda_available = player.scores.kills.is_some()
        && player.scores.deaths.is_some()
        && player.scores.assists.is_some();
    let level = player.level;
    let creep_score = player.scores.creep_score;
    let creep_score_per_minute = creep_score
        .map(f64::from)
        .and_then(|value| rate_per_minute(value, game_time));
    let kill_participation_percent = kill_participation_percent(player, players);
    let observable_held_value = observable_held_value(active, player);
    let observable_value_per_minute =
        observable_held_value.and_then(|value| rate_per_minute(value, game_time));

    Some(LiveGameTickPayload {
        game_time,
        active_player: ActivePlayerPayload {
            summoner_name: player_name,
            champion_name: player.champion_name.clone(),
            current_gold: current_gold.unwrap_or(0.0),
            level: level.unwrap_or(0),
            creep_score: creep_score.unwrap_or(0),
            creep_score_per_minute: creep_score_per_minute.unwrap_or(0.0),
            kill_participation_percent: kill_participation_percent.unwrap_or(0.0),
            observable_held_value: observable_held_value.unwrap_or(0.0),
            observable_value_per_minute: observable_value_per_minute.unwrap_or(0.0),
            // Current cash plus held items is not lifetime earned gold.
            earned_gold_per_minute: 0.0,
            xp_progress_percent: None,
            kda: Kda {
                kills: player.scores.kills.unwrap_or(0),
                deaths: player.scores.deaths.unwrap_or(0),
                assists: player.scores.assists.unwrap_or(0),
            },
            // Riot's official local API does not expose live champion damage.
            dpm: 0.0,
        },
        // It also does not expose exact team total gold. Do not present item
        // inventory value as if it were earned-gold advantage.
        team_gold_delta: 0,
        objectives: ObjectivesPayload {
            dragon_type: timers.last_dragon_type.clone(),
            dragon_timer: remaining_seconds(timers.dragon_respawn_at, stats.game_time),
            baron_timer: remaining_seconds(timers.baron_respawn_at, stats.game_time),
        },
        metric_availability: MetricAvailabilityPayload {
            current_gold: current_gold.is_some(),
            kda: kda_available,
            dpm: false,
            team_gold_delta: false,
            level: level.is_some(),
            creep_score: creep_score.is_some(),
            creep_score_per_minute: creep_score_per_minute.is_some(),
            kill_participation_percent: kill_participation_percent.is_some(),
            observable_held_value: observable_held_value.is_some(),
            observable_value_per_minute: observable_value_per_minute.is_some(),
            earned_gold_per_minute: false,
            xp_progress_percent: false,
        },
        metric_sources: MetricSourcesPayload {
            observable_held_value: observable_held_value
                .map(|_| ObservableValueSource::CurrentGoldPlusCurrentInventoryListedValue),
            observable_value_per_minute: observable_value_per_minute
                .map(|_| ObservableValueSource::CurrentGoldPlusCurrentInventoryListedValue),
        },
    })
}

fn build_player_update(player: &PlayerData) -> Option<PlayerStatsPayload> {
    let team = player_team(&player.team)?;

    Some(PlayerStatsPayload {
        summoner_name: player_display_name(player),
        champion_name: player.champion_name.clone(),
        team,
        level: player.level.unwrap_or(0),
        creep_score: player.scores.creep_score.unwrap_or(0),
        items: player
            .items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|item| item.item_id)
            .filter(|item_id| *item_id != 0)
            .collect(),
    })
}

fn player_team(value: &str) -> Option<PlayerTeam> {
    match value.trim().to_ascii_uppercase().as_str() {
        "ORDER" => Some(PlayerTeam::Order),
        // Riot's wire value is normally CHAOS; Aura's public contract calls
        // the red side HARMONY, so both inputs map to that stable output.
        "CHAOS" | "HARMONY" => Some(PlayerTeam::Harmony),
        _ => None,
    }
}

fn kill_participation_percent(player: &PlayerData, players: &[PlayerData]) -> Option<f64> {
    let team = player_team(&player.team)?;
    let participations = u64::from(player.scores.kills?) + u64::from(player.scores.assists?);
    let mut team_kills = 0_u64;
    let mut teammate_seen = false;

    for teammate in players {
        if player_team(&teammate.team) != Some(team) {
            continue;
        }
        teammate_seen = true;
        team_kills = team_kills.checked_add(u64::from(teammate.scores.kills?))?;
    }

    if !teammate_seen || team_kills == 0 || participations > team_kills {
        return None;
    }
    Some(participations as f64 / team_kills as f64 * 100.0)
}

fn observable_held_value(active: &ActivePlayerData, player: &PlayerData) -> Option<f64> {
    let current_gold = finite_non_negative_option(active.current_gold)?;
    let items = player.items.as_ref()?;
    let mut inventory_value = 0.0;

    for item in items.iter().filter(|item| item.item_id != 0) {
        let price = finite_non_negative_option(item.price)?;
        let count = item.count?;
        inventory_value += price * f64::from(count);
        if !inventory_value.is_finite() {
            return None;
        }
    }

    let total = current_gold + inventory_value;
    total.is_finite().then_some(total)
}

fn rate_per_minute(value: f64, game_time: f64) -> Option<f64> {
    if !value.is_finite() || value < 0.0 || !game_time.is_finite() || game_time <= 0.0 {
        return None;
    }
    let rate = value * 60.0 / game_time;
    rate.is_finite().then_some(rate)
}

fn find_active_player<'a>(
    summoner_name: &str,
    players: &'a [PlayerData],
) -> Option<&'a PlayerData> {
    let full_name = normalize_name(summoner_name);
    if full_name.is_empty() {
        return None;
    }

    if let Some(exact) = players
        .iter()
        .find(|player| normalize_name(&player_display_name(player)) == full_name)
    {
        return Some(exact);
    }

    // Some client versions expose `Name#TAG` on only one of the two endpoints.
    // Use the game-name portion only when it identifies exactly one player.
    let game_name = full_name.split('#').next().unwrap_or(&full_name);
    let mut candidates = players.iter().filter(|player| {
        normalize_name(&player_display_name(player))
            .split('#')
            .next()
            .is_some_and(|candidate| candidate == game_name)
    });
    let first = candidates.next()?;
    if candidates.next().is_some() {
        None
    } else {
        Some(first)
    }
}

fn player_display_name(player: &PlayerData) -> String {
    preferred_riot_name(
        &player.summoner_name,
        &player.riot_id,
        &player.riot_id_game_name,
        &player.riot_id_tag_line,
    )
}

fn preferred_riot_name(
    summoner_name: &str,
    riot_id: &str,
    riot_id_game_name: &str,
    riot_id_tag_line: &str,
) -> String {
    let riot_id = riot_id.trim();
    if !riot_id.is_empty() {
        return riot_id.to_string();
    }

    let game_name = riot_id_game_name.trim();
    let tag_line = riot_id_tag_line.trim();
    if !game_name.is_empty() && !tag_line.is_empty() {
        return format!("{game_name}#{tag_line}");
    }
    if !game_name.is_empty() {
        return game_name.to_string();
    }

    summoner_name.trim().to_string()
}

fn normalize_name(value: &str) -> String {
    value.trim().to_lowercase()
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

fn finite_non_negative_option(value: Option<f64>) -> Option<f64> {
    value.filter(|number| number.is_finite() && *number >= 0.0)
}

fn remaining_seconds(respawn_at: Option<f64>, game_time: f64) -> f64 {
    respawn_at
        .filter(|value| value.is_finite())
        .map(|value| (value - finite_non_negative(game_time)).max(0.0))
        .unwrap_or(0.0)
}

/// Dragon/Baron only: the Live Client Data feed has no camp/buff-kill events,
/// so jungle-camp timers remain explicit manual controls in the overlay.
fn compute_objective_timers(game_time: f64, events: &[GameEvent]) -> ObjectiveTimers {
    let last_dragon = events
        .iter()
        .rfind(|event| event.event_name == "DragonKill");
    let last_baron = events.iter().rfind(|event| event.event_name == "BaronKill");

    ObjectiveTimers {
        game_time: finite_non_negative(game_time),
        dragon_respawn_at: last_dragon
            .map(|event| event.event_time + DRAGON_RESPAWN_SECS)
            .filter(|value| value.is_finite()),
        baron_respawn_at: last_baron
            .map(|event| event.event_time + BARON_RESPAWN_SECS)
            .filter(|value| value.is_finite()),
        last_dragon_type: last_dragon.and_then(|event| event.dragon_type.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_player(name: &str, team: &str) -> PlayerData {
        PlayerData {
            summoner_name: name.into(),
            riot_id: String::new(),
            riot_id_game_name: String::new(),
            riot_id_tag_line: String::new(),
            champion_name: "Briar".into(),
            team: team.into(),
            level: Some(9),
            scores: PlayerScores {
                kills: Some(3),
                deaths: Some(1),
                assists: Some(4),
                creep_score: Some(91),
            },
            items: Some(vec![
                PlayerItem {
                    item_id: 3078,
                    price: Some(3_333.0),
                    count: Some(1),
                },
                PlayerItem {
                    item_id: 0,
                    price: None,
                    count: None,
                },
            ]),
        }
    }

    #[test]
    fn builds_truthful_tick_without_fabricating_unavailable_metrics() {
        let stats = GameStats { game_time: 600.0 };
        let active = ActivePlayerData {
            summoner_name: "Aura Player#EUNE".into(),
            riot_id: String::new(),
            riot_id_game_name: String::new(),
            riot_id_tag_line: String::new(),
            current_gold: Some(725.0),
        };
        let mut teammate = sample_player("Blue Teammate#EUNE", "ORDER");
        teammate.scores.kills = Some(7);
        teammate.scores.assists = Some(2);
        let players = vec![sample_player("Aura Player#EUNE", "ORDER"), teammate];
        let timers = ObjectiveTimers {
            game_time: 600.0,
            dragon_respawn_at: Some(630.0),
            baron_respawn_at: None,
            last_dragon_type: Some("Mountain".into()),
        };

        let payload = build_live_game_tick(&stats, &active, &players, &timers)
            .expect("active player should resolve");

        assert_eq!(payload.active_player.champion_name, "Briar");
        assert_eq!(payload.active_player.current_gold, 725.0);
        assert_eq!(payload.active_player.kda.kills, 3);
        assert_eq!(payload.active_player.level, 9);
        assert_eq!(payload.active_player.creep_score, 91);
        assert!((payload.active_player.creep_score_per_minute - 9.1).abs() < f64::EPSILON);
        assert!((payload.active_player.kill_participation_percent - 70.0).abs() < f64::EPSILON);
        assert_eq!(payload.active_player.observable_held_value, 4_058.0);
        assert!((payload.active_player.observable_value_per_minute - 405.8).abs() < 0.000_001);
        assert_eq!(payload.active_player.earned_gold_per_minute, 0.0);
        assert_eq!(payload.active_player.xp_progress_percent, None);
        assert_eq!(payload.objectives.dragon_timer, 30.0);
        assert_eq!(payload.objectives.baron_timer, 0.0);
        assert_eq!(payload.active_player.dpm, 0.0);
        assert_eq!(payload.team_gold_delta, 0);
        assert!(!payload.metric_availability.dpm);
        assert!(!payload.metric_availability.team_gold_delta);
        assert!(payload.metric_availability.current_gold);
        assert!(payload.metric_availability.kda);
        assert!(payload.metric_availability.level);
        assert!(payload.metric_availability.creep_score);
        assert!(payload.metric_availability.creep_score_per_minute);
        assert!(payload.metric_availability.kill_participation_percent);
        assert!(payload.metric_availability.observable_held_value);
        assert!(payload.metric_availability.observable_value_per_minute);
        assert!(!payload.metric_availability.earned_gold_per_minute);
        assert!(!payload.metric_availability.xp_progress_percent);
        assert_eq!(
            payload.metric_sources.observable_held_value,
            Some(ObservableValueSource::CurrentGoldPlusCurrentInventoryListedValue)
        );
        assert_eq!(
            payload.metric_sources.observable_value_per_minute,
            Some(ObservableValueSource::CurrentGoldPlusCurrentInventoryListedValue)
        );
    }

    #[test]
    fn active_player_name_fallback_requires_a_unique_game_name() {
        let unique = vec![sample_player("Aura Player", "ORDER")];
        assert!(find_active_player("Aura Player#EUNE", &unique).is_some());

        let ambiguous = vec![
            sample_player("Aura Player#EUNE", "ORDER"),
            sample_player("Aura Player#EUW", "CHAOS"),
        ];
        assert!(find_active_player("Aura Player#NA", &ambiguous).is_none());
    }

    #[test]
    fn decodes_responses_containing_both_legacy_and_riot_id_fields() {
        let active: ActivePlayerData = serde_json::from_value(serde_json::json!({
            "summonerName": "Legacy Display",
            "riotId": "Aura Player#EUNE",
            "riotIdGameName": "Aura Player",
            "riotIdTagLine": "EUNE",
            "currentGold": 550.0
        }))
        .expect("both identity shapes must not cause a duplicate serde field");
        let player: PlayerData = serde_json::from_value(serde_json::json!({
            "summonerName": "Legacy Display",
            "riotId": "Aura Player#EUNE",
            "riotIdGameName": "Aura Player",
            "riotIdTagLine": "EUNE",
            "championName": "Briar",
            "team": "ORDER",
            "level": 8,
            "scores": {},
            "items": []
        }))
        .expect("playerlist identity fields must decode independently");

        assert_eq!(
            preferred_riot_name(
                &active.summoner_name,
                &active.riot_id,
                &active.riot_id_game_name,
                &active.riot_id_tag_line
            ),
            "Aura Player#EUNE"
        );
        assert_eq!(player_display_name(&player), "Aura Player#EUNE");
    }

    #[test]
    fn player_update_maps_chaos_to_public_harmony_and_filters_empty_items() {
        let payload = build_player_update(&sample_player("Red", "CHAOS"))
            .expect("known team should produce an update");

        assert_eq!(payload.team, PlayerTeam::Harmony);
        assert_eq!(payload.creep_score, 91);
        assert_eq!(payload.items, vec![3078]);
    }

    #[test]
    fn incomplete_item_prices_disable_the_observable_value_estimate() {
        let stats = GameStats { game_time: 600.0 };
        let active = ActivePlayerData {
            summoner_name: "Aura Player".into(),
            riot_id: String::new(),
            riot_id_game_name: String::new(),
            riot_id_tag_line: String::new(),
            current_gold: Some(500.0),
        };
        let mut player = sample_player("Aura Player", "ORDER");
        player.items = Some(vec![PlayerItem {
            item_id: 3078,
            price: None,
            count: Some(1),
        }]);
        let payload = build_live_game_tick(&stats, &active, &[player], &ObjectiveTimers::default())
            .expect("identity should still resolve");

        assert_eq!(payload.active_player.observable_held_value, 0.0);
        assert_eq!(payload.active_player.observable_value_per_minute, 0.0);
        assert_eq!(payload.active_player.earned_gold_per_minute, 0.0);
        assert!(!payload.metric_availability.observable_held_value);
        assert!(!payload.metric_availability.observable_value_per_minute);
        assert!(!payload.metric_availability.earned_gold_per_minute);
        assert_eq!(payload.metric_sources.observable_held_value, None);
        assert_eq!(payload.metric_sources.observable_value_per_minute, None);
    }

    #[test]
    fn zero_team_kills_and_missing_xp_are_reported_unavailable() {
        let stats = GameStats { game_time: 120.0 };
        let active = ActivePlayerData {
            summoner_name: "Aura Player".into(),
            riot_id: String::new(),
            riot_id_game_name: String::new(),
            riot_id_tag_line: String::new(),
            current_gold: Some(500.0),
        };
        let mut player = sample_player("Aura Player", "ORDER");
        player.scores.kills = Some(0);
        player.scores.assists = Some(0);
        let payload = build_live_game_tick(&stats, &active, &[player], &ObjectiveTimers::default())
            .expect("identity should still resolve");

        assert_eq!(payload.active_player.kill_participation_percent, 0.0);
        assert!(!payload.metric_availability.kill_participation_percent);
        assert_eq!(payload.active_player.xp_progress_percent, None);
        assert!(!payload.metric_availability.xp_progress_percent);
    }

    #[test]
    fn missing_current_gold_and_kda_fields_use_flagged_sentinels() {
        let stats = GameStats { game_time: 120.0 };
        let active = ActivePlayerData {
            summoner_name: "Aura Player".into(),
            riot_id: String::new(),
            riot_id_game_name: String::new(),
            riot_id_tag_line: String::new(),
            current_gold: None,
        };
        let mut player = sample_player("Aura Player", "ORDER");
        player.scores.kills = None;
        player.scores.deaths = None;
        player.scores.assists = None;
        let payload = build_live_game_tick(&stats, &active, &[player], &ObjectiveTimers::default())
            .expect("identity and other fields should survive partial metrics");

        assert_eq!(payload.active_player.current_gold, 0.0);
        assert_eq!(payload.active_player.kda, Kda::default());
        assert!(!payload.metric_availability.current_gold);
        assert!(!payload.metric_availability.kda);
        assert!(!payload.metric_availability.observable_held_value);
        assert!(!payload.metric_availability.observable_value_per_minute);
    }

    #[test]
    fn active_tick_cadence_skips_windows_process_table_scans() {
        assert!(should_scan_process_table(false));
        assert!(!should_scan_process_table(true));
    }

    #[test]
    fn objective_timers_use_the_last_matching_events() {
        let events = vec![
            GameEvent {
                event_name: "DragonKill".into(),
                event_time: 100.0,
                dragon_type: Some("Air".into()),
            },
            GameEvent {
                event_name: "DragonKill".into(),
                event_time: 410.0,
                dragon_type: Some("Infernal".into()),
            },
            GameEvent {
                event_name: "BaronKill".into(),
                event_time: 500.0,
                dragon_type: None,
            },
        ];

        let timers = compute_objective_timers(600.0, &events);
        assert_eq!(timers.dragon_respawn_at, Some(710.0));
        assert_eq!(timers.baron_respawn_at, Some(860.0));
        assert_eq!(timers.last_dragon_type.as_deref(), Some("Infernal"));
    }
}
