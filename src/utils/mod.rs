pub fn generate_api_key() -> String {
    format!("sk-octopus-{}", nanoid::nanoid!(32))
}

pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // 简易快速 token 估算：英文约 4 字符/token，中日韩约 1-2 字符/token
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}
