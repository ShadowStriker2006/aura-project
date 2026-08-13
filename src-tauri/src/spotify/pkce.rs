use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};

fn log_ok(msg: &str) {
    println!("[AURA::SPOTIFY::PKCE][OK] {}", msg);
}

#[derive(Debug, Clone)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

/// 96 random bytes -> base64url verifier (well within RFC 7636's 43-128 char range),
/// then its S256 challenge.
pub fn generate() -> PkcePair {
    let mut raw = [0u8; 96];
    OsRng.fill_bytes(&mut raw);

    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);

    log_ok("PKCE pair generated");
    PkcePair {
        verifier,
        challenge,
    }
}
