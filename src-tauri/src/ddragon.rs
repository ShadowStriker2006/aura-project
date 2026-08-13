use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Debug)]
pub enum DDragonError {
    FetchFailed(String, String),
    DecodeFailed(String, String),
    EmptyVersionList,
}
impl std::fmt::Display for DDragonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DDragonError::FetchFailed(what, reason) => {
                write!(f, "{} fetch failed: {}", what, reason)
            }
            DDragonError::DecodeFailed(what, reason) => {
                write!(f, "{} decode failed: {}", what, reason)
            }
            DDragonError::EmptyVersionList => write!(f, "versions.json returned an empty list"),
        }
    }
}
impl std::error::Error for DDragonError {}

fn log_ok(msg: &str) {
    println!("[AURA::DDRAGON][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::DDRAGON][ERR] {}", msg);
}

#[derive(Debug, Clone, Default)]
pub struct MetadataMaps {
    pub champions_by_id: HashMap<String, String>, // "266" -> "Aatrox" (display name)
    pub image_id_by_champion_id: HashMap<String, String>, // "62" -> "MonkeyKing" (DDragon's internal id — NOT always the same as display name, e.g. Wukong)
    pub items_by_id: HashMap<String, String>,             // "3031" -> "Infinity Edge"
    pub rune_trees: Vec<RuneTree>,
    pub version: String,
}

#[derive(Clone, Default)]
pub struct DDragonCache(
    pub Arc<RwLock<Option<MetadataMaps>>>,
    Arc<RwLock<HashMap<String, ChampionDetail>>>,
);

#[derive(Deserialize)]
struct ChampionFile {
    data: HashMap<String, ChampionEntry>,
}
#[derive(Deserialize)]
struct ChampionEntry {
    key: String, // confirmed: numeric id, but serialized as a JSON string
    name: String,
}

#[derive(Deserialize)]
struct ItemFile {
    data: HashMap<String, ItemEntry>,
}
#[derive(Deserialize)]
struct ItemEntry {
    name: String,
    #[serde(default)]
    maps: HashMap<String, bool>,
    #[serde(default, rename = "inStore")]
    in_store: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuneTree {
    pub id: u32,
    pub key: String,
    pub icon: String,
    pub name: String,
    pub slots: Vec<RuneSlot>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuneSlot {
    pub runes: Vec<RuneEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuneEntry {
    pub id: u32,
    pub key: String,
    pub icon: String,
    pub name: String,
    #[serde(rename = "shortDesc")]
    pub short_desc: String,
    #[serde(rename = "longDesc")]
    pub long_desc: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChampionAbility {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChampionDetail {
    pub id: String,
    pub name: String,
    pub title: String,
    pub lore: String,
    pub tags: Vec<String>,
    pub partype: String,
    pub passive: ChampionAbility,
    pub spells: Vec<ChampionAbility>,
    pub stats: HashMap<String, f64>,
    pub info: ChampionInfo,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ChampionInfo {
    pub attack: u8,
    pub defense: u8,
    pub magic: u8,
    pub difficulty: u8,
}

#[derive(Deserialize)]
struct ChampionDetailFile {
    data: HashMap<String, ChampionDetail>,
}

/// Fetches the current patch's champion/item name maps straight into memory —
/// no local cache file, re-fetched fresh every launch.
pub async fn refresh(cache: &DDragonCache) -> Result<(), DDragonError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|e| DDragonError::FetchFailed("http client".into(), e.to_string()))?; // trusted public HTTPS

    let versions: Vec<String> = client
        .get("https://ddragon.leagueoflegends.com/api/versions.json")
        .header("Accept-Charset", "utf-8")
        .send()
        .await
        .map_err(|e| DDragonError::FetchFailed("versions.json".into(), e.to_string()))?
        .json()
        .await
        .map_err(|e| DDragonError::DecodeFailed("versions.json".into(), e.to_string()))?;

    let version = versions
        .first()
        .cloned()
        .ok_or(DDragonError::EmptyVersionList)?;
    log_ok(&format!("current patch resolved: {}", version));

    let champ_file: ChampionFile = client
        .get(format!(
            "https://ddragon.leagueoflegends.com/cdn/{}/data/en_US/champion.json",
            version
        ))
        .header("Accept-Charset", "utf-8")
        .send()
        .await
        .map_err(|e| DDragonError::FetchFailed("champion.json".into(), e.to_string()))?
        .json()
        .await
        .map_err(|e| DDragonError::DecodeFailed("champion.json".into(), e.to_string()))?;

    // Keyed by name with numeric id nested in "key" — invert it so id-based
    // lookups (what LCU/telemetry actually hand us) are O(1). The outer JSON
    // key itself is DDragon's *internal* id string, used for image paths —
    // capturing it here rather than discarding it, since it's not always the
    // same as the display name (Wukong's internal id is "MonkeyKing").
    let mut champions_by_id: HashMap<String, String> = HashMap::new();
    let mut image_id_by_champion_id: HashMap<String, String> = HashMap::new();
    for (ddragon_image_id, entry) in champ_file.data.into_iter() {
        image_id_by_champion_id.insert(entry.key.clone(), ddragon_image_id);
        champions_by_id.insert(entry.key, entry.name);
    }

    let item_file: ItemFile = client
        .get(format!(
            "https://ddragon.leagueoflegends.com/cdn/{}/data/en_US/item.json",
            version
        ))
        .header("Accept-Charset", "utf-8")
        .send()
        .await
        .map_err(|e| DDragonError::FetchFailed("item.json".into(), e.to_string()))?
        .json()
        .await
        .map_err(|e| DDragonError::DecodeFailed("item.json".into(), e.to_string()))?;

    // Already keyed by id directly — no inversion needed here.
    let items_by_id: HashMap<String, String> = item_file
        .data
        .into_iter()
        .filter(|(_, entry)| {
            entry.maps.get("11").copied().unwrap_or(false) && entry.in_store != Some(false)
        })
        .map(|(id, entry)| (id, entry.name))
        .collect();

    let rune_trees: Vec<RuneTree> = client
        .get(format!(
            "https://ddragon.leagueoflegends.com/cdn/{}/data/en_US/runesReforged.json",
            version
        ))
        .header("Accept-Charset", "utf-8")
        .send()
        .await
        .map_err(|e| DDragonError::FetchFailed("runesReforged.json".into(), e.to_string()))?
        .json()
        .await
        .map_err(|e| DDragonError::DecodeFailed("runesReforged.json".into(), e.to_string()))?;

    log_ok(&format!(
        "loaded {} champions, {} items, {} rune trees for patch {}",
        champions_by_id.len(),
        items_by_id.len(),
        rune_trees.len(),
        version
    ));
    *cache.0.write().await = Some(MetadataMaps {
        champions_by_id,
        image_id_by_champion_id,
        items_by_id,
        rune_trees,
        version,
    });
    Ok(())
}

/// Bounded retries with backoff — a single failed attempt at launch (e.g. app
/// starts before Wi-Fi reconnects) shouldn't leave the whole session without
/// name mappings. Gives up after 5 tries rather than hammering the endpoint forever.
pub async fn run_with_retry(cache: DDragonCache) {
    let mut attempt: u32 = 0;
    loop {
        match refresh(&cache).await {
            Ok(_) => {
                log_ok("metadata cache ready");
                return;
            }
            Err(e) => {
                attempt += 1;
                log_err(&format!("attempt {} failed: {}", attempt, e));
                if attempt >= 5 {
                    log_err(
                        "giving up after 5 attempts — champion/item names unavailable this session",
                    );
                    return;
                }
                tokio::time::sleep(Duration::from_secs((attempt * 5) as u64)).await;
            }
        }
    }
}

#[allow(dead_code)]
pub async fn champion_name(cache: &DDragonCache, id: &str) -> Option<String> {
    cache
        .0
        .read()
        .await
        .as_ref()?
        .champions_by_id
        .get(id)
        .cloned()
}

#[allow(dead_code)]
pub async fn item_name(cache: &DDragonCache, id: &str) -> Option<String> {
    cache.0.read().await.as_ref()?.items_by_id.get(id).cloned()
}

// Frontend fetches these ONCE and does local lookups from then on — no
// per-id IPC round trips needed.
#[tauri::command]
pub async fn get_champion_map(
    cache: tauri::State<'_, DDragonCache>,
) -> Result<HashMap<String, String>, String> {
    cache
        .0
        .read()
        .await
        .as_ref()
        .map(|m| m.champions_by_id.clone())
        .ok_or_else(|| "metadata cache not ready yet".to_string())
}

#[tauri::command]
pub async fn get_item_map(
    cache: tauri::State<'_, DDragonCache>,
) -> Result<HashMap<String, String>, String> {
    cache
        .0
        .read()
        .await
        .as_ref()
        .map(|m| m.items_by_id.clone())
        .ok_or_else(|| "metadata cache not ready yet".to_string())
}

/// Exposes the resolved patch version so the frontend can build image URLs
/// (champion squares, item icons, profile icons) against the SAME version the
/// name maps came from, instead of doing its own separate versions.json fetch
/// that could theoretically race to a different, mismatched version.
#[tauri::command]
pub async fn get_ddragon_version(cache: tauri::State<'_, DDragonCache>) -> Result<String, String> {
    cache
        .0
        .read()
        .await
        .as_ref()
        .map(|m| m.version.clone())
        .ok_or_else(|| "metadata cache not ready yet".to_string())
}

/// Numeric champion id -> DDragon's internal image id (NOT always the same as
/// the display name from get_champion_map — e.g. Wukong's is "MonkeyKing").
/// Needed to build correct champion square/splash URLs.
#[tauri::command]
pub async fn get_champion_image_id_map(
    cache: tauri::State<'_, DDragonCache>,
) -> Result<HashMap<String, String>, String> {
    cache
        .0
        .read()
        .await
        .as_ref()
        .map(|m| m.image_id_by_champion_id.clone())
        .ok_or_else(|| "metadata cache not ready yet".to_string())
}

#[tauri::command]
pub async fn get_rune_trees(
    cache: tauri::State<'_, DDragonCache>,
) -> Result<Vec<RuneTree>, String> {
    cache
        .0
        .read()
        .await
        .as_ref()
        .map(|metadata| metadata.rune_trees.clone())
        .ok_or_else(|| "metadata cache not ready yet".to_string())
}

#[tauri::command]
pub async fn get_champion_details(
    cache: tauri::State<'_, DDragonCache>,
    champion_image_id: String,
) -> Result<ChampionDetail, String> {
    if champion_image_id.is_empty()
        || champion_image_id.len() > 40
        || !champion_image_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err("invalid champion identifier".into());
    }

    if let Some(details) = cache.1.read().await.get(&champion_image_id).cloned() {
        return Ok(details);
    }

    let version = cache
        .0
        .read()
        .await
        .as_ref()
        .map(|metadata| metadata.version.clone())
        .ok_or_else(|| "metadata cache not ready yet".to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(format!(
            "https://ddragon.leagueoflegends.com/cdn/{version}/data/en_US/champion/{champion_image_id}.json"
        ))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "champion details request timed out".to_string()
            } else if error.is_connect() {
                "could not connect to Riot's game-data service".to_string()
            } else {
                "champion details network request failed".to_string()
            }
        })?
        .error_for_status()
        .map_err(|error| {
            let status = error.status().map(|value| value.as_u16()).unwrap_or(0);
            format!("Riot's game-data service returned HTTP {status}")
        })?;
    let mut file: ChampionDetailFile = response
        .json()
        .await
        .map_err(|error| format!("champion details decode failed: {error}"))?;
    let details = file
        .data
        .remove(&champion_image_id)
        .ok_or_else(|| "champion details were missing from the response".to_string())?;
    cache
        .1
        .write()
        .await
        .insert(champion_image_id, details.clone());
    Ok(details)
}
