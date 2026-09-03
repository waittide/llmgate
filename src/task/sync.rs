use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Result;
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    model::{Channel, ChannelModel, ChannelModelSource, ChannelProvider},
    op::{AppState, channel},
    relay::upstream::{build_upstream_request, get_client},
};

static LAST_SYNC_TIME: AtomicI64 = AtomicI64::new(0);

pub fn get_last_sync_time() -> i64 {
    LAST_SYNC_TIME.load(Ordering::SeqCst)
}

pub async fn fetch_upstream_models(channel: &Channel) -> Result<Vec<String>> {
    let client = get_client();
    let resp = build_upstream_request(client, channel, "/models", None, 15)?
        .send()
        .await?;

    let body: Value = resp.json().await?;
    let mut models = Vec::new();

    match channel.channel_type {
        ChannelProvider::Gemini => {
            if let Some(models_arr) = body.get("models").and_then(|m| m.as_array()) {
                for item in models_arr {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        let clean_name = name.strip_prefix("models/").unwrap_or(name);
                        models.push(clean_name.to_string());
                    }
                }
            }
        }
        _ => {
            if let Some(data_arr) = body.get("data").and_then(|d| d.as_array()) {
                for item in data_arr {
                    if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                        models.push(id.to_string());
                    }
                }
            }
        }
    }

    if let Some(regex_str) = &channel.match_regex
        && !regex_str.trim().is_empty()
        && let Ok(re) = regex::Regex::new(regex_str)
    {
        models.retain(|m| re.is_match(m));
    }

    Ok(models)
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
                            id:         0,
                            channel_id: ch.id,
                            name:       f,
                            source:     ChannelModelSource::Auto,
                            metrics:    Default::default(),
                        });
                    }
                }

                let update_req = crate::model::ChannelUpdateRequest {
                    id:             ch.id,
                    name:           None,
                    channel_type:   None,
                    enabled:        None,
                    base_url:       None,
                    key:            None,
                    models:         Some(existing_models),
                    auto_sync:      None,
                    custom_header:  None,
                    param_override: None,
                    match_regex:    None,
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
