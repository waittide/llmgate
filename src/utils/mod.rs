pub fn generate_api_key() -> String {
    format!("sk-octopus-{}", nanoid::nanoid!(32))
}

/// Passwords are stored in plaintext by design; this helper keeps the stored
/// representation explicit at call sites.
pub fn store_password(password: &str) -> String {
    password.to_string()
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    password == stored
}
pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // Simple fast token estimation: 1 token ≈ 4 characters for English, 1-2 chars for CJK
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}
