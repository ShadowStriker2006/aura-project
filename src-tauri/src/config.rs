use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use url::Url;

use crate::advisor::AdvisorConfig;
use crate::credentials;
use crate::riotapi::RiotApiState;

const DEFAULT_SPOTIFY_CLIENT_ID: &str = "681dfe3599314fd2adde1cd53ab731a8";
const DEFAULT_SPOTIFY_REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
const DEFAULT_SPOTIFY_SCOPES: &str = "streaming user-read-email user-read-private user-read-playback-state user-modify-playback-state";
const REQUIRED_SPOTIFY_SCOPES: [&str; 5] = [
    "streaming",
    "user-read-email",
    "user-read-private",
    "user-read-playback-state",
    "user-modify-playback-state",
];

#[derive(Debug, Clone)]
pub struct SpotifyConfig {
    pub client_id: String,
    pub redirect_uri: String,
    pub callback_port: u16,
    pub callback_path: String,
    pub scopes: Vec<String>,
}

impl SpotifyConfig {
    fn from_values(
        client_id: String,
        redirect_uri: String,
        scopes: String,
    ) -> Result<Self, String> {
        if client_id.len() != 32 || !client_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(
                "SPOTIFY_CLIENT_ID must be the 32-character client ID from the Spotify dashboard"
                    .into(),
            );
        }

        let parsed = Url::parse(&redirect_uri)
            .map_err(|_| "SPOTIFY_REDIRECT_URI is not a valid URL".to_string())?;
        if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
            return Err(
                "SPOTIFY_REDIRECT_URI must use an http://127.0.0.1 loopback address".into(),
            );
        }
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(
                "SPOTIFY_REDIRECT_URI cannot contain credentials, a query, or a fragment".into(),
            );
        }
        let callback_port = parsed
            .port()
            .ok_or_else(|| "SPOTIFY_REDIRECT_URI must include an explicit port".to_string())?;
        if callback_port < 1024 {
            return Err("SPOTIFY_REDIRECT_URI port must be 1024 or higher".into());
        }
        let callback_path = parsed.path().to_string();
        if callback_path.is_empty() || callback_path == "/" {
            return Err("SPOTIFY_REDIRECT_URI must include a callback path".into());
        }

        let mut unique = BTreeSet::new();
        for scope in scopes.split_ascii_whitespace() {
            if !scope
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(format!("SPOTIFY_SCOPES contains an invalid scope: {scope}"));
            }
            unique.insert(scope.to_string());
        }
        for required in REQUIRED_SPOTIFY_SCOPES {
            if !unique.contains(required) {
                return Err(format!("SPOTIFY_SCOPES must include {required}"));
            }
        }

        Ok(Self {
            client_id,
            redirect_uri,
            callback_port,
            callback_path,
            scopes: unique.into_iter().collect(),
        })
    }

    pub fn scopes_string(&self) -> String {
        self.scopes.join(" ")
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub riot_api_key: Option<String>,
    pub riot_api_source: String,
    pub spotify: Result<SpotifyConfig, String>,
    pub advisor: AdvisorConfig,
}

impl RuntimeConfig {
    pub fn from_environment() -> Self {
        let environment_key = non_empty_env("RIOT_API_KEY");
        let (riot_api_key, riot_api_source) = match environment_key {
            Some(key) => (Some(key), "Environment variable".to_string()),
            None => match credentials::load_riot_api_key() {
                Ok(Some(key)) => (Some(key), "Windows Credential Manager".to_string()),
                Ok(None) => (None, "Not configured".to_string()),
                Err(error) => {
                    eprintln!("[AURA::CONFIG][ERR] {error}");
                    (None, "Credential store unavailable".to_string())
                }
            },
        };
        let client_id = non_empty_env("SPOTIFY_CLIENT_ID")
            .unwrap_or_else(|| DEFAULT_SPOTIFY_CLIENT_ID.to_string());
        let redirect_uri = non_empty_env("SPOTIFY_REDIRECT_URI")
            .unwrap_or_else(|| DEFAULT_SPOTIFY_REDIRECT_URI.to_string());
        let scopes =
            non_empty_env("SPOTIFY_SCOPES").unwrap_or_else(|| DEFAULT_SPOTIFY_SCOPES.to_string());
        let advisor = AdvisorConfig::from_environment();

        Self {
            riot_api_key,
            riot_api_source,
            spotify: SpotifyConfig::from_values(client_id, redirect_uri, scopes),
            advisor,
        }
    }

    pub fn status(&self) -> IntegrationConfigStatus {
        let advisor_configured = self.advisor.is_configured();
        let advisor_mode = self.advisor.safe_mode().to_string();
        let advisor_error = self.advisor.safe_error();
        match &self.spotify {
            Ok(spotify) => IntegrationConfigStatus {
                riot_api_configured: self.riot_api_key.is_some(),
                riot_api_source: self.riot_api_source.clone(),
                spotify_configured: true,
                spotify_redirect_uri: spotify.redirect_uri.clone(),
                spotify_scopes: spotify.scopes.clone(),
                spotify_error: None,
                advisor_configured,
                advisor_mode,
                advisor_error,
            },
            Err(error) => IntegrationConfigStatus {
                riot_api_configured: self.riot_api_key.is_some(),
                riot_api_source: self.riot_api_source.clone(),
                spotify_configured: false,
                spotify_redirect_uri: String::new(),
                spotify_scopes: Vec::new(),
                spotify_error: Some(error.clone()),
                advisor_configured,
                advisor_mode,
                advisor_error,
            },
        }
    }

    pub fn log_safe_summary(&self) {
        println!(
            "[AURA::CONFIG][OK] Riot API credential configured: {}",
            self.riot_api_key.is_some()
        );
        match &self.spotify {
            Ok(config) => println!(
                "[AURA::CONFIG][OK] Spotify OAuth configured: redirect={}, scopes={}",
                config.redirect_uri,
                config.scopes_string()
            ),
            Err(error) => eprintln!("[AURA::CONFIG][ERR] Spotify OAuth disabled: {error}"),
        }
        self.advisor.log_safe_summary();
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationConfigStatus {
    pub riot_api_configured: bool,
    pub riot_api_source: String,
    pub spotify_configured: bool,
    pub spotify_redirect_uri: String,
    pub spotify_scopes: Vec<String>,
    pub spotify_error: Option<String>,
    pub advisor_configured: bool,
    pub advisor_mode: String,
    pub advisor_error: Option<String>,
}

#[derive(Clone)]
pub struct ConfigState {
    status: Arc<RwLock<IntegrationConfigStatus>>,
    environment_key: Option<String>,
}

impl ConfigState {
    pub fn new(status: IntegrationConfigStatus) -> Self {
        Self {
            status: Arc::new(RwLock::new(status)),
            environment_key: non_empty_env("RIOT_API_KEY"),
        }
    }

    fn set_riot_status(&self, configured: bool, source: &str) {
        if let Ok(mut status) = self.status.write() {
            status.riot_api_configured = configured;
            status.riot_api_source = source.to_string();
        }
    }
}

#[tauri::command]
pub fn get_integration_config(state: tauri::State<'_, ConfigState>) -> IntegrationConfigStatus {
    state
        .status
        .read()
        .map(|status| status.clone())
        .unwrap_or(IntegrationConfigStatus {
            riot_api_configured: false,
            riot_api_source: "Status unavailable".into(),
            spotify_configured: false,
            spotify_redirect_uri: String::new(),
            spotify_scopes: Vec::new(),
            spotify_error: Some("configuration state lock failed".into()),
            advisor_configured: false,
            advisor_mode: "local_heuristic".into(),
            advisor_error: Some("configuration state lock failed".into()),
        })
}

fn validate_riot_api_key(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if !(20..=128).contains(&trimmed.len())
        || !trimmed.starts_with("RGAPI-")
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(
            "Enter a valid Riot development or production API key beginning with RGAPI-".into(),
        );
    }
    Ok(trimmed.to_string())
}

#[tauri::command]
pub async fn save_riot_api_key(
    config: tauri::State<'_, ConfigState>,
    riot: tauri::State<'_, RiotApiState>,
    api_key: String,
) -> Result<IntegrationConfigStatus, String> {
    let validated = validate_riot_api_key(&api_key)?;
    credentials::save_riot_api_key(&validated)?;
    riot.set_api_key(Some(validated)).await;
    config.set_riot_status(true, "Windows Credential Manager");
    println!("[AURA::CONFIG][OK] Riot API credential saved to Windows Credential Manager");
    Ok(get_integration_config(config))
}

#[tauri::command]
pub async fn clear_riot_api_key(
    config: tauri::State<'_, ConfigState>,
    riot: tauri::State<'_, RiotApiState>,
) -> Result<IntegrationConfigStatus, String> {
    credentials::delete_riot_api_key()?;
    if let Some(environment_key) = config.environment_key.clone() {
        riot.set_api_key(Some(environment_key)).await;
        config.set_riot_status(true, "Environment variable");
    } else {
        riot.set_api_key(None).await;
        config.set_riot_status(false, "Not configured");
    }
    println!("[AURA::CONFIG][OK] stored Riot API credential cleared");
    Ok(get_integration_config(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_loopback_pkce_configuration() {
        let config = SpotifyConfig::from_values(
            DEFAULT_SPOTIFY_CLIENT_ID.into(),
            DEFAULT_SPOTIFY_REDIRECT_URI.into(),
            DEFAULT_SPOTIFY_SCOPES.into(),
        )
        .expect("default Spotify configuration should be valid");

        assert_eq!(config.callback_port, 8888);
        assert_eq!(config.callback_path, "/callback");
    }

    #[test]
    fn rejects_non_loopback_redirects() {
        let error = SpotifyConfig::from_values(
            DEFAULT_SPOTIFY_CLIENT_ID.into(),
            "https://example.com/callback".into(),
            DEFAULT_SPOTIFY_SCOPES.into(),
        )
        .expect_err("remote redirects must be rejected");

        assert!(error.contains("127.0.0.1"));
    }

    #[test]
    fn requires_playback_scopes() {
        let error = SpotifyConfig::from_values(
            DEFAULT_SPOTIFY_CLIENT_ID.into(),
            DEFAULT_SPOTIFY_REDIRECT_URI.into(),
            "streaming user-read-email user-read-private user-read-playback-state".into(),
        )
        .expect_err("modify scope is required for player controls");

        assert!(error.contains("user-modify-playback-state"));
    }

    #[test]
    fn requires_web_playback_streaming_scopes() {
        let error = SpotifyConfig::from_values(
            DEFAULT_SPOTIFY_CLIENT_ID.into(),
            DEFAULT_SPOTIFY_REDIRECT_URI.into(),
            "user-read-playback-state user-modify-playback-state".into(),
        )
        .expect_err("embedded playback scopes are required");

        assert!(error.contains("streaming"));
    }

    #[test]
    fn accepts_riot_api_key_shape_without_exposing_a_real_key() {
        let test_key = ["RGAPI", "test-key-shape-000000000000"].join("-");
        let value = validate_riot_api_key(&test_key).expect("valid key shape should be accepted");
        assert!(value.starts_with("RGAPI-"));
    }

    #[test]
    fn rejects_invalid_riot_api_key_shape() {
        assert!(validate_riot_api_key("not-a-riot-key").is_err());
        assert!(validate_riot_api_key("RGAPI-key with spaces").is_err());
    }
}
