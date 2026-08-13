use futures_util::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::RwLock;
use url::Url;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const POLICY_NOTICE: &str = "Recommendations preserve player choice. Aura uses only supplied visible/current-game data, never infers hidden information or MMR/ELO, and does not guarantee outcomes.";
const PROVIDER_SAMPLE_NOTICE: &str =
    "Provider-reported aggregate sample; Aura has not independently verified the sample size.";
const LOCAL_SAMPLE_NOTICE: &str =
    "Deterministic local heuristic; this is not aggregate or mass-match data.";

#[derive(Clone)]
struct Secret(Arc<str>);

impl Secret {
    fn new(value: String) -> Self {
        Self(Arc::<str>::from(value))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug)]
pub struct AdvisorConfig {
    endpoint: Option<Url>,
    token: Option<Secret>,
    validation_error: Option<String>,
    local_development_endpoint: bool,
}

impl AdvisorConfig {
    pub fn from_environment() -> Self {
        let endpoint = non_empty_env("AURA_META_API_URL");
        let token = non_empty_env("AURA_META_API_TOKEN");

        let Some(endpoint_value) = endpoint else {
            let validation_error = token
                .as_ref()
                .map(|_| "AURA_META_API_TOKEN is set but AURA_META_API_URL is missing".to_string());
            return Self {
                endpoint: None,
                token: None,
                validation_error,
                local_development_endpoint: false,
            };
        };

        let parsed = match validate_feed_endpoint(&endpoint_value) {
            Ok(url) => url,
            Err(error) => {
                return Self {
                    endpoint: None,
                    token: None,
                    validation_error: Some(error),
                    local_development_endpoint: false,
                };
            }
        };

        let token = match token {
            Some(value) => match validate_token(value) {
                Ok(value) => Some(Secret::new(value)),
                Err(error) => {
                    return Self {
                        endpoint: None,
                        token: None,
                        validation_error: Some(error),
                        local_development_endpoint: false,
                    };
                }
            },
            None => None,
        };

        let local_development_endpoint =
            parsed.scheme() == "http" && parsed.host_str() == Some("127.0.0.1");
        Self {
            endpoint: Some(parsed),
            token,
            validation_error: None,
            local_development_endpoint,
        }
    }

    fn from_values(endpoint: Option<&str>, token: Option<&str>) -> Self {
        match endpoint {
            Some(endpoint) => {
                let parsed = match validate_feed_endpoint(endpoint) {
                    Ok(url) => url,
                    Err(error) => {
                        return Self {
                            endpoint: None,
                            token: None,
                            validation_error: Some(error),
                            local_development_endpoint: false,
                        };
                    }
                };
                let token = match token {
                    Some(value) => match validate_token(value.to_string()) {
                        Ok(value) => Some(Secret::new(value)),
                        Err(error) => {
                            return Self {
                                endpoint: None,
                                token: None,
                                validation_error: Some(error),
                                local_development_endpoint: false,
                            };
                        }
                    },
                    None => None,
                };
                let local_development_endpoint =
                    parsed.scheme() == "http" && parsed.host_str() == Some("127.0.0.1");
                Self {
                    endpoint: Some(parsed),
                    token,
                    validation_error: None,
                    local_development_endpoint,
                }
            }
            None => Self {
                endpoint: None,
                token: None,
                validation_error: token
                    .map(|_| "AURA_META_API_TOKEN is set but AURA_META_API_URL is missing".into()),
                local_development_endpoint: false,
            },
        }
    }

    pub fn is_configured(&self) -> bool {
        self.endpoint.is_some() && self.validation_error.is_none()
    }

    pub fn safe_mode(&self) -> &'static str {
        if self.local_development_endpoint {
            "local_development_feed"
        } else if self.is_configured() {
            "aggregate_cloud"
        } else {
            "local_heuristic"
        }
    }

    pub fn safe_error(&self) -> Option<String> {
        self.validation_error.clone()
    }

    pub fn log_safe_summary(&self) {
        if let Some(error) = &self.validation_error {
            eprintln!("[AURA::ADVISOR][ERR] aggregate feed disabled: {error}");
        } else if self.local_development_endpoint {
            println!("[AURA::ADVISOR][OK] local development aggregate feed configured");
        } else if self.is_configured() {
            println!("[AURA::ADVISOR][OK] HTTPS aggregate feed configured");
        } else {
            println!("[AURA::ADVISOR][OK] deterministic local advisor enabled");
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_feed_endpoint(value: &str) -> Result<Url, String> {
    let parsed =
        Url::parse(value).map_err(|_| "AURA_META_API_URL is not a valid URL".to_string())?;
    let is_https = parsed.scheme() == "https";
    let is_loopback_development =
        parsed.scheme() == "http" && parsed.host_str() == Some("127.0.0.1");
    if !is_https && !is_loopback_development {
        return Err(
            "AURA_META_API_URL must use HTTPS, except http://127.0.0.1 for local development"
                .into(),
        );
    }
    if parsed.host_str().is_none() {
        return Err("AURA_META_API_URL must include a host".into());
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "AURA_META_API_URL cannot contain user information, a query, or a fragment".into(),
        );
    }
    Ok(parsed)
}

fn validate_token(value: String) -> Result<String, String> {
    if value.len() > 4096 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("AURA_META_API_TOKEN must contain 1-4096 printable ASCII characters".into());
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvisorProvenance {
    #[serde(default)]
    pub mode: String,
    pub source: String,
    pub patch: String,
    pub queue: String,
    pub rank_range: String,
    pub region: String,
    pub sample_size: u64,
    pub generated_at: String,
    #[serde(default)]
    pub sample_size_note: String,
    pub methodology: String,
    pub methodology_url: Option<String>,
    pub source_url: Option<String>,
    pub dataset_version: String,
    pub schema_version: String,
}

impl AdvisorProvenance {
    fn local(request: &CommonRequest) -> Self {
        Self {
            mode: "local_heuristic".into(),
            source: "Aura deterministic local rules".into(),
            patch: value_or(&request.patch, "current patch not supplied"),
            queue: request.queue_label(),
            rank_range: "not applicable".into(),
            region: value_or(&request.region, "not supplied"),
            sample_size: 0,
            generated_at: generated_now(),
            sample_size_note: LOCAL_SAMPLE_NOTICE.into(),
            methodology: "Deterministic role and visible-state rules bundled with Aura; no aggregate population model.".into(),
            methodology_url: None,
            source_url: None,
            dataset_version: "local-rules-v1".into(),
            schema_version: "1".into(),
        }
    }

    fn personal(request: &CommonRequest, sample_size: usize) -> Self {
        Self {
            mode: "local_personal_review".into(),
            source: "User-supplied recent-match sample".into(),
            patch: value_or(&request.patch, "current patch not supplied"),
            queue: request.queue_label(),
            rank_range: "not calculated".into(),
            region: value_or(&request.region, "not supplied"),
            sample_size: sample_size as u64,
            generated_at: generated_now(),
            sample_size_note:
                "Local user sample supplied to this command; not an aggregate population estimate."
                    .into(),
            methodology: "Per-match KDA, farm, vision, death, and objective-context review calculated only from the supplied user sample.".into(),
            methodology_url: None,
            source_url: None,
            dataset_version: "personal-review-v1".into(),
            schema_version: "1".into(),
        }
    }

    fn as_cloud_output(&self) -> Self {
        let mut output = self.clone();
        if output.mode != "local_development_feed" {
            output.mode = "aggregate_cloud".into();
        }
        output.sample_size_note = PROVIDER_SAMPLE_NOTICE.into();
        output
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AggregateFeed {
    provenance: AdvisorProvenance,
    champions: Vec<ChampionAggregate>,
    #[serde(default)]
    matchups: Vec<MatchupAggregate>,
    #[serde(default)]
    synergies: Vec<SynergyAggregate>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChampionAggregate {
    champion_id: u32,
    name: String,
    role: String,
    games: u64,
    win_rate: f64,
    pick_rate: f64,
    ban_rate: f64,
    #[serde(default)]
    provider_score: Option<f64>,
    #[serde(default)]
    strengths: Vec<String>,
    #[serde(default)]
    tradeoffs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchupAggregate {
    champion_id: u32,
    enemy_champion_id: u32,
    role: String,
    games: u64,
    win_rate: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct SynergyAggregate {
    champion_id: u32,
    ally_champion_id: u32,
    role: String,
    games: u64,
    win_rate: f64,
}

struct CachedFeed {
    feed: Arc<AggregateFeed>,
    fetched_at: Instant,
}

#[derive(Clone)]
pub struct AdvisorState {
    config: AdvisorConfig,
    client: Client,
    cache: Arc<RwLock<Option<CachedFeed>>>,
    last_error: Arc<RwLock<Option<String>>>,
    refreshing: Arc<AtomicBool>,
}

impl AdvisorState {
    pub fn new(config: AdvisorConfig) -> Result<Self, String> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .pool_max_idle_per_host(1)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Aura/0.6 aggregate-advisor")
            .build()
            .map_err(|_| "could not initialize the aggregate-feed HTTP client".to_string())?;
        Ok(Self {
            config,
            client,
            cache: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            refreshing: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn fallback(error: String) -> Self {
        let mut config = AdvisorConfig::from_values(None, None);
        config.validation_error = Some(error);
        Self {
            config,
            client: Client::new(),
            cache: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            refreshing: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn cached_feed(&self, allow_stale: bool) -> Option<(Arc<AggregateFeed>, bool)> {
        self.cache.read().await.as_ref().and_then(|cached| {
            let transport_stale = cached.fetched_at.elapsed() >= CACHE_TTL;
            let stale = transport_stale || dataset_is_stale(&cached.feed.provenance);
            if !transport_stale || allow_stale {
                Some((Arc::clone(&cached.feed), stale))
            } else {
                None
            }
        })
    }

    async fn refresh(&self) -> Result<(), String> {
        let endpoint = self.config.endpoint.clone().ok_or_else(|| {
            self.config
                .validation_error
                .clone()
                .unwrap_or_else(|| "aggregate feed is not configured".into())
        })?;
        let _guard = RefreshGuard::acquire(&self.refreshing)?;

        let result = self.fetch_feed(endpoint).await;
        match result {
            Ok(feed) => {
                *self.cache.write().await = Some(CachedFeed {
                    feed: Arc::new(feed),
                    fetched_at: Instant::now(),
                });
                *self.last_error.write().await = None;
                println!("[AURA::ADVISOR][OK] aggregate feed refreshed in volatile RAM");
                Ok(())
            }
            Err(error) => {
                *self.last_error.write().await = Some(error.clone());
                eprintln!("[AURA::ADVISOR][ERR] aggregate feed refresh failed: {error}");
                Err(error)
            }
        }
    }

    async fn fetch_feed(&self, endpoint: Url) -> Result<AggregateFeed, String> {
        let mut request = self.client.get(endpoint).header(ACCEPT, "application/json");
        if let Some(token) = &self.config.token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token.expose()));
        }
        let response = request
            .send()
            .await
            .map_err(|error| safe_network_error(&error))?;

        if response.status() != StatusCode::OK {
            return Err(format!(
                "aggregate feed returned HTTP {}",
                response.status().as_u16()
            ));
        }
        if let Some(length) = response.content_length() {
            if length > MAX_RESPONSE_BYTES as u64 {
                return Err("aggregate feed response exceeds the 2 MiB limit".into());
            }
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(|value| value.trim().to_ascii_lowercase())
            .ok_or_else(|| "aggregate feed did not return a valid JSON Content-Type".to_string())?;
        if content_type != "application/json" && !content_type.ends_with("+json") {
            return Err("aggregate feed did not return application/json".into());
        }

        let mut body = Vec::with_capacity(64 * 1024);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| safe_network_error(&error))?;
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err("aggregate feed response exceeds the 2 MiB limit".into());
            }
            body.extend_from_slice(&chunk);
        }

        let mut feed: AggregateFeed = serde_json::from_slice(&body).map_err(|_| {
            "aggregate feed response does not match the required schema".to_string()
        })?;
        validate_feed(&feed)?;
        feed.provenance.mode = self.config.safe_mode().into();
        Ok(feed)
    }
}

struct RefreshGuard<'a>(&'a AtomicBool);

impl<'a> RefreshGuard<'a> {
    fn acquire(refreshing: &'a AtomicBool) -> Result<Self, String> {
        refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "aggregate feed refresh is already in progress".to_string())?;
        Ok(Self(refreshing))
    }
}

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn safe_network_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "aggregate feed request timed out".into()
    } else if error.is_connect() {
        "could not connect to the aggregate feed".into()
    } else if error.is_redirect() {
        "aggregate feed redirects are not permitted".into()
    } else {
        "aggregate feed network request failed".into()
    }
}

fn validate_feed(feed: &AggregateFeed) -> Result<(), String> {
    validate_provenance(&feed.provenance)?;
    if feed.champions.len() < 3 {
        return Err("aggregate feed must contain at least three champion records".into());
    }

    let mut unique = HashSet::new();
    for champion in &feed.champions {
        validate_label("champion name", &champion.name, 80)?;
        validate_label("champion role", &champion.role, 32)?;
        if champion.champion_id == 0 || champion.games == 0 {
            return Err("aggregate champion records require non-zero IDs and games".into());
        }
        if champion.games > feed.provenance.sample_size {
            return Err("aggregate champion games cannot exceed provenance sample_size".into());
        }
        validate_rate("champion win_rate", champion.win_rate)?;
        validate_rate("champion pick_rate", champion.pick_rate)?;
        validate_rate("champion ban_rate", champion.ban_rate)?;
        if champion
            .provider_score
            .is_some_and(|score| !score.is_finite() || !(0.0..=100.0).contains(&score))
        {
            return Err("champion provider_score must be between 0 and 100".into());
        }
        if !unique.insert((champion.champion_id, normalize_role(&champion.role))) {
            return Err("aggregate feed contains a duplicate champion/role record".into());
        }
        for statement in champion.strengths.iter().chain(champion.tradeoffs.iter()) {
            validate_label("champion evidence statement", statement, 240)?;
        }
    }

    let mut unique_matchups = HashSet::new();
    for matchup in &feed.matchups {
        if matchup.champion_id == 0 || matchup.enemy_champion_id == 0 || matchup.games == 0 {
            return Err("matchup records require non-zero IDs and games".into());
        }
        if matchup.games > feed.provenance.sample_size {
            return Err("aggregate matchup games cannot exceed provenance sample_size".into());
        }
        validate_label("matchup role", &matchup.role, 32)?;
        validate_rate("matchup win_rate", matchup.win_rate)?;
        if !unique_matchups.insert((
            matchup.champion_id,
            matchup.enemy_champion_id,
            normalize_role(&matchup.role),
        )) {
            return Err("aggregate feed contains a duplicate matchup/role record".into());
        }
    }
    let mut unique_synergies = HashSet::new();
    for synergy in &feed.synergies {
        if synergy.champion_id == 0 || synergy.ally_champion_id == 0 || synergy.games == 0 {
            return Err("synergy records require non-zero IDs and games".into());
        }
        if synergy.games > feed.provenance.sample_size {
            return Err("aggregate synergy games cannot exceed provenance sample_size".into());
        }
        validate_label("synergy role", &synergy.role, 32)?;
        validate_rate("synergy win_rate", synergy.win_rate)?;
        if !unique_synergies.insert((
            synergy.champion_id,
            synergy.ally_champion_id,
            normalize_role(&synergy.role),
        )) {
            return Err("aggregate feed contains a duplicate synergy/role record".into());
        }
    }
    Ok(())
}

fn validate_provenance(provenance: &AdvisorProvenance) -> Result<(), String> {
    validate_label("provenance source", &provenance.source, 160)?;
    validate_label("provenance patch", &provenance.patch, 40)?;
    validate_label("provenance queue", &provenance.queue, 80)?;
    validate_label("provenance rank_range", &provenance.rank_range, 80)?;
    validate_label("provenance region", &provenance.region, 80)?;
    validate_label("provenance generated_at", &provenance.generated_at, 80)?;
    let generated_at = parse_generated_at(&provenance.generated_at)?;
    if generated_at > OffsetDateTime::now_utc() + time::Duration::hours(24) {
        return Err("aggregate feed provenance generated_at is too far in the future".into());
    }
    validate_label("provenance methodology", &provenance.methodology, 320)?;
    validate_label("dataset_version", &provenance.dataset_version, 80)?;
    validate_label("schema_version", &provenance.schema_version, 40)?;
    if provenance.sample_size == 0 {
        return Err("aggregate feed provenance sample_size must be greater than zero".into());
    }
    let source_url = provenance
        .source_url
        .as_deref()
        .or(provenance.methodology_url.as_deref())
        .ok_or_else(|| {
            "aggregate feed provenance requires source_url or methodology_url".to_string()
        })?;
    validate_reference_url(source_url)?;
    if let Some(value) = &provenance.source_url {
        validate_reference_url(value)?;
    }
    if let Some(value) = &provenance.methodology_url {
        validate_reference_url(value)?;
    }
    Ok(())
}

fn validate_reference_url(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|_| "provenance URL is invalid".to_string())?;
    let allowed =
        url.scheme() == "https" || (url.scheme() == "http" && url.host_str() == Some("127.0.0.1"));
    if !allowed || !url.username().is_empty() || url.password().is_some() {
        return Err("provenance URLs must use HTTPS without embedded credentials".into());
    }
    Ok(())
}

fn parse_generated_at(value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| "aggregate feed provenance generated_at must be RFC 3339".to_string())
}

fn dataset_is_stale(provenance: &AdvisorProvenance) -> bool {
    parse_generated_at(&provenance.generated_at)
        .map(|generated_at| OffsetDateTime::now_utc() - generated_at > time::Duration::hours(72))
        .unwrap_or(true)
}

fn validate_label(name: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > maximum
        || value.chars().any(|character| character.is_control())
    {
        return Err(format!("{name} is missing or invalid"));
    }
    Ok(())
}

fn validate_rate(name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!("{name} must be a ratio between 0 and 1"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvisorStatus {
    pub configured: bool,
    pub ready: bool,
    pub stale: bool,
    pub refreshing: bool,
    pub mode: String,
    pub source: String,
    pub message: String,
    pub last_error: Option<String>,
    pub cache_age_seconds: Option<u64>,
    pub cache_ttl_seconds: u64,
    pub max_response_bytes: usize,
    pub provenance: Option<AdvisorProvenance>,
    pub policy_notice: String,
}

async fn status_for(state: &AdvisorState) -> AdvisorStatus {
    let cache = state.cache.read().await;
    let (ready, stale, age, provenance) = match cache.as_ref() {
        Some(cached) => {
            let elapsed = cached.fetched_at.elapsed();
            let stale = elapsed >= CACHE_TTL || dataset_is_stale(&cached.feed.provenance);
            (
                true,
                stale,
                Some(elapsed.as_secs()),
                Some(cached.feed.provenance.as_cloud_output()),
            )
        }
        None => (false, false, None, None),
    };
    let configured = state.config.is_configured();
    let config_error = state.config.validation_error.clone();
    let last_error = state.last_error.read().await.clone().or(config_error);
    let mode = provenance
        .as_ref()
        .map(|value| value.mode.as_str())
        .unwrap_or_else(|| state.config.safe_mode());
    let source = provenance
        .as_ref()
        .map(|value| value.source.clone())
        .unwrap_or_else(|| "Aura deterministic local rules".into());
    let message = if ready && stale {
        "Cached aggregate data or its provider snapshot is stale; recommendations disclose that limitation."
    } else if ready {
        "Aggregate data is cached in volatile RAM. Sample sizes are provider-reported, not independently verified."
    } else if configured {
        "Aggregate feed is configured but has not completed a successful refresh; local heuristics remain available."
    } else {
        "Using deterministic local heuristics with sample_size 0; no mass-match dataset is claimed."
    };
    AdvisorStatus {
        configured,
        ready,
        stale,
        refreshing: state.refreshing.load(Ordering::Acquire),
        mode: mode.into(),
        source,
        message: message.into(),
        last_error,
        cache_age_seconds: age,
        cache_ttl_seconds: CACHE_TTL.as_secs(),
        max_response_bytes: MAX_RESPONSE_BYTES,
        provenance,
        policy_notice: POLICY_NOTICE.into(),
    }
}

#[tauri::command]
pub async fn advisor_status(
    state: tauri::State<'_, AdvisorState>,
) -> Result<AdvisorStatus, String> {
    Ok(status_for(&state).await)
}

#[tauri::command]
pub async fn advisor_refresh(
    state: tauri::State<'_, AdvisorState>,
) -> Result<AdvisorStatus, String> {
    if !state.config.is_configured() {
        return Ok(status_for(&state).await);
    }
    state.refresh().await?;
    Ok(status_for(&state).await)
}

pub async fn warm_cache(state: AdvisorState) {
    if state.config.is_configured() {
        let _ = state.refresh().await;
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CommonRequest {
    #[serde(deserialize_with = "null_to_default")]
    pub role: String,
    #[serde(deserialize_with = "null_to_default")]
    pub patch: String,
    #[serde(deserialize_with = "null_to_default")]
    pub region: String,
    pub queue_id: Option<u32>,
    #[serde(deserialize_with = "null_to_default")]
    pub queue: String,
    #[serde(deserialize_with = "null_to_default")]
    pub rank_range: String,
    #[serde(deserialize_with = "null_to_default")]
    pub gameflow_phase: String,
    pub selected_champion_id: Option<u32>,
    pub ally_champion_ids: Vec<u32>,
    pub enemy_champion_ids: Vec<u32>,
    #[serde(deserialize_with = "null_to_default")]
    pub context_captured_at: String,
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl CommonRequest {
    fn queue_label(&self) -> String {
        if !self.queue.trim().is_empty() {
            self.queue.trim().to_string()
        } else if let Some(queue_id) = self.queue_id {
            format!("queue {queue_id}")
        } else {
            "not supplied".into()
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CatalogChampion {
    #[serde(deserialize_with = "null_to_default")]
    pub id: u32,
    #[serde(deserialize_with = "null_to_default")]
    pub name: String,
    #[serde(deserialize_with = "null_to_default")]
    pub image_id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DraftRequest {
    #[serde(flatten)]
    pub common: CommonRequest,
    pub champion_catalog: Vec<CatalogChampion>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TelemetrySnapshot {
    pub game_time: Option<f64>,
    pub dragon_respawn_at: Option<f64>,
    pub baron_respawn_at: Option<f64>,
    pub received_at_ms: Option<u64>,
    pub age_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LiveRequest {
    #[serde(flatten)]
    pub common: CommonRequest,
    pub telemetry: Option<TelemetrySnapshot>,
    pub game_time: Option<f64>,
    pub dragon_respawn_at: Option<f64>,
    pub baron_respawn_at: Option<f64>,
    pub telemetry_received_at_ms: Option<u64>,
    pub telemetry_age_ms: Option<u64>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub cs: Option<u32>,
    pub vision_score: Option<u32>,
    pub current_gold: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MatchSample {
    #[serde(deserialize_with = "null_to_default")]
    pub match_id: String,
    pub queue_id: Option<u32>,
    #[serde(deserialize_with = "null_to_default")]
    pub game_mode: String,
    pub game_creation_ms: Option<u64>,
    pub game_duration_secs: u64,
    #[serde(deserialize_with = "null_to_default")]
    pub champion_name: String,
    pub win: bool,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub cs: u32,
    pub gold: u32,
    pub vision_score: u32,
    pub items: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PostGameRequest {
    #[serde(flatten)]
    pub common: CommonRequest,
    pub latest_match: Option<MatchSample>,
    pub recent_matches: Vec<MatchSample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvisorAlternative {
    pub rank: usize,
    pub champion_id: Option<u32>,
    pub champion: String,
    pub title: String,
    pub reason: String,
    pub tradeoff: String,
    pub confidence: f64,
    pub score: f64,
    pub win_rate: Option<f64>,
    pub sample_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvisorResponse {
    pub phase: String,
    pub advisory_label: String,
    pub headline: String,
    pub mandate: String,
    pub recommended_champion: Option<String>,
    pub recommended_champion_id: Option<u32>,
    pub confidence: f64,
    pub reasoning: Vec<String>,
    pub actions: Vec<String>,
    pub warnings: Vec<String>,
    pub alternatives: Vec<AdvisorAlternative>,
    pub provenance: AdvisorProvenance,
    pub used_fallback: bool,
    pub policy_notice: String,
}

#[derive(Clone)]
struct RankedCandidate {
    champion_id: u32,
    name: String,
    score: f64,
    win_rate: Option<f64>,
    games: Option<u64>,
    reason: String,
    tradeoff: String,
}

#[tauri::command]
pub async fn advisor_draft_mandate(
    state: tauri::State<'_, AdvisorState>,
    request: DraftRequest,
) -> Result<AdvisorResponse, String> {
    validate_common_request(&request.common)?;

    let mut stale_warning = None;
    if state.config.is_configured() && state.cached_feed(false).await.is_none() {
        if let Err(error) = state.refresh().await {
            stale_warning = Some(error);
        }
    }
    if let Some((feed, stale)) = state.cached_feed(true).await {
        let ranked = rank_cloud_draft(&feed, &request);
        if ranked.len() >= 3 {
            return Ok(cloud_draft_response(
                &request,
                &feed,
                ranked,
                stale,
                stale_warning,
            ));
        }
        stale_warning = Some(
            "aggregate feed had fewer than three eligible role/candidate records; local heuristic used"
                .into(),
        );
    }
    Ok(local_draft_response(&request, stale_warning))
}

#[tauri::command]
pub async fn advisor_live_orders(
    state: tauri::State<'_, AdvisorState>,
    request: LiveRequest,
) -> Result<AdvisorResponse, String> {
    validate_common_request(&request.common)?;
    validate_live_request(&request)?;
    let (feed, stale) = state
        .cached_feed(true)
        .await
        .map(|(feed, stale)| (Some(feed), stale))
        .unwrap_or((None, false));
    Ok(live_response(&request, feed.as_deref(), stale))
}

#[tauri::command]
pub async fn advisor_post_game(
    _state: tauri::State<'_, AdvisorState>,
    request: PostGameRequest,
) -> Result<AdvisorResponse, String> {
    validate_common_request(&request.common)?;
    Ok(post_game_response(&request))
}

fn validate_common_request(request: &CommonRequest) -> Result<(), String> {
    for (name, value, maximum) in [
        ("role", request.role.as_str(), 32),
        ("patch", request.patch.as_str(), 40),
        ("region", request.region.as_str(), 40),
        ("queue", request.queue.as_str(), 80),
        ("rank_range", request.rank_range.as_str(), 80),
        ("gameflow_phase", request.gameflow_phase.as_str(), 80),
        (
            "context_captured_at",
            request.context_captured_at.as_str(),
            80,
        ),
    ] {
        if value.len() > maximum || value.chars().any(|character| character.is_control()) {
            return Err(format!("advisor request field {name} is invalid"));
        }
    }
    if request.ally_champion_ids.len() > 5 || request.enemy_champion_ids.len() > 5 {
        return Err("advisor requests accept at most five visible champions per team".into());
    }
    Ok(())
}

fn validate_live_request(request: &LiveRequest) -> Result<(), String> {
    let phase = normalize_context(&request.common.gameflow_phase);
    if !matches!(phase.as_str(), "inprogress" | "reconnect") {
        return Err("live analysis requires an active League match".into());
    }
    let telemetry = request.telemetry.as_ref();
    let received_at = request
        .telemetry_received_at_ms
        .or_else(|| telemetry.and_then(|value| value.received_at_ms));
    let age = request
        .telemetry_age_ms
        .or_else(|| telemetry.and_then(|value| value.age_ms));
    let game_time = request
        .game_time
        .or_else(|| telemetry.and_then(|value| value.game_time));
    if received_at.is_none() || age.is_none() || game_time.is_none() {
        return Err("live telemetry is not ready; wait for the in-game data feed and retry".into());
    }
    if age.is_some_and(|value| value > 15_000) {
        return Err("live telemetry is stale; wait for the next in-game update and retry".into());
    }
    if game_time.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err("live telemetry contains an invalid game clock".into());
    }
    Ok(())
}

fn rank_cloud_draft(feed: &AggregateFeed, request: &DraftRequest) -> Vec<RankedCandidate> {
    let role = normalize_role(&request.common.role);
    let catalog: HashSet<u32> = request
        .champion_catalog
        .iter()
        .filter(|champion| champion.id > 0)
        .map(|champion| champion.id)
        .collect();
    let unavailable: HashSet<u32> = request
        .common
        .ally_champion_ids
        .iter()
        .chain(request.common.enemy_champion_ids.iter())
        .filter(|champion_id| Some(**champion_id) != request.common.selected_champion_id)
        .copied()
        .collect();

    let mut ranked: Vec<RankedCandidate> = feed
        .champions
        .iter()
        .filter(|champion| normalize_role(&champion.role) == role)
        .filter(|champion| catalog.is_empty() || catalog.contains(&champion.champion_id))
        .filter(|champion| !unavailable.contains(&champion.champion_id))
        .map(|champion| {
            let base = champion
                .provider_score
                .unwrap_or(50.0 + (champion.win_rate - 0.5) * 100.0);
            let matchup_adjustment: f64 = feed
                .matchups
                .iter()
                .filter(|record| {
                    record.champion_id == champion.champion_id
                        && normalize_role(&record.role) == role
                        && request
                            .common
                            .enemy_champion_ids
                            .contains(&record.enemy_champion_id)
                })
                .map(|record| (record.win_rate - 0.5) * 20.0 * evidence_weight(record.games))
                .sum();
            let synergy_adjustment: f64 = feed
                .synergies
                .iter()
                .filter(|record| {
                    record.champion_id == champion.champion_id
                        && normalize_role(&record.role) == role
                        && request
                            .common
                            .ally_champion_ids
                            .contains(&record.ally_champion_id)
                })
                .map(|record| (record.win_rate - 0.5) * 12.0 * evidence_weight(record.games))
                .sum();
            let evidence = champion.strengths.first().cloned().unwrap_or_else(|| {
                format!(
                    "Provider reports {:.1}% win rate across {} role games.",
                    champion.win_rate * 100.0,
                    champion.games
                )
            });
            let tradeoff = champion.tradeoffs.first().cloned().unwrap_or_else(|| {
                "Review comfort, lane matchup, and team execution before choosing.".into()
            });
            RankedCandidate {
                champion_id: champion.champion_id,
                name: champion.name.clone(),
                score: base + matchup_adjustment + synergy_adjustment,
                win_rate: Some(champion.win_rate),
                games: Some(champion.games),
                reason: evidence,
                tradeoff,
            }
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.champion_id.cmp(&right.champion_id))
    });
    ranked
}

fn evidence_weight(games: u64) -> f64 {
    ((games as f64).ln_1p() / 10.0).clamp(0.15, 1.0)
}

fn cloud_draft_response(
    request: &DraftRequest,
    feed: &AggregateFeed,
    ranked: Vec<RankedCandidate>,
    stale: bool,
    refresh_error: Option<String>,
) -> AdvisorResponse {
    let primary = &ranked[0];
    let margin = (primary.score - ranked[1].score).max(0.0);
    let context_warnings = provenance_context_warnings(&feed.provenance, &request.common);
    let confidence = if context_warnings.is_empty() {
        (0.58 + margin / 100.0).clamp(0.58, 0.9)
    } else {
        (0.52 + margin / 200.0).clamp(0.52, 0.65)
    };
    let mut warnings = vec![PROVIDER_SAMPLE_NOTICE.into()];
    warnings.extend(context_warnings);
    if stale {
        warnings.push(
            "The aggregate evidence is stale because its RAM cache or provider snapshot exceeded the freshness window."
                .into(),
        );
    }
    if let Some(error) = refresh_error {
        warnings.push(format!("Refresh failed safely: {error}."));
    }
    AdvisorResponse {
        phase: "draft".into(),
        advisory_label: "BEST STATISTICAL FIT".into(),
        headline: format!("{} ranks first for the supplied draft context", primary.name),
        mandate: format!(
            "Consider {} as the best statistical fit; compare the alternatives and tradeoffs before locking in.",
            primary.name
        ),
        recommended_champion: Some(primary.name.clone()),
        recommended_champion_id: Some(primary.champion_id),
        confidence,
        reasoning: vec![
            primary.reason.clone(),
            format!("Tradeoff: {}", primary.tradeoff),
            format!(
                "The ranking used only the supplied {} role, visible ally/enemy IDs, and provider aggregate records.",
                value_or(&request.common.role, "unspecified")
            ),
            "The score is a ranking score, not a win probability or guaranteed result.".into(),
        ],
        actions: vec![
            "Compare the top three choices against your champion comfort.".into(),
            "Confirm the visible matchup and team composition before selecting.".into(),
            "Use the listed tradeoffs to choose the plan your team can execute.".into(),
        ],
        warnings,
        alternatives: ranked
            .iter()
            .skip(1)
            .take(2)
            .enumerate()
            .map(|(index, candidate)| alternative_from_candidate(index + 2, candidate, true))
            .collect(),
        provenance: feed.provenance.as_cloud_output(),
        used_fallback: false,
        policy_notice: POLICY_NOTICE.into(),
    }
}

fn local_draft_response(request: &DraftRequest, feed_warning: Option<String>) -> AdvisorResponse {
    let ranked = rank_local_draft(request);
    let primary = ranked
        .first()
        .cloned()
        .unwrap_or_else(|| fallback_candidate("A champion you play comfortably", 1));
    let mut warnings = vec![LOCAL_SAMPLE_NOTICE.into()];
    if let Some(warning) = feed_warning {
        warnings.push(warning);
    }
    AdvisorResponse {
        phase: "draft".into(),
        advisory_label: "LOCAL HEURISTIC FIT".into(),
        headline: format!("{} is the first local-rule option", primary.name),
        mandate: format!(
            "Consider {}; this is a deterministic local heuristic, so prioritize comfort and compare the alternatives.",
            primary.name
        ),
        recommended_champion: Some(primary.name.clone()),
        recommended_champion_id: (primary.champion_id > 0).then_some(primary.champion_id),
        confidence: 0.35,
        reasoning: vec![
            "No eligible aggregate dataset was available for this request.".into(),
            "The ordering is deterministic and role-oriented; it does not claim population win rates."
                .into(),
            format!("Primary-option tradeoff: {}", primary.tradeoff),
            "Visible ally/enemy selections were excluded when their champion IDs were supplied.".into(),
        ],
        actions: vec![
            "Choose the highest-ranked option you can execute reliably.".into(),
            "Compare at least two alternatives before committing.".into(),
            "Treat lane familiarity and team communication as decisive context.".into(),
        ],
        warnings,
        alternatives: ranked
            .iter()
            .skip(1)
            .take(2)
            .enumerate()
            .map(|(index, candidate)| alternative_from_candidate(index + 2, candidate, false))
            .collect(),
        provenance: AdvisorProvenance::local(&request.common),
        used_fallback: true,
        policy_notice: POLICY_NOTICE.into(),
    }
}

fn rank_local_draft(request: &DraftRequest) -> Vec<RankedCandidate> {
    let role = normalize_role(&request.common.role);
    let preferred = local_role_pool(&role);
    let unavailable: HashSet<u32> = request
        .common
        .ally_champion_ids
        .iter()
        .chain(request.common.enemy_champion_ids.iter())
        .filter(|champion_id| Some(**champion_id) != request.common.selected_champion_id)
        .copied()
        .collect();
    let by_name: HashMap<String, &CatalogChampion> = request
        .champion_catalog
        .iter()
        .filter(|champion| champion.id > 0 && !unavailable.contains(&champion.id))
        .map(|champion| (champion.name.to_ascii_lowercase(), champion))
        .collect();

    let mut result = Vec::new();
    for (index, name) in preferred.iter().enumerate() {
        let catalog = by_name.get(&name.to_ascii_lowercase());
        if !by_name.is_empty() && catalog.is_none() {
            continue;
        }
        result.push(RankedCandidate {
            champion_id: catalog.map(|value| value.id).unwrap_or_default(),
            name: catalog
                .map(|value| value.name.clone())
                .unwrap_or_else(|| (*name).to_string()),
            score: 40.0 - index as f64,
            win_rate: None,
            games: None,
            reason: "Stable role-oriented option from Aura's deterministic local rule set.".into(),
            tradeoff:
                "Local rules cannot measure the current patch, rank bracket, or matchup population."
                    .into(),
        });
    }
    if result.len() < 3 {
        let mut remainder: Vec<&CatalogChampion> = request
            .champion_catalog
            .iter()
            .filter(|champion| champion.id > 0 && !unavailable.contains(&champion.id))
            .filter(|champion| {
                !result
                    .iter()
                    .any(|existing| existing.champion_id == champion.id)
            })
            .collect();
        remainder.sort_by(|left, right| left.name.cmp(&right.name));
        for champion in remainder.into_iter().take(3 - result.len()) {
            result.push(RankedCandidate {
                champion_id: champion.id,
                name: champion.name.clone(),
                score: 20.0,
                win_rate: None,
                games: None,
                reason: "Deterministic catalog fallback; no aggregate performance claim.".into(),
                tradeoff: "Aura has no aggregate evidence for this fallback ordering.".into(),
            });
        }
    }
    result
}

fn local_role_pool(role: &str) -> &'static [&'static str] {
    match role {
        "top" => &["Ornn", "Malphite", "Shen", "Garen", "Cho'Gath"],
        "jungle" => &["Amumu", "Vi", "Warwick", "Nocturne", "Sejuani"],
        "mid" => &["Annie", "Ahri", "Orianna", "Malzahar", "Galio"],
        "bottom" => &["Ashe", "Miss Fortune", "Jinx", "Caitlyn", "Sivir"],
        "support" => &["Leona", "Nautilus", "Janna", "Braum", "Nami"],
        _ => &["Malphite", "Amumu", "Annie", "Ashe", "Leona"],
    }
}

fn fallback_candidate(name: &str, champion_id: u32) -> RankedCandidate {
    RankedCandidate {
        champion_id,
        name: name.into(),
        score: 0.0,
        win_rate: None,
        games: None,
        reason: "No structured candidate data was supplied.".into(),
        tradeoff: "No statistical comparison is available.".into(),
    }
}

fn alternative_from_candidate(
    rank: usize,
    candidate: &RankedCandidate,
    cloud: bool,
) -> AdvisorAlternative {
    AdvisorAlternative {
        rank,
        champion_id: (candidate.champion_id > 0).then_some(candidate.champion_id),
        champion: candidate.name.clone(),
        title: candidate.name.clone(),
        reason: candidate.reason.clone(),
        tradeoff: candidate.tradeoff.clone(),
        confidence: if cloud { 0.55 } else { 0.3 },
        score: round(candidate.score, 3),
        win_rate: candidate.win_rate.map(|value| round(value, 4)),
        sample_size: candidate.games,
    }
}

fn live_response(
    request: &LiveRequest,
    feed: Option<&AggregateFeed>,
    stale: bool,
) -> AdvisorResponse {
    let telemetry = request.telemetry.as_ref();
    let game_time = request
        .game_time
        .or_else(|| telemetry.and_then(|value| value.game_time))
        .unwrap_or(0.0)
        .max(0.0);
    let dragon_at = request
        .dragon_respawn_at
        .or_else(|| telemetry.and_then(|value| value.dragon_respawn_at));
    let baron_at = request
        .baron_respawn_at
        .or_else(|| telemetry.and_then(|value| value.baron_respawn_at));
    let telemetry_age = request
        .telemetry_age_ms
        .or_else(|| telemetry.and_then(|value| value.age_ms));

    let mut priorities: Vec<(i32, String, String)> = Vec::new();
    add_objective_priority(
        &mut priorities,
        "Dragon",
        game_time,
        dragon_at,
        "Set river vision, synchronize recalls, and preserve key cooldowns.",
    );
    add_objective_priority(
        &mut priorities,
        "Baron",
        game_time,
        baron_at,
        "Control top-side vision, fix side waves, and avoid isolated deaths.",
    );
    if request.current_gold.is_some_and(|gold| gold >= 1300) {
        priorities.push((
            88,
            "Convert held gold".into(),
            "Recall on a safe tempo before the next contest; do not carry an unspent item breakpoint."
                .into(),
        ));
    }
    if let (Some(cs), true) = (request.cs, game_time >= 600.0) {
        let cs_per_minute = cs as f64 / (game_time / 60.0);
        if cs_per_minute < 5.0 {
            priorities.push((
                58,
                "Recover safe economy".into(),
                "Collect the nearest safe wave or camp without abandoning the next visible objective."
                    .into(),
            ));
        }
    }
    if request.deaths.is_some_and(|deaths| deaths >= 5) {
        priorities.push((
            82,
            "Reduce isolation risk".into(),
            "Move with vision and an ally; deny the opponent another low-information pick.".into(),
        ));
    }
    if priorities.is_empty() {
        priorities.push((
            50,
            "Stabilize the next map cycle".into(),
            "Fix the nearest wave, refresh vision with teammates, and preserve tempo for the next visible objective."
                .into(),
        ));
    }
    if priorities.len() < 3 {
        priorities.push((
            42,
            "Protect the next information cycle".into(),
            "Refresh only safe vision, track visible opponents, and avoid crossing unseen terrain alone."
                .into(),
        ));
    }
    if priorities.len() < 3 {
        priorities.push((
            38,
            "Maintain teamfight readiness".into(),
            "Keep critical cooldowns available and position within support range of the visible team."
                .into(),
        ));
    }
    priorities.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    priorities.truncate(3);

    let mut reasoning = vec![
        format!("Current visible game clock: {:.0} seconds.", game_time),
        "Priorities are derived only from supplied telemetry and visible champion IDs.".into(),
    ];
    let mut warnings = Vec::new();
    let provenance = if let Some(feed) = feed {
        if let Some(champion_id) = request.common.selected_champion_id {
            let role = normalize_role(&request.common.role);
            if !matches!(role.as_str(), "" | "auto") {
                if let Some(stat) = feed.champions.iter().find(|record| {
                    record.champion_id == champion_id && normalize_role(&record.role) == role
                }) {
                    reasoning.push(format!(
                        "Provider aggregate context for {} in {}: {:.1}% reported win rate across {} games; this does not expose hidden live-game information.",
                        stat.name,
                        role,
                        stat.win_rate * 100.0,
                        stat.games
                    ));
                }
            }
        }
        warnings.push(PROVIDER_SAMPLE_NOTICE.into());
        warnings.extend(provenance_context_warnings(
            &feed.provenance,
            &request.common,
        ));
        if stale {
            warnings.push(
                "Aggregate context is stale because its RAM cache or provider snapshot exceeded the freshness window."
                    .into(),
            );
        }
        feed.provenance.as_cloud_output()
    } else {
        warnings.push(LOCAL_SAMPLE_NOTICE.into());
        AdvisorProvenance::local(&request.common)
    };
    if telemetry_age.is_some_and(|age| age > 5000) {
        warnings.push("Telemetry is more than five seconds old; verify the live state.".into());
    }
    let headline = priorities[0].1.clone();
    let mandate = priorities[0].2.clone();
    let alternatives = priorities
        .iter()
        .skip(1)
        .enumerate()
        .map(|(index, (_, title, action))| AdvisorAlternative {
            rank: index + 2,
            champion_id: None,
            champion: title.clone(),
            title: title.clone(),
            reason: action.clone(),
            tradeoff: "Re-evaluate if the visible objective timer or team position changes.".into(),
            confidence: if feed.is_some() { 0.58 } else { 0.42 },
            score: 0.0,
            win_rate: None,
            sample_size: None,
        })
        .collect();
    AdvisorResponse {
        phase: "live".into(),
        advisory_label: "VISIBLE-STATE PRIORITY".into(),
        headline,
        mandate,
        recommended_champion: None,
        recommended_champion_id: None,
        confidence: if feed.is_some() { 0.62 } else { 0.45 },
        reasoning,
        actions: priorities
            .iter()
            .map(|(_, title, action)| format!("{title}: {action}"))
            .collect(),
        warnings,
        alternatives,
        provenance,
        used_fallback: feed.is_none(),
        policy_notice: POLICY_NOTICE.into(),
    }
}

fn add_objective_priority(
    priorities: &mut Vec<(i32, String, String)>,
    objective: &str,
    game_time: f64,
    respawn_at: Option<f64>,
    action: &str,
) {
    let Some(respawn_at) = respawn_at.filter(|value| value.is_finite() && *value >= 0.0) else {
        return;
    };
    let remaining = (respawn_at - game_time).max(0.0);
    if remaining <= 90.0 {
        let urgency = if remaining <= 30.0 { 100 } else { 90 };
        priorities.push((
            urgency,
            format!("{objective} setup in {:.0}s", remaining),
            action.into(),
        ));
    }
}

fn post_game_response(request: &PostGameRequest) -> AdvisorResponse {
    let latest = request
        .latest_match
        .as_ref()
        .or_else(|| request.recent_matches.first());
    let Some(match_data) = latest else {
        return AdvisorResponse {
            phase: "post_game".into(),
            advisory_label: "LOCAL REVIEW UNAVAILABLE".into(),
            headline: "No completed-match sample was supplied".into(),
            mandate: "Load a recent completed match to receive a personal-data review.".into(),
            recommended_champion: None,
            recommended_champion_id: None,
            confidence: 0.2,
            reasoning: vec![
                "Aura will not invent a post-game breakdown without match evidence.".into(),
            ],
            actions: vec!["Load recent match history and retry.".into()],
            warnings: vec![LOCAL_SAMPLE_NOTICE.into()],
            alternatives: Vec::new(),
            provenance: AdvisorProvenance::personal(&request.common, 0),
            used_fallback: true,
            policy_notice: POLICY_NOTICE.into(),
        };
    };
    let samples: Vec<&MatchSample> = if request.recent_matches.is_empty() {
        vec![match_data]
    } else {
        request.recent_matches.iter().collect()
    };
    let sample_size = samples.len();

    let minutes = (match_data.game_duration_secs as f64 / 60.0).max(1.0);
    let cs_per_minute = match_data.cs as f64 / minutes;
    let vision_per_minute = match_data.vision_score as f64 / minutes;
    let kda = (match_data.kills + match_data.assists) as f64 / match_data.deaths.max(1) as f64;
    let mut findings: Vec<(i32, String, String)> = Vec::new();
    if match_data.deaths >= 7 {
        findings.push((
            100,
            "Lower preventable deaths".into(),
            format!(
                "Review the final 10 seconds before each of the {} deaths and label missing vision, numbers disadvantage, or cooldown misuse.",
                match_data.deaths
            ),
        ));
    }
    if cs_per_minute < 5.5 {
        findings.push((
            80,
            "Improve safe resource collection".into(),
            format!(
                "The supplied match shows {:.1} CS/min; review missed waves and unsafe rotations.",
                cs_per_minute
            ),
        ));
    }
    if vision_per_minute < 0.5 {
        findings.push((
            70,
            "Raise useful vision uptime".into(),
            format!(
                "The supplied match shows {:.2} vision score/min; tie wards to objective and rotation timings.",
                vision_per_minute
            ),
        ));
    }
    if sample_size > 1 {
        let recent_deaths = samples
            .iter()
            .map(|sample| sample.deaths as f64)
            .sum::<f64>()
            / sample_size as f64;
        let recent_cs_per_minute = samples
            .iter()
            .map(|sample| {
                let minutes = (sample.game_duration_secs as f64 / 60.0).max(1.0);
                sample.cs as f64 / minutes
            })
            .sum::<f64>()
            / sample_size as f64;
        let recent_vision_per_minute = samples
            .iter()
            .map(|sample| {
                let minutes = (sample.game_duration_secs as f64 / 60.0).max(1.0);
                sample.vision_score as f64 / minutes
            })
            .sum::<f64>()
            / sample_size as f64;
        let recent_wins = samples.iter().filter(|sample| sample.win).count();
        let trend = if recent_deaths >= 6.0 {
            (
                "Reduce recurring high-death games",
                format!(
                    "Across {sample_size} supplied matches, deaths averaged {recent_deaths:.1}; review the first avoidable death in each replay."
                ),
            )
        } else if recent_cs_per_minute < 5.5 {
            (
                "Raise the recent farming floor",
                format!(
                    "Across {sample_size} supplied matches, farm averaged {recent_cs_per_minute:.1} CS/min; identify repeated missed-wave and rotation costs."
                ),
            )
        } else if recent_vision_per_minute < 0.5 {
            (
                "Improve repeatable vision timing",
                format!(
                    "Across {sample_size} supplied matches, vision averaged {recent_vision_per_minute:.2}/min; review ward timing before visible contests."
                ),
            )
        } else {
            (
                "Preserve the recent baseline",
                format!(
                    "The supplied sample is {recent_wins}-{losses} with {recent_cs_per_minute:.1} CS/min; identify the repeatable setup behind its strongest games.",
                    losses = sample_size - recent_wins
                ),
            )
        };
        findings.push((85, trend.0.into(), trend.1));
    }
    if findings.is_empty() {
        findings.push((
            60,
            "Preserve the strongest repeatable pattern".into(),
            format!(
                "The supplied match produced {:.2} KDA and {:.1} CS/min; identify the decisions that created those outcomes.",
                kda, cs_per_minute
            ),
        ));
    }
    if findings.len() < 3 {
        findings.push((
            45,
            "Review the opening ten minutes".into(),
            "Mark the first lane, pathing, or rotation decision that changed resource or tempo access."
                .into(),
        ));
    }
    if findings.len() < 3 {
        findings.push((
            40,
            "Review objective transitions".into(),
            "If a replay or timeline is available, check whether recalls, waves, vision, and grouping aligned before objective contests."
                .into(),
        ));
    }
    findings.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    findings.truncate(3);
    let headline = findings[0].1.clone();
    let mandate = findings[0].2.clone();
    let alternatives = findings
        .iter()
        .skip(1)
        .enumerate()
        .map(|(index, (_, title, action))| AdvisorAlternative {
            rank: index + 2,
            champion_id: None,
            champion: title.clone(),
            title: title.clone(),
            reason: action.clone(),
            tradeoff: "This finding is limited to the supplied personal match sample.".into(),
            confidence: 0.5,
            score: 0.0,
            win_rate: None,
            sample_size: Some(sample_size as u64),
        })
        .collect();
    AdvisorResponse {
        phase: "post_game".into(),
        advisory_label: "PERSONAL MATCH REVIEW".into(),
        headline,
        mandate,
        recommended_champion: None,
        recommended_champion_id: None,
        confidence: (0.42 + sample_size.min(10) as f64 * 0.025).clamp(0.42, 0.67),
        reasoning: vec![
            format!(
                "Latest supplied {} match: {} / {} / {}, {:.1} CS/min, {:.2} vision/min.",
                value_or(&match_data.champion_name, "champion"),
                match_data.kills,
                match_data.deaths,
                match_data.assists,
                cs_per_minute,
                vision_per_minute
            ),
            format!("Review is limited to {sample_size} locally supplied match record(s)."),
            "No population, hidden-rating, or guaranteed-outcome inference is made.".into(),
        ],
        actions: findings
            .iter()
            .map(|(_, title, action)| format!("{title}: {action}"))
            .collect(),
        warnings: vec![
            "This is a personal-data review, not an aggregate mass-match analysis.".into(),
        ],
        alternatives,
        provenance: AdvisorProvenance::personal(&request.common, sample_size),
        used_fallback: true,
        policy_notice: POLICY_NOTICE.into(),
    }
}

fn provenance_context_warnings(
    provenance: &AdvisorProvenance,
    request: &CommonRequest,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Ok(generated_at) = parse_generated_at(&provenance.generated_at) {
        let age = OffsetDateTime::now_utc() - generated_at;
        if age > time::Duration::hours(72) {
            warnings.push(format!(
                "Provider snapshot age: {} hours. Treat the aggregate evidence as stale until the provider publishes a current snapshot.",
                age.whole_hours()
            ));
        }
    }
    let request_patch = patch_family(&request.patch);
    let provider_patch = patch_family(&provenance.patch);
    if !request_patch.is_empty()
        && !provider_patch.is_empty()
        && !is_all_coverage(&provenance.patch)
        && request_patch != provider_patch
    {
        warnings.push(format!(
            "Context mismatch: the request reports patch {}, while the provider snapshot reports {}; treat the ranking as cross-patch evidence.",
            request.patch, provenance.patch
        ));
    }
    if !request.queue.trim().is_empty()
        && !coverage_matches(
            &provenance.queue,
            &request.queue,
            &["all queues", "any queue"],
        )
    {
        warnings.push(format!(
            "Context mismatch: the request reports {}, while the provider snapshot covers {}; queue context did not affect the ranking.",
            request.queue, provenance.queue
        ));
    }
    if !request.region.trim().is_empty()
        && !coverage_matches(
            &provenance.region,
            &request.region,
            &["global", "worldwide", "all regions", "multi region"],
        )
    {
        warnings.push(format!(
            "Context mismatch: the request reports region {}, while the provider snapshot covers {}; region context did not affect the ranking.",
            request.region, provenance.region
        ));
    }
    if !request.rank_range.trim().is_empty()
        && !rank_coverage_matches(&provenance.rank_range, &request.rank_range)
    {
        warnings.push(format!(
            "Context mismatch: the request reports rank {}, while the provider snapshot covers {}; rank context did not affect the ranking.",
            request.rank_range, provenance.rank_range
        ));
    }
    warnings
}

fn coverage_matches(provider: &str, requested: &str, all_values: &[&str]) -> bool {
    let provider_normalized = normalize_context(provider);
    let requested_normalized = normalize_context(requested);
    provider_normalized == requested_normalized
        || all_values
            .iter()
            .map(|value| normalize_context(value))
            .any(|value| provider_normalized == value)
        || (!requested_normalized.is_empty() && provider_normalized.contains(&requested_normalized))
}

fn rank_coverage_matches(provider: &str, requested: &str) -> bool {
    let provider_normalized = normalize_context(provider);
    if ["allranks", "ironchallenger", "alltier", "alltiers"].contains(&provider_normalized.as_str())
    {
        return true;
    }
    let requested_tier = requested
        .split_whitespace()
        .next()
        .map(normalize_context)
        .unwrap_or_default();
    provider_normalized == normalize_context(requested)
        || (!requested_tier.is_empty() && provider_normalized.contains(&requested_tier))
}

fn patch_family(value: &str) -> String {
    let mut parts = value
        .trim()
        .split('.')
        .filter(|part| !part.is_empty())
        .take(2);
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("{major}.{minor}").to_ascii_lowercase(),
        (Some(single), None) => single.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn is_all_coverage(value: &str) -> bool {
    matches!(
        normalize_context(value).as_str(),
        "all" | "allpatches" | "multipatch"
    )
}

fn normalize_context(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_role(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "adc" | "bot" | "bottom" | "carry" => "bottom".into(),
        "sup" | "support" | "utility" => "support".into(),
        "jg" | "jgl" | "jungle" => "jungle".into(),
        "middle" | "mid" => "mid".into(),
        "top" => "top".into(),
        other => other.to_string(),
    }
}

fn value_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.into()
    } else {
        value.trim().into()
    }
}

fn generated_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("unix:{seconds}")
        })
}

fn round(value: f64, decimals: i32) -> f64 {
    let factor = 10_f64.powi(decimals);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> AdvisorProvenance {
        AdvisorProvenance {
            mode: "aggregate_cloud".into(),
            source: "Example aggregate provider".into(),
            patch: "16.15".into(),
            queue: "Ranked Solo".into(),
            rank_range: "Gold-Challenger".into(),
            region: "global".into(),
            sample_size: 50_000,
            generated_at: "2026-07-29T12:00:00Z".into(),
            sample_size_note: "provider value".into(),
            methodology: "Aggregate champion, matchup, and synergy weighting.".into(),
            methodology_url: Some("https://example.com/methodology".into()),
            source_url: Some("https://example.com/dataset".into()),
            dataset_version: "2026-07-29".into(),
            schema_version: "1".into(),
        }
    }

    fn champion(id: u32, name: &str, score: f64) -> ChampionAggregate {
        ChampionAggregate {
            champion_id: id,
            name: name.into(),
            role: "jungle".into(),
            games: 1_000,
            win_rate: 0.51,
            pick_rate: 0.1,
            ban_rate: 0.02,
            provider_score: Some(score),
            strengths: vec![format!("{name} evidence")],
            tradeoffs: vec![format!("{name} tradeoff")],
        }
    }

    #[test]
    fn endpoint_rejects_credentials_query_fragment_and_remote_http() {
        for invalid in [
            "http://example.com/feed",
            "https://user:pass@example.com/feed",
            "https://example.com/feed?token=secret",
            "https://example.com/feed#fragment",
            "http://localhost:9000/feed",
        ] {
            assert!(
                validate_feed_endpoint(invalid).is_err(),
                "{invalid} must be rejected"
            );
        }
        assert!(validate_feed_endpoint("https://example.com/feed").is_ok());
        assert!(validate_feed_endpoint("http://127.0.0.1:9000/feed").is_ok());
    }

    #[test]
    fn secret_debug_is_always_redacted() {
        let secret = Secret::new("private-token-value".into());
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert!(!format!("{secret:?}").contains("private-token-value"));
    }

    #[test]
    fn feed_requires_complete_provenance() {
        let mut missing_source = provenance();
        missing_source.source.clear();
        assert!(validate_provenance(&missing_source).is_err());

        let mut zero_sample = provenance();
        zero_sample.sample_size = 0;
        assert!(validate_provenance(&zero_sample).is_err());

        let mut missing_reference = provenance();
        missing_reference.source_url = None;
        missing_reference.methodology_url = None;
        assert!(validate_provenance(&missing_reference).is_err());
    }

    #[test]
    fn cloud_ranking_is_deterministic_and_uses_visible_matchups() {
        let feed = AggregateFeed {
            provenance: provenance(),
            champions: vec![
                champion(1, "Alpha", 70.0),
                champion(2, "Beta", 69.0),
                champion(3, "Gamma", 68.0),
            ],
            matchups: vec![MatchupAggregate {
                champion_id: 2,
                enemy_champion_id: 99,
                role: "jungle".into(),
                games: 20_000,
                win_rate: 0.68,
            }],
            synergies: Vec::new(),
        };
        let request = DraftRequest {
            common: CommonRequest {
                role: "jungle".into(),
                enemy_champion_ids: vec![99],
                ..Default::default()
            },
            champion_catalog: Vec::new(),
        };
        let first = rank_cloud_draft(&feed, &request);
        let second = rank_cloud_draft(&feed, &request);
        assert_eq!(first[0].name, "Beta");
        assert_eq!(first[0].name, second[0].name);
        assert_eq!(first[1].name, second[1].name);
    }

    #[test]
    fn cloud_ranking_excludes_supplied_visible_picks() {
        let feed = AggregateFeed {
            provenance: provenance(),
            champions: vec![
                champion(1, "Alpha", 80.0),
                champion(2, "Beta", 70.0),
                champion(3, "Gamma", 60.0),
                champion(4, "Delta", 50.0),
            ],
            matchups: Vec::new(),
            synergies: Vec::new(),
        };
        let request = DraftRequest {
            common: CommonRequest {
                role: "jungle".into(),
                ally_champion_ids: vec![1],
                enemy_champion_ids: vec![2],
                ..Default::default()
            },
            champion_catalog: Vec::new(),
        };
        let ranked = rank_cloud_draft(&feed, &request);
        assert_eq!(
            ranked
                .iter()
                .map(|entry| entry.champion_id)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn local_fallback_discloses_zero_aggregate_sample_and_alternatives() {
        let request = DraftRequest {
            common: CommonRequest {
                role: "support".into(),
                patch: "16.15".into(),
                ..Default::default()
            },
            champion_catalog: Vec::new(),
        };
        let response = local_draft_response(&request, None);
        assert!(response.used_fallback);
        assert_eq!(response.provenance.sample_size, 0);
        assert!(response
            .provenance
            .sample_size_note
            .contains("not aggregate"));
        assert_eq!(response.alternatives.len(), 2);
        assert!(!response.mandate.to_ascii_lowercase().contains("guarantee"));
    }

    #[test]
    fn provider_sample_is_always_labeled_unverified() {
        let output = provenance().as_cloud_output();
        assert!(output
            .sample_size_note
            .contains("not independently verified"));
    }

    #[test]
    fn nullable_frontend_context_fields_decode_safely() {
        let request: DraftRequest = serde_json::from_value(serde_json::json!({
            "role": "auto",
            "patch": null,
            "region": null,
            "queue_id": null,
            "queue": null,
            "rank_range": null,
            "gameflow_phase": null,
            "context_captured_at": "2026-07-29T12:00:00Z",
            "champion_catalog": []
        }))
        .expect("nullable optional frontend context should decode");

        assert!(request.common.patch.is_empty());
        assert!(request.common.queue.is_empty());
        assert!(request.common.region.is_empty());

        let post: PostGameRequest = serde_json::from_value(serde_json::json!({
            "patch": null,
            "queue": null,
            "rank_range": null,
            "latest_match": {
                "match_id": null,
                "game_mode": null,
                "champion_name": null
            },
            "recent_matches": []
        }))
        .expect("nullable match strings should decode");
        let latest = post.latest_match.expect("latest match should be present");
        assert!(latest.match_id.is_empty());
        assert!(latest.game_mode.is_empty());
        assert!(latest.champion_name.is_empty());
    }

    #[test]
    fn post_game_review_returns_two_alternative_priorities() {
        let request = PostGameRequest {
            common: CommonRequest::default(),
            latest_match: Some(MatchSample {
                game_duration_secs: 1_800,
                champion_name: "Example".into(),
                win: true,
                kills: 10,
                deaths: 2,
                assists: 8,
                cs: 210,
                vision_score: 25,
                ..Default::default()
            }),
            recent_matches: Vec::new(),
        };
        let response = post_game_response(&request);
        assert_eq!(response.alternatives.len(), 2);
        assert_eq!(response.provenance.mode, "local_personal_review");
    }

    #[test]
    fn selected_local_candidate_is_not_excluded_with_other_visible_picks() {
        let feed = AggregateFeed {
            provenance: provenance(),
            champions: vec![
                champion(1, "Alpha", 80.0),
                champion(2, "Beta", 70.0),
                champion(3, "Gamma", 60.0),
                champion(4, "Delta", 50.0),
            ],
            matchups: Vec::new(),
            synergies: Vec::new(),
        };
        let request = DraftRequest {
            common: CommonRequest {
                role: "jungle".into(),
                selected_champion_id: Some(1),
                ally_champion_ids: vec![1],
                enemy_champion_ids: vec![2],
                ..Default::default()
            },
            champion_catalog: Vec::new(),
        };

        let ranked = rank_cloud_draft(&feed, &request);
        assert_eq!(ranked[0].champion_id, 1);
        assert!(!ranked.iter().any(|entry| entry.champion_id == 2));
    }

    #[test]
    fn feed_rejects_duplicate_normalized_roles_and_matchup_rows() {
        let mut duplicate_role = champion(1, "Alpha duplicate", 60.0);
        duplicate_role.role = "jgl".into();
        let duplicate_champion_feed = AggregateFeed {
            provenance: provenance(),
            champions: vec![
                champion(1, "Alpha", 80.0),
                duplicate_role,
                champion(2, "Beta", 70.0),
            ],
            matchups: Vec::new(),
            synergies: Vec::new(),
        };
        assert!(validate_feed(&duplicate_champion_feed).is_err());

        let matchup = MatchupAggregate {
            champion_id: 1,
            enemy_champion_id: 99,
            role: "jungle".into(),
            games: 500,
            win_rate: 0.54,
        };
        let duplicate_matchup_feed = AggregateFeed {
            provenance: provenance(),
            champions: vec![
                champion(1, "Alpha", 80.0),
                champion(2, "Beta", 70.0),
                champion(3, "Gamma", 60.0),
            ],
            matchups: vec![matchup.clone(), matchup],
            synergies: Vec::new(),
        };
        assert!(validate_feed(&duplicate_matchup_feed).is_err());
    }

    #[test]
    fn feed_rejects_row_counts_above_reported_sample() {
        let mut provider = provenance();
        provider.sample_size = 100;
        let feed = AggregateFeed {
            provenance: provider,
            champions: vec![
                champion(1, "Alpha", 80.0),
                champion(2, "Beta", 70.0),
                champion(3, "Gamma", 60.0),
            ],
            matchups: Vec::new(),
            synergies: Vec::new(),
        };
        assert!(validate_feed(&feed).is_err());
    }

    #[test]
    fn provenance_context_mismatches_are_explicit() {
        let mut provider = provenance();
        provider.patch = "16.14".into();
        provider.queue = "Ranked Solo".into();
        provider.region = "EUW".into();
        provider.rank_range = "Diamond".into();
        provider.generated_at = generated_now();
        let request = CommonRequest {
            patch: "16.15.1".into(),
            queue: "ARAM".into(),
            region: "EUNE".into(),
            rank_range: "Gold IV".into(),
            ..Default::default()
        };

        let warnings = provenance_context_warnings(&provider, &request);
        assert_eq!(warnings.len(), 4);
        assert!(warnings.iter().all(|warning| warning.contains("mismatch")));
    }

    #[test]
    fn local_development_feed_mode_remains_truthful() {
        let mut provider = provenance();
        provider.mode = "local_development_feed".into();
        assert_eq!(provider.as_cloud_output().mode, "local_development_feed");
    }

    #[test]
    fn recent_matches_influence_post_game_findings() {
        let repeated = MatchSample {
            game_duration_secs: 1_800,
            champion_name: "Example".into(),
            deaths: 8,
            cs: 150,
            vision_score: 8,
            ..Default::default()
        };
        let request = PostGameRequest {
            common: CommonRequest::default(),
            latest_match: Some(repeated.clone()),
            recent_matches: vec![repeated.clone(), repeated.clone(), repeated],
        };
        let response = post_game_response(&request);
        let output = response.actions.join(" ");
        assert!(output.contains("Across 3 supplied matches"));
        assert_eq!(response.provenance.sample_size, 3);
    }

    #[test]
    fn local_generation_time_is_rfc3339() {
        assert!(parse_generated_at(&generated_now()).is_ok());
    }

    #[test]
    fn live_analysis_requires_active_fresh_telemetry() {
        let mut request = LiveRequest::default();
        assert!(validate_live_request(&request).is_err());

        request.common.gameflow_phase = "InProgress".into();
        request.game_time = Some(60.0);
        request.telemetry_received_at_ms = Some(1);
        request.telemetry_age_ms = Some(20_000);
        assert!(validate_live_request(&request).is_err());

        request.telemetry_age_ms = Some(500);
        assert!(validate_live_request(&request).is_ok());
    }
}
