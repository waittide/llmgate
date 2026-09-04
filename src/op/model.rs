use anyhow::{Result, anyhow};
use sqlx::Row;

use crate::{
    model::{ModelInfo, ModelPrice},
    op::AppState,
};

pub async fn refresh_model_cache(state: &AppState) -> Result<()> {
    let rows = sqlx::query("SELECT name, input, output, cache_read, cache_write FROM model_infos")
        .fetch_all(&state.db)
        .await?;

    let mut local_prices = std::collections::HashMap::new();
    for row in rows {
        let name: String = row.get("name");
        let price = ModelPrice {
            input:       row.get("input"),
            output:      row.get("output"),
            cache_read:  row.get("cache_read"),
            cache_write: row.get("cache_write"),
        };
        local_prices.insert(name.to_lowercase(), price);
    }
    // 原子同步内存缓存，避免留出空窗期
    for (k, v) in &local_prices {
        state.model_prices.insert(k.clone(), v.clone());
    }
    state
        .model_prices
        .retain(|k, _| local_prices.contains_key(k));
    Ok(())
}

pub fn list_model_infos(state: &AppState) -> Vec<ModelInfo> {
    let mut list: Vec<ModelInfo> = state
        .model_prices
        .iter()
        .map(|entry| ModelInfo {
            name:  entry.key().clone(),
            price: entry.value().clone(),
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

pub fn get_model_price(state: &AppState, model_name: &str) -> Option<ModelPrice> {
    let lower = model_name.to_lowercase();
    if let Some(p) = state.model_prices.get(&lower) {
        return Some(p.clone());
    }
    crate::task::price::get_catalog_price(&lower)
}

pub async fn batch_create_model_infos(state: &AppState, infos: Vec<ModelInfo>) -> Result<()> {
    for info in infos {
        let name_lower = info.name.to_lowercase();
        sqlx::query(
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
        .execute(&state.db)
        .await?;

        state.model_prices.insert(name_lower, info.price);
    }
    Ok(())
}

pub async fn update_model_info(state: &AppState, info: &ModelInfo) -> Result<()> {
    let name_lower = info.name.to_lowercase();
    sqlx::query(
        r#"
        UPDATE model_infos
        SET input = ?, output = ?, cache_read = ?, cache_write = ?
        WHERE name = ?
        "#,
    )
    .bind(info.price.input)
    .bind(info.price.output)
    .bind(info.price.cache_read)
    .bind(info.price.cache_write)
    .bind(&name_lower)
    .execute(&state.db)
    .await?;

    state.model_prices.insert(name_lower, info.price.clone());
    Ok(())
}

pub async fn delete_model_info(state: &AppState, name: &str) -> Result<()> {
    let name_lower = name.to_lowercase();
    // 检查是否仍被某个渠道模型引用
    for cm in state.channel_models.iter() {
        if cm.value().name.to_lowercase() == name_lower {
            return Err(anyhow!("模型仍被渠道引用"));
        }
    }

    sqlx::query("DELETE FROM model_infos WHERE name = ?")
        .bind(&name_lower)
        .execute(&state.db)
        .await?;

    state.model_prices.remove(&name_lower);
    Ok(())
}

pub async fn cleanup_ghosts(state: &AppState) -> Result<()> {
    sqlx::query("DELETE FROM model_infos WHERE LOWER(name) NOT IN (SELECT LOWER(name) FROM channel_models)")
        .execute(&state.db)
        .await?;

    refresh_model_cache(state).await?;
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test_support::setup_test_state;

    #[tokio::test]
    async fn test_cleanup_ghosts_removes_ghosts_from_db_and_cache() {
        let (state, db_path) = setup_test_state("model").await;

        // 插入一个有效模型价格与一个无引用的幽灵模型价格
        let infos = vec![
            ModelInfo {
                name:  "valid-model".to_string(),
                price: ModelPrice {
                    input: 1.0,
                    output: 2.0,
                    ..Default::default()
                },
            },
            ModelInfo {
                name:  "ghost-model".to_string(),
                price: ModelPrice {
                    input: 3.0,
                    output: 4.0,
                    ..Default::default()
                },
            },
        ];
        batch_create_model_infos(&state, infos).await.unwrap();
        assert!(state.model_prices.contains_key("ghost-model"));
        assert!(state.model_prices.contains_key("valid-model"));

        // 模拟创建引用 "valid-model" 的渠道模型
        let ch_res = sqlx::query(
            "INSERT INTO channels (name, type, base_url, key) VALUES ('c1', 'openai', 'https://api.openai.com', 'k1')",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let ch_id = ch_res.last_insert_rowid();
        sqlx::query("INSERT INTO channel_models (channel_id, name, source) VALUES (?, 'valid-model', 'manual')")
            .bind(ch_id)
            .execute(&state.db)
            .await
            .unwrap();

        // 清理无渠道引用的幽灵模型
        cleanup_ghosts(&state).await.unwrap();

        // 幽灵模型应从数据库与内存缓存中一并删除
        assert!(!state.model_prices.contains_key("ghost-model"));
        assert!(state.model_prices.contains_key("valid-model"));

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM model_infos WHERE name = 'ghost-model'")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count.0, 0);

        let _ = std::fs::remove_file(db_path);
    }
}
