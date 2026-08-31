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

pub fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn verify_password(password: &str, hashed: &str) -> bool {
    let computed = hash_password(password);
    computed == hashed
}

pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // Simple fast token estimation: 1 token ≈ 4 characters for English, 1-2 chars for CJK
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}
