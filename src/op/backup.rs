use std::collections::HashMap;

use anyhow::Result;

use crate::{
    model::{BackupData, DBImportResult},
    op::{
        AppState,
        apikey::list_api_keys,
        channel::list_channels,
        group::list_groups,
        llm::list_llm_infos,
        setting::list_settings,
        stats::{get_stats_daily, get_stats_hourly, get_stats_total, list_stats_api_keys},
    },
};

pub async fn export_backup(state: &AppState) -> Result<BackupData> {
    let settings = list_settings(state);
    let api_keys = list_api_keys(state);
    let channels = list_channels(state);
    let channel_models: Vec<_> = state
        .channel_models
        .iter()
        .map(|e| e.value().clone())
        .collect();
    let groups = list_groups(state);
    let group_items: Vec<_> = groups.iter().flat_map(|g| g.items.clone()).collect();
    let llm_infos = list_llm_infos(state);
    let stats_total = Some(vec![get_stats_total(state)]);
    let stats_daily = get_stats_daily(state).await.unwrap_or_default();
    let stats_hourly = get_stats_hourly(state).to_vec();
    let stats_api_key = list_stats_api_keys(state);

    Ok(BackupData {
        version: Some(3),
        exported_at: Some(chrono::Utc::now().to_rfc3339()),
        settings,
        api_keys,
        channels,
        channel_models,
        groups,
        group_items,
        llm_infos,
        stats_total,
        stats_daily,
        stats_hourly,
        stats_api_key,
    })
}

pub async fn import_backup(state: &AppState, backup: BackupData) -> Result<DBImportResult> {
    let mut tx = state.db.begin().await?;
    let mut rows_affected = HashMap::new();

    // 1. Settings
    let mut setting_count = 0;
    for s in backup.settings {
        let res = sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(&s.key)
        .bind(&s.value)
        .execute(&mut *tx)
        .await;
        if res.is_ok() {
            setting_count += 1;
        }
    }
    rows_affected.insert("settings".to_string(), setting_count);

    // 2. API Keys
    let mut key_count = 0;
    for key in backup.api_keys {
        let res = if key.id > 0 {
            sqlx::query(
                r#"
                INSERT INTO api_keys (id, name, api_key, enabled, expire_at, max_cost, supported_models)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    api_key = excluded.api_key,
                    enabled = excluded.enabled,
                    expire_at = excluded.expire_at,
                    max_cost = excluded.max_cost,
                    supported_models = excluded.supported_models
                "#,
            )
            .bind(key.id)
            .bind(&key.name)
            .bind(&key.api_key)
            .bind(key.enabled as i64)
            .bind(key.expire_at)
            .bind(key.max_cost)
            .bind(&key.supported_models)
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query(
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
            .execute(&mut *tx)
            .await
        };

        if res.is_ok() {
            key_count += 1;
        }
    }
    rows_affected.insert("api_keys".to_string(), key_count);

    // 3. Channels
    let mut channel_count = 0;
    for ch in &backup.channels {
        let custom_header_json = serde_json::to_string(&ch.custom_header).unwrap_or_else(|_| "[]".to_string());
        let (res, inserted_id) = if ch.id > 0 {
            let r = sqlx::query(
                r#"
                INSERT INTO channels (id, name, type, enabled, base_url, key, auto_sync, custom_header, param_override, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    type = excluded.type,
                    enabled = excluded.enabled,
                    base_url = excluded.base_url,
                    key = excluded.key,
                    auto_sync = excluded.auto_sync,
                    custom_header = excluded.custom_header,
                    param_override = excluded.param_override,
                    input_token = excluded.input_token,
                    output_token = excluded.output_token,
                    input_cost = excluded.input_cost,
                    output_cost = excluded.output_cost,
                    wait_time = excluded.wait_time,
                    request_success = excluded.request_success,
                    request_failed = excluded.request_failed
                "#,
            )
            .bind(ch.id)
            .bind(&ch.name)
            .bind(ch.channel_type.as_str())
            .bind(ch.enabled as i64)
            .bind(&ch.base_url)
            .bind(&ch.key)
            .bind(ch.auto_sync as i64)
            .bind(&custom_header_json)
            .bind(&ch.param_override)
            .bind(ch.metrics.input_token)
            .bind(ch.metrics.output_token)
            .bind(ch.metrics.input_cost)
            .bind(ch.metrics.output_cost)
            .bind(ch.metrics.wait_time)
            .bind(ch.metrics.request_success)
            .bind(ch.metrics.request_failed)
            .execute(&mut *tx)
            .await;
            (r, ch.id)
        } else {
            let r = sqlx::query(
                r#"
                INSERT INTO channels (name, type, enabled, base_url, key, auto_sync, custom_header, param_override)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&ch.name)
            .bind(ch.channel_type.as_str())
            .bind(ch.enabled as i64)
            .bind(&ch.base_url)
            .bind(&ch.key)
            .bind(ch.auto_sync as i64)
            .bind(&custom_header_json)
            .bind(&ch.param_override)
            .execute(&mut *tx)
            .await;
            let id = r.as_ref().map(|res| res.last_insert_rowid()).unwrap_or(0);
            (r, id)
        };

        if res.is_ok() {
            channel_count += 1;
        }

        // Also insert nested channel models if present with correct channel_id
        if inserted_id > 0 {
            for cm in &ch.models {
                let _ = sqlx::query(
                    "INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?) ON CONFLICT(channel_id, name) DO NOTHING",
                )
                .bind(inserted_id)
                .bind(&cm.name)
                .bind(cm.source.as_str())
                .execute(&mut *tx)
                .await;
            }
        }
    }
    rows_affected.insert("channels".to_string(), channel_count);

    // 4. Channel Models (top-level)
    let mut cm_count = 0;
    for cm in backup.channel_models {
        let res = if cm.id > 0 {
            sqlx::query(
                r#"
                INSERT INTO channel_models (id, channel_id, name, source, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    channel_id = excluded.channel_id,
                    name = excluded.name,
                    source = excluded.source,
                    input_token = excluded.input_token,
                    output_token = excluded.output_token,
                    input_cost = excluded.input_cost,
                    output_cost = excluded.output_cost,
                    wait_time = excluded.wait_time,
                    request_success = excluded.request_success,
                    request_failed = excluded.request_failed
                "#,
            )
            .bind(cm.id)
            .bind(cm.channel_id)
            .bind(&cm.name)
            .bind(cm.source.as_str())
            .bind(cm.metrics.input_token)
            .bind(cm.metrics.output_token)
            .bind(cm.metrics.input_cost)
            .bind(cm.metrics.output_cost)
            .bind(cm.metrics.wait_time)
            .bind(cm.metrics.request_success)
            .bind(cm.metrics.request_failed)
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query(
                "INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?) ON CONFLICT(channel_id, name) DO NOTHING",
            )
            .bind(cm.channel_id)
            .bind(&cm.name)
            .bind(cm.source.as_str())
            .execute(&mut *tx)
            .await
        };
        if res.is_ok() {
            cm_count += 1;
        }
    }
    rows_affected.insert("channel_models".to_string(), cm_count);

    // 5. Groups
    let mut group_count = 0;
    for grp in &backup.groups {
        let relay_config_json = serde_json::to_string(&grp.relay_config).unwrap_or_else(|_| "{}".to_string());
        let (res, inserted_id) = if grp.id > 0 {
            let r = sqlx::query(
                r#"
                INSERT INTO groups (id, name, mode, active_item_id, relay_config)
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    mode = excluded.mode,
                    active_item_id = excluded.active_item_id,
                    relay_config = excluded.relay_config
                "#,
            )
            .bind(grp.id)
            .bind(&grp.name)
            .bind(grp.mode.as_str())
            .bind(grp.active_item_id)
            .bind(&relay_config_json)
            .execute(&mut *tx)
            .await;
            (r, grp.id)
        } else {
            let r = sqlx::query("INSERT INTO groups (name, mode, active_item_id, relay_config) VALUES (?, ?, ?, ?)")
                .bind(&grp.name)
                .bind(grp.mode.as_str())
                .bind(grp.active_item_id)
                .bind(&relay_config_json)
                .execute(&mut *tx)
                .await;
            let id = r.as_ref().map(|res| res.last_insert_rowid()).unwrap_or(0);
            (r, id)
        };

        if res.is_ok() {
            group_count += 1;
        }

        // Also insert nested group items with correct group_id
        if inserted_id > 0 {
            for item in &grp.items {
                let _ = sqlx::query(
                    "INSERT INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?) ON CONFLICT(group_id, channel_model_id) DO NOTHING",
                )
                .bind(inserted_id)
                .bind(item.channel_model_id)
                .bind(item.priority)
                .execute(&mut *tx)
                .await;
            }
        }
    }
    rows_affected.insert("groups".to_string(), group_count);

    // 6. Group Items (top-level)
    let mut gi_count = 0;
    for item in backup.group_items {
        let res = if item.id > 0 {
            sqlx::query(
                r#"
                INSERT INTO group_items (id, group_id, channel_model_id, priority)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    group_id = excluded.group_id,
                    channel_model_id = excluded.channel_model_id,
                    priority = excluded.priority
                "#,
            )
            .bind(item.id)
            .bind(item.group_id)
            .bind(item.channel_model_id)
            .bind(item.priority)
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query(
                "INSERT INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?) ON CONFLICT(group_id, channel_model_id) DO NOTHING",
            )
            .bind(item.group_id)
            .bind(item.channel_model_id)
            .bind(item.priority)
            .execute(&mut *tx)
            .await
        };
        if res.is_ok() {
            gi_count += 1;
        }
    }
    rows_affected.insert("group_items".to_string(), gi_count);

    // 7. LLM Infos
    let mut llm_count = 0;
    for info in backup.llm_infos {
        let name_lower = info.name.to_lowercase();
        let res = sqlx::query(
            r#"
            INSERT INTO llm_infos (name, input, output, cache_read, cache_write)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(name) DO UPDATE SET
                input = excluded.input,
                output = excluded.output,
                cache_read = excluded.cache_read,
                cache_write = excluded.cache_write
            "#,
        )
        .bind(&name_lower)
        .bind(info.price.input)
        .bind(info.price.output)
        .bind(info.price.cache_read)
        .bind(info.price.cache_write)
        .execute(&mut *tx)
        .await;
        if res.is_ok() {
            llm_count += 1;
        }
    }
    rows_affected.insert("llm_infos".to_string(), llm_count);

    tx.commit().await?;
    state.refresh_all_caches().await?;
    Ok(DBImportResult { rows_affected })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        db::init_db,
        model::{Channel, ChannelModel, ChannelModelSource, ChannelProvider, Group, GroupMode},
    };

    async fn setup_test_state() -> (AppState, PathBuf) {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("octopus_backup_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();
        (state, db_path)
    }

    #[tokio::test]
    async fn test_import_backup_channels_and_groups_with_zero_id() {
        let (state, db_path) = setup_test_state().await;

        let backup = BackupData {
            channels: vec![Channel {
                id: 0, // Zero id import
                name: "imported-channel".to_string(),
                channel_type: ChannelProvider::OpenaiCompletion,
                enabled: true,
                base_url: "https://api.openai.com".to_string(),
                key: "sk-import".to_string(),
                models: vec![ChannelModel {
                    id:         0,
                    channel_id: 0,
                    name:       "gpt-4o".to_string(),
                    source:     ChannelModelSource::Manual,
                    metrics:    Default::default(),
                }],
                ..Default::default()
            }],
            groups: vec![Group {
                id:             0, // Zero id import
                name:           "imported-group".to_string(),
                mode:           GroupMode::Manual,
                active_item_id: 0,
                relay_config:   Default::default(),
                items:          Vec::new(),
            }],
            ..Default::default()
        };

        let res = import_backup(&state, backup).await.unwrap();
        assert_eq!(res.rows_affected.get("channels").copied(), Some(1));
        assert_eq!(res.rows_affected.get("groups").copied(), Some(1));

        // Verify channel model was inserted with foreign key to newly created channel
        let model_row: (i64, i64, String) =
            sqlx::query_as("SELECT id, channel_id, name FROM channel_models WHERE name = 'gpt-4o'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert!(model_row.1 > 0, "Nested channel model must have valid channel_id");

        let _ = std::fs::remove_file(db_path);
    }
}
