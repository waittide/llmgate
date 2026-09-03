use anyhow::{Result, anyhow};

use crate::{model::APIKey, op::AppState, utils::generate_api_key};

pub async fn refresh_apikey_cache(state: &AppState) -> Result<()> {
    let keys: Vec<APIKey> = sqlx::query_as("SELECT * FROM api_keys ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    let mut cache = crate::op::ApiKeyCache::default();
    for key in keys {
        cache.by_key.insert(key.api_key.clone(), key.id);
        cache.by_id.insert(key.id, key);
    }
    *state.api_keys.write() = cache;
    Ok(())
}

pub fn list_api_keys(state: &AppState) -> Vec<APIKey> {
    let mut list: Vec<APIKey> = state.api_keys.read().by_id.values().cloned().collect();
    list.sort_by_key(|k| k.id);
    list
}

pub fn get_api_key_by_key(state: &AppState, key_str: &str) -> Result<APIKey> {
    let cache = state.api_keys.read();
    let id = cache
        .by_key
        .get(key_str)
        .ok_or_else(|| anyhow!("API key not found"))?;
    cache
        .by_id
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow!("API key not found"))
}

pub fn get_api_key_by_id(state: &AppState, id: i64) -> Result<APIKey> {
    state
        .api_keys
        .read()
        .by_id
        .get(&id)
        .cloned()
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
    state.stats_api_keys.remove(&id);
    refresh_apikey_cache(state).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{db::init_db, op::stats::flush_stats_to_db};

    async fn setup_test_state() -> (AppState, PathBuf) {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("octopus_apikey_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();
        (state, db_path)
    }

    #[tokio::test]
    async fn test_delete_api_key_cleans_memory_stats_cache() {
        let (state, db_path) = setup_test_state().await;

        let mut key = APIKey {
            name: "test-delete-key".to_string(),
            ..Default::default()
        };
        create_api_key(&state, &mut key).await.unwrap();
        let key_id = key.id;

        // Simulate metrics in memory cache
        state.stats_api_keys.insert(
            key_id,
            crate::model::StatsAPIKey {
                api_key_id: key_id,
                metrics:    crate::model::StatsMetrics {
                    input_token: 100,
                    ..Default::default()
                },
            },
        );

        // Delete API key
        delete_api_key(&state, key_id).await.unwrap();

        // Verify removed from in-memory stats cache
        assert!(!state.stats_api_keys.contains_key(&key_id));

        // Flush to DB and verify it does NOT resurrect in stats_api_keys table
        flush_stats_to_db(&state).await.unwrap();

        let row_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM stats_api_keys WHERE api_key_id = ?")
            .bind(key_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(row_count.0, 0);

        let _ = std::fs::remove_file(db_path);
    }
}
