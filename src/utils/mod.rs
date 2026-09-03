use rand::RngExt;
use sha2::{Digest, Sha256};

pub fn generate_api_key() -> String {
    const KEY_CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut rng = rand::rng();
    let mut key = String::with_capacity(48);
    for _ in 0..48 {
        let idx = rng.random_range(0..KEY_CHARS.len());
        key.push(KEY_CHARS[idx] as char);
    }
    format!("sk-octopus-{}", key)
}

/// Hash a password using a random 16-byte salt and SHA-256 iterations.
/// Format: `pbkdf2:sha256:<salt_hex>:<hash_hex>`
pub fn hash_password(password: &str) -> String {
    let mut salt_bytes = [0u8; 16];
    rand::rng().fill(&mut salt_bytes);
    let salt_hex = hex::encode(salt_bytes);
    let hash_hex = compute_salted_hash(password, &salt_hex);
    format!("pbkdf2:sha256:{}:{}", salt_hex, hash_hex)
}

fn compute_salted_hash(password: &str, salt_hex: &str) -> String {
    let mut current = Sha256::digest(format!("{}:{}", salt_hex, password).as_bytes()).to_vec();
    for _ in 1..10_000 {
        let mut hasher = Sha256::new();
        hasher.update(&current);
        hasher.update(password.as_bytes());
        current = hasher.finalize().to_vec();
    }
    hex::encode(current)
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Verify password against hash. Supports both new `pbkdf2:sha256:...` format and legacy raw SHA-256 hashes.
pub fn verify_password(password: &str, hashed: &str) -> bool {
    if let Some(rest) = hashed.strip_prefix("pbkdf2:sha256:") {
        if let Some((salt_hex, expected_hash)) = rest.split_once(':') {
            let computed = compute_salted_hash(password, salt_hex);
            return constant_time_eq(computed.as_bytes(), expected_hash.as_bytes());
        }
    }

    // Legacy unsalted sha256 fallback
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let legacy_computed = hex::encode(hasher.finalize());
    constant_time_eq(legacy_computed.as_bytes(), hashed.as_bytes())
}

pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // Simple fast token estimation: 1 token ≈ 4 characters for English, 1-2 chars for CJK
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}
