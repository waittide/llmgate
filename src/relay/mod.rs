pub mod protocol;
pub mod route;
pub mod state;
pub mod upstream;

use crate::{
    model::StatsMetrics,
    op::{channel, group, llm, stats, AppState},
    relay::{
        protocol::{
            calculate_cost, extract_usage_from_json, parse_request_metadata,
            rewrite_model_in_json, ApiFormat,
        },
        state::Usage,
        upstream::{build_upstream_request, get_client_for_channel},
    },
    server::{middleware::AuthContext, resp::error},
    utils::estimate_tokens,
};
use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use bytes::Bytes;
use futures::StreamExt;
use std::{sync::Arc, time::Instant};
use tracing::{error as log_error, warn};

pub async fn forward_relay(
    state: AppState,
    auth_ctx: Arc<AuthContext>,
    format: ApiFormat,
    body_bytes: Bytes,
) -> Response {
    let metadata = match parse_request_metadata(&body_bytes) {
        Some(m) if !m.model.trim().is_empty() => m,
        _ => return error(StatusCode::BAD_REQUEST, "Invalid request: missing model"),
    };

    // 1. Check API Key supported_models whitelist
    if !auth_ctx.supported_models.is_empty() && !auth_ctx.supported_models.contains(&metadata.model)
    {
        return error(
            StatusCode::UNAUTHORIZED,
            "model not supported by this api key",
        );
    }

    // 2. Check if group exists
    let group_obj = match group::get_group_by_name(&state, &metadata.model) {
        Ok(g) => g,
        Err(_) => return error(StatusCode::NOT_FOUND, "model not found"),
    };

    let endpoint = match format {
        ApiFormat::AnthropicMessage => "/messages",
        ApiFormat::OpenAIResponse => "/responses",
        _ => "/chat/completions",
    };

    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    let req_id = state
        .request_state_manager
        .new_request(metadata.model.clone(), body_str.clone(), auth_ctx.api_key.id);

    let start_time = Instant::now();
    let mut round = 0;
    let max_attempts = group_obj.relay_config.member_max_attempts.max(1);

    while round < max_attempts {
        round += 1;

        let current_group = match group::get_group_by_name(&state, &metadata.model) {
            Ok(g) => g,
            Err(_) => break,
        };

        let item = match state.route_manager.pick_group_item(&current_group) {
            Some(it) => it,
            None => {
                warn!("No group item available for group: {}", metadata.model);
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                continue;
            }
        };

        let channel_model_id = item.channel_model_id;
        let channel_model = match state.channel_models.get(&channel_model_id) {
            Some(cm) => cm.clone(),
            None => {
                state
                    .route_manager
                    .report_failure(current_group.id, item.id, 60);
                continue;
            }
        };

        let ch = match channel::get_channel(&state, channel_model.channel_id) {
            Ok(c) if c.enabled => c,
            _ => {
                state
                    .route_manager
                    .report_failure(current_group.id, item.id, 60);
                continue;
            }
        };

        let client = match get_client_for_channel(&ch) {
            Ok(c) => c,
            Err(e) => {
                log_error!("Failed to create client for channel {}: {}", ch.name, e);
                state
                    .route_manager
                    .report_failure(current_group.id, item.id, 60);
                continue;
            }
        };

        // Rewrite model in json
        let rewritten_body = rewrite_model_in_json(&body_bytes, &channel_model.name);

        state.request_state_manager.update_round(
            req_id,
            round,
            ch.name.clone(),
            channel_model.name.clone(),
            true,
        );

        let timeout_secs = if metadata.stream {
            current_group
                .relay_config
                .member_stream_first_event_timeout_seconds
                .max(10) as u64
        } else {
            current_group
                .relay_config
                .member_non_stream_response_timeout_seconds
                .max(10) as u64
        };

        let req_builder = match build_upstream_request(
            &client,
            &ch,
            endpoint,
            Some(rewritten_body),
            timeout_secs,
        ) {
            Ok(rb) => rb,
            Err(_e) => {
                state
                    .route_manager
                    .report_failure(current_group.id, item.id, 60);
                continue;
            }
        };

        let resp_result = req_builder.send().await;
        match resp_result {
            Ok(upstream_resp) => {
                let status = upstream_resp.status();
                if status.is_success() {
                    // Success!
                    state.route_manager.report_success(
                        current_group.id,
                        item.id,
                        current_group.relay_config.member_affinity_seconds,
                    );

                    let price_opt = llm::get_llm_price(&state, &channel_model.name);

                    if metadata.stream {
                        state.request_state_manager.mark_committed(req_id);

                        let _state_clone = state.clone();
                        let channel_id = ch.id;
                        let cm_id = channel_model.id;
                        let api_key_id = auth_ctx.api_key.id;
                        let _price_clone = price_opt.clone();

                        let raw_stream = upstream_resp.bytes_stream();
                        let mapped_stream = raw_stream.map(move |chunk_res| {
                            match chunk_res {
                                Ok(bytes) => Ok::<Bytes, std::io::Error>(bytes),
                                Err(e) => Err(std::io::Error::other(e)),
                            }
                        });

                        let body = Body::from_stream(mapped_stream);

                        // Record approximate stats on background finish
                        let elapsed_ms = start_time.elapsed().as_millis() as i64;
                        let prompt_tokens = estimate_tokens(&body_str);
                        let completion_tokens = prompt_tokens; // approximate stream
                        let cost = calculate_cost(prompt_tokens, completion_tokens, price_opt.as_ref());

                        let delta = StatsMetrics {
                            input_token: prompt_tokens,
                            output_token: completion_tokens,
                            input_cost: (prompt_tokens as f64 / 1_000_000.0)
                                * price_opt.as_ref().map(|p| p.input).unwrap_or(0.0),
                            output_cost: (completion_tokens as f64 / 1_000_000.0)
                                * price_opt.as_ref().map(|p| p.output).unwrap_or(0.0),
                            wait_time: elapsed_ms,
                            request_success: 1,
                            request_failed: 0,
                        };
                        stats::record_metrics(&state, api_key_id, channel_id, cm_id, &delta);

                        state.request_state_manager.finish(
                            req_id,
                            "success",
                            Usage {
                                prompt_tokens,
                                completion_tokens,
                                total_tokens: prompt_tokens + completion_tokens,
                            },
                            cost,
                            "[stream completed]".to_string(),
                            None,
                        );

                        return Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, "text/event-stream")
                            .header(header::CACHE_CONTROL, "no-cache")
                            .header(header::CONNECTION, "keep-alive")
                            .body(body)
                            .unwrap();
                    } else {
                        // Non-streaming response
                        let resp_bytes = upstream_resp.bytes().await.unwrap_or_default();
                        let resp_str = String::from_utf8_lossy(&resp_bytes).to_string();

                        let usage = match serde_json::from_slice::<serde_json::Value>(&resp_bytes) {
                            Ok(val) => extract_usage_from_json(&val).unwrap_or(Usage {
                                prompt_tokens: estimate_tokens(&body_str),
                                completion_tokens: estimate_tokens(&resp_str),
                                total_tokens: estimate_tokens(&body_str) + estimate_tokens(&resp_str),
                            }),
                            Err(_) => Usage {
                                prompt_tokens: estimate_tokens(&body_str),
                                completion_tokens: estimate_tokens(&resp_str),
                                total_tokens: estimate_tokens(&body_str) + estimate_tokens(&resp_str),
                            },
                        };

                        let cost = calculate_cost(
                            usage.prompt_tokens,
                            usage.completion_tokens,
                            price_opt.as_ref(),
                        );

                        let elapsed_ms = start_time.elapsed().as_millis() as i64;
                        let delta = StatsMetrics {
                            input_token: usage.prompt_tokens,
                            output_token: usage.completion_tokens,
                            input_cost: (usage.prompt_tokens as f64 / 1_000_000.0)
                                * price_opt.as_ref().map(|p| p.input).unwrap_or(0.0),
                            output_cost: (usage.completion_tokens as f64 / 1_000_000.0)
                                * price_opt.as_ref().map(|p| p.output).unwrap_or(0.0),
                            wait_time: elapsed_ms,
                            request_success: 1,
                            request_failed: 0,
                        };

                        stats::record_metrics(&state, auth_ctx.api_key.id, ch.id, channel_model.id, &delta);
                        state.request_state_manager.finish(
                            req_id,
                            "success",
                            usage,
                            cost,
                            resp_str,
                            None,
                        );

                        return Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(resp_bytes))
                            .unwrap();
                    }
                } else {
                    let err_body = upstream_resp.text().await.unwrap_or_default();
                    warn!(
                        "Upstream channel {} returned error {}: {}",
                        ch.name, status, err_body
                    );
                    state.route_manager.report_failure(
                        current_group.id,
                        item.id,
                        current_group.relay_config.member_cooldown_seconds,
                    );
                }
            }
            Err(e) => {
                warn!("Upstream channel {} request failed: {}", ch.name, e);
                state.route_manager.report_failure(
                    current_group.id,
                    item.id,
                    current_group.relay_config.member_cooldown_seconds,
                );
            }
        }
    }

    state.request_state_manager.finish(
        req_id,
        "failed",
        Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
        0.0,
        String::new(),
        Some("All failover targets exhausted".to_string()),
    );

    error(
        StatusCode::BAD_GATEWAY,
        "Upstream request failed after all attempts",
    )
}
