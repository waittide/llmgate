pub fn generate_api_key() -> String {
    format!("sk-{}", nanoid::nanoid!(32))
}

pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // 简易快速 token 估算：英文约 4 字符/token，中日韩约 1-2 字符/token
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}

/// 按字节上限截断字符串（UTF-8 边界安全），用于限制入库的报文体大小。
pub fn truncate_utf8(s: &str, limit: usize) -> &str {
    if s.len() <= limit {
        return s;
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
