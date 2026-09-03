use crate::{
    model::{LLMInfo, LLMPrice},
    op::AppState,
};
use anyhow::{anyhow, Result};
use sqlx::Row;

pub async fn refresh_llm_cache(state: &AppState) -> Result<()> {
    let rows = sqlx::query("SELECT name, input, output, cache_read, cache_write FROM llm_infos")
        .fetch_all(&state.db)
        .await?;

    let mut local_prices = std::collections::HashMap::new();
    for row in rows {
        let name: String = row.get("name");
        let price = LLMPrice {
            input: row.get("input"),
            output: row.get("output"),
            cache_read: row.get("cache_read"),
            cache_write: row.get("cache_write"),
        };
        local_prices.insert(name.to_lowercase(), price);
    }
    // Atomically sync in-memory cache without leaving empty windows
    for (k, v) in &local_prices {
        state.llm_prices.insert(k.clone(), v.clone());
    }
    state.llm_prices.retain(|k, _| local_prices.contains_key(k));
    Ok(())
}

pub fn list_llm_infos(state: &AppState) -> Vec<LLMInfo> {
    let mut list: Vec<LLMInfo> = state
        .llm_prices
        .iter()
        .map(|entry| LLMInfo {
            name: entry.key().clone(),
            price: entry.value().clone(),
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

pub fn get_llm_price(state: &AppState, model_name: &str) -> Option<LLMPrice> {
    let lower = model_name.to_lowercase();
    if let Some(p) = state.llm_prices.get(&lower) {
        return Some(p.clone());
    }
    crate::task::price::get_catalog_price(&lower)
}

pub async fn batch_create_llm_infos(state: &AppState, infos: Vec<LLMInfo>) -> Result<()> {
    for info in infos {
        let name_lower = info.name.to_lowercase();
        sqlx::query(
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
        .execute(&state.db)
        .await?;

        state.llm_prices.insert(name_lower, info.price);
    }
    Ok(())
}

pub async fn update_llm_info(state: &AppState, info: &LLMInfo) -> Result<()> {
    let name_lower = info.name.to_lowercase();
    sqlx::query(
        r#"
        UPDATE llm_infos
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

    state.llm_prices.insert(name_lower, info.price.clone());
    Ok(())
}

pub async fn delete_llm_info(state: &AppState, name: &str) -> Result<()> {
    let name_lower = name.to_lowercase();
    // Check if referenced by any channel model
    for cm in state.channel_models.iter() {
        if cm.value().name.to_lowercase() == name_lower {
            return Err(anyhow!("Model is referenced by a channel"));
        }
    }

    sqlx::query("DELETE FROM llm_infos WHERE name = ?")
        .bind(&name_lower)
        .execute(&state.db)
        .await?;

    state.llm_prices.remove(&name_lower);
    Ok(())
}

pub async fn cleanup_ghosts(state: &AppState) -> Result<()> {
    sqlx::query(
        "DELETE FROM llm_infos WHERE LOWER(name) NOT IN (SELECT LOWER(name) FROM channel_models)"
    )
    .execute(&state.db)
    .await?;

    refresh_llm_cache(state).await?;
    Ok(())
}
