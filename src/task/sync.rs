use std::{
    collections::HashSet,
    sync::atomic::{AtomicI64, Ordering},
};

use anyhow::Result;
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    model::{Channel, ChannelModel, ChannelModelSource, ChannelProvider},
    op::{AppState, channel},
    relay::upstream::{build_upstream_headers, build_upstream_request, get_client},
};

static LAST_SYNC_TIME: AtomicI64 = AtomicI64::new(0);

pub fn get_last_sync_time() -> i64 {
    LAST_SYNC_TIME.load(Ordering::SeqCst)
}

pub async fn fetch_upstream_models(channel: &Channel) -> Result<Vec<String>> {
    let client = get_client();
    // 同步仅用首个 key：无失败反馈回路，轮询由转发路径负责
    let key_pool = channel.key_pool();
    let api_key = key_pool.first().map(String::as_str).unwrap_or("");
    // 模型列表没有会话语义：会话 ID 传空，渠道即便开启会话标识也不会写入该头
    let headers = build_upstream_headers(channel, api_key, None, "")?;
    let resp = build_upstream_request(client, channel, "/models", None, Some(15), headers)?
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
        ChannelProvider::OpenaiCompletion | ChannelProvider::OpenaiResponses | ChannelProvider::Anthropic => {
            if let Some(data_arr) = body.get("data").and_then(|d| d.as_array()) {
                for item in data_arr {
                    if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                        models.push(id.to_string());
                    }
                }
            }
        }
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

        let fetched = match fetch_upstream_models(&ch).await {
            Ok(fetched) => fetched,
            Err(e) => {
                warn!("渠道 {} 自动同步失败: {}", ch.name, e);
                continue;
            }
        };

        // 手工模型不受同步影响：它们既不被上游结果覆盖，也不算作新增
        let manual_names: HashSet<&str> = ch
            .models
            .iter()
            .filter(|m| m.source == ChannelModelSource::Manual)
            .map(|m| m.name.as_str())
            .collect();
        let current_auto: HashSet<&str> = ch
            .models
            .iter()
            .filter(|m| m.source == ChannelModelSource::Auto)
            .map(|m| m.name.as_str())
            .collect();
        let mut desired_auto: Vec<&str> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for name in &fetched {
            let name = name.as_str();
            if manual_names.contains(name) || !seen.insert(name) {
                continue;
            }
            desired_auto.push(name);
        }

        // 上游返回的模型集合与库中一致时跳过写回：update_channel 会连带重载渠道与
        // 分组两份全量缓存，没有变化就没有必要付出这份代价（每轮同步、每个渠道都会走到）。
        // 比较用集合语义：顺序差异不值得写一次库。
        if desired_auto.len() == current_auto.len() && desired_auto.iter().all(|name| current_auto.contains(name)) {
            continue;
        }

        let mut existing_models: Vec<ChannelModel> = ch
            .models
            .iter()
            .filter(|m| m.source == ChannelModelSource::Manual)
            .cloned()
            .collect();
        for name in desired_auto {
            if !current_auto.contains(name) {
                all_added_models.push(name.to_string());
            }
            existing_models.push(ChannelModel {
                id:         0,
                channel_id: ch.id,
                name:       name.to_string(),
                source:     ChannelModelSource::Auto,
                metrics:    Default::default(),
            });
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
            session_header: None,
        };
        if let Err(e) = channel::update_channel(state, &update_req).await {
            warn!("渠道 {} 同步结果写回失败: {}", ch.name, e);
        }
    }

    if !all_added_models.is_empty()
        && let Err(e) = crate::task::price::ensure_model_prices(state, &all_added_models).await
    {
        warn!("新增模型价格补齐失败: {}", e);
    }

    LAST_SYNC_TIME.store(chrono::Utc::now().timestamp(), Ordering::SeqCst);
    info!("渠道模型同步完成");
    Ok(())
}
