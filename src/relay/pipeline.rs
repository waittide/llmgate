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
        error::build_client_error,
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

    // 跨格式：原始格式 → IR → 目标格式。解析/序列化失败显式返回 400，
    // 禁止把错误格式的载荷静默转发给异构上游（会被记为渠道故障并触发冷却）。
    let source = serde_json::from_slice::<serde_json::Value>(&rewritten_body)
        .map_err(|e| format!("跨格式转换失败：请求体不是合法 JSON: {e}"))?;
    let parsed =
        ir::parse_request(format, &source).ok_or_else(|| "跨格式转换失败：请求体无法按源格式解析".to_string())?;
    let emitted = ir::emit_request(&parsed, target_format);
    if target_format == ApiFormat::OpenaiCompletion
        && let Err(msg) = validate_chat_tool_pairing(&emitted)
    {
        return Err(msg);
    }
    let body = serde_json::to_vec(&emitted).map_err(|e| format!("跨格式转换序列化失败: {e}"))?;
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
    key_index:           usize,
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
        key_index:        Some(fail.key_index),
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
    key_index:        usize,
}

async fn handle_successful_stream(
    state: &AppState,
    upstream_resp: reqwest::Response,
    ctx: SuccessContext<'_>,
) -> Response {
    // 上游声明成功却返回非 SSE 体（200 + JSON/HTML 错误页、忽略 stream 的完整应答）：
    // 按流处理只会给客户端一个“正常但为空”的流，上游错误被静默吞掉。
    if is_non_sse_content_type(upstream_resp.headers()) {
        return handle_non_sse_body(state, upstream_resp, ctx).await;
    }
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
        key_index: Some(ctx.key_index),
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

/// 上游是否明确声明了非 SSE 的 content-type。
///
/// content-type 缺失时按 SSE 处理：宽松的上游（尤其自建反代）常不带该头，
/// 只有明确声明了其他类型才判定“这不是流”。
fn is_non_sse_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| !content_type.to_ascii_lowercase().contains("event-stream"))
}

/// 流式请求收到非 SSE 成功响应时的处理。
///
/// - 响应体是错误信封或不是 JSON：上游把错误塞进了 200，按网关失败返回（502 + 客户端协议错误体）
/// - 响应体是合法 JSON 且不带 error：上游忽略了 stream 参数，按完整应答转换后返回
async fn handle_non_sse_body(state: &AppState, upstream_resp: reqwest::Response, ctx: SuccessContext<'_>) -> Response {
    let response_headers = serialize_headers(upstream_resp.headers());
    let raw_bytes = match upstream_resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            return upstream_body_failure(state, ctx, response_headers, format!("读取上游响应体失败: {e}"));
        }
    };
    let body = String::from_utf8_lossy(&raw_bytes);
    // 四种协议的错误体都包在顶层 error 里；非 JSON（HTML 错误页、纯文本）同样不是可用应答
    let is_error = serde_json::from_str::<Value>(&body)
        .ok()
        .is_none_or(|value| value.get("error").is_some());
    if is_error {
        return upstream_body_failure(state, ctx, response_headers, body.to_string());
    }
    complete_json_response(state, raw_bytes, response_headers, ctx)
}

/// 读取 `reqwest::Response` 中的完整应答体；读取失败按网关失败收尾。
async fn handle_successful_json(
    state: &AppState,
    upstream_resp: reqwest::Response,
    ctx: SuccessContext<'_>,
) -> Response {
    let response_headers = serialize_headers(upstream_resp.headers());
    let raw_bytes = match upstream_resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            return upstream_body_failure(state, ctx, response_headers, format!("读取上游响应体失败: {e}"));
        }
    };
    complete_json_response(state, raw_bytes, response_headers, ctx)
}

/// 上游 HTTP 200 但响应体不可用（读取失败 / 错误信封 / 非法内容）时的收尾。
///
/// 统一走失败路径：状态码 502、错误信封按客户端协议构造，并落盘失败日志，
/// 避免把上游塞进 200 的错误当作正常应答返回。
fn upstream_body_failure(
    state: &AppState,
    ctx: SuccessContext<'_>,
    response_headers: String,
    message: String,
) -> Response {
    let log_base = ctx.log_base.clone();
    build_failed_relay_response(
        state,
        FailedRelayContext {
            log_base,
            format: ctx.format,
            last_target_channel: ctx.channel.name.clone(),
            last_target_model: ctx.channel_model.name.clone(),
            last_response_url: ctx.log_base.response_url.clone(),
            last_response_headers: response_headers,
            last_status: StatusCode::BAD_GATEWAY,
            last_error_body: message,
            rounds: ctx.rounds,
            wait_time_ms: ctx.start_time.elapsed().as_millis() as i64,
            round: ctx.log_base.round,
            api_key_id: ctx.api_key_id,
        },
    )
}

/// 转换并按目标格式返回完整（非流式）应答体。
fn complete_json_response(
    state: &AppState,
    raw_bytes: Bytes,
    response_headers: String,
    mut ctx: SuccessContext<'_>,
) -> Response {
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
    let resp_bytes = if ctx.format == upstream_format {
        converted
            .as_ref()
            .and_then(|val| serde_json::to_vec(val).ok())
            .map(Bytes::from)
            .unwrap_or(raw_bytes)
    } else {
        match converted
            .as_ref()
            .and_then(|val| serde_json::to_vec(val).ok())
        {
            Some(bytes) => Bytes::from(bytes),
            None => {
                // 跨格式转换失败：透传会给客户端错误格式，按网关失败处理
                return upstream_body_failure(
                    state,
                    ctx,
                    response_headers,
                    "跨格式转换失败：上游响应无法解析为目标格式".to_string(),
                );
            }
        }
    };
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
        key_index:        Some(ctx.key_index),
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
    req_log.rounds_detail = log::serialize_rounds_detail(state, &ctx.rounds);

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
    /// 客户端请求协议；错误信封按它构造，避免把上游协议的错误体透传出去
    format:                ApiFormat,
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
    let built = build_client_error(ctx.format, ctx.last_status, &ctx.last_error_body);
    let response_bytes = built.body;
    let error_message = built.message;

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
    req_log.rounds_detail = log::serialize_rounds_detail(state, &ctx.rounds);
    state.in_flight.broadcast(RequestOverview::from(&req_log));
    state.in_flight.unregister(ctx.log_base.id as u64);

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = log::update_completed_log(&state_clone, &req_log).await {
            tracing::error!("写入已完成请求日志失败: {}", e);
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

/// 转发期兜底注销：`forward_relay` 的 future 在 await 点被 drop（客户端断连、
/// 运行时关闭）时，in_flight 注册表条目会永久泄漏、初始日志行停留在 RUNNING。
/// 本 guard 的 Drop 统一收尾：注销注册、广播并落盘一条 INTERRUPTED 日志。
/// 正常完成路径必须先调用 `defuse()`，把收尾交给各自的完成逻辑。
struct RelayGuard {
    state:      AppState,
    req_id:     u64,
    log_base:   RequestLog,
    started_at: i64,
    defused:    bool,
}

impl RelayGuard {
    fn defuse(mut self) {
        self.defused = true;
    }
}

impl Drop for RelayGuard {
    fn drop(&mut self) {
        if self.defused {
            return;
        }
        self.state.in_flight.unregister(self.req_id);
        let mut req_log = self.log_base.clone();
        req_log.status = request_status::INTERRUPTED.to_string();
        req_log.wait_time = chrono::Utc::now().timestamp_millis() - self.started_at;
        req_log.error = Some("客户端连接中断，转发提前终止".to_string());
        self.state
            .in_flight
            .broadcast(RequestOverview::from(&req_log));
        let state = self.state.clone();
        tokio::spawn(async move {
            if let Err(e) = log::update_completed_log(&state, &req_log).await {
                tracing::error!("写入中断请求日志失败: {}", e);
            }
        });
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
    let relay_guard = RelayGuard {
        state: state.clone(),
        req_id: request_id,
        log_base: log_base.clone(),
        started_at,
        defused: false,
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
    let mut items_tried = 0;
    let mut keys_tried: HashSet<usize> = HashSet::new();
    let mut item: Option<crate::model::GroupItem> = None;
    let mut last_target_channel = String::new();
    let mut last_target_model = String::new();
    let mut last_response_url = String::new();
    let mut last_response_headers = String::new();
    let mut last_status = StatusCode::BAD_GATEWAY;
    let mut last_error_body = String::new();
    let mut rounds: Vec<crate::model::RoundLog> = Vec::new();
    let mut attempted_items = HashSet::new();

    // 渠道级上下文: 同渠道轮换 key 时复用, 换渠道时重建
    let mut current_group = match group::get_group_by_name(&state, &metadata.model) {
        Ok(g) => g,
        Err(_) => {
            return build_failed_relay_response(
                &state,
                FailedRelayContext {
                    log_base: log_base.clone(),
                    format,
                    last_target_channel: String::new(),
                    last_target_model: String::new(),
                    last_response_url: String::new(),
                    last_response_headers: String::new(),
                    last_status: StatusCode::NOT_FOUND,
                    last_error_body: "分组不存在".to_string(),
                    rounds: Vec::new(),
                    wait_time_ms: 0,
                    round: 0,
                    api_key_id: auth_ctx.api_key.id,
                },
            );
        }
    };
    let mut channel_model: Option<crate::model::ChannelModel> = None;
    let mut ch: Option<crate::model::Channel> = None;
    let mut target_endpoint = String::new();
    let mut final_body = Vec::new();
    let mut key_pool: Vec<String> = Vec::new();

    while items_tried < max_attempts {
        if *cancel_rx.borrow() {
            last_error_body = "请求已被中断".to_string();
            last_status = StatusCode::from_u16(499).unwrap_or(StatusCode::BAD_REQUEST);
            break;
        }

        // 需要新渠道时(item 为空): 选择下一个可用渠道项(计一次渠道尝试)
        if item.is_none() {
            current_group = match group::get_group_by_name(&state, &metadata.model) {
                Ok(g) => g,
                Err(_) => break,
            };
            let picked =
                match state
                    .route_manager
                    .pick_candidate_item(&current_group, &attempted_items, |channel_model_id| {
                        state
                            .channel_models
                            .get(&channel_model_id)
                            .map(|cm| {
                                state
                                    .channels
                                    .get(&cm.channel_id)
                                    .is_some_and(|c| c.enabled)
                            })
                            .unwrap_or(false)
                    }) {
                    Some(it) => it,
                    None => break,
                };
            attempted_items.insert(picked.id);
            items_tried += 1;
            keys_tried.clear();
            item = Some(picked);

            let cm = match state
                .channel_models
                .get(&item.as_ref().unwrap().channel_model_id)
            {
                Some(cm) => cm.clone(),
                None => {
                    // 渠道模型缺失: 记一次渠道尝试并换下一个
                    continue;
                }
            };
            let channel = match channel::get_channel(&state, cm.channel_id) {
                Ok(c) if c.enabled => c,
                Ok(_) | Err(_) => {
                    // 渠道已被禁用/移除: 竞态兜底, 换下一个
                    continue;
                }
            };

            let (endpoint, body) = match prepare_upstream_payload(format, metadata.stream, &body_bytes, &channel, &cm) {
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
                    relay_guard.defuse();
                    return resp;
                }
            };

            channel_model = Some(cm);
            key_pool = channel.key_pool();
            ch = Some(channel);
            target_endpoint = endpoint;
            final_body = body;
        }

        let ch_ref = ch.as_ref().unwrap();
        let cm_ref = channel_model.as_ref().unwrap();
        round += 1;
        let round_start_time = Instant::now();

        // key 选择: 冷却机制自动轮换(失败的 key 已冷却); 全部试过则换渠道
        let key_idx = state.key_pool.pick(ch_ref.id, key_pool.len());
        if keys_tried.contains(&key_idx) {
            // 该渠道所有 key 均已尝试: 弃用此渠道, 换下一个
            item = None;
            continue;
        }
        keys_tried.insert(key_idx);

        // 本轮请求头使用本轮实际选中的 key
        let target_req_url = format!("{}{}", normalize_base_url(&ch_ref.base_url), target_endpoint);
        let channel_req_headers =
            super::upstream::build_upstream_headers(ch_ref, key_pool[key_idx].as_str(), Some(&headers))
                .map(|h| serialize_headers(&h))
                .unwrap_or_else(|_| request_headers.clone());
        let rewritten_body_str = String::from_utf8_lossy(&final_body).to_string();
        state.in_flight.broadcast(RequestOverview {
            id: request_id,
            status: request_status::RUNNING.to_string(),
            started_at: started_at_rfc3339.clone(),
            duration: start_time.elapsed().as_millis() as i64,
            model: metadata.model.clone(),
            round,
            target_channel: ch_ref.name.clone(),
            target_model: cm_ref.name.clone(),
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

        let request_builder = match build_upstream_request(
            get_client(),
            ch_ref,
            &key_pool[key_idx],
            &target_endpoint,
            Some(Bytes::from(final_body.clone())),
            timeout_secs,
            Some(&headers),
        ) {
            Ok(rb) => rb,
            Err(e) => {
                last_target_channel = ch_ref.name.clone();
                last_target_model = cm_ref.name.clone();
                last_error_body = format!("构建渠道 {} 请求失败: {}", ch_ref.name, e);
                last_status = StatusCode::BAD_GATEWAY;
                record_round_failure(
                    &state,
                    current_group.id,
                    item.as_ref().map(|i| i.id).unwrap_or(0),
                    current_group.relay_config.cooldown_seconds,
                    request_id,
                    &mut rounds,
                    RoundFailure {
                        round,
                        model: &metadata.model,
                        started_at: &started_at_rfc3339,
                        channel_name: &ch_ref.name,
                        model_name: &cm_ref.name,
                        target_req_url: &target_req_url,
                        channel_req_headers: &channel_req_headers,
                        rewritten_body_str: &rewritten_body_str,
                        response_headers: "",
                        status: 502,
                        duration_ms: round_start_time.elapsed().as_millis() as i64,
                        err_body: &last_error_body,
                        should_cooldown: true,
                        key_index: key_idx,
                    },
                );
                item = None;
                continue;
            }
        };

        let fut = request_builder.send();

        // 流式: 首事件超时说明请求已被上游受理并可能开始生成, 不轮换 key(避免重复生成)
        let send_result = if metadata.stream {
            match tokio::time::timeout(
                std::time::Duration::from_secs(
                    current_group
                        .relay_config
                        .stream_first_event_timeout_seconds
                        .max(5) as u64,
                ),
                fut,
            )
            .await
            {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err(format!(
                    "上游流式响应在 {}s 内未返回初始响应头",
                    current_group
                        .relay_config
                        .stream_first_event_timeout_seconds
                        .max(5)
                )),
            }
        } else {
            fut.await.map_err(|e| e.to_string())
        };

        match send_result {
            Ok(upstream_resp) => {
                let status = upstream_resp.status();
                if status.is_success() {
                    state
                        .route_manager
                        .report_success(current_group.id, item.as_ref().map(|i| i.id).unwrap_or(0));
                    state.key_pool.report_success(ch_ref.id, key_idx);
                    let price_opt = model::get_model_price(&state, &cm_ref.name);

                    let ctx = SuccessContext {
                        log_base: RequestLog {
                            round,
                            target_channel: ch_ref.name.clone(),
                            target_model: cm_ref.name.clone(),
                            request_url: target_req_url.clone(),
                            request_headers: channel_req_headers.clone(),
                            request_body: rewritten_body_str.clone(),
                            response_url: target_req_url.clone(),
                            ..log_base.clone()
                        },
                        format,
                        channel: ch_ref,
                        channel_model: cm_ref,
                        api_key_id: auth_ctx.api_key.id,
                        body_str: &body_str,
                        round_start_time,
                        start_time,
                        price_opt,
                        rounds,
                        key_index: key_idx,
                    };
                    if metadata.stream {
                        relay_guard.defuse();
                        return handle_successful_stream(&state, upstream_resp, ctx).await;
                    }
                    relay_guard.defuse();
                    return handle_successful_json(&state, upstream_resp, ctx).await;
                }

                let resp_headers = serialize_headers(upstream_resp.headers());
                let err_body =
                    truncate_utf8(&upstream_resp.text().await.unwrap_or_default(), RESPONSE_CAPTURE_LIMIT).to_string();
                let round_duration_ms = round_start_time.elapsed().as_millis() as i64;
                last_target_channel = ch_ref.name.clone();
                last_target_model = cm_ref.name.clone();
                last_response_url = target_req_url.clone();
                last_response_headers = resp_headers.clone();
                last_status = status;
                last_error_body = err_body.clone();

                // key 级失败(401/403/429): 冷却该 key, 同渠道换下一个
                state
                    .key_pool
                    .report_failure(ch_ref.id, key_idx, current_group.relay_config.cooldown_seconds);
                keys_tried.insert(key_idx);

                let is_client = is_client_error(status);
                record_round_failure(
                    &state,
                    current_group.id,
                    item.as_ref().map(|i| i.id).unwrap_or(0),
                    current_group.relay_config.cooldown_seconds,
                    request_id,
                    &mut rounds,
                    RoundFailure {
                        round,
                        model: &metadata.model,
                        started_at: &started_at_rfc3339,
                        channel_name: &ch_ref.name,
                        model_name: &cm_ref.name,
                        target_req_url: &target_req_url,
                        channel_req_headers: &channel_req_headers,
                        rewritten_body_str: &rewritten_body_str,
                        response_headers: &resp_headers,
                        status: status.as_u16(),
                        duration_ms: round_duration_ms,
                        err_body: &err_body,
                        should_cooldown: !is_client,
                        key_index: key_idx,
                    },
                );

                if is_client {
                    break;
                }
                // 该渠道 key 已穷尽或服务端失败: 换下一个渠道
                if !is_client && keys_tried.len() >= key_pool.len() {
                    item = None;
                }
            }
            Err(err_msg) => {
                let round_duration_ms = round_start_time.elapsed().as_millis() as i64;
                last_target_channel = ch_ref.name.clone();
                last_target_model = cm_ref.name.clone();
                last_response_url = target_req_url.clone();
                last_response_headers = String::new();
                last_status = if err_msg.contains("timeout") {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                last_error_body = err_msg.clone();

                // 连接层失败: 冷却该 key; 该渠道 key 全部试过后换下一个渠道
                state
                    .key_pool
                    .report_failure(ch_ref.id, key_idx, current_group.relay_config.cooldown_seconds);
                keys_tried.insert(key_idx);
                if keys_tried.len() >= key_pool.len() {
                    item = None;
                }

                record_round_failure(
                    &state,
                    current_group.id,
                    item.as_ref().map(|i| i.id).unwrap_or(0),
                    current_group.relay_config.cooldown_seconds,
                    request_id,
                    &mut rounds,
                    RoundFailure {
                        round,
                        model: &metadata.model,
                        started_at: &started_at_rfc3339,
                        channel_name: &ch_ref.name,
                        model_name: &cm_ref.name,
                        target_req_url: &target_req_url,
                        channel_req_headers: &channel_req_headers,
                        rewritten_body_str: &rewritten_body_str,
                        response_headers: "",
                        status: 502,
                        duration_ms: round_duration_ms,
                        err_body: &err_msg,
                        should_cooldown: true,
                        key_index: key_idx,
                    },
                );
            }
        }

        if items_tried < max_attempts && current_group.relay_config.retry_interval_seconds > 0 {
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
            format,
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

#[cfg(test)]
mod relay_guard_tests {
    use super::RelayGuard;
    use crate::test_support::setup_test_state;

    /// 客户端断连等场景下 forward_relay future 被 drop：guard 的 Drop 必须
    /// 注销 in_flight 条目（H2 修复），defuse 后则不得重复注销。
    #[tokio::test]
    async fn guard_drop_unregisters_but_defuse_preserves() {
        let (state, db_path) = setup_test_state("relay_guard").await;

        // 1. 未 defuse 的 guard：Drop 兜底注销
        let id = 4242u64;
        let rx = state.in_flight.register(id);
        {
            let guard = RelayGuard {
                state:      state.clone(),
                req_id:     id,
                log_base:   crate::model::RequestLog {
                    id: id as i64,
                    ..Default::default()
                },
                started_at: 0,
                defused:    false,
            };
            assert!(state.in_flight.interrupt(id)); // 条目存在且可中断
            drop(guard);
        }
        assert!(!state.in_flight.interrupt(id)); // 已被 Drop 注销
        assert!(*rx.borrow()); // 取消标志仍可由 watch 读到

        // 2. defuse 的 guard：条目保留，交给正常完成路径处理
        let id2 = 777u64;
        state.in_flight.register(id2);
        RelayGuard {
            state:      state.clone(),
            req_id:     id2,
            log_base:   crate::model::RequestLog {
                id: id2 as i64,
                ..Default::default()
            },
            started_at: 0,
            defused:    false,
        }
        .defuse();
        assert!(state.in_flight.interrupt(id2)); // 未被注销

        let _ = std::fs::remove_file(db_path);
    }
}
