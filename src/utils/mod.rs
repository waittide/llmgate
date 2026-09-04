pub fn generate_api_key() -> String {
    format!("sk-octopus-{}", nanoid::nanoid!(32))
}

pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // Simple fast token estimation: 1 token ≈ 4 characters for English, 1-2 chars for CJK
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}
