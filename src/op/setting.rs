use anyhow::Result;

use crate::{model::Setting, op::AppState};

pub async fn refresh_setting_cache(state: &AppState) -> Result<()> {
    let settings: Vec<Setting> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await?;

    // 原子同步内存缓存（与 model 缓存同模式），避免 clear+insert 留出的读取空窗
    for s in &settings {
        state.settings.insert(s.key.clone(), s.value.clone());
    }
    state
        .settings
        .retain(|k, _| settings.iter().any(|s| &s.key == k));
    Ok(())
}

pub fn list_settings(state: &AppState) -> Vec<Setting> {
    let mut list: Vec<Setting> = state
        .settings
        .iter()
        .map(|entry| Setting {
            key:   entry.key().clone(),
            value: entry.value().clone(),
        })
        .collect();
    list.sort_by(|a, b| a.key.cmp(&b.key));
    list
}

pub async fn set_setting_string(state: &AppState, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value) VALUES (?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(&state.db)
    .await?;

    state.settings.insert(key.to_string(), value.to_string());
    Ok(())
}
