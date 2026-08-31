use anyhow::Result;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub iat: usize,
    pub exp: usize,
}

pub fn generate_jwt(username: &str, secret: &str, expires_sec: i64) -> Result<(String, usize)> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
    let max_age = if expires_sec == -1 {
        30 * 24 * 3600 // 30 days
    } else if expires_sec > 0 {
        expires_sec as usize
    } else {
        15 * 60 // 15 mins default
    };

    let claims = Claims {
        sub: username.to_string(),
        iss: "octopus".to_string(),
        iat: now,
        exp: now + max_age,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;
    Ok((token, max_age))
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims> {
    let mut validation = Validation::default();
    validation.set_issuer(&["octopus"]);
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(token_data.claims)
}
