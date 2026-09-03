use crate::{model::Setting, op::AppState};
use anyhow::Result;

pub async fn refresh_setting_cache(state: &AppState) -> Result<()> {
    let settings: Vec<Setting> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await?;

    state.settings.clear();
    for s in settings {
        state.settings.insert(s.key, s.value);
    }
    Ok(())
}

pub fn list_settings(state: &AppState) -> Vec<Setting> {
    let mut list: Vec<Setting> = state
        .settings
        .iter()
        .map(|entry| Setting {
            key: entry.key().clone(),
            value: entry.value().clone(),
        })
        .collect();
    list.sort_by(|a, b| a.key.cmp(&b.key));
    list
}

#[allow(dead_code)]
pub fn get_setting_string(state: &AppState, key: &str) -> String {
    state
        .settings
        .get(key)
        .map(|entry| entry.value().clone())
        .unwrap_or_default()
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
