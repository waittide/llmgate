use crate::{
    model::{Channel, ChannelModel, ChannelModelSource, ChannelProvider},
    op::{channel, AppState},
    relay::upstream::{get_client_for_channel, normalize_base_url},
};
use anyhow::Result;
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};
use tracing::{info, warn};

static LAST_SYNC_TIME: AtomicI64 = AtomicI64::new(0);

pub fn get_last_sync_time() -> i64 {
    LAST_SYNC_TIME.load(Ordering::SeqCst)
}

pub async fn fetch_upstream_models(channel: &Channel) -> Result<Vec<String>> {
    let client = get_client_for_channel(channel)?;
    let mut models = match channel.channel_type {
        ChannelProvider::Anthropic => fetch_anthropic_models(&client, channel).await?,
        ChannelProvider::Gemini => fetch_gemini_models(&client, channel).await?,
        _ => fetch_openai_models(&client, channel).await?,
    };

    if let Some(regex_str) = &channel.match_regex
        && !regex_str.trim().is_empty()
        && let Ok(re) = regex::Regex::new(regex_str)
    {
        models.retain(|m| re.is_match(m));
    }

    Ok(models)
}

async fn fetch_openai_models(client: &reqwest::Client, channel: &Channel) -> Result<Vec<String>> {
    let base_url = normalize_base_url(&channel.base_url);
    let target_url = format!("{}/models", base_url);

    let mut req = client
        .get(&target_url)
        .header("Authorization", format!("Bearer {}", channel.key))
        .timeout(std::time::Duration::from_secs(15));

    for h in &channel.custom_header {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(h.header_key.as_bytes()),
            reqwest::header::HeaderValue::from_str(&h.header_value),
        ) {
            req = req.header(name, val);
        }
    }
    let resp = req.send().await?;

    let body: Value = resp.json().await?;
    let mut model_names = Vec::new();
    if let Some(data_arr) = body.get("data").and_then(|d| d.as_array()) {
        for item in data_arr {
            if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                model_names.push(id.to_string());
            }
        }
    }
    Ok(model_names)
}

async fn fetch_anthropic_models(client: &reqwest::Client, channel: &Channel) -> Result<Vec<String>> {
    let base_url = normalize_base_url(&channel.base_url);
    let target_url = format!("{}/models", base_url);

    let mut req = client
        .get(&target_url)
        .header("x-api-key", &channel.key)
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(15));

    for h in &channel.custom_header {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(h.header_key.as_bytes()),
            reqwest::header::HeaderValue::from_str(&h.header_value),
        ) {
            req = req.header(name, val);
        }
    }
    let resp = req.send().await?;
    let body: Value = resp.json().await?;
    let mut model_names = Vec::new();
    if let Some(data_arr) = body.get("data").and_then(|d| d.as_array()) {
        for item in data_arr {
            if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                model_names.push(id.to_string());
            }
        }
    }
    Ok(model_names)
}

async fn fetch_gemini_models(client: &reqwest::Client, channel: &Channel) -> Result<Vec<String>> {
    let base_url = normalize_base_url(&channel.base_url);
    let target_url = format!("{}/models", base_url);

    let mut req = client
        .get(&target_url)
        .header("x-goog-api-key", &channel.key)
        .timeout(std::time::Duration::from_secs(15));

    for h in &channel.custom_header {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(h.header_key.as_bytes()),
            reqwest::header::HeaderValue::from_str(&h.header_value),
        ) {
            req = req.header(name, val);
        }
    }
    let resp = req.send().await?;
    let body: Value = resp.json().await?;
    let mut model_names = Vec::new();
    if let Some(models_arr) = body.get("models").and_then(|m| m.as_array()) {
        for item in models_arr {
            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                let clean_name = name.strip_prefix("models/").unwrap_or(name);
                model_names.push(clean_name.to_string());
            }
        }
    }
    Ok(model_names)
}

pub async fn sync_all_channel_models(state: &AppState) -> Result<()> {
    let channels = channel::list_channels(state);
    let mut all_added_models = Vec::new();

    for ch in channels {
        if !ch.enabled || !ch.auto_sync {
            continue;
        }

        match fetch_upstream_models(&ch).await {
            Ok(fetched) => {
                let mut manual_names = std::collections::HashSet::new();
                let mut existing_models = Vec::new();

                for m in &ch.models {
                    if m.source == ChannelModelSource::Manual {
                        manual_names.insert(m.name.clone());
                        existing_models.push(m.clone());
                    }
                }

                for f in fetched {
                    if !manual_names.contains(&f) {
                        all_added_models.push(f.clone());
                        existing_models.push(ChannelModel {
                            id: 0,
                            channel_id: ch.id,
                            name: f,
                            source: ChannelModelSource::Auto,
                            metrics: Default::default(),
                        });
                    }
                }

                let update_req = crate::model::ChannelUpdateRequest {
                    id: ch.id,
                    name: None,
                    channel_type: None,
                    enabled: None,
                    base_url: None,
                    key: None,
                    models: Some(existing_models),
                    auto_sync: None,
                    custom_header: None,
                    param_override: None,
                    match_regex: None,
                };
                let _ = channel::update_channel(state, &update_req).await;
            }
            Err(e) => {
                warn!("Auto-sync failed for channel {}: {}", ch.name, e);
            }
        }
    }

    if !all_added_models.is_empty() {
        let _ = crate::task::price::ensure_model_prices(state, &all_added_models).await;
    }

    LAST_SYNC_TIME.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);
    info!("Channel models sync completed");
    Ok(())
}
