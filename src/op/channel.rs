use anyhow::{Result, anyhow};
use sqlx::Row;

use crate::{
    model::{
        Channel, ChannelModel, ChannelModelSource, ChannelProvider, ChannelUpdateRequest, CustomHeader, StatsMetrics,
    },
    op::AppState,
};

pub async fn refresh_channel_cache(state: &AppState) -> Result<()> {
    // 1. Fetch all channels into local map first to avoid cache-stampede / 404 race
    let rows = sqlx::query("SELECT id, name, type, enabled, base_url, key, auto_sync, custom_header, param_override, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed FROM channels ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    let mut local_channels = std::collections::HashMap::new();

    for row in rows {
        let id: i64 = row.get("id");
        let name: String = row.get("name");
        let channel_type_str: String = row.get("type");
        let enabled: bool = row.get::<i64, _>("enabled") != 0;
        let base_url: String = row.get("base_url");
        let key: String = row.get("key");
        let auto_sync: bool = row.get::<i64, _>("auto_sync") != 0;
        let custom_header_json: String = row.get("custom_header");
        let custom_header: Vec<CustomHeader> = serde_json::from_str(&custom_header_json).unwrap_or_default();
        let param_override: Option<String> = row.get("param_override");

        let metrics = StatsMetrics {
            input_token:     row.get("input_token"),
            output_token:    row.get("output_token"),
            input_cost:      row.get("input_cost"),
            output_cost:     row.get("output_cost"),
            wait_time:       row.get("wait_time"),
            request_success: row.get("request_success"),
            request_failed:  row.get("request_failed"),
        };

        let channel = Channel {
            id,
            name,
            channel_type: ChannelProvider::from_str(&channel_type_str),
            enabled,
            base_url,
            key,
            models: Vec::new(),
            auto_sync,
            custom_header,
            param_override,
            metrics,
        };

        local_channels.insert(id, channel);
    }

    // 2. Fetch all channel models into local map
    let model_rows = sqlx::query("SELECT id, channel_id, name, source, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed FROM channel_models ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    let mut local_models = std::collections::HashMap::new();
    for row in model_rows {
        let id: i64 = row.get("id");
        let channel_id: i64 = row.get("channel_id");
        let name: String = row.get("name");
        let source_str: String = row.get("source");
        let metrics = StatsMetrics {
            input_token:     row.get("input_token"),
            output_token:    row.get("output_token"),
            input_cost:      row.get("input_cost"),
            output_cost:     row.get("output_cost"),
            wait_time:       row.get("wait_time"),
            request_success: row.get("request_success"),
            request_failed:  row.get("request_failed"),
        };

        let channel_model = ChannelModel {
            id,
            channel_id,
            name,
            source: ChannelModelSource::from_str(&source_str),
            metrics,
        };

        if let Some(ch) = local_channels.get_mut(&channel_id) {
            ch.models.push(channel_model.clone());
        }
        local_models.insert(id, channel_model);
    }

    // Atomically sync in-memory cache without leaving empty windows
    for (k, v) in &local_channels {
        state.channels.insert(*k, v.clone());
    }
    state.channels.retain(|k, _| local_channels.contains_key(k));

    for (k, v) in &local_models {
        state.channel_models.insert(*k, v.clone());
    }
    state
        .channel_models
        .retain(|k, _| local_models.contains_key(k));
    Ok(())
}

pub fn list_channels(state: &AppState) -> Vec<Channel> {
    let mut list: Vec<Channel> = state
        .channels
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    list.sort_by_key(|c| c.id);
    list
}

pub fn get_channel(state: &AppState, id: i64) -> Result<Channel> {
    state
        .channels
        .get(&id)
        .map(|c| c.clone())
        .ok_or_else(|| anyhow!("Channel not found"))
}

pub async fn create_channel(state: &AppState, channel: &mut Channel) -> Result<()> {
    let custom_header_json = serde_json::to_string(&channel.custom_header)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query(
        r#"
        INSERT INTO channels (name, type, enabled, base_url, key, auto_sync, custom_header, param_override)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&channel.name)
    .bind(channel.channel_type.as_str())
    .bind(channel.enabled as i64)
    .bind(&channel.base_url)
    .bind(&channel.key)
    .bind(channel.auto_sync as i64)
    .bind(&custom_header_json)
    .bind(&channel.param_override)
    .execute(&mut *tx)
    .await?;

    let channel_id = result.last_insert_rowid();
    channel.id = channel_id;

    for model in &mut channel.models {
        model.channel_id = channel_id;
        let m_res = sqlx::query("INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?)")
            .bind(channel_id)
            .bind(&model.name)
            .bind(model.source.as_str())
            .execute(&mut *tx)
            .await?;
        model.id = m_res.last_insert_rowid();
    }

    tx.commit().await?;
    refresh_channel_cache(state).await?;
    Ok(())
}

pub async fn update_channel(state: &AppState, req: &ChannelUpdateRequest) -> Result<Channel> {
    let current = get_channel(state, req.id)?;

    let name = req.name.as_ref().unwrap_or(&current.name);
    let channel_type = req.channel_type.as_ref().unwrap_or(&current.channel_type);
    let enabled = req.enabled.unwrap_or(current.enabled);
    let base_url = req.base_url.as_ref().unwrap_or(&current.base_url);
    let key = req.key.as_ref().unwrap_or(&current.key);
    let auto_sync = req.auto_sync.unwrap_or(current.auto_sync);
    let custom_header = req.custom_header.as_ref().unwrap_or(&current.custom_header);
    let custom_header_json = serde_json::to_string(custom_header)?;
    let param_override = req
        .param_override
        .as_ref()
        .or(current.param_override.as_ref());

    let mut tx = state.db.begin().await?;
    sqlx::query(
        r#"
        UPDATE channels
        SET name = ?, type = ?, enabled = ?, base_url = ?, key = ?, auto_sync = ?, custom_header = ?, param_override = ?
        WHERE id = ?
        "#,
    )
    .bind(name)
    .bind(channel_type.as_str())
    .bind(enabled as i64)
    .bind(base_url)
    .bind(key)
    .bind(auto_sync as i64)
    .bind(&custom_header_json)
    .bind(param_override)
    .bind(req.id)
    .execute(&mut *tx)
    .await?;

    // Handle models update if provided (incremental update to preserve IDs and metrics)
    if let Some(models) = &req.models {
        let existing_models: Vec<(i64, String)> =
            sqlx::query_as::<_, (i64, String)>("SELECT id, name FROM channel_models WHERE channel_id = ?")
                .bind(req.id)
                .fetch_all(&mut *tx)
                .await?;

        let mut req_model_names = std::collections::HashSet::new();

        for model in models {
            req_model_names.insert(&model.name);
            if let Some((existing_id, _)) = existing_models.iter().find(|(_, name)| name == &model.name) {
                sqlx::query("UPDATE channel_models SET source = ? WHERE id = ?")
                    .bind(model.source.as_str())
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?)")
                    .bind(req.id)
                    .bind(&model.name)
                    .bind(model.source.as_str())
                    .execute(&mut *tx)
                    .await?;
            }
        }

        for (existing_id, existing_name) in existing_models {
            if !req_model_names.contains(&existing_name) {
                sqlx::query("DELETE FROM channel_models WHERE id = ?")
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;

    refresh_channel_cache(state).await?;
    crate::op::group::refresh_group_cache(state).await?;
    get_channel(state, req.id)
}

pub async fn set_channel_enabled(state: &AppState, id: i64, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE channels SET enabled = ? WHERE id = ?")
        .bind(enabled as i64)
        .bind(id)
        .execute(&state.db)
        .await?;

    if let Some(mut ch) = state.channels.get_mut(&id) {
        ch.enabled = enabled;
    }
    Ok(())
}

pub async fn delete_channel(state: &AppState, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;

    refresh_channel_cache(state).await?;
    crate::op::group::refresh_group_cache(state).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::db::init_db;
    async fn setup_test_state() -> (AppState, PathBuf) {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("octopus_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();
        (state, db_path)
    }
    #[tokio::test]
    async fn test_channel_models_incremental_update() {
        let (state, _path) = setup_test_state().await;

        let mut ch = Channel {
            name: "test-channel".to_string(),
            channel_type: ChannelProvider::OpenaiCompletion,
            enabled: true,
            base_url: "https://api.openai.com".to_string(),
            key: "sk-test".to_string(),
            models: vec![
                ChannelModel {
                    name: "gpt-4o".to_string(),
                    source: ChannelModelSource::Manual,
                    ..Default::default()
                },
                ChannelModel {
                    name: "gpt-4o-mini".to_string(),
                    source: ChannelModelSource::Auto,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        create_channel(&state, &mut ch).await.unwrap();
        let initial_channel = get_channel(&state, ch.id).unwrap();
        let gpt4o_id = initial_channel
            .models
            .iter()
            .find(|m| m.name == "gpt-4o")
            .unwrap()
            .id;

        // Update channel models: keep gpt-4o, remove gpt-4o-mini, add o1-preview
        let update_req = ChannelUpdateRequest {
            id:             ch.id,
            name:           None,
            channel_type:   None,
            enabled:        None,
            base_url:       None,
            key:            None,
            models:         Some(vec![
                ChannelModel {
                    name: "gpt-4o".to_string(),
                    source: ChannelModelSource::Manual,
                    ..Default::default()
                },
                ChannelModel {
                    name: "o1-preview".to_string(),
                    source: ChannelModelSource::Auto,
                    ..Default::default()
                },
            ]),
            auto_sync:      None,
            custom_header:  None,
            param_override: None,
        };

        let updated_channel = update_channel(&state, &update_req).await.unwrap();
        let new_gpt4o = updated_channel
            .models
            .iter()
            .find(|m| m.name == "gpt-4o")
            .unwrap();
        assert_eq!(new_gpt4o.id, gpt4o_id, "Existing model ID should be preserved");

        assert!(
            updated_channel
                .models
                .iter()
                .any(|m| m.name == "o1-preview")
        );
        assert!(
            !updated_channel
                .models
                .iter()
                .any(|m| m.name == "gpt-4o-mini")
        );

        // Verify foreign key cascade delete on channel deletion
        delete_channel(&state, ch.id).await.unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channel_models WHERE channel_id = ?")
            .bind(ch.id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(
            count.0, 0,
            "Channel models must be cascade deleted when channel is deleted"
        );
    }
}
