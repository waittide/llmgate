use std::collections::HashMap;

use anyhow::Result;
use tracing::warn;

use crate::{
    model::{BackupData, DBImportResult},
    op::{
        AppState,
        apikey::list_api_keys,
        channel::list_channels,
        group::list_groups,
        model::list_model_infos,
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
    let model_infos = list_model_infos(state);
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
        model_infos,
        stats_total,
        stats_daily,
        stats_hourly,
        stats_api_key,
    })
}

/// 将备份数据合并导入当前库：按 id upsert（同 id 覆盖、无 id 追加），并非清空后还原。
/// 单行失败不影响其余行，失败明细记 warn 日志，结果按表返回成功行数。
pub async fn import_backup(state: &AppState, backup: BackupData) -> Result<DBImportResult> {
    let mut tx = state.db.begin().await?;
    let mut rows_affected = HashMap::new();

    // 1. 系统设置 (Settings)
    let mut setting_count = 0;
    for s in backup.settings {
        let res = sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(&s.key)
        .bind(&s.value)
        .execute(&mut *tx)
        .await;
        if let Err(e) = &res {
            warn!("导入设置失败（key={}）: {}", s.key, e);
        } else {
            setting_count += 1;
        }
    }
    rows_affected.insert("settings".to_string(), setting_count);

    // 2. API 密钥 (API Keys)
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

        if let Err(e) = &res {
            warn!("导入 API 密钥失败（name={}）: {}", key.name, e);
        } else {
            key_count += 1;
        }
    }
    rows_affected.insert("api_keys".to_string(), key_count);

    // 3. 渠道配置 (Channels)
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

        if let Err(e) = &res {
            warn!("导入渠道失败（name={}）: {}", ch.name, e);
        } else {
            channel_count += 1;
        }

        // 若存在内嵌渠道模型，同时插入并关联对应的 channel_id
        if inserted_id > 0 {
            for cm in &ch.models {
                if let Err(e) = sqlx::query(
                    "INSERT INTO channel_models (channel_id, name, source) VALUES (?, ?, ?) ON CONFLICT(channel_id, name) DO NOTHING",
                )
                .bind(inserted_id)
                .bind(&cm.name)
                .bind(cm.source.as_str())
                .execute(&mut *tx)
                .await
                {
                    warn!("导入内嵌渠道模型失败（channel_id={inserted_id} name={}）: {}", cm.name, e);
                }
            }
        }
    }
    rows_affected.insert("channels".to_string(), channel_count);

    // 4. 顶层渠道模型 (Channel Models)
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
        if let Err(e) = &res {
            warn!("导入渠道模型失败（name={}）: {}", cm.name, e);
        } else {
            cm_count += 1;
        }
    }
    rows_affected.insert("channel_models".to_string(), cm_count);

    // 5. 分组配置 (Groups)
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

        if let Err(e) = &res {
            warn!("导入分组失败（name={}）: {}", grp.name, e);
        } else {
            group_count += 1;
        }

        // 若存在内嵌分组成员，同时插入并关联对应的 group_id
        if inserted_id > 0 {
            for item in &grp.items {
                if let Err(e) = sqlx::query(
                    "INSERT INTO group_items (group_id, channel_model_id, priority) VALUES (?, ?, ?) ON CONFLICT(group_id, channel_model_id) DO NOTHING",
                )
                .bind(inserted_id)
                .bind(item.channel_model_id)
                .bind(item.priority)
                .execute(&mut *tx)
                .await
                {
                    warn!("导入内嵌分组成员失败（group_id={inserted_id} channel_model_id={}）: {}", item.channel_model_id, e);
                }
            }
        }
    }
    rows_affected.insert("groups".to_string(), group_count);

    // 6. 顶层分组成员 (Group Items)
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
        if let Err(e) = &res {
            warn!("导入分组成员失败（model_id={}）: {}", item.channel_model_id, e);
        } else {
            gi_count += 1;
        }
    }
    rows_affected.insert("group_items".to_string(), gi_count);

    // 7. 模型价格信息 (模型价格信息)
    let mut model_count = 0;
    for info in backup.model_infos {
        let name_lower = info.name.to_lowercase();
        let res = sqlx::query(
            r#"
            INSERT INTO model_infos (name, input, output, cache_read, cache_write)
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
        if let Err(e) = &res {
            warn!("导入模型价格失败（name={}）: {}", name_lower, e);
        } else {
            model_count += 1;
        }
    }
    rows_affected.insert("model_infos".to_string(), model_count);

    tx.commit().await?;
    state.refresh_all_caches().await?;
    Ok(DBImportResult { rows_affected })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Channel, ChannelModel, ChannelModelSource, ChannelProvider, Group, GroupMode},
        test_support::setup_test_state,
    };

    #[tokio::test]
    async fn export_import_round_trip_preserves_entities() {
        use crate::{
            model::{Channel, ChannelModel, Group, GroupItem, GroupMode},
            test_support::setup_test_state,
        };

        let (src, p1) = setup_test_state("backup_rt_src").await;
        let mut ch = Channel {
            name: "c1".into(),
            base_url: "https://example.com".into(),
            key: "k1".into(),
            models: vec![ChannelModel {
                name: "m1".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        crate::op::channel::create_channel(&src, &mut ch)
            .await
            .unwrap();
        let mut group = Group {
            name: "m1".into(),
            mode: GroupMode::Failover,
            items: vec![GroupItem {
                channel_model_id: ch.models[0].id,
                ..Default::default()
            }],
            ..Default::default()
        };
        crate::op::group::create_group(&src, &mut group)
            .await
            .unwrap();
        let backup = export_backup(&src).await.unwrap();

        let (dst, p2) = setup_test_state("backup_rt_dst").await;
        import_backup(&dst, backup).await.unwrap();

        let re = export_backup(&dst).await.unwrap();
        let src_channels: Vec<String> = re.channels.iter().map(|c| c.name.clone()).collect();
        assert!(src_channels.contains(&"c1".to_string()));
        let src_groups: Vec<String> = re.groups.iter().map(|g| g.name.clone()).collect();
        assert!(src_groups.contains(&"m1".to_string()));
        let group = re.groups.iter().find(|g| g.name == "m1").unwrap();
        assert_eq!(group.items.len(), 1);
        assert_eq!(group.mode, GroupMode::Failover);

        let _ = std::fs::remove_file(p1);
        let _ = std::fs::remove_file(p2);
    }

    #[tokio::test]
    async fn test_import_backup_channels_and_groups_with_zero_id() {
        let (state, db_path) = setup_test_state("backup").await;

        let backup = BackupData {
            channels: vec![Channel {
                id: 0, // 零 ID 导入
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
                id:             0, // 零 ID 导入
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

        // 验证渠道模型已成功插入，且外键正确关联新创建的渠道
        let model_row: (i64, i64, String) =
            sqlx::query_as("SELECT id, channel_id, name FROM channel_models WHERE name = 'gpt-4o'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert!(model_row.1 > 0, "嵌套渠道模型必须关联有效的 channel_id");

        let _ = std::fs::remove_file(db_path);
    }
}
