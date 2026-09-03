use std::{
    sync::{
        LazyLock,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use dashmap::DashMap;
use serde_json::Value;
use tracing::{debug, warn};

use crate::{
    model::{LLMInfo, LLMPrice},
    op::{AppState, llm},
};

// In-memory catalog of all upstream model prices from models.dev (not in DB)
static GLOBAL_CATALOG_PRICES: LazyLock<DashMap<String, LLMPrice>> = LazyLock::new(DashMap::new);
static LAST_PRICE_UPDATE_TIME: AtomicI64 = AtomicI64::new(0);

pub fn get_last_price_update_time() -> i64 {
    LAST_PRICE_UPDATE_TIME.load(Ordering::SeqCst)
}

pub fn get_catalog_price(model_name: &str) -> Option<LLMPrice> {
    GLOBAL_CATALOG_PRICES
        .get(&model_name.to_lowercase())
        .map(|p| p.clone())
}

pub async fn fetch_catalog_from_models_dev() -> Result<()> {
    let client = crate::relay::upstream::get_client();

    let resp = client
        .get("https://models.dev/api.json")
        .header("User-Agent", "Octopus-Rust/0.1.0")
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    if !resp.status().is_success() {
        warn!("Failed to fetch pricing from models.dev: {}", resp.status());
        return Ok(());
    }

    let data: Value = resp.json().await?;
    if let Some(obj) = data.as_object() {
        for (_provider, prov_val) in obj {
            if let Some(models_obj) = prov_val.get("models").and_then(|m| m.as_object()) {
                for (model_name, model_val) in models_obj {
                    if let Some(cost_obj) = model_val.get("cost").and_then(|c| c.as_object()) {
                        let input = cost_obj
                            .get("input")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let output = cost_obj
                            .get("output")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let cache_read = cost_obj
                            .get("cache_read")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let cache_write = cost_obj
                            .get("cache_write")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);

                        GLOBAL_CATALOG_PRICES.insert(
                            model_name.to_lowercase(),
                            LLMPrice {
                                input,
                                output,
                                cache_read,
                                cache_write,
                            },
                        );
                    }
                }
            }
        }
    }

    LAST_PRICE_UPDATE_TIME.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);
    debug!(
        "Catalog prices refreshed from models.dev ({} models cached in memory)",
        GLOBAL_CATALOG_PRICES.len()
    );
    Ok(())
}

/// Ensures only the specified channel models exist in the database with catalog prices.
pub async fn ensure_model_prices(state: &AppState, model_names: &[String]) -> Result<()> {
    if GLOBAL_CATALOG_PRICES.is_empty() {
        let _ = fetch_catalog_from_models_dev().await;
    }

    let mut new_infos = Vec::new();
    for name in model_names {
        let lower = name.to_lowercase();
        let price = get_catalog_price(&lower).unwrap_or_default();
        new_infos.push(LLMInfo { name: lower, price });
    }

    if !new_infos.is_empty() {
        llm::batch_create_llm_infos(state, new_infos).await?;
    }
    Ok(())
}

/// Updates prices for the models currently configured in channels.
pub async fn update_llm_prices(state: &AppState) -> Result<()> {
    fetch_catalog_from_models_dev().await?;

    let existing_infos = llm::list_llm_infos(state);
    let mut updated_infos = Vec::new();

    for info in existing_infos {
        if let Some(new_price) = get_catalog_price(&info.name) {
            updated_infos.push(LLMInfo {
                name:  info.name,
                price: new_price,
            });
        }
    }

    if !updated_infos.is_empty() {
        llm::batch_create_llm_infos(state, updated_infos).await?;
    }

    debug!("Channel model prices updated");
    Ok(())
}

/// Cleans up ghost models and rebuilds prices from channels.
pub async fn rebuild_all_prices(state: &AppState) -> Result<()> {
    fetch_catalog_from_models_dev().await?;
    llm::cleanup_ghosts(state).await?;

    let mut channel_model_names = Vec::new();
    for cm in state.channel_models.iter() {
        channel_model_names.push(cm.value().name.clone());
    }

    ensure_model_prices(state, &channel_model_names).await?;
    Ok(())
}
