pub mod lockfile;
pub mod rest;
pub mod watcher;

use crate::perf::process_guard;
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalRiotAccount {
    pub game_name: String,
    pub tag_line: String,
    pub puuid: String,
    pub platform: String,
    pub profile_icon_id: Option<u32>,
    pub summoner_level: Option<u64>,
}

#[derive(Clone, Default)]
pub struct LocalRiotAccountState {
    account: Arc<RwLock<Option<LocalRiotAccount>>>,
}

impl LocalRiotAccountState {
    fn replace(&self, account: LocalRiotAccount) -> bool {
        let Ok(mut current) = self.account.write() else {
            log_err("local Riot account state lock failed");
            return false;
        };
        let changed = current.as_ref() != Some(&account);
        *current = Some(account);
        changed
    }

    fn get(&self) -> Option<LocalRiotAccount> {
        self.account.read().ok().and_then(|value| value.clone())
    }
}

#[tauri::command]
pub fn get_local_riot_account(
    state: tauri::State<'_, LocalRiotAccountState>,
) -> Option<LocalRiotAccount> {
    state.get()
}

fn log_ok(msg: &str) {
    println!("[AURA::LCU][OK] {msg}");
}

fn log_err(msg: &str) {
    eprintln!("[AURA::LCU][ERR] {msg}");
}

fn string_field(sources: &[&Value], names: &[&str]) -> Option<String> {
    sources.iter().find_map(|source| {
        names.iter().find_map(|name| {
            source
                .get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    })
}

fn number_field(sources: &[&Value], names: &[&str]) -> Option<u64> {
    sources.iter().find_map(|source| {
        names
            .iter()
            .find_map(|name| source.get(name).and_then(Value::as_u64))
    })
}

fn normalize_platform(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "na" | "na1" => Some("na1"),
        "euw" | "euw1" => Some("euw1"),
        "eun" | "eune" | "eun1" => Some("eun1"),
        "kr" => Some("kr"),
        "jp" | "jp1" => Some("jp1"),
        "br" | "br1" => Some("br1"),
        "lan" | "la1" => Some("la1"),
        "las" | "la2" => Some("la2"),
        "oce" | "oc1" | "oc" => Some("oc1"),
        "tr" | "tr1" => Some("tr1"),
        "ru" => Some("ru"),
        "ph" | "ph2" => Some("ph2"),
        "sg" | "sg2" => Some("sg2"),
        "th" | "th2" => Some("th2"),
        "tw" | "tw2" => Some("tw2"),
        "vn" | "vn2" => Some("vn2"),
        _ => None,
    }
}

fn split_riot_id(value: &str) -> Option<(String, String)> {
    let (game_name, tag_line) = value.rsplit_once('#')?;
    let game_name = game_name.trim();
    let tag_line = tag_line.trim();
    if game_name.is_empty() || tag_line.is_empty() {
        return None;
    }
    Some((game_name.to_string(), tag_line.to_string()))
}

fn parse_local_account(
    summoner: &Value,
    chat: &Value,
    region_locale: &Value,
) -> Result<LocalRiotAccount, String> {
    let identity_sources = [summoner, chat];
    let mut game_name = string_field(&identity_sources, &["gameName"]);
    let mut tag_line = string_field(&identity_sources, &["tagLine", "gameTag"]);

    if game_name.is_none() || tag_line.is_none() {
        if let Some((parsed_name, parsed_tag)) = string_field(
            &identity_sources,
            &["riotId", "riotID", "displayName", "name"],
        )
        .and_then(|value| split_riot_id(&value))
        {
            game_name.get_or_insert(parsed_name);
            tag_line.get_or_insert(parsed_tag);
        }
    }

    let game_name =
        game_name.ok_or_else(|| "League Client did not expose a Riot game name".to_string())?;
    let tag_line =
        tag_line.ok_or_else(|| "League Client did not expose a Riot tag line".to_string())?;
    let puuid = string_field(&identity_sources, &["puuid"])
        .ok_or_else(|| "League Client did not expose the current account PUUID".to_string())?;
    let platform = string_field(
        &[region_locale, summoner, chat],
        &["region", "webRegion", "rsoPlatformId", "platformId"],
    )
    .and_then(|value| normalize_platform(&value).map(str::to_string))
    .ok_or_else(|| "League Client region could not be mapped to a Riot platform".to_string())?;

    Ok(LocalRiotAccount {
        game_name,
        tag_line,
        puuid,
        platform,
        profile_icon_id: number_field(&identity_sources, &["profileIconId"])
            .and_then(|value| u32::try_from(value).ok()),
        summoner_level: number_field(&identity_sources, &["summonerLevel"]),
    })
}

async fn discover_local_account(
    creds: &lockfile::LcuCredentials,
) -> Result<LocalRiotAccount, String> {
    let (summoner, chat, region_locale) = tokio::join!(
        rest::lcu_request(
            creds,
            Method::GET,
            "/lol-summoner/v1/current-summoner",
            None
        ),
        rest::lcu_request(creds, Method::GET, "/lol-chat/v1/me", None),
        rest::lcu_request(creds, Method::GET, "/riotclient/region-locale", None),
    );

    let summoner_error = summoner.as_ref().err().map(ToString::to_string);
    let chat_error = chat.as_ref().err().map(ToString::to_string);
    let region_error = region_locale.as_ref().err().map(ToString::to_string);
    let summoner = summoner.unwrap_or(Value::Null);
    let chat = chat.unwrap_or(Value::Null);
    let region_locale = region_locale.unwrap_or(Value::Null);

    parse_local_account(&summoner, &chat, &region_locale).map_err(|parse_error| {
        let endpoint_errors = [summoner_error, chat_error, region_error]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ");
        if endpoint_errors.is_empty() {
            parse_error
        } else {
            format!("{parse_error}; local endpoint errors: {endpoint_errors}")
        }
    })
}

async fn discover_and_publish_local_account(
    app: &AppHandle,
    creds: &lockfile::LcuCredentials,
) -> bool {
    match discover_local_account(creds).await {
        Ok(account) => {
            let state = app.state::<LocalRiotAccountState>();
            let changed = state.replace(account.clone());
            if changed {
                log_ok("current Riot account discovered from League Client");
            }
            if let Err(error) = app.emit("lcu-current-summoner", &account) {
                log_err(&format!("emit current account failed: {error}"));
            }
            true
        }
        Err(error) => {
            log_err(&format!("local account discovery failed: {error}"));
            false
        }
    }
}

/// Re-checks the process table (zero disk I/O) every cycle and re-reads the
/// lockfile on each connection attempt. A restarted League Client always gets
/// a fresh port and password instead of a stale cached credential.
pub async fn run_lcu_supervisor(app: AppHandle) {
    loop {
        if process_guard::find_pid_by_name("LeagueClientUx.exe").is_ok() {
            sleep(Duration::from_millis(500)).await;
            match lockfile::read_credentials() {
                Ok(creds) => {
                    log_ok(&format!(
                        "client detected (pid={}), discovering local account and gameflow",
                        creds.pid
                    ));

                    if !discover_and_publish_local_account(&app, &creds).await {
                        let retry_app = app.clone();
                        let retry_creds = creds.clone();
                        tauri::async_runtime::spawn(async move {
                            for _ in 0..10 {
                                sleep(Duration::from_secs(3)).await;
                                if discover_and_publish_local_account(&retry_app, &retry_creds)
                                    .await
                                {
                                    break;
                                }
                            }
                        });
                    }

                    match rest::lcu_request(
                        &creds,
                        Method::GET,
                        "/lol-gameflow/v1/gameflow-phase",
                        None,
                    )
                    .await
                    {
                        Ok(phase) => {
                            if let Err(error) = app.emit("lcu-initial-phase", &phase) {
                                log_err(&format!("emit initial phase failed: {error}"));
                            }
                            if let Some(status) = phase
                                .as_str()
                                .and_then(crate::live_client::GameStatus::from_lcu_phase)
                            {
                                if let Err(error) =
                                    crate::live_client::stream_game_status(&app, status)
                                {
                                    log_err(&error);
                                }
                            }
                        }
                        Err(error) => log_err(&format!("initial phase fetch failed: {error}")),
                    }

                    log_ok("entering watcher");
                    if let Err(error) = watcher::run_once(app.clone(), &creds).await {
                        log_err(&format!("watcher exited: {error}"));
                    }
                }
                Err(error) => log_err(&format!("lockfile read failed post-detection: {error}")),
            }
        }
        sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn combines_tolerant_summoner_chat_and_region_shapes() {
        let summoner = json!({
            "puuid": "test-puuid-value",
            "profileIconId": 29,
            "summonerLevel": 321
        });
        let chat = json!({ "gameName": "ShadowStriker", "gameTag": "1386" });
        let region = json!({ "region": "EUNE" });

        let account = parse_local_account(&summoner, &chat, &region)
            .expect("account response should combine without requiring an id field");

        assert_eq!(account.game_name, "ShadowStriker");
        assert_eq!(account.tag_line, "1386");
        assert_eq!(account.platform, "eun1");
        assert_eq!(account.profile_icon_id, Some(29));
        assert_eq!(account.summoner_level, Some(321));
    }

    #[test]
    fn parses_composite_riot_id_and_direct_platform_id() {
        let account = parse_local_account(
            &json!({ "displayName": "Player Name#EUW", "puuid": "p" }),
            &Value::Null,
            &json!({ "rsoPlatformId": "EUW1" }),
        )
        .expect("composite Riot ID should be supported");

        assert_eq!(account.game_name, "Player Name");
        assert_eq!(account.tag_line, "EUW");
        assert_eq!(account.platform, "euw1");
    }
}
