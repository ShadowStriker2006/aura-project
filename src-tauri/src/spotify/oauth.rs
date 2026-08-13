use crate::config::SpotifyConfig;
use reqwest::Client;
use serde::Deserialize;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use url::Url;

use super::pkce;

#[derive(Debug, Clone)]
pub struct SpotifyTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Instant,
}

#[derive(Clone)]
pub struct SpotifyState {
    tokens: Arc<Mutex<Option<SpotifyTokens>>>,
    config: Arc<Result<SpotifyConfig, String>>,
}

impl SpotifyState {
    pub fn new(config: Result<SpotifyConfig, String>) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(None)),
            config: Arc::new(config),
        }
    }

    fn config(&self) -> Result<SpotifyConfig, OAuthError> {
        self.config
            .as_ref()
            .clone()
            .map_err(OAuthError::InvalidConfiguration)
    }
}

#[derive(Debug)]
pub enum OAuthError {
    InvalidConfiguration(String),
    ListenerBindFailed(String),
    CallbackFailed(String),
    AuthorizationDenied,
    NoCodeInCallback,
    StateMismatch,
    NotLoggedIn,
    MissingRefreshToken,
    TokenClientFailed(String),
    TokenExchangeFailed(u16),
    TokenDecodeFailed(String),
    BrowserLaunchFailed(String),
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OAuthError::InvalidConfiguration(reason) => {
                write!(f, "Spotify OAuth configuration is invalid: {reason}")
            }
            OAuthError::ListenerBindFailed(reason) => {
                write!(f, "callback listener bind failed: {reason}")
            }
            OAuthError::CallbackFailed(reason) => {
                write!(f, "callback handling failed: {reason}")
            }
            OAuthError::AuthorizationDenied => write!(f, "Spotify authorization was denied"),
            OAuthError::NoCodeInCallback => write!(f, "callback contained no authorization code"),
            OAuthError::StateMismatch => {
                write!(f, "state parameter mismatch; login was rejected")
            }
            OAuthError::NotLoggedIn => write!(f, "not logged in; connect Spotify first"),
            OAuthError::MissingRefreshToken => {
                write!(
                    f,
                    "Spotify returned no refresh token; connect the account again"
                )
            }
            OAuthError::TokenClientFailed(reason) => {
                write!(f, "Spotify HTTP client setup failed: {reason}")
            }
            OAuthError::TokenExchangeFailed(status) => {
                write!(f, "Spotify token endpoint returned HTTP {status}")
            }
            OAuthError::TokenDecodeFailed(reason) => {
                write!(f, "Spotify token response decode failed: {reason}")
            }
            OAuthError::BrowserLaunchFailed(reason) => {
                write!(f, "failed to launch the default browser: {reason}")
            }
        }
    }
}

impl std::error::Error for OAuthError {}

fn log_ok(message: &str) {
    println!("[AURA::SPOTIFY::OAUTH][OK] {message}");
}

fn log_err(message: &str) {
    eprintln!("[AURA::SPOTIFY::OAUTH][ERR] {message}");
}

/// Spotify Authorization Code with PKCE. The client ID is public, no client
/// secret is used, and access/refresh tokens stay in memory only.
pub async fn login(state: SpotifyState) -> Result<(), OAuthError> {
    let config = state.config()?;
    let listener = TcpListener::bind(("127.0.0.1", config.callback_port))
        .await
        .map_err(|error| {
            log_err(&format!(
                "bind 127.0.0.1:{} failed: {error}",
                config.callback_port
            ));
            OAuthError::ListenerBindFailed(error.to_string())
        })?;

    let pair = pkce::generate();
    let csrf_state = random_state();
    let auth_url = build_authorize_url(&config, &pair.challenge, &csrf_state)?;

    log_ok("opening the default browser for Spotify consent");
    launch_browser(&auth_url)?;

    let code = catch_callback(listener, &config.callback_path, &csrf_state).await?;
    log_ok("validated callback received; exchanging the authorization code");

    let tokens = exchange_code(&config, &code, &pair.verifier).await?;
    *state.tokens.lock().await = Some(tokens);
    log_ok("login complete; tokens cached in volatile memory");
    Ok(())
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    use rand::{rngs::OsRng, RngCore};
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn build_authorize_url(
    config: &SpotifyConfig,
    challenge: &str,
    csrf_state: &str,
) -> Result<String, OAuthError> {
    let mut url = Url::parse("https://accounts.spotify.com/authorize")
        .map_err(|error| OAuthError::InvalidConfiguration(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &config.redirect_uri)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge)
        .append_pair("state", csrf_state)
        .append_pair("scope", &config.scopes_string());
    Ok(url.into())
}

fn launch_browser(url: &str) -> Result<(), OAuthError> {
    // Avoid `cmd /C start`: OAuth URLs contain `&`, which cmd.exe treats as
    // command separators. FileProtocolHandler receives the complete URL.
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            log_err(&format!("default browser launch failed: {error}"));
            OAuthError::BrowserLaunchFailed(error.to_string())
        })
}

async fn catch_callback(
    listener: TcpListener,
    callback_path: &str,
    expected_state: &str,
) -> Result<String, OAuthError> {
    let attempt = async {
        loop {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|error| OAuthError::CallbackFailed(error.to_string()))?;

            let mut buffer = [0u8; 4096];
            let read = match tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
                .await
            {
                Ok(Ok(size)) => size,
                Ok(Err(error)) => {
                    log_err(&format!("callback socket read failed: {error}"));
                    continue;
                }
                Err(_) => {
                    log_err("ignored a callback socket that sent no request");
                    continue;
                }
            };

            let request = String::from_utf8_lossy(&buffer[..read]);
            let Some(params) = parse_callback_request(&request, callback_path) else {
                write_callback_response(&mut socket, false).await;
                continue;
            };

            if params.state.as_deref() != Some(expected_state) {
                write_callback_response(&mut socket, false).await;
                log_err("callback state did not match");
                return Err(OAuthError::StateMismatch);
            }
            if params.denied {
                write_callback_response(&mut socket, false).await;
                return Err(OAuthError::AuthorizationDenied);
            }
            let Some(code) = params.code else {
                write_callback_response(&mut socket, false).await;
                return Err(OAuthError::NoCodeInCallback);
            };

            write_callback_response(&mut socket, true).await;
            return Ok(code);
        }
    };

    match tokio::time::timeout(Duration::from_secs(120), attempt).await {
        Ok(result) => result,
        Err(_) => {
            log_err("timed out waiting for the Spotify redirect");
            Err(OAuthError::CallbackFailed(
                "timed out waiting for user consent".into(),
            ))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    denied: bool,
}

fn parse_callback_request(request: &str, callback_path: &str) -> Option<CallbackParams> {
    let mut request_parts = request.lines().next()?.split_ascii_whitespace();
    if request_parts.next()? != "GET" {
        return None;
    }
    let request_target = request_parts.next()?;
    let parsed = Url::parse(&format!("http://127.0.0.1{request_target}")).ok()?;
    if parsed.path() != callback_path {
        return None;
    }

    let mut code = None;
    let mut state = None;
    let mut denied = false;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => denied = true,
            _ => {}
        }
    }

    Some(CallbackParams {
        code,
        state,
        denied,
    })
}

async fn write_callback_response(socket: &mut TcpStream, success: bool) {
    let (status, body) = if success {
        (
            "200 OK",
            "<!doctype html><meta charset=\"utf-8\"><title>Aura connected</title><h3>Aura connected to Spotify. You can close this tab.</h3>",
        )
    } else {
        (
            "400 Bad Request",
            "<!doctype html><meta charset=\"utf-8\"><title>Aura connection failed</title><h3>Aura could not complete Spotify login. Return to Aura and try again.</h3>",
        )
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if let Err(error) = socket.write_all(response.as_bytes()).await {
        log_err(&format!("callback response write failed: {error}"));
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

static CLIENT: OnceLock<Client> = OnceLock::new();

fn client() -> Result<&'static Client, OAuthError> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let built = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| OAuthError::TokenClientFailed(error.to_string()))?;
    Ok(CLIENT.get_or_init(|| built))
}

async fn exchange_code(
    config: &SpotifyConfig,
    code: &str,
    verifier: &str,
) -> Result<SpotifyTokens, OAuthError> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("client_id", config.client_id.as_str()),
        ("code_verifier", verifier),
    ];

    let response = client()?
        .post("https://accounts.spotify.com/api/token")
        .form(&params)
        .send()
        .await
        .map_err(|error| OAuthError::TokenClientFailed(error.to_string()))?;

    if !response.status().is_success() {
        return Err(OAuthError::TokenExchangeFailed(response.status().as_u16()));
    }

    let parsed: TokenResponse = response
        .json()
        .await
        .map_err(|error| OAuthError::TokenDecodeFailed(error.to_string()))?;
    Ok(SpotifyTokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at: Instant::now() + Duration::from_secs(parsed.expires_in.saturating_sub(30)),
    })
}

/// Returns a valid access token, refreshing it when required. Tokens are never
/// logged or written to disk. The optional isolated Aura Player receives a
/// just-in-time token over a nonce-protected loopback endpoint because the
/// official Web Playback SDK requires it; the main dashboard never receives it.
pub async fn ensure_fresh(state: &SpotifyState) -> Result<String, OAuthError> {
    let refresh_token = {
        let guard = state.tokens.lock().await;
        let tokens = guard.as_ref().ok_or(OAuthError::NotLoggedIn)?;
        if Instant::now() < tokens.expires_at {
            return Ok(tokens.access_token.clone());
        }
        tokens
            .refresh_token
            .clone()
            .ok_or(OAuthError::MissingRefreshToken)?
    };

    log_ok("access token expired; refreshing");
    let config = state.config()?;
    let refreshed = refresh(&config, &refresh_token).await?;
    let access_token = refreshed.access_token.clone();
    *state.tokens.lock().await = Some(refreshed);
    Ok(access_token)
}

async fn refresh(config: &SpotifyConfig, refresh_token: &str) -> Result<SpotifyTokens, OAuthError> {
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", config.client_id.as_str()),
    ];

    let response = client()?
        .post("https://accounts.spotify.com/api/token")
        .form(&params)
        .send()
        .await
        .map_err(|error| OAuthError::TokenClientFailed(error.to_string()))?;

    if !response.status().is_success() {
        return Err(OAuthError::TokenExchangeFailed(response.status().as_u16()));
    }

    let parsed: TokenResponse = response
        .json()
        .await
        .map_err(|error| OAuthError::TokenDecodeFailed(error.to_string()))?;
    Ok(SpotifyTokens {
        access_token: parsed.access_token,
        refresh_token: parsed
            .refresh_token
            .or_else(|| Some(refresh_token.to_string())),
        expires_at: Instant::now() + Duration::from_secs(parsed.expires_in.saturating_sub(30)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SpotifyConfig {
        SpotifyConfig {
            client_id: "681dfe3599314fd2adde1cd53ab731a8".into(),
            redirect_uri: "http://127.0.0.1:8888/callback".into(),
            callback_port: 8888,
            callback_path: "/callback".into(),
            scopes: vec![
                "streaming".into(),
                "user-modify-playback-state".into(),
                "user-read-email".into(),
                "user-read-playback-state".into(),
                "user-read-private".into(),
            ],
        }
    }

    #[test]
    fn authorize_url_uses_pkce_without_a_client_secret() {
        let url = build_authorize_url(&test_config(), "challenge", "state")
            .expect("authorize URL should build");

        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=681dfe3599314fd2adde1cd53ab731a8"));
        assert!(url.contains("streaming"));
        assert!(!url.contains("client_secret"));
    }

    #[test]
    fn callback_parser_decodes_values_and_checks_path() {
        let parsed = parse_callback_request(
            "GET /callback?code=a%2Fb&state=abc HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "/callback",
        )
        .expect("valid callback should parse");

        assert_eq!(
            parsed,
            CallbackParams {
                code: Some("a/b".into()),
                state: Some("abc".into()),
                denied: false,
            }
        );
        assert!(parse_callback_request(
            "GET /wrong?code=a&state=abc HTTP/1.1\r\n\r\n",
            "/callback"
        )
        .is_none());
    }
}
