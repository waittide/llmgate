//! 转发管线：请求校验、载荷准备、key 轮换发送、故障转移与结果处理。

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode, Uri, header},
    response::Response,
};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::Value;

use crate::{
    model::{Channel, ChannelModel, ChannelProvider, Group, ModelPrice, RequestLog, StatsMetrics, request_status},
    op::{AppState, channel, group, log, model, stats},
    relay::{
        finalizer::{RESPONSE_CAPTURE_LIMIT, StreamFinalizer},
        ir,
        protocol::{
            ApiFormat, calculate_cost, calculate_input_cost, extract_usage_from_json, parse_request_metadata,
            rewrite_request_body_in_json,
        },
        state::{RequestOverview, Usage, millis_to_rfc3339},
        upstream::{build_upstream_request, get_client, normalize_base_url},
    },
    server::{middleware::AuthContext, resp::error},
    utils::{estimate_tokens, truncate_utf8},
};

/// 上游为 Chat Completions 时的形状校验：assistant 的每个 tool_call 都必须有
/// 配对的 tool 消息，且 tool 消息不得早于其 tool_calls。拦截本可在代理侧
/// 明确表达的 400，避免上游返回语义模糊的错误。
fn validate_chat_tool_pairing(body: &Value) -> Result<(), String> {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return Ok(());
    };
    let mut pending: Vec<String> = Vec::new();
    for msg in messages {
        match msg.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        pending.push(
                            call.get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                }
            }
            Some("tool") => {
                let id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match pending.iter().position(|p| p == id) {
                    Some(pos) => {
                        pending.remove(pos);
                    }
                    None => {
                        return Err(format!("tool 消息 tool_call_id={id} 没有对应的 assistant tool_calls"));
                    }
                }
            }
            Some("user") if !pending.is_empty() => {
                return Err("assistant tool_calls 尚未全部得到 tool 结果就出现了新的 user 消息".to_string());
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        return Err(format!("assistant tool_calls 缺少配对的 tool 消息: {pending:?}"));
    }
    Ok(())
}

/// 判断上游返回的 HTTP 状态码是否属于客户端语法或格式错误。
///
/// 鉴权失败 (401/403)、上游缺少模型 (404)、限流超额 (429) 或服务端故障 (5xx) 属于渠道问题，
/// 必须触发该渠道进入冷却并自动故障转移至其他健康渠道。
/// 仅 400 Bad Request 与 422 Unprocessable Entity 视为不可重试的客户端请求错误。
/// 将 HTTP HeaderMap 序列化为 JSON 字符串对象：header名 -> 逗号分隔的值列表。
pub(crate) fn serialize_headers(headers: &HeaderMap) -> String {
    let mut map: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (name, value) in headers.iter() {
        map.entry(name.as_str().to_string())
            .or_default()
            .push(value.to_str().unwrap_or("<non-utf8>").to_string());
    }
    let obj: serde_json::Map<String, serde_json::Value> = map
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v.join(", "))))
        .collect();
    serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default()
}

pub(crate) fn is_client_error(status: StatusCode) -> bool {
    matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY)
}

fn validate_relay_request(
    state: &AppState,
    auth_ctx: &AuthContext,
    body_bytes: &Bytes,
    path_model: Option<&str>,
) -> Result<(super::protocol::ChatRequestMetadata, Group), Box<Response>> {
    let mut metadata = parse_request_metadata(body_bytes).unwrap_or_default();
    // Gemini 客户端把模型放在 URL path（models/{model}:generateContent），
    // 请求体本身没有 model 字段；其余格式以请求体为准，path 模型兜底
    if metadata.model.trim().is_empty() {
        metadata.model = path_model.unwrap_or_default().trim_matches('/').to_string();
    }
    if metadata.model.trim().is_empty() {
        return Err(Box::new(error(
            StatusCode::BAD_REQUEST,
            "Invalid request: missing model",
        )));
    }

    if !auth_ctx.supported_models.is_empty() && !auth_ctx.supported_models.contains(&metadata.model) {
        return Err(Box::new(error(
            StatusCode::UNAUTHORIZED,
            "model not supported by this api key",
        )));
    }

    let group_obj = match group::get_group_by_name(state, &metadata.model) {
        Ok(g) => g,
        Err(_) => return Err(Box::new(error(StatusCode::NOT_FOUND, "model not found"))),
    };

    Ok((metadata, group_obj))
}

fn gemini_endpoint(ch: &Channel, model: &str, stream: bool) -> String {
    let model = model.strip_prefix("models/").unwrap_or(model);
    let version_prefix = if normalize_base_url(&ch.base_url).ends_with("/v1beta") {
        ""
    } else {
        "/v1beta"
    };
    if stream {
        format!("{version_prefix}/models/{model}:streamGenerateContent?alt=sse")
    } else {
        format!("{version_prefix}/models/{model}:generateContent")
    }
}

fn prepare_upstream_payload(
    format: ApiFormat,
    stream: bool,
    body_bytes: &Bytes,
    ch: &Channel,
    channel_model: &ChannelModel,
) -> Result<(String, Vec<u8>), String> {
    let rewritten_body = rewrite_request_body_in_json(body_bytes, &channel_model.name, ch.param_override.as_deref());
    let target_format = ir::channel_format(&ch.channel_type);
    let endpoint = match &ch.channel_type {
        ChannelProvider::Anthropic => "/messages".to_string(),
        ChannelProvider::Gemini => gemini_endpoint(ch, &channel_model.name, stream),
        ChannelProvider::OpenaiCompletion => "/chat/completions".to_string(),
        ChannelProvider::OpenaiResponses => "/responses".to_string(),
    };

    // 同格式直通：仅重写 model/param_override 后原样转发
    if format == target_format {
        return Ok((endpoint, rewritten_body));
    }

    // 跨格式：原始格式 → IR → 目标格式
    let converted = serde_json::from_slice::<serde_json::Value>(&rewritten_body)
        .ok()
        .and_then(|val| ir::parse_request(format, &val))
        .map(|req| ir::emit_request(&req, target_format));

    let body = match &converted {
        Some(out) => {
            if target_format == ApiFormat::OpenaiCompletion
                && let Err(msg) = validate_chat_tool_pairing(out)
            {
                return Err(msg);
            }
            serde_json::to_vec(out).ok()
        }
        None => None,
    }
    .unwrap_or(rewritten_body);
    Ok((endpoint, body))
}

struct RoundFailure<'a> {
    round:               i64,
    model:               &'a str,
    started_at:          &'a str,
    channel_name:        &'a str,
    model_name:          &'a str,
    target_req_url:      &'a str,
    channel_req_headers: &'a str,
    rewritten_body_str:  &'a str,
    response_headers:    &'a str,
    status:              u16,
    duration_ms:         i64,
    err_body:            &'a str,
    should_cooldown:     bool,
}
fn record_round_failure(
    state: &AppState,
    group_id: i64,
    item_id: i64,
    cooldown_seconds: i64,
    req_id: u64,
    rounds: &mut Vec<crate::model::RoundLog>,
    fail: RoundFailure<'_>,
) {
    let round_log = crate::model::RoundLog {
        round:            fail.round,
        channel_name:     fail.channel_name.to_string(),
        model_name:       fail.model_name.to_string(),
        request_url:      fail.target_req_url.to_string(),
        request_headers:  fail.channel_req_headers.to_string(),
        request_body:     fail.rewritten_body_str.to_string(),
        response_url:     fail.target_req_url.to_string(),
        response_headers: fail.response_headers.to_string(),
        response_body:    fail.err_body.to_string(),
        status:           fail.status,
        duration_ms:      fail.duration_ms,
        error:            Some(fail.err_body.to_string()),
    };
    rounds.push(round_log.clone());
    // 客户端错误不触发冷却；该轮连同错误详情已记录在请求日志中，不再输出到终端
    if fail.should_cooldown {
        state
            .route_manager
            .report_failure(group_id, item_id, cooldown_seconds);
    }

    state.in_flight.broadcast(RequestOverview {
        id: req_id,
        status: request_status::RUNNING.to_string(),
        started_at: fail.started_at.to_string(),
        duration: fail.duration_ms,
        model: fail.model.to_string(),
        round: fail.round,
        target_channel: fail.channel_name.to_string(),
        target_model: fail.model_name.to_string(),
        error: Some(fail.err_body.to_string()),
        rounds: rounds.clone(),
        ..Default::default()
    });
}

struct SuccessContext<'a> {
    /// 本轮日志基座：id/model/target/url/headers/body/round/created_at 已由 forward_relay 填好
    log_base:         RequestLog,
    format:           ApiFormat,
    channel:          &'a Channel,
    channel_model:    &'a ChannelModel,
    api_key_id:       i64,
    body_str:         &'a str,
    round_start_time: Instant,
    start_time:       Instant,
    price_opt:        Option<ModelPrice>,
    rounds:           Vec<crate::model::RoundLog>,
}

fn handle_successful_stream(state: &AppState, upstream_resp: reqwest::Response, ctx: SuccessContext<'_>) -> Response {
    state.in_flight.broadcast(RequestOverview {
        id: ctx.log_base.id as u64,
        status: request_status::COMMITTED.to_string(),
        started_at: millis_to_rfc3339(ctx.log_base.created_at),
        duration: ctx.start_time.elapsed().as_millis() as i64,
        model: ctx.log_base.model.clone(),
        round: ctx.log_base.round,
        target_channel: ctx.log_base.target_channel.clone(),
        target_model: ctx.log_base.target_model.clone(),
        sending: true,
        rounds: ctx.rounds.clone(),
        ..Default::default()
    });

    let mut log_template = ctx.log_base.clone();
    log_template.response_headers = serialize_headers(upstream_resp.headers());

    let raw_stream = upstream_resp.bytes_stream();
    let mapped_stream = raw_stream.map(move |chunk_res| match chunk_res {
        Ok(bytes) => Ok::<Bytes, std::io::Error>(bytes),
        Err(e) => Err(std::io::Error::other(e)),
    });

    let prompt_tokens = estimate_tokens(ctx.body_str);

    // 跨格式：上游 SSE → IR 事件 → 客户端 SSE；同格式直通
    let upstream_format = ir::channel_format(&ctx.channel.channel_type);
    let output_stream = if ctx.format == upstream_format {
        mapped_stream.boxed()
    } else {
        ir::streaming::translate(mapped_stream, upstream_format, ctx.format, ctx.log_base.model.clone())
    };
    let finalizer = StreamFinalizer {
        inner: output_stream,
        state: state.clone(),
        req_id: ctx.log_base.id as u64,
        api_key_id: ctx.api_key_id,
        channel_id: ctx.channel.id,
        channel_model_id: ctx.channel_model.id,
        price: ctx.price_opt,
        prompt_tokens,
        buffer: Vec::new(),
        output_bytes: 0,
        usage: None,
        start_time: ctx.start_time,
        done: false,
        log_template,
        previous_rounds: ctx.rounds,
        response_capture: Vec::new(),
        cancel_rx: state.in_flight.get_cancel_receiver(ctx.log_base.id as u64),
        interrupted: false,
        stall_timer: None,
        stall_timeout: Duration::from_secs(
            state
                .settings
                .get("stream_stall_timeout_seconds")
                .and_then(|e| e.value().parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(300),
        ),
        error_override: None,
    };
    let body = Body::from_stream(finalizer);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(body)
        .unwrap_or_else(|e| {
            tracing::error!("构建 SSE 响应失败: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Internal Server Error"))
                .unwrap_or_default()
        })
}

async fn handle_successful_json(
    state: &AppState,
    upstream_resp: reqwest::Response,
    mut ctx: SuccessContext<'_>,
) -> Response {
    let response_headers = serialize_headers(upstream_resp.headers());
    let raw_bytes = upstream_resp.bytes().await.unwrap_or_default();
    let parsed = serde_json::from_slice::<serde_json::Value>(&raw_bytes).ok();
    // 跨格式：上游格式 → IR → 客户端格式；同格式直通
    let upstream_format = ir::channel_format(&ctx.channel.channel_type);
    let converted = if ctx.format == upstream_format {
        parsed.clone()
    } else {
        parsed.as_ref().and_then(|val| {
            let resp = ir::parse_response(upstream_format, val, &ctx.log_base.model)?;
            Some(ir::emit_response(&resp, ctx.format))
        })
    };
    let resp_bytes = converted
        .as_ref()
        .and_then(|val| serde_json::to_vec(val).ok())
        .map(Bytes::from)
        .unwrap_or(raw_bytes);
    // 入库用截断版本，避免大响应撑爆 request_logs；返回给客户端的 resp_bytes 不受影响
    let resp_str = truncate_utf8(&String::from_utf8_lossy(&resp_bytes), RESPONSE_CAPTURE_LIMIT).to_string();

    let usage = converted
        .as_ref()
        .or(parsed.as_ref())
        .and_then(extract_usage_from_json)
        .unwrap_or_else(|| Usage {
            prompt_tokens: estimate_tokens(ctx.body_str),
            completion_tokens: estimate_tokens(&resp_str),
            total_tokens: estimate_tokens(ctx.body_str) + estimate_tokens(&resp_str),
            ..Default::default()
        });

    let cost = calculate_cost(
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
        ctx.price_opt.as_ref(),
    );
    let elapsed_ms = ctx.start_time.elapsed().as_millis() as i64;
    let round_duration_ms = ctx.round_start_time.elapsed().as_millis() as i64;

    let round_log = crate::model::RoundLog {
        round:            ctx.log_base.round,
        channel_name:     ctx.log_base.target_channel.clone(),
        model_name:       ctx.log_base.target_model.clone(),
        request_url:      ctx.log_base.request_url.clone(),
        request_headers:  ctx.log_base.request_headers.clone(),
        request_body:     ctx.log_base.request_body.clone(),
        response_url:     ctx.log_base.response_url.clone(),
        response_headers: response_headers.clone(),
        response_body:    resp_str.clone(),
        status:           200,
        duration_ms:      round_duration_ms,
        error:            None,
    };
    ctx.rounds.push(round_log.clone());
    let mut req_log = ctx.log_base.clone();
    req_log.status = request_status::SUCCESS.to_string();
    req_log.response_headers = response_headers;
    req_log.response_body = resp_str;
    req_log.prompt_tokens = usage.prompt_tokens;
    req_log.completion_tokens = usage.completion_tokens;
    req_log.total_tokens = usage.total_tokens;
    req_log.cache_read_tokens = usage.cache_read_tokens;
    req_log.cache_write_tokens = usage.cache_write_tokens;
    req_log.cost = cost;
    req_log.wait_time = elapsed_ms;
    req_log.error = None;
    req_log.rounds_detail = serde_json::to_string(&ctx.rounds).unwrap_or_default();

    state.in_flight.broadcast(RequestOverview::from(&req_log));
    state.in_flight.unregister(ctx.log_base.id as u64);

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = log::update_completed_log(&state_clone, &req_log).await {
            tracing::error!("写入已完成请求日志失败: {}", e);
        }
    });

    let output_cost =
        (usage.completion_tokens as f64 / 1_000_000.0) * ctx.price_opt.as_ref().map(|p| p.output).unwrap_or(0.0);
    let input_cost = calculate_input_cost(
        usage.prompt_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
        ctx.price_opt.as_ref(),
    );
    let delta = StatsMetrics {
        input_token: usage.prompt_tokens,
        output_token: usage.completion_tokens,
        input_cost,
        output_cost,
        wait_time: elapsed_ms,
        request_success: 1,
        request_failed: 0,
    };

    stats::record_metrics(state, ctx.api_key_id, ctx.channel.id, ctx.channel_model.id, &delta);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(resp_bytes))
        .unwrap_or_else(|e| {
            tracing::error!("构建 JSON 响应失败: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Internal Server Error"))
                .unwrap_or_default()
        })
}

struct FailedRelayContext {
    /// 客户端侧日志基座（记录原始请求 url/headers/body，与成功路径的上游侧信息不同）
    log_base:              RequestLog,
    last_target_channel:   String,
    last_target_model:     String,
    last_response_url:     String,
    last_response_headers: String,
    last_status:           StatusCode,
    last_error_body:       String,
    rounds:                Vec<crate::model::RoundLog>,
    wait_time_ms:          i64,
    round:                 i64,
    api_key_id:            i64,
}

fn build_failed_relay_response(state: &AppState, ctx: FailedRelayContext) -> Response {
    let failed_delta = StatsMetrics {
        input_token:     0,
        output_token:    0,
        input_cost:      0.0,
        output_cost:     0.0,
        wait_time:       ctx.wait_time_ms,
        request_success: 0,
        request_failed:  1,
    };
    stats::record_metrics(state, ctx.api_key_id, 0, 0, &failed_delta);
    let (response_bytes, error_message) = if !ctx.last_error_body.is_empty() {
        if serde_json::from_str::<serde_json::Value>(&ctx.last_error_body).is_ok() {
            (Bytes::from(ctx.last_error_body.clone()), ctx.last_error_body.clone())
        } else {
            let json_err = serde_json::json!({
                "error": {
                    "message": ctx.last_error_body,
                    "type": "upstream_error",
                    "code": ctx.last_status.as_u16()
                }
            });
            (
                Bytes::from(serde_json::to_vec(&json_err).unwrap_or_default()),
                ctx.last_error_body.clone(),
            )
        }
    } else {
        let fallback_msg = "上游请求在所有尝试后仍失败".to_string();
        let json_err = serde_json::json!({
            "error": {
                "message": fallback_msg,
                "type": "failover_exhausted",
                "code": 502
            }
        });
        (
            Bytes::from(serde_json::to_vec(&json_err).unwrap_or_default()),
            fallback_msg,
        )
    };

    let mut req_log = ctx.log_base.clone();
    req_log.status = request_status::FAILED.to_string();
    req_log.target_channel = ctx.last_target_channel;
    req_log.target_model = ctx.last_target_model;
    req_log.response_url = ctx.last_response_url;
    req_log.response_headers = ctx.last_response_headers;
    req_log.response_body = ctx.last_error_body;
    req_log.wait_time = ctx.wait_time_ms;
    req_log.round = ctx.round;
    req_log.error = Some(error_message);
    req_log.rounds_detail = serde_json::to_string(&ctx.rounds).unwrap_or_default();
    state.in_flight.broadcast(RequestOverview::from(&req_log));
    state.in_flight.unregister(ctx.log_base.id as u64);

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = log::update_completed_log(&state_clone, &req_log).await {
            tracing::error!("写入已失败请求日志失败: {}", e);
        }
    });
    Response::builder()
        .status(
            if ctx.last_status.is_client_error() || ctx.last_status.is_server_error() {
                ctx.last_status
            } else {
                StatusCode::BAD_GATEWAY
            },
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(response_bytes))
        .unwrap_or_else(|e| {
            tracing::error!("构建错误响应失败: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Internal Server Error"))
                .unwrap_or_default()
        })
}

/// 单渠道发送（含 key 级轮换）：401/403/429 且存在未试 key 时，冷却当前 key
/// 并顺延未试 key 重试（不消耗轮次）；key 穷尽后返回最后一个响应。
/// 传输错误/超时以 String 记述（含 "timeout" 字样者映射为网关超时）。
struct SendPlan<'a> {
    state:                    &'a AppState,
    channel:                  &'a Channel,
    key_pool:                 &'a [String],
    endpoint:                 &'a str,
    body:                     Bytes,
    client_headers:           &'a HeaderMap,
    stream:                   bool,
    timeout_secs:             Option<u64>,
    first_event_timeout_secs: u64,
    cooldown_seconds:         i64,
}

async fn send_with_key_rotation(plan: SendPlan<'_>) -> Result<(reqwest::Response, usize), String> {
    let SendPlan {
        state,
        channel: ch,
        key_pool,
        endpoint: target_endpoint,
        body: body_bytes,
        client_headers,
        stream,
        timeout_secs,
        first_event_timeout_secs,
        cooldown_seconds,
    } = plan;
    let client = get_client();
    let mut key_idx = state.key_pool.pick(ch.id, key_pool.len());
    let mut tried_keys = std::collections::HashSet::new();
    loop {
        let req_builder = match build_upstream_request(
            client,
            ch,
            &key_pool[key_idx],
            target_endpoint,
            Some(body_bytes.clone()),
            timeout_secs,
            Some(client_headers),
        ) {
            Ok(rb) => rb,
            Err(e) => break Err(format!("构建渠道 {} 请求失败: {}", ch.name, e)),
        };
        let fut = req_builder.send();
        let res = if stream {
            match tokio::time::timeout(std::time::Duration::from_secs(first_event_timeout_secs), fut).await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err(format!(
                    "上游流式响应在 {}s 内未返回初始响应头",
                    first_event_timeout_secs
                )),
            }
        } else {
            fut.await.map_err(|e| e.to_string())
        };
        match res {
            // key 级可重试错误：冷却当前 key，换未试过的 key 重试（不消耗轮次）
            Ok(resp) if matches!(resp.status().as_u16(), 401 | 403 | 429) && key_pool.len() > 1 => {
                state
                    .key_pool
                    .report_failure(ch.id, key_idx, cooldown_seconds);
                tried_keys.insert(key_idx);
                match (0..key_pool.len()).find(|idx| !tried_keys.contains(idx)) {
                    Some(next) => {
                        key_idx = next;
                        continue;
                    }
                    None => break Ok((resp, key_idx)),
                }
            }
            other => break other.map(|resp| (resp, key_idx)),
        }
    }
}

pub async fn forward_relay(
    state: AppState,
    auth_ctx: Arc<AuthContext>,
    format: ApiFormat,
    path_model: Option<String>,
    uri: Uri,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Response {
    let (mut metadata, group_obj) = match validate_relay_request(&state, &auth_ctx, &body_bytes, path_model.as_deref())
    {
        Ok(v) => v,
        Err(err_resp) => return *err_resp,
    };
    // Gemini 客户端的流式标志在 URL path（:streamGenerateContent），请求体没有 stream 字段
    if format == ApiFormat::GeminiGenerate {
        metadata.stream = uri.path().contains(":streamGenerateContent");
    }

    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    let request_url = uri.path().to_string();
    let request_headers = serialize_headers(&headers);
    let request_body = body_str.clone();
    let started_at = chrono::Utc::now().timestamp_millis();
    let start_time = Instant::now();

    let request_id = match log::create_initial_log(
        &state,
        &metadata.model,
        &request_url,
        &request_headers,
        &request_body,
        started_at,
    )
    .await
    {
        Ok(id) => id as u64,
        Err(e) => {
            tracing::warn!("创建初始请求日志失败，继续使用临时 ID: {}", e);
            chrono::Utc::now()
                .timestamp_nanos_opt()
                .unwrap_or(started_at) as u64
        }
    };
    let cancel_rx = state.in_flight.register(request_id);
    // 客户端侧日志基座：失败路径记录原始请求；成功路径按轮次克隆并覆盖为上游侧信息
    let log_base = RequestLog {
        id: request_id as i64,
        model: metadata.model.clone(),
        request_url: request_url.clone(),
        request_headers: request_headers.clone(),
        request_body: request_body.clone(),
        created_at: started_at,
        ..Default::default()
    };
    let started_at_rfc3339 = millis_to_rfc3339(started_at);
    state.in_flight.broadcast(RequestOverview {
        id: request_id,
        status: request_status::RUNNING.to_string(),
        started_at: started_at_rfc3339.clone(),
        model: metadata.model.clone(),
        round: 1,
        ..Default::default()
    });

    let max_attempts = group_obj.relay_config.max_attempts.max(1);

    let mut round = 0;
    let mut last_target_channel = String::new();
    let mut last_target_model = String::new();
    let mut last_response_url = String::new();
    let mut last_response_headers = String::new();
    let mut last_status = StatusCode::BAD_GATEWAY;
    let mut last_error_body = String::new();
    let mut rounds: Vec<crate::model::RoundLog> = Vec::new();
    let mut attempted_items = HashSet::new();

    while round < max_attempts {
        if *cancel_rx.borrow() {
            last_error_body = "请求已被中断".to_string();
            last_status = StatusCode::from_u16(499).unwrap_or(StatusCode::BAD_REQUEST);
            break;
        }
        round += 1;
        let round_start_time = Instant::now();

        let current_group = match group::get_group_by_name(&state, &metadata.model) {
            Ok(g) => g,
            Err(_) => break,
        };

        let item = match state
            .route_manager
            .pick_candidate_item(&current_group, &attempted_items)
        {
            Some(it) => it,
            None => {
                break;
            }
        };
        attempted_items.insert(item.id);

        let channel_model = match state.channel_models.get(&item.channel_model_id) {
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
            Ok(_) | Err(_) => {
                state
                    .route_manager
                    .report_failure(current_group.id, item.id, 60);
                continue;
            }
        };

        let (target_endpoint, final_body) =
            match prepare_upstream_payload(format, metadata.stream, &body_bytes, &ch, &channel_model) {
                Ok(pair) => pair,
                Err(msg) => {
                    // 转换结果未通过目标形状校验：代理侧明确的 400，不透传上游
                    let resp = error(StatusCode::BAD_REQUEST, &msg);
                    log::update_completed_log(
                        &state,
                        &RequestLog {
                            id: request_id as i64,
                            model: metadata.model.clone(),
                            status: request_status::FAILED.to_string(),
                            error: Some(msg),
                            created_at: started_at,
                            ..log_base.clone()
                        },
                    )
                    .await
                    .unwrap_or(());
                    state.in_flight.unregister(request_id);
                    return resp;
                }
            };
        let target_req_url = format!("{}{}", normalize_base_url(&ch.base_url), target_endpoint);
        let key_pool = ch.key_pool();
        let channel_req_headers = super::upstream::build_upstream_headers(&ch, key_pool[0].as_str(), Some(&headers))
            .map(|h| serialize_headers(&h))
            .unwrap_or_else(|_| request_headers.clone());
        let rewritten_body_str = String::from_utf8_lossy(&final_body).to_string();
        let round_log_base = RequestLog {
            round,
            target_channel: ch.name.clone(),
            target_model: channel_model.name.clone(),
            request_url: target_req_url.clone(),
            request_headers: channel_req_headers.clone(),
            request_body: rewritten_body_str.clone(),
            response_url: target_req_url.clone(),
            ..log_base.clone()
        };

        state.in_flight.broadcast(RequestOverview {
            id: request_id,
            status: request_status::RUNNING.to_string(),
            started_at: started_at_rfc3339.clone(),
            duration: start_time.elapsed().as_millis() as i64,
            model: metadata.model.clone(),
            round,
            target_channel: ch.name.clone(),
            target_model: channel_model.name.clone(),
            sending: true,
            rounds: rounds.clone(),
            ..Default::default()
        });

        let timeout_secs = if metadata.stream {
            None
        } else {
            Some(
                current_group
                    .relay_config
                    .non_stream_timeout_seconds
                    .max(10) as u64,
            )
        };

        let send_res = send_with_key_rotation(SendPlan {
            state: &state,
            channel: &ch,
            key_pool: &key_pool,
            endpoint: &target_endpoint,
            body: Bytes::from(final_body),
            client_headers: &headers,
            stream: metadata.stream,
            timeout_secs,
            first_event_timeout_secs: current_group
                .relay_config
                .stream_first_event_timeout_seconds
                .max(5) as u64,
            cooldown_seconds: current_group.relay_config.cooldown_seconds,
        })
        .await;

        match send_res {
            Ok((upstream_resp, key_idx)) => {
                let status = upstream_resp.status();
                if status.is_success() {
                    state
                        .route_manager
                        .report_success(current_group.id, item.id);
                    state.key_pool.report_success(ch.id, key_idx);
                    let price_opt = model::get_model_price(&state, &channel_model.name);

                    let ctx = SuccessContext {
                        log_base: round_log_base,
                        format,
                        channel: &ch,
                        channel_model: &channel_model,
                        api_key_id: auth_ctx.api_key.id,
                        body_str: &body_str,
                        round_start_time,
                        start_time,
                        price_opt,
                        rounds,
                    };
                    if metadata.stream {
                        return handle_successful_stream(&state, upstream_resp, ctx);
                    }
                    return handle_successful_json(&state, upstream_resp, ctx).await;
                } else {
                    let resp_headers = serialize_headers(upstream_resp.headers());
                    let err_body =
                        truncate_utf8(&upstream_resp.text().await.unwrap_or_default(), RESPONSE_CAPTURE_LIMIT)
                            .to_string();
                    let round_duration_ms = round_start_time.elapsed().as_millis() as i64;
                    last_target_channel = ch.name.clone();
                    last_target_model = channel_model.name.clone();
                    last_response_url = target_req_url.clone();
                    last_response_headers = resp_headers.clone();
                    last_status = status;
                    last_error_body = err_body.clone();

                    let is_client = is_client_error(status);
                    record_round_failure(
                        &state,
                        current_group.id,
                        item.id,
                        current_group.relay_config.cooldown_seconds,
                        request_id,
                        &mut rounds,
                        RoundFailure {
                            round,
                            model: &metadata.model,
                            started_at: &started_at_rfc3339,
                            channel_name: &ch.name,
                            model_name: &channel_model.name,
                            target_req_url: &target_req_url,
                            channel_req_headers: &channel_req_headers,
                            rewritten_body_str: &rewritten_body_str,
                            response_headers: &resp_headers,
                            status: status.as_u16(),
                            duration_ms: round_duration_ms,
                            err_body: &err_body,
                            should_cooldown: !is_client,
                        },
                    );

                    if is_client {
                        break;
                    }
                }
            }
            Err(err_msg) => {
                let round_duration_ms = round_start_time.elapsed().as_millis() as i64;
                last_target_channel = ch.name.clone();
                last_target_model = channel_model.name.clone();
                last_response_url = target_req_url.clone();
                last_response_headers = String::new();
                last_status = if err_msg.contains("timeout") {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                last_error_body = err_msg.clone();

                record_round_failure(
                    &state,
                    current_group.id,
                    item.id,
                    current_group.relay_config.cooldown_seconds,
                    request_id,
                    &mut rounds,
                    RoundFailure {
                        round,
                        model: &metadata.model,
                        started_at: &started_at_rfc3339,
                        channel_name: &ch.name,
                        model_name: &channel_model.name,
                        target_req_url: &target_req_url,
                        channel_req_headers: &channel_req_headers,
                        rewritten_body_str: &rewritten_body_str,
                        response_headers: "",
                        status: 502,
                        duration_ms: round_duration_ms,
                        err_body: &err_msg,
                        should_cooldown: true,
                    },
                );
            }
        }

        if round < max_attempts && current_group.relay_config.retry_interval_seconds > 0 {
            let mut rx = cancel_rx.clone();
            tokio::select! {
                _ = rx.changed() => {
                    if *rx.borrow() {
                        last_error_body = "请求已被中断".to_string();
                        last_status = StatusCode::from_u16(499).unwrap_or(StatusCode::BAD_REQUEST);
                        break;
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(
                    current_group.relay_config.retry_interval_seconds as u64,
                )) => {}
            }
        }
    }

    build_failed_relay_response(
        &state,
        FailedRelayContext {
            log_base,
            last_target_channel,
            last_target_model,
            last_response_url,
            last_response_headers,
            last_status,
            last_error_body,
            rounds,
            wait_time_ms: start_time.elapsed().as_millis() as i64,
            round,
            api_key_id: auth_ctx.api_key.id,
        },
    )
}

#[cfg(test)]
mod validate_tests {
    use serde_json::json;

    use super::validate_chat_tool_pairing;

    #[test]
    fn accepts_paired_tool_round_trip() {
        let b = json!({ "messages": [
            { "role": "user", "content": "q" },
            { "role": "assistant", "tool_calls": [
                { "id": "c1", "type": "function", "function": { "name": "f", "arguments": "{}" } }
            ] },
            { "role": "tool", "tool_call_id": "c1", "content": "r" }
        ] });
        assert!(validate_chat_tool_pairing(&b).is_ok());
    }

    #[test]
    fn rejects_tool_message_without_call() {
        let b = json!({ "messages": [
            { "role": "user", "content": "q" },
            { "role": "tool", "tool_call_id": "c9", "content": "r" }
        ] });
        assert!(validate_chat_tool_pairing(&b).is_err());
    }

    #[test]
    fn rejects_missing_tool_result() {
        let b = json!({ "messages": [
            { "role": "assistant", "tool_calls": [
                { "id": "c1", "type": "function", "function": { "name": "f", "arguments": "{}" } }
            ] },
            { "role": "user", "content": "next" }
        ] });
        assert!(validate_chat_tool_pairing(&b).is_err());
    }
}
