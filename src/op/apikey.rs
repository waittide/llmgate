use crate::{
    model::APIKey,
    op::AppState,
    utils::generate_api_key,
};
use anyhow::{anyhow, Result};

pub async fn refresh_apikey_cache(state: &AppState) -> Result<()> {
    state.api_keys.clear();
    state.api_key_str_index.clear();

    let keys: Vec<APIKey> = sqlx::query_as("SELECT * FROM api_keys ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    for key in keys {
        state.api_key_str_index.insert(key.api_key.clone(), key.id);
        state.api_keys.insert(key.id, key);
    }
    Ok(())
}

pub fn list_api_keys(state: &AppState) -> Vec<APIKey> {
    let mut list: Vec<APIKey> = state.api_keys.iter().map(|entry| entry.value().clone()).collect();
    list.sort_by_key(|k| k.id);
    list
}

pub fn get_api_key_by_key(state: &AppState, key_str: &str) -> Result<APIKey> {
    let id = state
        .api_key_str_index
        .get(key_str)
        .map(|entry| *entry.value())
        .ok_or_else(|| anyhow!("API key not found"))?;

    get_api_key_by_id(state, id)
}

pub fn get_api_key_by_id(state: &AppState, id: i64) -> Result<APIKey> {
    state
        .api_keys
        .get(&id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow!("API key not found with id: {}", id))
}

pub async fn create_api_key(state: &AppState, key: &mut APIKey) -> Result<()> {
    if key.api_key.trim().is_empty() {
        key.api_key = generate_api_key();
    }

    let result = sqlx::query(
        r#"
        INSERT INTO api_keys (name, api_key, enabled, expire_at, max_cost, supported_models)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&key.name)
    .bind(&key.api_key)
    .bind(key.enabled as i64)
    .bind(key.expire_at)
    .bind(key.max_cost)
    .bind(&key.supported_models)
    .execute(&state.db)
    .await?;

    key.id = result.last_insert_rowid();

    // Ensure stats_api_keys row exists
    let _ = sqlx::query(
        "INSERT OR IGNORE INTO stats_api_keys (api_key_id, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed) VALUES (?, 0, 0, 0, 0, 0, 0, 0)"
    )
    .bind(key.id)
    .execute(&state.db)
    .await;

    refresh_apikey_cache(state).await?;
    Ok(())
}

pub async fn update_api_key(state: &AppState, key: &APIKey) -> Result<APIKey> {
    let current = get_api_key_by_id(state, key.id)?;
    let api_key_str = if key.api_key.trim().is_empty() {
        &current.api_key
    } else {
        &key.api_key
    };

    sqlx::query(
        r#"
        UPDATE api_keys
        SET name = ?, api_key = ?, enabled = ?, expire_at = ?, max_cost = ?, supported_models = ?
        WHERE id = ?
        "#,
    )
    .bind(&key.name)
    .bind(api_key_str)
    .bind(key.enabled as i64)
    .bind(key.expire_at)
    .bind(key.max_cost)
    .bind(&key.supported_models)
    .bind(key.id)
    .execute(&state.db)
    .await?;

    refresh_apikey_cache(state).await?;
    get_api_key_by_id(state, key.id)
}

pub async fn delete_api_key(state: &AppState, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM api_keys WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;

    sqlx::query("DELETE FROM stats_api_keys WHERE api_key_id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;

    refresh_apikey_cache(state).await?;
    Ok(())
}
