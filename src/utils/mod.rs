use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn generate_api_key() -> String {
    format!("sk-octopus-{}", nanoid::nanoid!(32))
}

fn hash_password_with_salt(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b"$");
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in result {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{:02x}", byte);
    }
    hex
}

/// Hashes passwords using salted SHA-256 for secure storage.
pub fn store_password(password: &str) -> String {
    let salt = nanoid::nanoid!(16);
    let hash = hash_password_with_salt(password, &salt);
    format!("sha256${}${}", salt, hash)
}

/// Verifies a password against the stored representation in constant time.
/// Supports both salted sha256$ hashes and legacy plaintext for seamless backward compatibility.
pub fn verify_password(password: &str, stored: &str) -> bool {
    if let Some(rest) = stored.strip_prefix("sha256$")
        && let Some((salt, expected_hash)) = rest.split_once('$')
    {
        let actual_hash = hash_password_with_salt(password, salt);
        return actual_hash
            .as_bytes()
            .ct_eq(expected_hash.as_bytes())
            .into();
    }
    // Backward compatibility for legacy plaintext storage
    password.as_bytes().ct_eq(stored.as_bytes()).into()
}
pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    // Simple fast token estimation: 1 token ≈ 4 characters for English, 1-2 chars for CJK
    let char_count = text.chars().count();
    (char_count as f64 / 3.0).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hash_and_verify() {
        let raw = "admin123456";
        let stored = store_password(raw);
        assert!(stored.starts_with("sha256$"));
        assert!(verify_password(raw, &stored));
        assert!(!verify_password("wrong_password", &stored));
    }

    #[test]
    fn test_legacy_plaintext_password_verification() {
        let raw = "legacy_admin_pwd";
        assert!(verify_password(raw, raw));
        assert!(!verify_password("wrong_pwd", raw));
    }
}
