use std::fs;
use std::path::Path;

const DEFAULT_LOCKFILE_PATHS: [&str; 3] = [
    r"C:\Riot Games\League of Legends\lockfile",
    r"D:\Riot Games\League of Legends\lockfile",
    r"E:\Riot Games\League of Legends\lockfile",
];

#[derive(Debug, Clone)]
pub struct LcuCredentials {
    pub pid: u32,
    pub port: u16,
    pub password: String,
    pub protocol: String,
}

#[derive(Debug)]
pub enum LockfileError {
    NotFound,
    ReadFailed(std::io::Error),
    MalformedFormat(String),
    PortParseFailed(String),
}

impl std::fmt::Display for LockfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockfileError::NotFound => write!(f, "lockfile not present at expected path"),
            LockfileError::ReadFailed(e) => write!(f, "lockfile read failed: {}", e),
            LockfileError::MalformedFormat(s) => write!(f, "lockfile malformed, got: {}", s),
            LockfileError::PortParseFailed(s) => write!(f, "port field unparsable: {}", s),
        }
    }
}
impl std::error::Error for LockfileError {}

fn log_ok(msg: &str) {
    println!("[AURA::LCU::LOCKFILE][OK] {}", msg);
}
fn log_err(msg: &str) {
    eprintln!("[AURA::LCU::LOCKFILE][ERR] {}", msg);
}

/// Single-shot read, called only after the process table confirms LeagueClientUx.exe
/// is alive — the one disk touch per client session, never polled during matches.
pub fn read_credentials() -> Result<LcuCredentials, LockfileError> {
    let override_path = std::env::var("AURA_LEAGUE_LOCKFILE").ok();
    let path = override_path
        .as_deref()
        .map(Path::new)
        .filter(|candidate| candidate.exists())
        .or_else(|| {
            DEFAULT_LOCKFILE_PATHS
                .iter()
                .map(Path::new)
                .find(|candidate| candidate.exists())
        })
        .ok_or_else(|| {
            log_err(
                "lockfile not found; set AURA_LEAGUE_LOCKFILE for a custom League installation",
            );
            LockfileError::NotFound
        })?;

    let raw = fs::read_to_string(path).map_err(|e| {
        log_err(&format!("fs::read_to_string failed: {}", e));
        LockfileError::ReadFailed(e)
    })?;

    let fields: Vec<&str> = raw.trim().split(':').collect();
    if fields.len() != 5 {
        log_err(&format!(
            "expected 5 colon-delimited fields, got {}",
            fields.len()
        ));
        return Err(LockfileError::MalformedFormat(raw));
    }

    let pid: u32 = fields[1].parse().unwrap_or(0);
    let port: u16 = fields[2].parse().map_err(|_| {
        log_err(&format!("could not parse port from '{}'", fields[2]));
        LockfileError::PortParseFailed(fields[2].to_string())
    })?;

    let creds = LcuCredentials {
        pid,
        port,
        password: fields[3].to_string(),
        protocol: fields[4].to_string(),
    };

    log_ok(&format!(
        "parsed lockfile OK, port={}, protocol={}",
        creds.port, creds.protocol
    ));
    Ok(creds)
}

pub fn build_auth_header(creds: &LcuCredentials) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let raw = format!("riot:{}", creds.password);
    format!("Basic {}", STANDARD.encode(raw))
}
