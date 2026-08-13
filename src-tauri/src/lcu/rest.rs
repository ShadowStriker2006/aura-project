use reqwest::{Client, Method};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

use super::lockfile::{build_auth_header, LcuCredentials};

#[derive(Debug)]
pub enum RestError {
    ClientBuildFailed(String),
    RequestFailed(String),
    BadStatus(u16, String),
    DecodeFailed(String),
}
impl std::fmt::Display for RestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestError::ClientBuildFailed(s) => write!(f, "reqwest client build failed: {}", s),
            RestError::RequestFailed(s) => write!(f, "request failed: {}", s),
            RestError::BadStatus(code, body) => write!(f, "LCU returned {}: {}", code, body),
            RestError::DecodeFailed(s) => write!(f, "response decode failed: {}", s),
        }
    }
}
impl std::error::Error for RestError {}

fn log_ok(msg: &str) {
    println!("[AURA::LCU::REST][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::LCU::REST][ERR] {}", msg);
}

// Dedicated client that accepts the LCU's self-signed local cert.
// Scoped to this module only — NEVER reused for Spotify or any external HTTPS call.
static LCU_CLIENT: OnceLock<Client> = OnceLock::new();

fn client() -> Result<&'static Client, RestError> {
    if let Some(c) = LCU_CLIENT.get() {
        return Ok(c);
    }
    let built = Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(3))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|e| RestError::ClientBuildFailed(e.to_string()))?;
    Ok(LCU_CLIENT.get_or_init(|| built))
}

/// One-shot authenticated call against the LCU REST surface.
pub async fn lcu_request(
    creds: &LcuCredentials,
    method: Method,
    endpoint: &str,
    body: Option<Value>,
) -> Result<Value, RestError> {
    let url = format!("{}://127.0.0.1:{}{}", creds.protocol, creds.port, endpoint);
    let c = client()?;

    let mut req = c
        .request(method.clone(), &url)
        .header("Authorization", build_auth_header(creds));

    if let Some(b) = &body {
        req = req.json(b);
    }

    let resp = req.send().await.map_err(|e| {
        log_err(&format!(
            "{} {} -> transport error: {}",
            method, endpoint, e
        ));
        RestError::RequestFailed(e.to_string())
    })?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        log_err(&format!("{} {} -> HTTP {}", method, endpoint, status));
        return Err(RestError::BadStatus(status.as_u16(), text));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| RestError::DecodeFailed(e.to_string()))?;
    if text.is_empty() {
        log_ok(&format!("{} {} -> 200 (empty body)", method, endpoint));
        return Ok(Value::Null);
    }

    serde_json::from_str(&text).map_err(|e| {
        log_err(&format!("json decode failed for {}: {}", endpoint, e));
        RestError::DecodeFailed(e.to_string())
    })
}
