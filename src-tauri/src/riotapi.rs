use futures_util::{stream, StreamExt};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OnceCell};
use url::Url;

pub(crate) mod timeline;
pub use timeline::MatchTimelineReplay;

// Aura may discover the locally signed-in PUUID from the League Client, but
// all public profile, rank, mastery, and match data is still read through
// Riot's supported web APIs. Manual Riot-ID selection resolves through
// Account-V1; local selection uses Account-V1 to recover the canonical visible
// Riot ID before the same public API pipeline runs.

fn log_ok(msg: &str) {
    println!("[AURA::RIOTAPI][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::RIOTAPI][ERR] {}", msg);
}

#[derive(Debug)]
pub enum RiotApiError {
    MissingApiKey,
    UnknownPlatform(String),
    RequestFailed(String),
    RateLimited,
    BadStatus(u16),
    DecodeFailed(String),
    NotIdentified,
}
impl std::fmt::Display for RiotApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiotApiError::MissingApiKey => write!(
                f,
                "RIOT_API_KEY is not configured; set it before starting Aura"
            ),
            RiotApiError::UnknownPlatform(p) => write!(f, "unrecognized platform '{}'", p),
            RiotApiError::RequestFailed(s) => write!(f, "request failed: {}", s),
            RiotApiError::RateLimited => {
                write!(f, "rate limited by Riot API (429) — try again shortly")
            }
            RiotApiError::BadStatus(401 | 403) => {
                write!(f, "Riot API key was rejected or has expired")
            }
            RiotApiError::BadStatus(404) => write!(f, "Riot account or match data was not found"),
            RiotApiError::BadStatus(code) => write!(f, "Riot API returned HTTP {}", code),
            RiotApiError::DecodeFailed(s) => write!(f, "response decode failed: {}", s),
            RiotApiError::NotIdentified => write!(f, "no Riot ID set yet — call set_riot_id first"),
        }
    }
}
impl std::error::Error for RiotApiError {}

/// Platform routing (na1, euw1, kr, ...) — used by Summoner-V4/League-V4.
/// Kept as a distinct concept from RegionGroup on purpose: these two routing
/// schemes are not interchangeable, and confusing them is the single most
/// common mistake made against this API.
///
/// SEA platforms (PH2/SG2/TH2/TW2/VN2) included below. Confirmed by directly
/// fetching Riot's live account-v1 API explorer page (not a cached/dated
/// snapshot): every operation on that page — getByPuuid, getByRiotId,
/// getByAccessToken, getActiveShard, getActiveRegion — lists only
/// AMERICAS/ASIA/EUROPE in its "Select Region to Execute Against" control,
/// no SEA option anywhere. Match-V5 supporting "sea" is confirmed separately
/// via an official Riot Developer Relations announcement. Two different
/// endpoints, two different supported region lists — handled via two
/// separate mapping methods below so this can't get silently conflated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Na1,
    Euw1,
    Eun1,
    Kr,
    Jp1,
    Br1,
    La1,
    La2,
    Oc1,
    Tr1,
    Ru,
    Ph2,
    Sg2,
    Th2,
    Tw2,
    Vn2,
}

/// Regional routing (americas, europe, asia, sea) — used by Match-V5 and Account-V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionGroup {
    Americas,
    Europe,
    Asia,
    Sea,
}

impl Platform {
    fn from_str(s: &str) -> Result<Self, RiotApiError> {
        match s.to_lowercase().as_str() {
            "na1" => Ok(Platform::Na1),
            "euw1" => Ok(Platform::Euw1),
            "eun1" => Ok(Platform::Eun1),
            "kr" => Ok(Platform::Kr),
            "jp1" => Ok(Platform::Jp1),
            "br1" => Ok(Platform::Br1),
            "la1" => Ok(Platform::La1),
            "la2" => Ok(Platform::La2),
            "oc1" => Ok(Platform::Oc1),
            "tr1" => Ok(Platform::Tr1),
            "ru" => Ok(Platform::Ru),
            "ph2" => Ok(Platform::Ph2),
            "sg2" => Ok(Platform::Sg2),
            "th2" => Ok(Platform::Th2),
            "tw2" => Ok(Platform::Tw2),
            "vn2" => Ok(Platform::Vn2),
            other => Err(RiotApiError::UnknownPlatform(other.to_string())),
        }
    }

    /// For Match-V5 (and anything else that genuinely supports "sea").
    fn region_group(&self) -> RegionGroup {
        match self {
            Platform::Na1 | Platform::Br1 | Platform::La1 | Platform::La2 => RegionGroup::Americas,
            Platform::Euw1 | Platform::Eun1 | Platform::Tr1 | Platform::Ru => RegionGroup::Europe,
            Platform::Kr | Platform::Jp1 => RegionGroup::Asia,
            Platform::Oc1
            | Platform::Ph2
            | Platform::Sg2
            | Platform::Th2
            | Platform::Tw2
            | Platform::Vn2 => RegionGroup::Sea,
        }
    }

    /// For Account-V1 specifically — confirmed via Riot's own developer
    /// portal that this endpoint supports only americas/asia/europe, "sea"
    /// is not a valid value here even though it is for Match-V5. SEA
    /// platforms route through "asia" for this one call — per Riot's docs,
    /// any region can resolve any account, this is purely about which
    /// cluster you're allowed to ask, not about where the account "lives."
    fn region_group_for_account_v1(&self) -> RegionGroup {
        match self.region_group() {
            RegionGroup::Sea => RegionGroup::Asia,
            other => other,
        }
    }

    fn host(&self) -> &'static str {
        match self {
            Platform::Na1 => "na1.api.riotgames.com",
            Platform::Euw1 => "euw1.api.riotgames.com",
            Platform::Eun1 => "eun1.api.riotgames.com",
            Platform::Kr => "kr.api.riotgames.com",
            Platform::Jp1 => "jp1.api.riotgames.com",
            Platform::Br1 => "br1.api.riotgames.com",
            Platform::La1 => "la1.api.riotgames.com",
            Platform::La2 => "la2.api.riotgames.com",
            Platform::Oc1 => "oc1.api.riotgames.com",
            Platform::Tr1 => "tr1.api.riotgames.com",
            Platform::Ru => "ru.api.riotgames.com",
            Platform::Ph2 => "ph2.api.riotgames.com",
            Platform::Sg2 => "sg2.api.riotgames.com",
            Platform::Th2 => "th2.api.riotgames.com",
            Platform::Tw2 => "tw2.api.riotgames.com",
            Platform::Vn2 => "vn2.api.riotgames.com",
        }
    }
}

impl RegionGroup {
    fn host(&self) -> &'static str {
        match self {
            RegionGroup::Americas => "americas.api.riotgames.com",
            RegionGroup::Europe => "europe.api.riotgames.com",
            RegionGroup::Asia => "asia.api.riotgames.com",
            RegionGroup::Sea => "sea.api.riotgames.com",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    puuid: String,
    region_group: RegionGroup,
    platform: Platform,
}

#[derive(Clone)]
pub struct RiotApiState {
    api_key: Arc<Mutex<Option<Arc<str>>>>,
    identity: Arc<Mutex<Option<Identity>>>,
    ranked_entries: Arc<Mutex<Option<RankedEntriesCache>>>,
    ranked_entries_fetch: Arc<Mutex<Option<(Identity, RankedEntriesFlight)>>>,
    recent_matches: Arc<Mutex<Option<RecentMatchCache>>>,
    match_details: Arc<Mutex<HashMap<String, MatchDetail>>>,
    match_timelines: Arc<Mutex<TimelineCache>>,
    match_timeline_fetch: Arc<Mutex<()>>,
    champion_masteries: Arc<Mutex<Option<ChampionMasteryCache>>>,
}

impl RiotApiState {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key: Arc::new(Mutex::new(api_key.map(Arc::<str>::from))),
            identity: Arc::new(Mutex::new(None)),
            ranked_entries: Arc::new(Mutex::new(None)),
            ranked_entries_fetch: Arc::new(Mutex::new(None)),
            recent_matches: Arc::new(Mutex::new(None)),
            match_details: Arc::new(Mutex::new(HashMap::new())),
            match_timelines: Arc::new(Mutex::new(TimelineCache::default())),
            match_timeline_fetch: Arc::new(Mutex::new(())),
            champion_masteries: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_api_key(&self, api_key: Option<String>) {
        self.update_identity(None, true).await;
        *self.api_key.lock().await = api_key.map(Arc::<str>::from);
        self.clear_match_cache().await;
    }

    /// Changes the selected identity and invalidates rank state while holding
    /// the identity lock. That lock ordering prevents an old request from
    /// repopulating the rank cache between an identity change and its clear.
    async fn update_identity(&self, next: Option<Identity>, force_clear_ranked: bool) -> bool {
        let mut identity = self.identity.lock().await;
        let changed = *identity != next;
        if changed || force_clear_ranked {
            *identity = next;
            *self.ranked_entries_fetch.lock().await = None;
            *self.ranked_entries.lock().await = None;
        }
        changed
    }

    async fn ranked_entries_flight(&self, identity: &Identity) -> RankedEntriesFlight {
        let mut active = self.ranked_entries_fetch.lock().await;
        match active.as_ref() {
            Some((active_identity, flight)) if active_identity == identity => flight.clone(),
            _ => {
                let flight = Arc::new(OnceCell::new());
                *active = Some((identity.clone(), flight.clone()));
                flight
            }
        }
    }

    async fn clear_match_cache(&self) {
        let _timeline_guard = self.match_timeline_fetch.lock().await;
        *self.recent_matches.lock().await = None;
        self.match_details.lock().await.clear();
        self.match_timelines.lock().await.clear();
        *self.champion_masteries.lock().await = None;
    }
}

#[derive(Default)]
struct TimelineCache {
    entries: HashMap<String, MatchTimelineReplay>,
    access_order: VecDeque<String>,
}

impl TimelineCache {
    fn get(&mut self, match_id: &str) -> Option<MatchTimelineReplay> {
        let replay = self.entries.get(match_id).cloned()?;
        self.access_order.retain(|cached| cached != match_id);
        self.access_order.push_back(match_id.to_string());
        Some(replay)
    }

    fn insert(&mut self, match_id: String, replay: MatchTimelineReplay, capacity: usize) {
        self.entries.insert(match_id.clone(), replay);
        self.access_order.retain(|cached| cached != &match_id);
        self.access_order.push_back(match_id);
        while self.entries.len() > capacity {
            if let Some(evicted) = self.access_order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }
    }

    fn retain_ids(&mut self, retained: &HashSet<String>) {
        self.entries
            .retain(|match_id, _| retained.contains(match_id));
        self.access_order
            .retain(|match_id| retained.contains(match_id));
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }
}

const MATCH_CACHE_TTL: Duration = Duration::from_secs(120);
const MASTERY_CACHE_TTL: Duration = Duration::from_secs(120);
const RANKED_ENTRIES_CACHE_TTL: Duration = Duration::from_secs(120);
type RankedEntriesFlight = Arc<OnceCell<Result<Vec<RankedEntry>, String>>>;

struct RankedEntriesCache {
    identity: Identity,
    created_at: Instant,
    entries: Vec<RankedEntry>,
}

impl RankedEntriesCache {
    fn get(&self, identity: &Identity) -> Option<Vec<RankedEntry>> {
        if &self.identity == identity && self.created_at.elapsed() < RANKED_ENTRIES_CACHE_TTL {
            Some(self.entries.clone())
        } else {
            None
        }
    }
}

struct RecentMatchCache {
    puuid: String,
    requested_count: u32,
    created_at: Instant,
    summaries: Vec<MatchSummary>,
}

struct ChampionMasteryCache {
    puuid: String,
    created_at: Instant,
    masteries: Vec<ChampionMastery>,
}

static CLIENT: OnceLock<Client> = OnceLock::new();
fn client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(4)
            .build()
            .expect("Riot API HTTP client configuration is valid")
    })
}

async fn get_json(
    state: &RiotApiState,
    operation: &str,
    url: &str,
) -> Result<serde_json::Value, RiotApiError> {
    let key = state
        .api_key
        .lock()
        .await
        .clone()
        .ok_or(RiotApiError::MissingApiKey)?;
    let resp = client()
        .get(url)
        .header("X-Riot-Token", key.as_ref())
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RiotApiError::RequestFailed(safe_request_error(&error)))?;

    let status = resp.status();
    if status.as_u16() == 429 {
        log_err(&format!("{operation} was rate limited"));
        return Err(RiotApiError::RateLimited);
    }
    if !status.is_success() {
        log_err(&format!("{operation} returned HTTP {}", status.as_u16()));
        return Err(RiotApiError::BadStatus(status.as_u16()));
    }

    resp.json()
        .await
        .map_err(|e| RiotApiError::DecodeFailed(e.to_string()))
}

/// Typed variant used for large Match-V5 timelines. Deserializing directly
/// lets Serde skip heavyweight fields Aura does not use instead of first
/// allocating the complete response as a generic JSON value.
async fn get_json_as<T: DeserializeOwned>(
    state: &RiotApiState,
    operation: &str,
    url: &str,
) -> Result<T, RiotApiError> {
    let key = state
        .api_key
        .lock()
        .await
        .clone()
        .ok_or(RiotApiError::MissingApiKey)?;
    let resp = client()
        .get(url)
        .header("X-Riot-Token", key.as_ref())
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| RiotApiError::RequestFailed(safe_request_error(&error)))?;

    let status = resp.status();
    if status.as_u16() == 429 {
        log_err(&format!("{operation} was rate limited"));
        return Err(RiotApiError::RateLimited);
    }
    if !status.is_success() {
        log_err(&format!("{operation} returned HTTP {}", status.as_u16()));
        return Err(RiotApiError::BadStatus(status.as_u16()));
    }
    const MAX_TYPED_JSON_BYTES: usize = 16 * 1024 * 1024;
    if resp
        .content_length()
        .is_some_and(|length| length > MAX_TYPED_JSON_BYTES as u64)
    {
        return Err(RiotApiError::DecodeFailed(
            "response exceeded Aura's 16 MiB safety limit".into(),
        ));
    }

    let initial_capacity = resp
        .content_length()
        .unwrap_or_default()
        .min(MAX_TYPED_JSON_BYTES as u64) as usize;
    let mut body = Vec::with_capacity(initial_capacity);
    let mut chunks = resp.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk =
            chunk.map_err(|error| RiotApiError::RequestFailed(safe_request_error(&error)))?;
        if body.len().saturating_add(chunk.len()) > MAX_TYPED_JSON_BYTES {
            return Err(RiotApiError::DecodeFailed(
                "response exceeded Aura's 16 MiB safety limit".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<T>(&body)
        .map_err(|error| RiotApiError::DecodeFailed(error.to_string()))
}

fn safe_request_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else {
        "network transport failed".into()
    }
}

#[derive(Deserialize)]
struct AccountResponse {
    puuid: String,
    #[serde(default, rename = "gameName")]
    game_name: String,
    #[serde(default, rename = "tagLine")]
    tag_line: String,
}

#[derive(Serialize)]
pub struct RiotProfileTarget {
    pub puuid: String,
    pub game_name: String,
    pub tag_line: String,
    pub platform: String,
}

fn validate_puuid(value: &str) -> Result<String, RiotApiError> {
    let value = value.trim();
    if !(16..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(RiotApiError::RequestFailed("invalid Riot PUUID".into()));
    }
    Ok(value.to_string())
}

async fn cache_identity(state: &RiotApiState, puuid: String, platform: Platform) {
    let next_identity = Identity {
        puuid,
        region_group: platform.region_group(),
        platform,
    };
    let identity_changed = state.update_identity(Some(next_identity), false).await;
    if identity_changed {
        state.clear_match_cache().await;
    }
}

/// Resolves a Riot ID (name#tag) to a PUUID via Account-V1, and caches it —
/// everything else in this module reuses the cached identity rather than
/// re-resolving it on every call.
#[tauri::command]
pub async fn set_riot_id(
    state: tauri::State<'_, RiotApiState>,
    game_name: String,
    tag_line: String,
    platform: String,
) -> Result<(), String> {
    let plat = Platform::from_str(&platform).map_err(|e| e.to_string())?;

    // Account-V1 doesn't support "sea" — SEA platforms route through "asia"
    // for this one call specifically (see region_group_for_account_v1).
    let account_v1_region = plat.region_group_for_account_v1();
    let url = account_lookup_url(account_v1_region, &game_name, &tag_line)
        .map_err(|error| error.to_string())?;

    let json = get_json(state.inner(), "account lookup", &url)
        .await
        .map_err(|e| e.to_string())?;
    let account: AccountResponse = serde_json::from_value(json)
        .map_err(|e| RiotApiError::DecodeFailed(e.to_string()).to_string())?;

    // Match-V5 genuinely does support "sea" — stores the real mapping here,
    // not the account-v1-specific one used just above.
    cache_identity(state.inner(), account.puuid, plat).await;
    log_ok("Riot identity resolved and cached in volatile memory");
    Ok(())
}

/// Selects a profile from a PUUID returned by the local League Client or
/// Match-V5. Account-V1 supplies the canonical visible Riot ID while the
/// PUUID avoids ambiguity around renamed accounts.
#[tauri::command]
pub async fn select_riot_profile(
    state: tauri::State<'_, RiotApiState>,
    puuid: String,
    platform: String,
    fallback_game_name: String,
    fallback_tag_line: String,
) -> Result<RiotProfileTarget, String> {
    let platform_value = Platform::from_str(&platform).map_err(|error| error.to_string())?;
    let puuid = validate_puuid(&puuid).map_err(|error| error.to_string())?;
    cache_identity(state.inner(), puuid.clone(), platform_value).await;

    let account_url = account_by_puuid_url(platform_value.region_group_for_account_v1(), &puuid)
        .map_err(|error| error.to_string())?;
    let canonical = match get_json(state.inner(), "account lookup by PUUID", &account_url).await {
        Ok(json) => serde_json::from_value::<AccountResponse>(json)
            .map_err(|error| RiotApiError::DecodeFailed(error.to_string()).to_string())
            .ok(),
        Err(error) => {
            log_err(&format!("canonical Riot ID lookup failed: {error}"));
            None
        }
    };

    let game_name = canonical
        .as_ref()
        .map(|account| account.game_name.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_game_name.trim())
        .to_string();
    let tag_line = canonical
        .as_ref()
        .map(|account| account.tag_line.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_tag_line.trim())
        .to_string();

    if game_name.is_empty() || tag_line.is_empty() {
        return Err(
            "Riot ID is unavailable for this player; refresh the match and try again".into(),
        );
    }

    log_ok("Riot profile target selected in volatile memory");
    Ok(RiotProfileTarget {
        puuid,
        game_name,
        tag_line,
        platform: platform.to_ascii_lowercase(),
    })
}

fn account_lookup_url(
    region: RegionGroup,
    game_name: &str,
    tag_line: &str,
) -> Result<String, RiotApiError> {
    let mut url = Url::parse(&format!("https://{}/", region.host()))
        .map_err(|error| RiotApiError::RequestFailed(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| RiotApiError::RequestFailed("could not build account URL".into()))?
        .extend([
            "riot",
            "account",
            "v1",
            "accounts",
            "by-riot-id",
            game_name,
            tag_line,
        ]);
    Ok(url.into())
}

fn account_by_puuid_url(region: RegionGroup, puuid: &str) -> Result<String, RiotApiError> {
    let mut url = Url::parse(&format!("https://{}/", region.host()))
        .map_err(|error| RiotApiError::RequestFailed(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| RiotApiError::RequestFailed("could not build account URL".into()))?
        .extend(["riot", "account", "v1", "accounts", "by-puuid", puuid]);
    Ok(url.into())
}

#[derive(Deserialize)]
struct SummonerResponse {
    #[serde(rename = "profileIconId")]
    profile_icon_id: u32,
    #[serde(rename = "summonerLevel")]
    summoner_level: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RankedEntry {
    #[serde(rename(deserialize = "queueType"))]
    pub queue_type: String,
    pub tier: String,
    pub rank: String,
    #[serde(rename(deserialize = "leaguePoints"))]
    pub league_points: u32,
    pub wins: u32,
    pub losses: u32,
    #[serde(default, rename(deserialize = "hotStreak"))]
    pub hot_streak: bool,
    #[serde(default)]
    pub veteran: bool,
    #[serde(default, rename(deserialize = "freshBlood"))]
    pub fresh_blood: bool,
    #[serde(default, rename(deserialize = "miniSeries"))]
    pub mini_series: Option<MiniSeries>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MiniSeries {
    #[serde(default)]
    pub losses: u32,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub target: u32,
    #[serde(default)]
    pub wins: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChampionMastery {
    #[serde(default, rename(deserialize = "championId"))]
    pub champion_id: u32,
    #[serde(default, rename(deserialize = "championLevel"))]
    pub champion_level: u32,
    #[serde(default, rename(deserialize = "championPoints"))]
    pub champion_points: u64,
    #[serde(default, rename(deserialize = "lastPlayTime"))]
    pub last_play_time: i64,
    #[serde(default, rename(deserialize = "championPointsUntilNextLevel"))]
    pub champion_points_until_next_level: i64,
    #[serde(default, rename(deserialize = "championPointsSinceLastLevel"))]
    pub champion_points_since_last_level: i64,
    #[serde(default, rename(deserialize = "tokensEarned"))]
    pub tokens_earned: u32,
    #[serde(default, rename(deserialize = "chestGranted"))]
    pub chest_granted: bool,
}

#[derive(Serialize)]
pub struct SummonerProfile {
    pub profile_icon_id: u32,
    pub summoner_level: u64,
}

/// Loads the visible account card separately from match history so the
/// frontend can show profile details immediately instead of displaying one
/// long, ambiguous loading message.
#[tauri::command]
pub async fn get_summoner_profile(
    state: tauri::State<'_, RiotApiState>,
) -> Result<SummonerProfile, String> {
    let identity = state
        .identity
        .lock()
        .await
        .clone()
        .ok_or(RiotApiError::NotIdentified)
        .map_err(|error| error.to_string())?;

    let summoner_url = platform_url(
        identity.platform,
        &[
            "lol",
            "summoner",
            "v4",
            "summoners",
            "by-puuid",
            &identity.puuid,
        ],
    )?;
    let summoner_json = get_json(state.inner(), "summoner profile lookup", &summoner_url)
        .await
        .map_err(|error| error.to_string())?;
    let summoner: SummonerResponse = serde_json::from_value(summoner_json)
        .map_err(|error| RiotApiError::DecodeFailed(error.to_string()).to_string())?;

    Ok(SummonerProfile {
        profile_icon_id: summoner.profile_icon_id,
        summoner_level: summoner.summoner_level,
    })
}

/// Loads Solo/Duo and Flex rank independently from the visible Summoner-V4
/// profile. Concurrent callers share one fetch and subsequent calls reuse the
/// result from volatile RAM for two minutes.
#[tauri::command]
pub async fn get_league_entries(
    state: tauri::State<'_, RiotApiState>,
) -> Result<Vec<RankedEntry>, String> {
    let (identity, cached_entries) = {
        let selected = state.identity.lock().await;
        let identity = selected
            .clone()
            .ok_or(RiotApiError::NotIdentified)
            .map_err(|error| error.to_string())?;
        let entries = state
            .ranked_entries
            .lock()
            .await
            .as_ref()
            .and_then(|cache| cache.get(&identity));
        (identity, entries)
    };
    if let Some(entries) = cached_entries {
        log_ok("served ranked entries from volatile RAM cache");
        return Ok(entries);
    }

    let flight = state.ranked_entries_flight(&identity).await;
    let result = flight
        .get_or_init(|| fetch_league_entries(state.inner(), &identity))
        .await
        .clone();
    let mut active = state.ranked_entries_fetch.lock().await;
    if active
        .as_ref()
        .is_some_and(|(_, active_flight)| Arc::ptr_eq(active_flight, &flight))
    {
        *active = None;
    }
    result
}

async fn fetch_league_entries(
    state: &RiotApiState,
    identity: &Identity,
) -> Result<Vec<RankedEntry>, String> {
    if state.identity.lock().await.as_ref() != Some(identity) {
        return Err("Riot profile changed while ranked entries were loading; retry Profile".into());
    }
    let rank_url = ranked_entries_url(identity.platform, &identity.puuid)?;
    let rank_json = get_json(state, "ranked entries lookup", &rank_url)
        .await
        .map_err(|error| error.to_string())?;
    let entries: Vec<RankedEntry> = serde_json::from_value(rank_json)
        .map_err(|error| RiotApiError::DecodeFailed(error.to_string()).to_string())?;
    let entries = normalize_ranked_entries(entries);

    let selected = state.identity.lock().await;
    if selected.as_ref() != Some(identity) {
        return Err("Riot profile changed while ranked entries were loading; retry Profile".into());
    }
    *state.ranked_entries.lock().await = Some(RankedEntriesCache {
        identity: identity.clone(),
        created_at: Instant::now(),
        entries: entries.clone(),
    });
    drop(selected);
    log_ok("fetched and cached ranked entries in volatile RAM");
    Ok(entries)
}

fn normalize_ranked_entries(mut entries: Vec<RankedEntry>) -> Vec<RankedEntry> {
    entries.retain(|entry| {
        entry.queue_type == "RANKED_SOLO_5x5" || entry.queue_type == "RANKED_FLEX_SR"
    });
    entries
}

fn platform_url(platform: Platform, segments: &[&str]) -> Result<String, String> {
    let mut url = Url::parse(&format!("https://{}/", platform.host()))
        .map_err(|error| format!("could not build Riot platform URL: {error}"))?;
    url.path_segments_mut()
        .map_err(|_| "could not build Riot platform URL".to_string())?
        .extend(segments.iter().copied());
    Ok(url.into())
}

fn ranked_entries_url(platform: Platform, puuid: &str) -> Result<String, String> {
    platform_url(
        platform,
        &["lol", "league", "v4", "entries", "by-puuid", puuid],
    )
}

fn champion_masteries_url(platform: Platform, puuid: &str) -> Result<String, String> {
    platform_url(
        platform,
        &[
            "lol",
            "champion-mastery",
            "v4",
            "champion-masteries",
            "by-puuid",
            puuid,
        ],
    )
}

fn normalize_masteries(mut masteries: Vec<ChampionMastery>) -> Vec<ChampionMastery> {
    masteries.retain(|entry| entry.champion_id > 0);
    masteries.sort_by(|left, right| {
        right
            .champion_points
            .cmp(&left.champion_points)
            .then_with(|| left.champion_id.cmp(&right.champion_id))
    });

    // Riot should return one row per champion, but retain only the highest-point
    // row if a malformed or duplicated response ever reaches Aura. dedup_by_key
    // is insufficient here because point-first sorting does not make duplicate
    // champion IDs adjacent.
    let mut seen_champion_ids = HashSet::with_capacity(masteries.len());
    masteries.retain(|entry| seen_champion_ids.insert(entry.champion_id));
    masteries.truncate(200);
    masteries
}

/// Loads all mastery entries once per selected profile and keeps the compact
/// result in volatile RAM for two minutes. One list request is cheaper than a
/// network request whenever the user opens a different champion card.
#[tauri::command]
pub async fn get_champion_masteries(
    state: tauri::State<'_, RiotApiState>,
) -> Result<Vec<ChampionMastery>, String> {
    let identity = state
        .identity
        .lock()
        .await
        .clone()
        .ok_or(RiotApiError::NotIdentified)
        .map_err(|error| error.to_string())?;

    if let Some(cache) = state.champion_masteries.lock().await.as_ref() {
        if cache.puuid == identity.puuid && cache.created_at.elapsed() < MASTERY_CACHE_TTL {
            log_ok("served champion mastery from volatile RAM cache");
            return Ok(cache.masteries.clone());
        }
    }

    let url = champion_masteries_url(identity.platform, &identity.puuid)?;
    let mut masteries: Vec<ChampionMastery> =
        get_json_as(state.inner(), "champion mastery lookup", &url)
            .await
            .map_err(|error| error.to_string())?;
    if state.identity.lock().await.as_ref() != Some(&identity) {
        return Err("Riot profile changed while champion mastery was loading; retry".into());
    }

    masteries = normalize_masteries(masteries);
    *state.champion_masteries.lock().await = Some(ChampionMasteryCache {
        puuid: identity.puuid,
        created_at: Instant::now(),
        masteries: masteries.clone(),
    });
    log_ok("fetched and cached champion mastery in volatile RAM");
    Ok(masteries)
}

#[derive(Deserialize)]
struct MatchResponse {
    info: MatchInfo,
}

#[derive(Default, Deserialize)]
struct MatchInfo {
    #[serde(rename = "gameCreation", default)]
    game_creation: i64,
    #[serde(rename = "gameStartTimestamp", default)]
    game_start_timestamp: i64,
    #[serde(rename = "gameEndTimestamp", default)]
    game_end_timestamp: i64,
    #[serde(rename = "gameDuration", default)]
    game_duration: i64,
    #[serde(rename = "gameMode", default)]
    game_mode: String,
    #[serde(rename = "gameType", default)]
    game_type: String,
    #[serde(rename = "gameVersion", default)]
    game_version: String,
    #[serde(rename = "platformId", default)]
    platform_id: String,
    #[serde(rename = "mapId", default)]
    map_id: u32,
    #[serde(rename = "queueId", default)]
    queue_id: u32,
    #[serde(default)]
    participants: Vec<ParticipantInfo>,
    #[serde(default)]
    teams: Vec<RawTeamInfo>,
}

#[derive(Default, Deserialize)]
struct ParticipantInfo {
    #[serde(rename = "participantId", default)]
    participant_id: u8,
    #[serde(rename = "teamId", default)]
    team_id: u16,
    #[serde(default)]
    puuid: String,
    #[serde(rename = "riotIdGameName", default)]
    riot_id_game_name: String,
    #[serde(rename = "riotIdTagline", default)]
    riot_id_tag_line: String,
    #[serde(rename = "summonerName", default)]
    summoner_name: String,
    #[serde(rename = "profileIcon", default)]
    profile_icon_id: u32,
    #[serde(rename = "championId", default)]
    champion_id: u32,
    #[serde(rename = "championName", default)]
    champion_name: String,
    #[serde(rename = "champLevel", default)]
    champion_level: u32,
    #[serde(rename = "teamPosition", default)]
    team_position: String,
    #[serde(rename = "individualPosition", default)]
    individual_position: String,
    #[serde(default)]
    lane: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    kills: u32,
    #[serde(default)]
    deaths: u32,
    #[serde(default)]
    assists: u32,
    #[serde(rename = "doubleKills", default)]
    double_kills: u32,
    #[serde(rename = "tripleKills", default)]
    triple_kills: u32,
    #[serde(rename = "quadraKills", default)]
    quadra_kills: u32,
    #[serde(rename = "pentaKills", default)]
    penta_kills: u32,
    #[serde(rename = "largestMultiKill", default)]
    largest_multi_kill: u32,
    #[serde(rename = "largestKillingSpree", default)]
    largest_killing_spree: u32,
    #[serde(default)]
    win: bool,
    #[serde(rename = "totalMinionsKilled", default)]
    total_minions: u32,
    #[serde(rename = "neutralMinionsKilled", default)]
    neutral_minions: u32,
    #[serde(rename = "goldEarned", default)]
    gold_earned: u32,
    #[serde(rename = "goldSpent", default)]
    gold_spent: u32,
    #[serde(rename = "visionScore", default)]
    vision_score: u32,
    #[serde(rename = "wardsPlaced", default)]
    wards_placed: u32,
    #[serde(rename = "wardsKilled", default)]
    wards_killed: u32,
    #[serde(rename = "detectorWardsPlaced", default)]
    control_wards: u32,
    #[serde(rename = "visionWardsBoughtInGame", default)]
    vision_wards_bought: u32,
    #[serde(rename = "totalDamageDealt", default)]
    total_damage_dealt: u64,
    #[serde(rename = "totalDamageDealtToChampions", default)]
    total_damage_dealt_to_champions: u64,
    #[serde(rename = "totalDamageTaken", default)]
    total_damage_taken: u64,
    #[serde(rename = "damageSelfMitigated", default)]
    damage_self_mitigated: u64,
    #[serde(rename = "damageDealtToObjectives", default)]
    damage_dealt_to_objectives: u64,
    #[serde(rename = "damageDealtToTurrets", default)]
    damage_dealt_to_turrets: u64,
    #[serde(rename = "totalHeal", default)]
    total_healing: u64,
    #[serde(rename = "totalHealsOnTeammates", default)]
    healing_on_teammates: u64,
    #[serde(rename = "totalDamageShieldedOnTeammates", default)]
    shielding_on_teammates: u64,
    #[serde(rename = "timeCCingOthers", default)]
    time_ccing_others_secs: u64,
    #[serde(rename = "totalTimeSpentDead", default)]
    time_spent_dead_secs: u64,
    #[serde(rename = "turretTakedowns", default)]
    turret_takedowns: u32,
    #[serde(rename = "inhibitorTakedowns", default)]
    inhibitor_takedowns: u32,
    #[serde(rename = "objectivesStolen", default)]
    objectives_stolen: u32,
    #[serde(rename = "firstBloodKill", default)]
    first_blood_kill: bool,
    #[serde(rename = "firstBloodAssist", default)]
    first_blood_assist: bool,
    #[serde(rename = "summoner1Id", default)]
    summoner_spell_1_id: u32,
    #[serde(rename = "summoner2Id", default)]
    summoner_spell_2_id: u32,
    #[serde(default)]
    item0: u32,
    #[serde(default)]
    item1: u32,
    #[serde(default)]
    item2: u32,
    #[serde(default)]
    item3: u32,
    #[serde(default)]
    item4: u32,
    #[serde(default)]
    item5: u32,
    #[serde(default)]
    item6: u32,
    #[serde(default)]
    perks: ParticipantPerks,
}

#[derive(Default, Deserialize)]
struct ParticipantPerks {
    #[serde(default)]
    styles: Vec<ParticipantPerkStyle>,
    #[serde(rename = "statPerks", default)]
    stat_perks: ParticipantStatPerks,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ParticipantStatPerks {
    #[serde(default)]
    pub defense: u32,
    #[serde(default)]
    pub flex: u32,
    #[serde(default)]
    pub offense: u32,
}

#[derive(Default, Deserialize)]
struct ParticipantPerkStyle {
    #[serde(default)]
    description: String,
    #[serde(default)]
    style: u32,
    #[serde(default)]
    selections: Vec<ParticipantPerkSelection>,
}

#[derive(Default, Deserialize)]
struct ParticipantPerkSelection {
    #[serde(default)]
    perk: u32,
    #[serde(default)]
    var1: i64,
    #[serde(default)]
    var2: i64,
    #[serde(default)]
    var3: i64,
}

#[derive(Default, Deserialize)]
struct RawTeamInfo {
    #[serde(rename = "teamId", default)]
    team_id: u16,
    #[serde(default)]
    win: bool,
    #[serde(default)]
    bans: Vec<RawBanInfo>,
    #[serde(default)]
    objectives: RawTeamObjectives,
}

#[derive(Default, Deserialize)]
struct RawBanInfo {
    #[serde(rename = "championId", default)]
    champion_id: i32,
    #[serde(rename = "pickTurn", default)]
    pick_turn: u32,
}

#[derive(Default, Deserialize)]
struct RawTeamObjectives {
    #[serde(default)]
    baron: RawObjectiveInfo,
    #[serde(default)]
    champion: RawObjectiveInfo,
    #[serde(default)]
    dragon: RawObjectiveInfo,
    #[serde(default)]
    horde: RawObjectiveInfo,
    #[serde(default)]
    inhibitor: RawObjectiveInfo,
    #[serde(rename = "riftHerald", default)]
    rift_herald: RawObjectiveInfo,
    #[serde(default)]
    tower: RawObjectiveInfo,
    #[serde(default)]
    atakhan: RawObjectiveInfo,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ObjectiveInfo {
    #[serde(default)]
    pub first: bool,
    #[serde(default)]
    pub kills: u32,
}

type RawObjectiveInfo = ObjectiveInfo;

#[derive(Clone, Serialize)]
pub struct PerkSelectionDetail {
    pub perk: u32,
    pub var1: i64,
    pub var2: i64,
    pub var3: i64,
}

#[derive(Clone, Serialize)]
pub struct PerkStyleDetail {
    pub description: String,
    pub style: u32,
    pub selections: Vec<PerkSelectionDetail>,
}

#[derive(Clone, Serialize)]
pub struct MatchParticipantDetail {
    pub is_me: bool,
    pub puuid: String,
    pub participant_id: u8,
    pub team_id: u16,
    pub riot_id: String,
    pub game_name: String,
    pub tag_line: String,
    pub summoner_name: String,
    pub profile_icon_id: u32,
    pub champion_id: u32,
    pub champion_name: String,
    pub champion_level: u32,
    pub team_position: String,
    pub individual_position: String,
    pub lane: String,
    pub role: String,
    pub win: bool,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub kda: f64,
    pub cs: u32,
    pub csm: f64,
    /// Percentage in the inclusive 0..=100 range.
    pub kill_participation: f64,
    pub team_kills: u32,
    /// Percentage in the inclusive 0..=100 range.
    pub damage_share: f64,
    /// Percentage in the inclusive 0..=100 range.
    pub gold_share: f64,
    pub gold: u32,
    pub gold_spent: u32,
    pub vision_score: u32,
    pub wards_placed: u32,
    pub wards_killed: u32,
    pub control_wards: u32,
    pub vision_wards_bought: u32,
    pub total_damage_dealt: u64,
    pub total_damage_dealt_to_champions: u64,
    pub total_damage_taken: u64,
    pub damage_self_mitigated: u64,
    pub damage_dealt_to_objectives: u64,
    pub damage_dealt_to_turrets: u64,
    pub total_healing: u64,
    pub healing_on_teammates: u64,
    pub shielding_on_teammates: u64,
    pub time_ccing_others_secs: u64,
    pub time_spent_dead_secs: u64,
    pub turret_takedowns: u32,
    pub inhibitor_takedowns: u32,
    pub objectives_stolen: u32,
    pub first_blood_kill: bool,
    pub first_blood_assist: bool,
    pub double_kills: u32,
    pub triple_kills: u32,
    pub quadra_kills: u32,
    pub penta_kills: u32,
    pub largest_multi_kill: u32,
    pub largest_killing_spree: u32,
    pub items: Vec<u32>,
    pub summoner_spell_ids: Vec<u32>,
    pub perk_ids: Vec<u32>,
    pub perk_styles: Vec<PerkStyleDetail>,
    pub stat_perks: ParticipantStatPerks,
}

#[derive(Clone, Serialize)]
pub struct MatchBanDetail {
    pub champion_id: i32,
    pub pick_turn: u32,
}

#[derive(Clone, Default, Serialize)]
pub struct MatchTeamObjectives {
    pub baron: ObjectiveInfo,
    pub champion: ObjectiveInfo,
    pub dragon: ObjectiveInfo,
    pub horde: ObjectiveInfo,
    pub inhibitor: ObjectiveInfo,
    pub rift_herald: ObjectiveInfo,
    pub tower: ObjectiveInfo,
    pub atakhan: ObjectiveInfo,
}

#[derive(Clone, Serialize)]
pub struct MatchTeamDetail {
    pub team_id: u16,
    pub win: bool,
    pub kills: u32,
    pub gold: u64,
    pub damage_to_champions: u64,
    pub bans: Vec<MatchBanDetail>,
    pub objectives: MatchTeamObjectives,
}

#[derive(Clone, Serialize)]
pub struct MatchDetail {
    pub match_id: String,
    pub game_creation_ms: i64,
    pub game_start_ms: i64,
    pub game_end_ms: i64,
    pub game_duration_secs: i64,
    pub game_duration_minutes: f64,
    pub game_mode: String,
    pub game_type: String,
    pub game_version: String,
    pub platform_id: String,
    pub map_id: u32,
    pub queue_id: u32,
    pub player: MatchParticipantDetail,
    pub participants: Vec<MatchParticipantDetail>,
    pub teams: Vec<MatchTeamDetail>,
}

#[derive(Serialize, Clone)]
pub struct MatchSummary {
    pub match_id: String,
    pub champion_name: String,
    pub champion_id: u32,
    pub team_id: u16,
    pub team_position: String,
    pub win: bool,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub kda: f64,
    pub cs: u32,
    pub csm: f64,
    pub kill_participation: f64,
    pub gold: u32,
    pub vision_score: u32,
    pub wards_placed: u32,
    pub wards_killed: u32,
    pub control_wards: u32,
    pub total_damage_dealt_to_champions: u64,
    pub total_damage_taken: u64,
    pub damage_dealt_to_objectives: u64,
    pub items: Vec<u32>,
    pub summoner_spell_ids: Vec<u32>,
    pub perk_ids: Vec<u32>,
    pub game_mode: String,
    pub queue_id: u32,
    pub game_duration_secs: i64,
    pub game_creation_ms: i64,
}

/// Fetches match ids for the resolved identity, then match details for each —
/// intentionally on-demand (dashboard load / manual refresh), not a polling
/// loop. Match history doesn't change except when a game just finished, and
/// this shouldn't compete with anything during an active match anyway.
#[tauri::command]
pub async fn fetch_recent_matches(
    state: tauri::State<'_, RiotApiState>,
    count: u32,
) -> Result<Vec<MatchSummary>, String> {
    let identity = state
        .identity
        .lock()
        .await
        .clone()
        .ok_or(RiotApiError::NotIdentified)
        .map_err(|e| e.to_string())?;

    let requested_count = count.min(20);
    if requested_count == 0 {
        return Ok(Vec::new());
    }

    {
        let cache = state.recent_matches.lock().await;
        if let Some(cache) = cache.as_ref() {
            if cache.puuid == identity.puuid
                && cache.requested_count >= requested_count
                && cache.created_at.elapsed() <= MATCH_CACHE_TTL
            {
                let take = requested_count.min(cache.summaries.len() as u32) as usize;
                log_ok("served recent match summaries from volatile RAM cache");
                return Ok(cache.summaries[..take].to_vec());
            }
        }
    }

    let ids_url = match_ids_url(identity.region_group, &identity.puuid, requested_count)
        .map_err(|error| error.to_string())?;
    let ids_json = get_json(state.inner(), "match-list lookup", &ids_url)
        .await
        .map_err(|e| e.to_string())?;
    let match_ids: Vec<String> = serde_json::from_value(ids_json).map_err(|e| e.to_string())?;

    // Four bounded concurrent requests keep ten-match history comfortably
    // below the UI deadline without creating a thread/request burst.
    let request_state = state.inner().clone();
    let puuid = identity.puuid.clone();
    let region_group = identity.region_group;
    let mut indexed_matches: Vec<(usize, Option<(MatchSummary, MatchDetail)>)> =
        stream::iter(match_ids.into_iter().enumerate())
            .map(|(index, match_id)| {
                let request_state = request_state.clone();
                let puuid = puuid.clone();
                async move {
                    let parsed_match = match match_detail_url(region_group, &match_id) {
                        Ok(match_url) => {
                            match get_json(&request_state, "match-detail lookup", &match_url).await
                            {
                                Ok(json) => match serde_json::from_value::<MatchResponse>(json) {
                                    Ok(parsed) => {
                                        build_match_payload(match_id, parsed.info, &puuid)
                                    }
                                    Err(error) => {
                                        log_err(&format!("match response decode failed: {error}"));
                                        None
                                    }
                                },
                                Err(error) => {
                                    log_err(&format!("match-detail lookup failed: {error}"));
                                    None
                                }
                            }
                        }
                        Err(error) => {
                            log_err(&format!("match URL build failed: {error}"));
                            None
                        }
                    };
                    (index, parsed_match)
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;
    indexed_matches.sort_by_key(|(index, _)| *index);

    let mut summaries = Vec::with_capacity(indexed_matches.len());
    let mut details = HashMap::with_capacity(indexed_matches.len());
    for (_, parsed_match) in indexed_matches {
        if let Some((summary, detail)) = parsed_match {
            details.insert(summary.match_id.clone(), detail);
            summaries.push(summary);
        }
    }

    let detail_ids = details.keys().cloned().collect::<HashSet<_>>();
    {
        let _timeline_guard = state.match_timeline_fetch.lock().await;
        if state.identity.lock().await.as_ref() != Some(&identity) {
            return Err(
                "Riot profile changed while recent matches were loading; retry Profile".into(),
            );
        }
        *state.match_details.lock().await = details;
        state.match_timelines.lock().await.retain_ids(&detail_ids);
        *state.recent_matches.lock().await = Some(RecentMatchCache {
            puuid: identity.puuid,
            requested_count,
            created_at: Instant::now(),
            summaries: summaries.clone(),
        });
    }

    log_ok(&format!("fetched {} match summaries", summaries.len()));
    Ok(summaries)
}

/// Returns an immutable Match-V5 snapshot already fetched for the Profile page.
/// This command never performs network I/O, so expanding a match is instant and
/// cannot consume additional Riot rate-limit budget.
#[tauri::command]
pub async fn get_match_detail(
    state: tauri::State<'_, RiotApiState>,
    match_id: String,
) -> Result<MatchDetail, String> {
    let match_id = match_id.trim();
    if match_id.is_empty() || match_id.len() > 96 {
        return Err("invalid match id".into());
    }

    state
        .match_details
        .lock()
        .await
        .get(match_id)
        .cloned()
        .ok_or_else(|| "match detail is not in the current RAM cache; refresh Profile".into())
}

#[derive(Clone, Copy, Default)]
struct TeamTotals {
    kills: u32,
    gold: u64,
    damage_to_champions: u64,
}

fn build_match_payload(
    match_id: String,
    info: MatchInfo,
    player_puuid: &str,
) -> Option<(MatchSummary, MatchDetail)> {
    let duration_secs = normalized_game_duration_secs(&info);
    let duration_minutes = duration_secs.max(0) as f64 / 60.0;

    let mut team_totals: HashMap<u16, TeamTotals> = HashMap::new();
    for participant in &info.participants {
        let totals = team_totals.entry(participant.team_id).or_default();
        totals.kills = totals.kills.saturating_add(participant.kills);
        totals.gold = totals.gold.saturating_add(participant.gold_earned as u64);
        totals.damage_to_champions = totals
            .damage_to_champions
            .saturating_add(participant.total_damage_dealt_to_champions);
    }

    let participants: Vec<MatchParticipantDetail> = info
        .participants
        .into_iter()
        .map(|participant| {
            let totals = team_totals
                .get(&participant.team_id)
                .copied()
                .unwrap_or_default();
            build_participant_detail(participant, player_puuid, duration_minutes, totals)
        })
        .collect();
    let player = participants
        .iter()
        .find(|participant| participant.is_me)
        .cloned()?;

    let mut teams: Vec<MatchTeamDetail> = info
        .teams
        .into_iter()
        .map(|team| {
            let totals = team_totals.get(&team.team_id).copied().unwrap_or_default();
            MatchTeamDetail {
                team_id: team.team_id,
                win: team.win,
                kills: totals.kills,
                gold: totals.gold,
                damage_to_champions: totals.damage_to_champions,
                bans: team
                    .bans
                    .into_iter()
                    .map(|ban| MatchBanDetail {
                        champion_id: ban.champion_id,
                        pick_turn: ban.pick_turn,
                    })
                    .collect(),
                objectives: MatchTeamObjectives {
                    baron: team.objectives.baron,
                    champion: team.objectives.champion,
                    dragon: team.objectives.dragon,
                    horde: team.objectives.horde,
                    inhibitor: team.objectives.inhibitor,
                    rift_herald: team.objectives.rift_herald,
                    tower: team.objectives.tower,
                    atakhan: team.objectives.atakhan,
                },
            }
        })
        .collect();

    // Mode-specific responses can omit the team block. Preserve participant
    // stats and synthesize zero-objective team rows instead of dropping the
    // whole match.
    for (team_id, totals) in team_totals {
        if teams.iter().any(|team| team.team_id == team_id) {
            continue;
        }
        let win = participants
            .iter()
            .find(|participant| participant.team_id == team_id)
            .is_some_and(|participant| participant.win);
        teams.push(MatchTeamDetail {
            team_id,
            win,
            kills: totals.kills,
            gold: totals.gold,
            damage_to_champions: totals.damage_to_champions,
            bans: Vec::new(),
            objectives: MatchTeamObjectives::default(),
        });
    }
    teams.sort_by_key(|team| team.team_id);

    let summary = MatchSummary {
        match_id: match_id.clone(),
        champion_name: player.champion_name.clone(),
        champion_id: player.champion_id,
        team_id: player.team_id,
        team_position: player.team_position.clone(),
        win: player.win,
        kills: player.kills,
        deaths: player.deaths,
        assists: player.assists,
        kda: player.kda,
        cs: player.cs,
        csm: player.csm,
        kill_participation: player.kill_participation,
        gold: player.gold,
        vision_score: player.vision_score,
        wards_placed: player.wards_placed,
        wards_killed: player.wards_killed,
        control_wards: player.control_wards,
        total_damage_dealt_to_champions: player.total_damage_dealt_to_champions,
        total_damage_taken: player.total_damage_taken,
        damage_dealt_to_objectives: player.damage_dealt_to_objectives,
        items: player.items.clone(),
        summoner_spell_ids: player.summoner_spell_ids.clone(),
        perk_ids: player.perk_ids.clone(),
        game_mode: info.game_mode.clone(),
        queue_id: info.queue_id,
        game_duration_secs: duration_secs,
        game_creation_ms: info.game_creation,
    };
    let detail = MatchDetail {
        match_id,
        game_creation_ms: info.game_creation,
        game_start_ms: info.game_start_timestamp,
        game_end_ms: info.game_end_timestamp,
        game_duration_secs: duration_secs,
        game_duration_minutes: round_to(duration_minutes, 2),
        game_mode: info.game_mode,
        game_type: info.game_type,
        game_version: info.game_version,
        platform_id: info.platform_id,
        map_id: info.map_id,
        queue_id: info.queue_id,
        player,
        participants,
        teams,
    };
    Some((summary, detail))
}

fn build_participant_detail(
    participant: ParticipantInfo,
    player_puuid: &str,
    duration_minutes: f64,
    team_totals: TeamTotals,
) -> MatchParticipantDetail {
    let cs = participant
        .total_minions
        .saturating_add(participant.neutral_minions);
    let csm = if duration_minutes > 0.0 {
        cs as f64 / duration_minutes
    } else {
        0.0
    };
    let kda = if participant.deaths == 0 {
        participant.kills.saturating_add(participant.assists) as f64
    } else {
        participant.kills.saturating_add(participant.assists) as f64 / participant.deaths as f64
    };
    let kill_participation = percentage(
        participant.kills.saturating_add(participant.assists) as u64,
        team_totals.kills as u64,
    );
    let damage_share = percentage(
        participant.total_damage_dealt_to_champions,
        team_totals.damage_to_champions,
    );
    let gold_share = percentage(participant.gold_earned as u64, team_totals.gold);
    let riot_id = if participant.riot_id_game_name.is_empty() {
        participant.summoner_name.clone()
    } else if participant.riot_id_tag_line.is_empty() {
        participant.riot_id_game_name.clone()
    } else {
        format!(
            "{}#{}",
            participant.riot_id_game_name, participant.riot_id_tag_line
        )
    };
    let items = [
        participant.item0,
        participant.item1,
        participant.item2,
        participant.item3,
        participant.item4,
        participant.item5,
        participant.item6,
    ]
    .into_iter()
    .filter(|item| *item != 0)
    .collect();
    let summoner_spell_ids = [
        participant.summoner_spell_1_id,
        participant.summoner_spell_2_id,
    ]
    .into_iter()
    .filter(|spell| *spell != 0)
    .collect();
    let perk_styles: Vec<PerkStyleDetail> = participant
        .perks
        .styles
        .into_iter()
        .map(|style| PerkStyleDetail {
            description: style.description,
            style: style.style,
            selections: style
                .selections
                .into_iter()
                .map(|selection| PerkSelectionDetail {
                    perk: selection.perk,
                    var1: selection.var1,
                    var2: selection.var2,
                    var3: selection.var3,
                })
                .collect(),
        })
        .collect();
    let perk_ids = perk_styles
        .iter()
        .flat_map(|style| style.selections.iter())
        .map(|selection| selection.perk)
        .filter(|perk| *perk != 0)
        .collect();

    MatchParticipantDetail {
        is_me: participant.puuid == player_puuid,
        puuid: participant.puuid.clone(),
        participant_id: participant.participant_id,
        team_id: participant.team_id,
        riot_id,
        game_name: participant.riot_id_game_name,
        tag_line: participant.riot_id_tag_line,
        summoner_name: participant.summoner_name,
        profile_icon_id: participant.profile_icon_id,
        champion_id: participant.champion_id,
        champion_name: participant.champion_name,
        champion_level: participant.champion_level,
        team_position: participant.team_position,
        individual_position: participant.individual_position,
        lane: participant.lane,
        role: participant.role,
        win: participant.win,
        kills: participant.kills,
        deaths: participant.deaths,
        assists: participant.assists,
        kda: round_to(kda, 2),
        cs,
        csm: round_to(csm, 2),
        kill_participation,
        team_kills: team_totals.kills,
        damage_share,
        gold_share,
        gold: participant.gold_earned,
        gold_spent: participant.gold_spent,
        vision_score: participant.vision_score,
        wards_placed: participant.wards_placed,
        wards_killed: participant.wards_killed,
        control_wards: participant.control_wards,
        vision_wards_bought: participant.vision_wards_bought,
        total_damage_dealt: participant.total_damage_dealt,
        total_damage_dealt_to_champions: participant.total_damage_dealt_to_champions,
        total_damage_taken: participant.total_damage_taken,
        damage_self_mitigated: participant.damage_self_mitigated,
        damage_dealt_to_objectives: participant.damage_dealt_to_objectives,
        damage_dealt_to_turrets: participant.damage_dealt_to_turrets,
        total_healing: participant.total_healing,
        healing_on_teammates: participant.healing_on_teammates,
        shielding_on_teammates: participant.shielding_on_teammates,
        time_ccing_others_secs: participant.time_ccing_others_secs,
        time_spent_dead_secs: participant.time_spent_dead_secs,
        turret_takedowns: participant.turret_takedowns,
        inhibitor_takedowns: participant.inhibitor_takedowns,
        objectives_stolen: participant.objectives_stolen,
        first_blood_kill: participant.first_blood_kill,
        first_blood_assist: participant.first_blood_assist,
        double_kills: participant.double_kills,
        triple_kills: participant.triple_kills,
        quadra_kills: participant.quadra_kills,
        penta_kills: participant.penta_kills,
        largest_multi_kill: participant.largest_multi_kill,
        largest_killing_spree: participant.largest_killing_spree,
        items,
        summoner_spell_ids,
        perk_ids,
        perk_styles,
        stat_perks: participant.perks.stat_perks,
    }
}

fn normalized_game_duration_secs(info: &MatchInfo) -> i64 {
    if info.game_duration > 86_400 {
        // Match-V5 responses before patch 11.20 represented this value in
        // milliseconds. No valid League match lasts longer than one day.
        info.game_duration / 1_000
    } else if info.game_duration > 0 {
        info.game_duration
    } else if info.game_end_timestamp > info.game_start_timestamp {
        (info.game_end_timestamp - info.game_start_timestamp) / 1_000
    } else {
        0
    }
}

fn percentage(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        round_to(
            (numerator as f64 / denominator as f64 * 100.0).clamp(0.0, 100.0),
            2,
        )
    }
}

fn round_to(value: f64, decimals: i32) -> f64 {
    let scale = 10_f64.powi(decimals);
    (value * scale).round() / scale
}

fn match_ids_url(region: RegionGroup, puuid: &str, count: u32) -> Result<String, RiotApiError> {
    let mut url = Url::parse(&format!("https://{}/", region.host()))
        .map_err(|error| RiotApiError::RequestFailed(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| RiotApiError::RequestFailed("could not build match-list URL".into()))?
        .extend(["lol", "match", "v5", "matches", "by-puuid", puuid, "ids"]);
    url.query_pairs_mut()
        .append_pair("start", "0")
        .append_pair("count", &count.to_string());
    Ok(url.into())
}

fn match_detail_url(region: RegionGroup, match_id: &str) -> Result<String, RiotApiError> {
    let mut url = Url::parse(&format!("https://{}/", region.host()))
        .map_err(|error| RiotApiError::RequestFailed(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| RiotApiError::RequestFailed("could not build match-detail URL".into()))?
        .extend(["lol", "match", "v5", "matches", match_id]);
    Ok(url.into())
}

fn match_timeline_url(region: RegionGroup, match_id: &str) -> Result<String, RiotApiError> {
    let mut url = Url::parse(&format!("https://{}/", region.host()))
        .map_err(|error| RiotApiError::RequestFailed(error.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| RiotApiError::RequestFailed("could not build match-timeline URL".into()))?
        .extend(["lol", "match", "v5", "matches", match_id, "timeline"]);
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity(puuid: &str, platform: Platform) -> Identity {
        Identity {
            puuid: puuid.into(),
            region_group: platform.region_group(),
            platform,
        }
    }

    fn test_ranked_entry(queue_type: &str) -> RankedEntry {
        RankedEntry {
            queue_type: queue_type.into(),
            tier: "GOLD".into(),
            rank: "II".into(),
            league_points: 72,
            wins: 18,
            losses: 12,
            hot_streak: false,
            veteran: false,
            fresh_blood: false,
            mini_series: None,
        }
    }

    #[test]
    fn sea_uses_different_account_and_match_routes() {
        let platform = Platform::from_str("sg2").expect("SG2 should be supported");
        let oce = Platform::from_str("oc1").expect("OCE should be supported");

        assert_eq!(platform.region_group_for_account_v1(), RegionGroup::Asia);
        assert_eq!(platform.region_group(), RegionGroup::Sea);
        assert_eq!(oce.region_group_for_account_v1(), RegionGroup::Asia);
        assert_eq!(oce.region_group(), RegionGroup::Sea);
    }

    #[test]
    fn account_url_percent_encodes_riot_id_segments() {
        let url = account_lookup_url(RegionGroup::Europe, "Name / Space", "EU#1")
            .expect("account URL should build");

        assert!(url.contains("Name%20%2F%20Space"));
        assert!(url.contains("EU%231"));
        assert!(!url.contains("Name / Space"));
    }

    #[test]
    fn profile_target_puuid_is_validated_and_encoded() {
        let puuid = "player-puuid_00000000000000000001";
        assert_eq!(validate_puuid(puuid).unwrap(), puuid);
        assert!(validate_puuid("short").is_err());
        assert!(validate_puuid("invalid puuid with spaces").is_err());

        let url = account_by_puuid_url(RegionGroup::Europe, puuid)
            .expect("PUUID account URL should build");
        assert!(url.ends_with(puuid));
        assert!(url.contains("/accounts/by-puuid/"));
    }

    #[test]
    fn match_count_is_encoded_as_a_query_value() {
        let url = match_ids_url(RegionGroup::Americas, "test-puuid", 7)
            .expect("match-list URL should build");

        assert!(url.ends_with("/ids?start=0&count=7"));
    }

    #[test]
    fn current_summoner_response_does_not_require_legacy_summoner_id() {
        let response: SummonerResponse = serde_json::from_value(serde_json::json!({
            "puuid": "test-puuid",
            "profileIconId": 29,
            "summonerLevel": 321,
            "revisionDate": 1_725_000_000_000_u64
        }))
        .expect("current SUMMONER-V4 response should decode without an id field");

        assert_eq!(response.profile_icon_id, 29);
        assert_eq!(response.summoner_level, 321);
    }

    #[test]
    fn ranked_entries_use_puuid_route() {
        let url = ranked_entries_url(Platform::Eun1, "encrypted-puuid")
            .expect("LEAGUE-V4 rank URL should build");

        assert_eq!(
            url,
            "https://eun1.api.riotgames.com/lol/league/v4/entries/by-puuid/encrypted-puuid"
        );
    }

    #[test]
    fn summoner_profile_contract_is_independent_from_ranked_entries() {
        let profile = SummonerProfile {
            profile_icon_id: 29,
            summoner_level: 321,
        };
        let json = serde_json::to_value(profile).expect("summoner profile should serialize");

        assert_eq!(json["profile_icon_id"], 29);
        assert_eq!(json["summoner_level"], 321);
        assert!(json.get("ranked_entries").is_none());
    }

    #[test]
    fn ranked_entries_filter_keeps_only_supported_profile_queues() {
        let entries = normalize_ranked_entries(vec![
            test_ranked_entry("RANKED_FLEX_SR"),
            test_ranked_entry("CHERRY"),
            test_ranked_entry("RANKED_SOLO_5x5"),
        ]);

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.queue_type.as_str())
                .collect::<Vec<_>>(),
            vec!["RANKED_FLEX_SR", "RANKED_SOLO_5x5"]
        );
    }

    #[test]
    fn ranked_entries_cache_is_scoped_to_identity_and_ttl() {
        let selected = test_identity("selected-puuid", Platform::Eun1);
        let other = test_identity("other-puuid", Platform::Eun1);
        let mut cache = RankedEntriesCache {
            identity: selected.clone(),
            created_at: Instant::now(),
            entries: vec![test_ranked_entry("RANKED_SOLO_5x5")],
        };

        assert_eq!(cache.get(&selected).expect("fresh cache").len(), 1);
        assert!(cache.get(&other).is_none());
        cache.created_at = Instant::now()
            .checked_sub(RANKED_ENTRIES_CACHE_TTL + Duration::from_millis(1))
            .expect("test instant should support a short subtraction");
        assert!(cache.get(&selected).is_none());
    }

    #[tokio::test]
    async fn ranked_in_flight_request_is_shared_and_invalidated_with_identity() {
        let state = RiotApiState::new(Some("test-key".into()));
        let first = test_identity("first-puuid", Platform::Eun1);
        let second = test_identity("second-puuid", Platform::Euw1);
        assert!(state.update_identity(Some(first.clone()), false).await);
        *state.ranked_entries.lock().await = Some(RankedEntriesCache {
            identity: first.clone(),
            created_at: Instant::now(),
            entries: vec![test_ranked_entry("RANKED_SOLO_5x5")],
        });

        let first_flight = state.ranked_entries_flight(&first).await;
        let shared_flight = state.ranked_entries_flight(&first).await;
        assert!(Arc::ptr_eq(&first_flight, &shared_flight));
        assert!(!state.update_identity(Some(first.clone()), false).await);
        assert!(state.ranked_entries.lock().await.is_some());
        assert!(state.ranked_entries_fetch.lock().await.is_some());

        assert!(state.update_identity(Some(second.clone()), false).await);
        assert_eq!(state.identity.lock().await.as_ref(), Some(&second));
        assert!(state.ranked_entries.lock().await.is_none());
        assert!(state.ranked_entries_fetch.lock().await.is_none());
    }

    #[tokio::test]
    async fn api_key_changes_clear_identity_rank_cache_and_in_flight_request() {
        let state = RiotApiState::new(Some("old-key".into()));
        let identity = test_identity("selected-puuid", Platform::Eun1);
        assert!(state.update_identity(Some(identity.clone()), false).await);
        *state.ranked_entries.lock().await = Some(RankedEntriesCache {
            identity: identity.clone(),
            created_at: Instant::now(),
            entries: vec![test_ranked_entry("RANKED_SOLO_5x5")],
        });
        let _flight = state.ranked_entries_flight(&identity).await;

        state.set_api_key(Some("new-key".into())).await;

        assert!(state.identity.lock().await.is_none());
        assert!(state.ranked_entries.lock().await.is_none());
        assert!(state.ranked_entries_fetch.lock().await.is_none());
        assert_eq!(state.api_key.lock().await.as_deref(), Some("new-key"));
    }

    #[test]
    fn champion_masteries_use_eun1_puuid_route() {
        let url = champion_masteries_url(Platform::Eun1, "encrypted-puuid")
            .expect("CHAMPION-MASTERY-V4 URL should build");

        assert_eq!(
            url,
            "https://eun1.api.riotgames.com/lol/champion-mastery/v4/champion-masteries/by-puuid/encrypted-puuid"
        );
    }

    #[test]
    fn current_mastery_response_decodes_without_legacy_fields() {
        let mastery: ChampionMastery = serde_json::from_value(serde_json::json!({
            "championId": 103,
            "championLevel": 7,
            "championPoints": 345_678,
            "lastPlayTime": 1_765_000_000_000_i64,
            "championPointsUntilNextLevel": 0,
            "championPointsSinceLastLevel": 345_678,
            "markRequiredForNextLevel": 2,
            "championSeasonMilestone": 4,
            "milestoneGrades": ["S", "A"]
        }))
        .expect("current ChampionMasteryDto should decode without legacy token fields");

        assert_eq!(mastery.champion_id, 103);
        assert_eq!(mastery.champion_level, 7);
        assert_eq!(mastery.champion_points, 345_678);
        assert_eq!(mastery.last_play_time, 1_765_000_000_000_i64);
        assert_eq!(mastery.tokens_earned, 0);
        assert!(!mastery.chest_granted);
    }

    #[test]
    fn ranked_entry_decodes_current_flags_series_and_defaults() {
        let ranked: RankedEntry = serde_json::from_value(serde_json::json!({
            "queueType": "RANKED_SOLO_5x5",
            "tier": "GOLD",
            "rank": "II",
            "leaguePoints": 72,
            "wins": 18,
            "losses": 12,
            "hotStreak": true,
            "veteran": true,
            "freshBlood": false,
            "miniSeries": {
                "losses": 1,
                "progress": "WLN",
                "target": 3,
                "wins": 1
            }
        }))
        .expect("current LeagueEntryDTO should decode");

        assert!(ranked.hot_streak);
        assert!(ranked.veteran);
        assert!(!ranked.fresh_blood);
        let series = ranked.mini_series.expect("mini series should be preserved");
        assert_eq!(series.wins, 1);
        assert_eq!(series.losses, 1);
        assert_eq!(series.target, 3);
        assert_eq!(series.progress, "WLN");

        let defaults: RankedEntry = serde_json::from_value(serde_json::json!({
            "queueType": "RANKED_FLEX_SR",
            "tier": "SILVER",
            "rank": "I",
            "leaguePoints": 20,
            "wins": 3,
            "losses": 2
        }))
        .expect("optional rank flags should default cleanly");
        assert!(!defaults.hot_streak);
        assert!(!defaults.veteran);
        assert!(!defaults.fresh_blood);
        assert!(defaults.mini_series.is_none());
    }

    #[test]
    fn profile_contract_serializes_snake_case_fields() {
        let mastery = ChampionMastery {
            champion_id: 103,
            champion_level: 7,
            champion_points: 345_678,
            last_play_time: 1_765_000_000_000_i64,
            champion_points_until_next_level: 0,
            champion_points_since_last_level: 345_678,
            tokens_earned: 2,
            chest_granted: true,
        };
        let mastery_json = serde_json::to_value(mastery).expect("mastery should serialize");
        assert_eq!(mastery_json["champion_id"], 103);
        assert_eq!(mastery_json["champion_points"], 345_678);
        assert!(mastery_json.get("championId").is_none());
        assert!(mastery_json.get("championPoints").is_none());

        let ranked = RankedEntry {
            queue_type: "RANKED_SOLO_5x5".into(),
            tier: "GOLD".into(),
            rank: "II".into(),
            league_points: 72,
            wins: 18,
            losses: 12,
            hot_streak: true,
            veteran: false,
            fresh_blood: true,
            mini_series: None,
        };
        let ranked_json = serde_json::to_value(ranked).expect("rank entry should serialize");
        assert_eq!(ranked_json["queue_type"], "RANKED_SOLO_5x5");
        assert_eq!(ranked_json["league_points"], 72);
        assert_eq!(ranked_json["hot_streak"], true);
        assert_eq!(ranked_json["fresh_blood"], true);
        assert!(ranked_json.get("queueType").is_none());
        assert!(ranked_json.get("leaguePoints").is_none());
    }

    #[test]
    fn mastery_normalization_filters_sorts_dedupes_and_caps() {
        let mastery = |champion_id: u32, champion_points: u64| ChampionMastery {
            champion_id,
            champion_level: 7,
            champion_points,
            last_play_time: 0,
            champion_points_until_next_level: 0,
            champion_points_since_last_level: 0,
            tokens_earned: 0,
            chest_granted: false,
        };
        let mut raw = (1..=205_u32)
            .map(|champion_id| mastery(champion_id, champion_id as u64))
            .collect::<Vec<_>>();
        raw.push(mastery(50, 1_000_000));
        raw.push(mastery(0, 2_000_000));

        let normalized = normalize_masteries(raw);

        assert_eq!(normalized.len(), 200);
        assert_eq!(normalized[0].champion_id, 50);
        assert_eq!(normalized[0].champion_points, 1_000_000);
        assert_eq!(
            normalized
                .iter()
                .filter(|entry| entry.champion_id == 50)
                .count(),
            1
        );
        assert!(normalized.iter().all(|entry| entry.champion_id > 0));
        assert!(normalized.windows(2).all(|window| {
            window[0].champion_points > window[1].champion_points
                || (window[0].champion_points == window[1].champion_points
                    && window[0].champion_id < window[1].champion_id)
        }));
    }

    #[test]
    fn detailed_match_payload_calculates_player_and_team_metrics() {
        let response: MatchResponse = serde_json::from_str(
            r#"{
            "info": {
                "gameCreation": 1725000000000,
                "gameDuration": 1200,
                "gameMode": "CLASSIC",
                "gameType": "MATCHED_GAME",
                "gameVersion": "26.15.1",
                "platformId": "EUN1",
                "mapId": 11,
                "queueId": 420,
                "participants": [
                    {
                        "participantId": 1,
                        "teamId": 100,
                        "puuid": "player-puuid",
                        "riotIdGameName": "Aura Tester",
                        "riotIdTagline": "EUNE",
                        "championId": 103,
                        "championName": "Ahri",
                        "champLevel": 16,
                        "teamPosition": "MIDDLE",
                        "kills": 4,
                        "deaths": 2,
                        "assists": 6,
                        "win": true,
                        "totalMinionsKilled": 180,
                        "neutralMinionsKilled": 20,
                        "goldEarned": 12000,
                        "goldSpent": 11200,
                        "visionScore": 25,
                        "wardsPlaced": 10,
                        "wardsKilled": 3,
                        "detectorWardsPlaced": 2,
                        "totalDamageDealtToChampions": 20000,
                        "totalDamageTaken": 8000,
                        "damageDealtToObjectives": 4500,
                        "summoner1Id": 4,
                        "summoner2Id": 14,
                        "item0": 6655,
                        "item1": 3020,
                        "perks": {
                            "styles": [{
                                "description": "primaryStyle",
                                "style": 8100,
                                "selections": [{ "perk": 8112 }]
                            }],
                            "statPerks": { "defense": 5001, "flex": 5008, "offense": 5005 }
                        }
                    },
                    {
                        "participantId": 2,
                        "teamId": 100,
                        "puuid": "ally-puuid",
                        "championId": 64,
                        "championName": "LeeSin",
                        "kills": 6,
                        "deaths": 3,
                        "assists": 4,
                        "win": true,
                        "goldEarned": 8000,
                        "totalDamageDealtToChampions": 10000
                    },
                    {
                        "participantId": 6,
                        "teamId": 200,
                        "puuid": "enemy-puuid",
                        "championId": 238,
                        "championName": "Zed",
                        "kills": 5,
                        "deaths": 5,
                        "assists": 1,
                        "win": false,
                        "goldEarned": 9000,
                        "totalDamageDealtToChampions": 15000
                    }
                ],
                "teams": [
                    {
                        "teamId": 100,
                        "win": true,
                        "bans": [{ "championId": 84, "pickTurn": 1 }],
                        "objectives": {
                            "champion": { "first": true, "kills": 10 },
                            "dragon": { "first": true, "kills": 3 },
                            "tower": { "first": true, "kills": 7 }
                        }
                    },
                    {
                        "teamId": 200,
                        "win": false,
                        "objectives": { "champion": { "kills": 5 } }
                    }
                ]
            }
        }"#,
        )
        .expect("representative Match-V5 response should decode");

        let (summary, detail) =
            build_match_payload("EUN1_123".into(), response.info, "player-puuid")
                .expect("identified participant should produce a report");

        assert_eq!(summary.match_id, "EUN1_123");
        assert_eq!(summary.champion_name, "Ahri");
        assert_eq!(summary.cs, 200);
        assert_eq!(summary.csm, 10.0);
        assert_eq!(summary.kda, 5.0);
        assert_eq!(summary.kill_participation, 100.0);
        assert_eq!(summary.items, vec![6655, 3020]);
        assert_eq!(summary.summoner_spell_ids, vec![4, 14]);
        assert_eq!(summary.perk_ids, vec![8112]);

        assert_eq!(detail.participants.len(), 3);
        assert_eq!(detail.player.riot_id, "Aura Tester#EUNE");
        assert_eq!(detail.player.damage_share, 66.67);
        assert_eq!(detail.player.gold_share, 60.0);
        let blue = detail
            .teams
            .iter()
            .find(|team| team.team_id == 100)
            .expect("blue team should be present");
        assert_eq!(blue.kills, 10);
        assert_eq!(blue.objectives.dragon.kills, 3);
        assert_eq!(blue.bans[0].champion_id, 84);
    }

    #[test]
    fn omitted_zero_fields_do_not_drop_a_match_report() {
        let response: MatchResponse = serde_json::from_value(serde_json::json!({
            "info": {
                "gameStartTimestamp": 10_000,
                "gameEndTimestamp": 610_000,
                "participants": [{
                    "participantId": 1,
                    "teamId": 100,
                    "puuid": "player-puuid",
                    "championName": "Yuumi"
                }]
            }
        }))
        .expect("Riot responses may omit empty values");

        let (summary, detail) = build_match_payload("TEST_1".into(), response.info, "player-puuid")
            .expect("minimal participant should remain usable");

        assert_eq!(summary.game_duration_secs, 600);
        assert_eq!(summary.kills, 0);
        assert_eq!(summary.kill_participation, 0.0);
        assert_eq!(detail.participants.len(), 1);
        assert_eq!(detail.teams.len(), 1, "missing team block is synthesized");
        assert!(detail.player.items.is_empty());
    }

    #[test]
    fn legacy_millisecond_duration_and_small_percentages_are_preserved() {
        let info = MatchInfo {
            game_duration: 1_800_000,
            ..MatchInfo::default()
        };

        assert_eq!(normalized_game_duration_secs(&info), 1_800);
        assert_eq!(percentage(1, 100), 1.0);
        assert_eq!(percentage(250, 100), 100.0);
    }
}
