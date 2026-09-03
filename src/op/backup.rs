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
    let mut rows_affected = HashMap::new();

    // 1. Settings
    let mut setting_count = 0;
    for s in backup.settings {
        let res = sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(&s.key)
        .bind(&s.value)
        .execute(&state.db)
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
            .execute(&state.db)
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
            .execute(&state.db)
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
        let res = if ch.id > 0 {
            sqlx::query(
                r#"
                INSERT INTO channels (id, name, type, enabled, base_url, key, auto_sync, custom_header, param_override, match_regex, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    type = excluded.type,
                    enabled = excluded.enabled,
                    base_url = excluded.base_url,
                    key = excluded.key,
                    auto_sync = excluded.auto_sync,
                    custom_header = excluded.custom_header,
                    param_override = excluded.param_override,
                    match_regex = excluded.match_regex,
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
            .bind(&ch.match_regex)
            .bind(ch.metrics.input_token)
            .bind(ch.metrics.output_token)
            .bind(ch.metrics.input_cost)
            .bind(ch.metrics.output_cost)
            .bind(ch.metrics.wait_time)
            .bind(ch.metrics.request_success)
            .bind(ch.metrics.request_failed)
            .execute(&state.db)
            .await
        } else {
            sqlx::query(
                r#"
                INSERT INTO channels (name, type, enabled, base_url, key, auto_sync, custom_header, param_override, match_regex)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            .bind(&ch.match_regex)
            .execute(&state.db)
            .await
        };

        if res.is_ok() {
            channel_count += 1;
        }

        // Also insert nested channel models if present
        for cm in &ch.models {
            let ch_id = if cm.channel_id > 0 { cm.channel_id } else { ch.id };
            let _ = sqlx::query(
                "INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?) ON CONFLICT(channel_id, name) DO NOTHING",
            )
            .bind(ch_id)
            .bind(&cm.name)
            .bind(cm.source.as_str())
            .execute(&state.db)
            .await;
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
            .execute(&state.db)
            .await
        } else {
            sqlx::query(
                "INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?) ON CONFLICT(channel_id, name) DO NOTHING",
            )
            .bind(cm.channel_id)
            .bind(&cm.name)
            .bind(cm.source.as_str())
            .execute(&state.db)
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
        let res = if grp.id > 0 {
            sqlx::query(
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
            .execute(&state.db)
            .await
        } else {
            sqlx::query("INSERT INTO groups (name, mode, active_item_id, relay_config) VALUES (?, ?, ?, ?)")
                .bind(&grp.name)
                .bind(grp.mode.as_str())
                .bind(grp.active_item_id)
                .bind(&relay_config_json)
                .execute(&state.db)
                .await
        };

        if res.is_ok() {
            group_count += 1;
        }

        // Also insert nested group items
        for item in &grp.items {
            let g_id = if item.group_id > 0 { item.group_id } else { grp.id };
            let _ = sqlx::query(
                "INSERT INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?) ON CONFLICT(group_id, channel_model_id) DO NOTHING",
            )
            .bind(g_id)
            .bind(item.channel_model_id)
            .bind(item.priority)
            .execute(&state.db)
            .await;
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
            .execute(&state.db)
            .await
        } else {
            sqlx::query(
                "INSERT INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?) ON CONFLICT(group_id, channel_model_id) DO NOTHING",
            )
            .bind(item.group_id)
            .bind(item.channel_model_id)
            .bind(item.priority)
            .execute(&state.db)
            .await
        };
        if res.is_ok() {
            gi_count += 1;
        }
    }
    rows_affected.insert("group_items".to_string(), gi_count);

    // 7. LLM Infos
    let llm_count = backup.llm_infos.len() as i64;
    let _ = crate::op::llm::batch_create_llm_infos(state, backup.llm_infos).await;
    rows_affected.insert("llm_infos".to_string(), llm_count);
    state.refresh_all_caches().await?;
    Ok(DBImportResult { rows_affected })
}
