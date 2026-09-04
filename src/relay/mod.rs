pub mod anthropic_stream;
pub mod protocol;
pub mod route;
pub mod state;
pub mod stream;
pub mod upstream;

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode, Uri, header},
    response::Response,
};
use bytes::Bytes;
use futures::{Stream, StreamExt};

use crate::{
    model::{Channel, ChannelModel, ChannelProvider, Group, LLMPrice, RequestLog, StatsMetrics},
    op::{AppState, channel, group, llm, log, stats},
    relay::{
        anthropic_stream::{OpenaiStreamFormat, OpenaiToAnthropicStream},
        protocol::{
            ApiFormat, calculate_cost, extract_usage_from_json, parse_request_metadata, rewrite_request_body_in_json,
        },
        state::{RequestOverview, Usage},
        stream::{ChatCompletionsToResponsesStream, ProviderToOpenaiStream},
        upstream::{build_upstream_request, get_client, normalize_base_url},
    },
    server::{middleware::AuthContext, resp::error},
    utils::estimate_tokens,
};
/// Cap for captured streaming response bodies to bound log storage.
const RESPONSE_CAPTURE_LIMIT: usize = 64 * 1024;

/// Serializes an HTTP header map as a JSON object: name -> comma-joined values.
fn serialize_headers(headers: &HeaderMap) -> String {
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

/// Streams the upstream body while accumulating real usage, then records metrics
/// exactly once when the stream ends (or is dropped, e.g. client disconnect).
struct StreamFinalizer<S> {
    inner:            S,
    state:            AppState,
    req_id:           u64,
    api_key_id:       i64,
    channel_id:       i64,
    channel_model_id: i64,
    price:            Option<LLMPrice>,
    prompt_tokens:    i64,
    buffer:           Vec<u8>,
    output_bytes:     usize,
    usage:            Option<Usage>,
    start_time:       Instant,
    done:             bool,
    log_template:     RequestLog,
    previous_rounds:  Vec<crate::model::RoundLog>,
    response_capture: Vec<u8>,
    cancel_rx:        Option<tokio::sync::watch::Receiver<bool>>,
    interrupted:      bool,
}

/// Drains complete SSE events from `buffer`, accumulating any `usage` found.
const SSE_BUFFER_LIMIT: usize = 64 * 1024; // 64KB max temporary SSE accumulation buffer

/// Handles events split across arbitrary stream chunk boundaries.
pub(crate) fn sse_event_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 2);
        }
        if i + 3 < buf.len() && buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i + 4);
        }
    }
    None
}

fn extract_usage_from_sse_buffer(buffer: &mut Vec<u8>, usage: &mut Option<Usage>) {
    while let Some(end) = sse_event_end(buffer) {
        let event_bytes = &buffer[..end];
        // Fast path: skip string allocation and line parsing if the event doesn't contain "usage"
        if event_bytes.windows(7).any(|w| w == b"\"usage\"") {
            let text = String::from_utf8_lossy(event_bytes);
            for line in text.lines() {
                if let Some(data) = line.trim_start().strip_prefix("data:") {
                    let trimmed = data.trim();
                    if !trimmed.is_empty()
                        && trimmed != "[DONE]"
                        && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
                        && let Some(u) = extract_usage_from_json(&val)
                    {
                        // Merge across events (e.g. Anthropic message_start carries
                        // input_tokens while message_delta carries output_tokens).
                        // Keep any already-recorded non-zero counts.
                        let merged = match usage {
                            Some(existing) => Usage {
                                prompt_tokens:     if existing.prompt_tokens > 0 {
                                    existing.prompt_tokens
                                } else {
                                    u.prompt_tokens
                                },
                                completion_tokens: if existing.completion_tokens > 0 {
                                    existing.completion_tokens
                                } else {
                                    u.completion_tokens
                                },
                                total_tokens:      if existing.total_tokens > 0 {
                                    existing.total_tokens
                                } else {
                                    u.total_tokens
                                },
                            },
                            None => u,
                        };
                        *usage = Some(merged);
                    }
                }
            }
        }
        buffer.drain(..end);
    }
    // Guard against malformed streams without event boundaries
    if buffer.len() > SSE_BUFFER_LIMIT {
        buffer.drain(..buffer.len() - (SSE_BUFFER_LIMIT / 2));
    }
}

/// Determine if an upstream error indicates a client-side syntax/format issue.
///
/// Authentication/authorization (401/403), missing model/endpoint on upstream (404),
/// rate-limits/quota exhaustion (429), and server errors (5xx) are upstream channel issues.
/// They must trigger channel cooldown and failover to other healthy channels.
/// Only 400 Bad Request and 422 Unprocessable Entity are considered non-retryable client errors.
fn is_client_error(status: StatusCode) -> bool {
    matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY)
}

impl<S> StreamFinalizer<S> {
    fn extract_usage_from_sse(&mut self) {
        extract_usage_from_sse_buffer(&mut self.buffer, &mut self.usage);
    }

    fn finalize(&mut self, completed: bool) {
        if self.done {
            return;
        }
        self.done = true;

        let est_completion = (self.output_bytes as f64 / 3.0).ceil() as i64;
        let (prompt_tokens, completion_tokens) = match &self.usage {
            Some(u) => (
                if u.prompt_tokens > 0 {
                    u.prompt_tokens
                } else {
                    self.prompt_tokens
                },
                if u.completion_tokens > 0 {
                    u.completion_tokens
                } else {
                    est_completion
                },
            ),
            None => (self.prompt_tokens, est_completion),
        };

        let cost = calculate_cost(prompt_tokens, completion_tokens, self.price.as_ref());
        let elapsed_ms = self.start_time.elapsed().as_millis() as i64;
        let delta = StatsMetrics {
            input_token:     prompt_tokens,
            output_token:    completion_tokens,
            input_cost:      (prompt_tokens as f64 / 1_000_000.0) * self.price.as_ref().map(|p| p.input).unwrap_or(0.0),
            output_cost:     (completion_tokens as f64 / 1_000_000.0)
                * self.price.as_ref().map(|p| p.output).unwrap_or(0.0),
            wait_time:       elapsed_ms,
            request_success: if completed { 1 } else { 0 },
            request_failed:  if completed { 0 } else { 1 },
        };
        stats::record_metrics(
            &self.state,
            self.api_key_id,
            self.channel_id,
            self.channel_model_id,
            &delta,
        );
        let status_str = if self.interrupted {
            "interrupted"
        } else if completed {
            "success"
        } else {
            "stream_error"
        };

        let response_body_str = String::from_utf8_lossy(&self.response_capture).to_string();
        let error_opt = if completed {
            None
        } else if self.interrupted {
            Some("stream interrupted by user".to_string())
        } else if !response_body_str.is_empty() {
            Some(response_body_str.clone())
        } else {
            Some("stream interrupted".to_string())
        };

        let mut all_rounds = self.previous_rounds.clone();
        all_rounds.push(crate::model::RoundLog {
            round:            self.log_template.round,
            channel_name:     self.log_template.target_channel.clone(),
            model_name:       self.log_template.target_model.clone(),
            request_url:      self.log_template.request_url.clone(),
            request_headers:  self.log_template.request_headers.clone(),
            request_body:     self.log_template.request_body.clone(),
            response_url:     self.log_template.response_url.clone(),
            response_headers: self.log_template.response_headers.clone(),
            response_body:    response_body_str.clone(),
            status:           if completed { 200 } else { 500 },
            duration_ms:      elapsed_ms,
            error:            error_opt.clone(),
        });

        let mut req_log = self.log_template.clone();
        req_log.status = status_str.to_string();
        req_log.response_body = response_body_str;
        req_log.prompt_tokens = prompt_tokens;
        req_log.completion_tokens = completion_tokens;
        req_log.total_tokens = prompt_tokens + completion_tokens;
        req_log.cost = cost;
        req_log.wait_time = elapsed_ms;
        req_log.error = error_opt;
        req_log.rounds_detail = serde_json::to_string(&all_rounds).unwrap_or_default();

        let overview = RequestOverview::from(&req_log);
        self.state.in_flight.broadcast(overview);
        self.state.in_flight.unregister(self.req_id);

        let state = self.state.clone();
        tokio::spawn(async move {
            if let Err(e) = log::update_completed_log(&state, &req_log).await {
                tracing::error!("Failed to update completed log in db: {}", e);
            }
        });
    }
}

impl<S> Stream for StreamFinalizer<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        let this = self.as_mut().get_mut();
        if let Some(rx) = &this.cancel_rx
            && *rx.borrow()
        {
            this.interrupted = true;
            this.finalize(false);
            return Poll::Ready(None);
        }
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.output_bytes += bytes.len();
                this.buffer.extend_from_slice(&bytes);
                this.extract_usage_from_sse();
                if this.response_capture.len() < RESPONSE_CAPTURE_LIMIT {
                    let take = (RESPONSE_CAPTURE_LIMIT - this.response_capture.len()).min(bytes.len());
                    this.response_capture.extend_from_slice(&bytes[..take]);
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                this.finalize(false);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                this.finalize(true);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for StreamFinalizer<S> {
    fn drop(&mut self) {
        // Body dropped before completing (client disconnect): record the attempt.
        self.finalize(false);
    }
}

fn validate_relay_request(
    state: &AppState,
    auth_ctx: &AuthContext,
    body_bytes: &Bytes,
) -> Result<(protocol::ChatRequestMetadata, Group), Box<Response>> {
    let metadata = match parse_request_metadata(body_bytes) {
        Some(m) if !m.model.trim().is_empty() => m,
        _ => {
            return Err(Box::new(error(
                StatusCode::BAD_REQUEST,
                "Invalid request: missing model",
            )));
        }
    };

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
) -> (String, Vec<u8>) {
    let rewritten_body = rewrite_request_body_in_json(body_bytes, &channel_model.name, ch.param_override.as_deref());
    let parsed: Option<serde_json::Value> = serde_json::from_slice(&rewritten_body).ok();

    let (endpoint, converted_val) = match (format, &ch.channel_type) {
        (ApiFormat::OpenaiCompletion, ChannelProvider::Anthropic) => (
            "/messages".to_string(),
            parsed.as_ref().map(protocol::convert_openai_to_anthropic),
        ),
        (ApiFormat::OpenAIResponse, ChannelProvider::Anthropic) => {
            let val = parsed.as_ref().map(|v| {
                let completions = protocol::convert_responses_to_chat_completions(v);
                protocol::convert_openai_to_anthropic(&completions)
            });
            ("/messages".to_string(), val)
        }
        (ApiFormat::OpenaiCompletion, ChannelProvider::Gemini) => (
            gemini_endpoint(ch, &channel_model.name, stream),
            parsed.as_ref().map(protocol::convert_openai_to_gemini),
        ),
        (ApiFormat::OpenAIResponse, ChannelProvider::Gemini) => {
            let val = parsed.as_ref().map(|v| {
                let completions = protocol::convert_responses_to_chat_completions(v);
                protocol::convert_openai_to_gemini(&completions)
            });
            (gemini_endpoint(ch, &channel_model.name, stream), val)
        }
        (ApiFormat::AnthropicMessage, ChannelProvider::OpenaiCompletion) => (
            "/chat/completions".to_string(),
            parsed
                .as_ref()
                .map(protocol::convert_anthropic_to_openai_chat_request),
        ),
        (ApiFormat::AnthropicMessage, ChannelProvider::OpenaiResponses) => {
            let val = parsed.as_ref().map(|v| {
                let chat = protocol::convert_anthropic_to_openai_chat_request(v);
                protocol::convert_chat_completions_to_responses(&chat)
            });
            ("/responses".to_string(), val)
        }
        (ApiFormat::AnthropicMessage, ChannelProvider::Gemini) => {
            let val = parsed.as_ref().map(|v| {
                let chat = protocol::convert_anthropic_to_openai_chat_request(v);
                protocol::convert_openai_to_gemini(&chat)
            });
            (gemini_endpoint(ch, &channel_model.name, stream), val)
        }
        (ApiFormat::OpenAIResponse, ChannelProvider::OpenaiCompletion) => (
            "/chat/completions".to_string(),
            parsed
                .as_ref()
                .map(protocol::convert_responses_to_chat_completions),
        ),
        (ApiFormat::OpenaiCompletion, ChannelProvider::OpenaiResponses) => (
            "/responses".to_string(),
            parsed
                .as_ref()
                .map(protocol::convert_chat_completions_to_responses),
        ),
        _ => {
            let endpoint = match ch.channel_type {
                ChannelProvider::Anthropic => "/messages",
                ChannelProvider::OpenaiResponses => "/responses",
                ChannelProvider::OpenaiCompletion => "/chat/completions",
                ChannelProvider::Gemini => return (gemini_endpoint(ch, &channel_model.name, stream), rewritten_body),
            };
            (endpoint.to_string(), None)
        }
    };

    let body = converted_val
        .and_then(|val| serde_json::to_vec(&val).ok())
        .unwrap_or(rewritten_body);

    (endpoint, body)
}

struct RoundFailure<'a> {
    round:               i64,
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
    if fail.should_cooldown {
        state
            .route_manager
            .report_failure(group_id, item_id, cooldown_seconds);
    } else {
        tracing::warn!(
            "Upstream returned client error {} from channel {}: {}, not triggering cooldown",
            fail.status,
            fail.channel_name,
            fail.err_body
        );
    }

    state.in_flight.broadcast(RequestOverview {
        id:             req_id,
        status:         "running".to_string(),
        started_at:     String::new(),
        duration:       fail.duration_ms,
        model:          String::new(),
        usage:          Usage::default(),
        cost:           0.0,
        round:          fail.round,
        target_channel: fail.channel_name.to_string(),
        target_model:   fail.model_name.to_string(),
        sending:        false,
        error:          Some(fail.err_body.to_string()),
        rounds:         rounds.clone(),
    });
}

struct SuccessContext<'a> {
    request_id:          u64,
    model:               &'a str,
    format:              ApiFormat,
    channel:             &'a Channel,
    channel_model:       &'a ChannelModel,
    api_key_id:          i64,
    target_req_url:      &'a str,
    channel_req_headers: &'a str,
    rewritten_body_str:  &'a str,
    body_str:            &'a str,
    started_at:          i64,
    round:               i64,
    round_start_time:    Instant,
    start_time:          Instant,
    price_opt:           Option<LLMPrice>,
    rounds:              Vec<crate::model::RoundLog>,
}

fn handle_successful_stream(state: &AppState, upstream_resp: reqwest::Response, ctx: SuccessContext<'_>) -> Response {
    state.in_flight.broadcast(RequestOverview {
        id:             ctx.request_id,
        status:         "committed".to_string(),
        started_at:     chrono::DateTime::from_timestamp_millis(ctx.started_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
        duration:       ctx.start_time.elapsed().as_millis() as i64,
        model:          ctx.model.to_string(),
        usage:          Usage::default(),
        cost:           0.0,
        round:          ctx.round,
        target_channel: ctx.channel.name.clone(),
        target_model:   ctx.channel_model.name.clone(),
        sending:        true,
        error:          None,
        rounds:         ctx.rounds.clone(),
    });

    let log_template = RequestLog {
        id: ctx.request_id as i64,
        model: ctx.model.to_string(),
        target_channel: ctx.channel.name.clone(),
        target_model: ctx.channel_model.name.clone(),
        request_url: ctx.target_req_url.to_string(),
        request_headers: ctx.channel_req_headers.to_string(),
        request_body: ctx.rewritten_body_str.to_string(),
        response_url: ctx.target_req_url.to_string(),
        response_headers: serialize_headers(upstream_resp.headers()),
        created_at: ctx.started_at,
        round: ctx.round,
        ..Default::default()
    };

    let raw_stream = upstream_resp.bytes_stream();
    let mapped_stream = raw_stream.map(move |chunk_res| match chunk_res {
        Ok(bytes) => Ok::<Bytes, std::io::Error>(bytes),
        Err(e) => Err(std::io::Error::other(e)),
    });

    let prompt_tokens = estimate_tokens(ctx.body_str);

    let output_stream = match ctx.channel.channel_type {
        ChannelProvider::Anthropic if ctx.format != ApiFormat::AnthropicMessage => ProviderToOpenaiStream::new(
            mapped_stream,
            ctx.model.to_string(),
            protocol::anthropic_sse_event_to_openai_chunk,
        )
        .boxed(),
        ChannelProvider::Gemini => ProviderToOpenaiStream::new(
            mapped_stream,
            ctx.model.to_string(),
            protocol::gemini_sse_event_to_openai_chunk,
        )
        .boxed(),
        _ => mapped_stream.boxed(),
    };
    let output_stream = if ctx.format == ApiFormat::AnthropicMessage
        && ctx.channel.channel_type != ChannelProvider::Anthropic
    {
        let source_format = if ctx.channel.channel_type == ChannelProvider::OpenaiResponses {
            OpenaiStreamFormat::Responses
        } else {
            OpenaiStreamFormat::ChatCompletions
        };
        OpenaiToAnthropicStream::new(output_stream, ctx.model.to_string(), source_format).boxed()
    } else if ctx.format == ApiFormat::OpenAIResponse && ctx.channel.channel_type != ChannelProvider::OpenaiResponses {
        ChatCompletionsToResponsesStream::new(output_stream, ctx.model.to_string()).boxed()
    } else {
        output_stream
    };
    let finalizer = StreamFinalizer {
        inner: output_stream,
        state: state.clone(),
        req_id: ctx.request_id,
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
        cancel_rx: state.in_flight.get_cancel_receiver(ctx.request_id),
        interrupted: false,
    };
    let body = Body::from_stream(finalizer);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(body)
        .unwrap_or_else(|e| {
            tracing::error!("Failed to build SSE response: {}", e);
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
    let converted_to_chat = parsed
        .as_ref()
        .and_then(|val| match ctx.channel.channel_type {
            ChannelProvider::Anthropic if ctx.format != ApiFormat::AnthropicMessage => {
                Some(protocol::convert_anthropic_to_openai_chat_response(val, ctx.model))
            }
            ChannelProvider::Gemini => Some(protocol::convert_gemini_to_openai_chat_response(val, ctx.model)),
            _ => None,
        });
    let converted = if ctx.format == ApiFormat::AnthropicMessage
        && ctx.channel.channel_type != ChannelProvider::Anthropic
    {
        parsed.as_ref().map(|val| {
            if ctx.channel.channel_type == ChannelProvider::OpenaiResponses {
                protocol::convert_responses_to_anthropic_response(val, ctx.model)
            } else {
                let chat = converted_to_chat.as_ref().unwrap_or(val);
                protocol::convert_openai_chat_to_anthropic_response(chat, ctx.model)
            }
        })
    } else if ctx.format == ApiFormat::OpenAIResponse && ctx.channel.channel_type != ChannelProvider::OpenaiResponses {
        converted_to_chat
            .as_ref()
            .or(parsed.as_ref())
            .map(|val| protocol::convert_openai_chat_to_responses_response(val, ctx.model))
    } else {
        converted_to_chat
    };
    let resp_bytes = converted
        .as_ref()
        .and_then(|val| serde_json::to_vec(val).ok())
        .map(Bytes::from)
        .unwrap_or(raw_bytes);
    let resp_str = String::from_utf8_lossy(&resp_bytes).to_string();

    let usage = converted
        .as_ref()
        .or(parsed.as_ref())
        .and_then(extract_usage_from_json)
        .unwrap_or_else(|| Usage {
            prompt_tokens:     estimate_tokens(ctx.body_str),
            completion_tokens: estimate_tokens(&resp_str),
            total_tokens:      estimate_tokens(ctx.body_str) + estimate_tokens(&resp_str),
        });

    let cost = calculate_cost(usage.prompt_tokens, usage.completion_tokens, ctx.price_opt.as_ref());
    let elapsed_ms = ctx.start_time.elapsed().as_millis() as i64;
    let round_duration_ms = ctx.round_start_time.elapsed().as_millis() as i64;

    let round_log = crate::model::RoundLog {
        round:            ctx.round,
        channel_name:     ctx.channel.name.clone(),
        model_name:       ctx.channel_model.name.clone(),
        request_url:      ctx.target_req_url.to_string(),
        request_headers:  ctx.channel_req_headers.to_string(),
        request_body:     ctx.rewritten_body_str.to_string(),
        response_url:     ctx.target_req_url.to_string(),
        response_headers: response_headers.clone(),
        response_body:    resp_str.clone(),
        status:           200,
        duration_ms:      round_duration_ms,
        error:            None,
    };
    ctx.rounds.push(round_log.clone());
    let req_log = RequestLog {
        id: ctx.request_id as i64,
        status: "success".to_string(),
        model: ctx.model.to_string(),
        target_channel: ctx.channel.name.clone(),
        target_model: ctx.channel_model.name.clone(),
        request_url: ctx.target_req_url.to_string(),
        request_headers: ctx.channel_req_headers.to_string(),
        request_body: ctx.rewritten_body_str.to_string(),
        response_url: ctx.target_req_url.to_string(),
        response_headers,
        response_body: resp_str.clone(),
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cost,
        wait_time: elapsed_ms,
        round: ctx.round,
        error: None,
        rounds_detail: serde_json::to_string(&ctx.rounds).unwrap_or_default(),
        created_at: ctx.started_at,
    };

    if let Err(e) = log::update_completed_log(state, &req_log).await {
        tracing::error!("Failed to update completed log in db: {}", e);
    }
    state.in_flight.broadcast(RequestOverview::from(&req_log));
    state.in_flight.unregister(ctx.request_id);

    let delta = StatsMetrics {
        input_token:     usage.prompt_tokens,
        output_token:    usage.completion_tokens,
        input_cost:      (usage.prompt_tokens as f64 / 1_000_000.0)
            * ctx.price_opt.as_ref().map(|p| p.input).unwrap_or(0.0),
        output_cost:     (usage.completion_tokens as f64 / 1_000_000.0)
            * ctx.price_opt.as_ref().map(|p| p.output).unwrap_or(0.0),
        wait_time:       elapsed_ms,
        request_success: 1,
        request_failed:  0,
    };

    stats::record_metrics(state, ctx.api_key_id, ctx.channel.id, ctx.channel_model.id, &delta);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(resp_bytes))
        .unwrap_or_else(|e| {
            tracing::error!("Failed to build JSON response: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Internal Server Error"))
                .unwrap_or_default()
        })
}

struct FailedRelayContext<'a> {
    request_id:            u64,
    model:                 &'a str,
    last_target_channel:   String,
    last_target_model:     String,
    request_url:           String,
    request_headers:       String,
    request_body:          String,
    last_response_url:     String,
    last_response_headers: String,
    last_status:           StatusCode,
    last_error_body:       String,
    rounds:                Vec<crate::model::RoundLog>,
    started_at:            i64,
    wait_time_ms:          i64,
    round:                 i64,
    api_key_id:            i64,
}

fn build_failed_relay_response(state: &AppState, ctx: FailedRelayContext<'_>) -> Response {
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
        let fallback_msg = "Upstream request failed after all attempts".to_string();
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

    let req_log = RequestLog {
        id:                ctx.request_id as i64,
        status:            "failed".to_string(),
        model:             ctx.model.to_string(),
        target_channel:    ctx.last_target_channel,
        target_model:      ctx.last_target_model,
        request_url:       ctx.request_url,
        request_headers:   ctx.request_headers,
        request_body:      ctx.request_body,
        response_url:      ctx.last_response_url,
        response_headers:  ctx.last_response_headers,
        response_body:     ctx.last_error_body,
        prompt_tokens:     0,
        completion_tokens: 0,
        total_tokens:      0,
        cost:              0.0,
        wait_time:         ctx.wait_time_ms,
        round:             ctx.round,
        error:             Some(error_message),
        rounds_detail:     serde_json::to_string(&ctx.rounds).unwrap_or_default(),
        created_at:        ctx.started_at,
    };
    state.in_flight.broadcast(RequestOverview::from(&req_log));
    state.in_flight.unregister(ctx.request_id);

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = log::update_completed_log(&state_clone, &req_log).await {
            tracing::error!("Failed to update failed log in db: {}", e);
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
            tracing::error!("Failed to build error response: {}", e);
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("Bad Gateway"))
                .unwrap_or_default()
        })
}

pub async fn forward_relay(
    state: AppState,
    auth_ctx: Arc<AuthContext>,
    format: ApiFormat,
    uri: Uri,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Response {
    let (metadata, group_obj) = match validate_relay_request(&state, &auth_ctx, &body_bytes) {
        Ok(v) => v,
        Err(err_resp) => return *err_resp,
    };

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
            tracing::error!("Failed to create initial request log: {}", e);
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to initialize request log");
        }
    };

    let cancel_rx = state.in_flight.register(request_id);
    let started_at_rfc3339 = chrono::DateTime::from_timestamp_millis(started_at)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();
    state.in_flight.broadcast(RequestOverview {
        id:             request_id,
        status:         "running".to_string(),
        started_at:     started_at_rfc3339.clone(),
        duration:       0,
        model:          metadata.model.clone(),
        usage:          Usage::default(),
        cost:           0.0,
        round:          1,
        target_channel: String::new(),
        target_model:   String::new(),
        sending:        false,
        error:          None,
        rounds:         Vec::new(),
    });

    let max_attempts = group_obj.relay_config.member_max_attempts.max(1);

    let mut round = 0;
    let mut last_target_channel = String::new();
    let mut last_target_model = String::new();
    let mut last_response_url = String::new();
    let mut last_response_headers = String::new();
    let mut last_status = StatusCode::BAD_GATEWAY;
    let mut last_error_body = String::new();
    let mut rounds: Vec<crate::model::RoundLog> = Vec::new();

    while round < max_attempts {
        if *cancel_rx.borrow() {
            last_error_body = "request interrupted".to_string();
            last_status = StatusCode::from_u16(499).unwrap_or(StatusCode::BAD_REQUEST);
            break;
        }
        round += 1;
        let round_start_time = Instant::now();

        let current_group = match group::get_group_by_name(&state, &metadata.model) {
            Ok(g) => g,
            Err(_) => break,
        };

        let item = match state.route_manager.pick_group_item(&current_group) {
            Some(it) => it,
            None => {
                if current_group.items.is_empty() {
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }
        };

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
            _ => {
                state
                    .route_manager
                    .report_failure(current_group.id, item.id, 60);
                continue;
            }
        };

        let (target_endpoint, final_body) =
            prepare_upstream_payload(format, metadata.stream, &body_bytes, &ch, &channel_model);
        let target_req_url = format!("{}{}", normalize_base_url(&ch.base_url), target_endpoint);
        let channel_req_headers = upstream::build_upstream_headers(&ch)
            .map(|h| serialize_headers(&h))
            .unwrap_or_else(|_| request_headers.clone());
        let rewritten_body_str = String::from_utf8_lossy(&final_body).to_string();

        state.in_flight.broadcast(RequestOverview {
            id: request_id,
            status: "running".to_string(),
            started_at: started_at_rfc3339.clone(),
            duration: start_time.elapsed().as_millis() as i64,
            model: metadata.model.clone(),
            usage: Usage::default(),
            cost: 0.0,
            round,
            target_channel: ch.name.clone(),
            target_model: channel_model.name.clone(),
            sending: true,
            error: None,
            rounds: rounds.clone(),
        });

        let timeout_secs = if metadata.stream {
            None
        } else {
            Some(
                current_group
                    .relay_config
                    .member_non_stream_response_timeout_seconds
                    .max(10) as u64,
            )
        };

        let client = get_client();
        let req_builder = match build_upstream_request(client, &ch, &target_endpoint, Some(final_body), timeout_secs) {
            Ok(rb) => rb,
            Err(e) => {
                let err_msg = format!("Failed to build request for channel {}: {}", ch.name, e);
                last_target_channel = ch.name.clone();
                last_target_model = channel_model.name.clone();
                last_error_body = err_msg.clone();

                record_round_failure(
                    &state,
                    current_group.id,
                    item.id,
                    60,
                    request_id,
                    &mut rounds,
                    RoundFailure {
                        round,
                        channel_name: &ch.name,
                        model_name: &channel_model.name,
                        target_req_url: &target_req_url,
                        channel_req_headers: &channel_req_headers,
                        rewritten_body_str: &rewritten_body_str,
                        response_headers: "",
                        status: 500,
                        duration_ms: round_start_time.elapsed().as_millis() as i64,
                        err_body: &err_msg,
                        should_cooldown: true,
                    },
                );
                if round < max_attempts && current_group.relay_config.member_retry_interval_seconds > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_secs(
                        current_group.relay_config.member_retry_interval_seconds as u64,
                    ))
                    .await;
                }
                continue;
            }
        };
        let send_future = req_builder.send();
        let send_res = if metadata.stream {
            let first_event_timeout_secs = current_group
                .relay_config
                .member_stream_first_event_timeout_seconds
                .max(5) as u64;
            match tokio::time::timeout(std::time::Duration::from_secs(first_event_timeout_secs), send_future).await {
                Ok(res) => res.map_err(|e| e.to_string()),
                Err(_) => Err(format!(
                    "upstream stream response timeout after {}s waiting for initial headers",
                    first_event_timeout_secs
                )),
            }
        } else {
            send_future.await.map_err(|e| e.to_string())
        };

        match send_res {
            Ok(upstream_resp) => {
                let status = upstream_resp.status();
                if status.is_success() {
                    state
                        .route_manager
                        .report_success(current_group.id, item.id);
                    let price_opt = llm::get_llm_price(&state, &channel_model.name);

                    if metadata.stream {
                        return handle_successful_stream(
                            &state,
                            upstream_resp,
                            SuccessContext {
                                request_id,
                                model: &metadata.model,
                                format,
                                channel: &ch,
                                channel_model: &channel_model,
                                api_key_id: auth_ctx.api_key.id,
                                target_req_url: &target_req_url,
                                channel_req_headers: &channel_req_headers,
                                rewritten_body_str: &rewritten_body_str,
                                body_str: &body_str,
                                started_at,
                                round,
                                round_start_time,
                                start_time,
                                price_opt,
                                rounds,
                            },
                        );
                    } else {
                        return handle_successful_json(
                            &state,
                            upstream_resp,
                            SuccessContext {
                                request_id,
                                model: &metadata.model,
                                format,
                                channel: &ch,
                                channel_model: &channel_model,
                                api_key_id: auth_ctx.api_key.id,
                                target_req_url: &target_req_url,
                                channel_req_headers: &channel_req_headers,
                                rewritten_body_str: &rewritten_body_str,
                                body_str: &body_str,
                                started_at,
                                round,
                                round_start_time,
                                start_time,
                                price_opt,
                                rounds,
                            },
                        )
                        .await;
                    }
                } else {
                    let resp_headers = serialize_headers(upstream_resp.headers());
                    let err_body = upstream_resp.text().await.unwrap_or_default();
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
                        current_group.relay_config.member_cooldown_seconds,
                        request_id,
                        &mut rounds,
                        RoundFailure {
                            round,
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
                    current_group.relay_config.member_cooldown_seconds,
                    request_id,
                    &mut rounds,
                    RoundFailure {
                        round,
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

        if round < max_attempts && current_group.relay_config.member_retry_interval_seconds > 0 {
            let mut rx = cancel_rx.clone();
            tokio::select! {
                _ = rx.changed() => {
                    if *rx.borrow() {
                        last_error_body = "request interrupted".to_string();
                        last_status = StatusCode::from_u16(499).unwrap_or(StatusCode::BAD_REQUEST);
                        break;
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(
                    current_group.relay_config.member_retry_interval_seconds as u64,
                )) => {}
            }
        }
    }

    build_failed_relay_response(
        &state,
        FailedRelayContext {
            request_id,
            model: &metadata.model,
            last_target_channel,
            last_target_model,
            request_url,
            request_headers,
            request_body,
            last_response_url,
            last_response_headers,
            last_status,
            last_error_body,
            rounds,
            started_at,
            wait_time_ms: start_time.elapsed().as_millis() as i64,
            round,
            api_key_id: auth_ctx.api_key.id,
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_event_end_finds_lf_and_crlf() {
        assert_eq!(sse_event_end(b"data: a\n\n"), Some(9));
        assert_eq!(sse_event_end(b"data: a\r\n\r\n"), Some(11));
        assert_eq!(sse_event_end(b"data: a\n"), None);
    }

    #[test]
    fn extracts_openai_usage_across_split_chunks() {
        let mut buffer = Vec::new();
        let mut usage = None;
        let pieces = [
            b"data: ".as_slice(),
            b"{\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":7,\"total_tokens\":12}}".as_slice(),
            b"\n\ndata: [DONE]\n\n".as_slice(),
        ];
        for piece in pieces {
            buffer.extend_from_slice(piece);
            extract_usage_from_sse_buffer(&mut buffer, &mut usage);
        }
        let u = usage.expect("usage extracted");
        assert_eq!(u.prompt_tokens, 5);
        assert_eq!(u.completion_tokens, 7);
        assert_eq!(u.total_tokens, 12);
        assert!(buffer.is_empty());
    }

    #[test]
    fn extracts_anthropic_message_delta_output_tokens() {
        let mut buffer = b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n".to_vec();
        let mut usage = None;
        extract_usage_from_sse_buffer(&mut buffer, &mut usage);
        let u = usage.expect("usage extracted");
        assert_eq!(u.completion_tokens, 9);
        assert_eq!(u.prompt_tokens, 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn ignores_non_json_sse_lines() {
        let mut buffer = b"event: done\n: comment\ndata: [DONE]\n\n".to_vec();
        let mut usage = None;
        extract_usage_from_sse_buffer(&mut buffer, &mut usage);
        assert!(usage.is_none());
        assert!(buffer.is_empty());
    }

    #[test]
    fn round_logs_serialization_and_deserialization() {
        use crate::model::RoundLog;

        let rounds = vec![
            RoundLog {
                round:            1,
                channel_name:     "openai-primary".to_string(),
                model_name:       "gpt-4o".to_string(),
                request_url:      "https://api.openai.com/v1/chat/completions".to_string(),
                request_headers:  "{\"authorization\":\"Bearer key1\"}".to_string(),
                request_body:     "{\"model\":\"gpt-4o\",\"messages\":[]}".to_string(),
                response_url:     "https://api.openai.com/v1/chat/completions".to_string(),
                response_headers: "{\"content-type\":\"application/json\"}".to_string(),
                response_body:    "{\"error\":{\"message\":\"Rate limit reached\"}}".to_string(),
                status:           429,
                duration_ms:      120,
                error:            Some("{\"error\":{\"message\":\"Rate limit reached\"}}".to_string()),
            },
            RoundLog {
                round:            2,
                channel_name:     "openai-backup".to_string(),
                model_name:       "gpt-4o".to_string(),
                request_url:      "https://api.openai-backup.com/v1/chat/completions".to_string(),
                request_headers:  "{\"authorization\":\"Bearer key2\"}".to_string(),
                request_body:     "{\"model\":\"gpt-4o\",\"messages\":[]}".to_string(),
                response_url:     "https://api.openai-backup.com/v1/chat/completions".to_string(),
                response_headers: "{\"content-type\":\"application/json\"}".to_string(),
                response_body:    "{\"choices\":[{\"message\":{\"content\":\"Hello\"}}]}".to_string(),
                status:           200,
                duration_ms:      350,
                error:            None,
            },
        ];

        let json_str = serde_json::to_string(&rounds).expect("serialize rounds");
        let parsed: Vec<RoundLog> = serde_json::from_str(&json_str).expect("deserialize rounds");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].round, 1);
        assert_eq!(parsed[0].channel_name, "openai-primary");
        assert_eq!(parsed[0].status, 429);
        assert!(parsed[0].error.is_some());

        assert_eq!(parsed[1].round, 2);
        assert_eq!(parsed[1].channel_name, "openai-backup");
        assert_eq!(parsed[1].status, 200);
        assert!(parsed[1].error.is_none());
    }
    #[test]
    fn test_param_override_applied_to_request_body() {
        let original = br#"{"model":"gpt-4o","temperature":0.7,"messages":[]}"#;
        let override_json = r#"{"temperature":0.2,"max_tokens":1024}"#;
        let rewritten = rewrite_request_body_in_json(original, "gpt-4o-custom", Some(override_json));
        let val: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(val.get("model").unwrap().as_str().unwrap(), "gpt-4o-custom");
        assert_eq!(val.get("temperature").unwrap().as_f64().unwrap(), 0.2);
        assert_eq!(val.get("max_tokens").unwrap().as_i64().unwrap(), 1024);
    }
    #[test]
    fn test_is_client_error() {
        assert!(is_client_error(StatusCode::BAD_REQUEST));
        assert!(is_client_error(StatusCode::UNPROCESSABLE_ENTITY));
        // Upstream auth failure / channel issues should NOT be classified as client errors
        assert!(!is_client_error(StatusCode::UNAUTHORIZED));
        assert!(!is_client_error(StatusCode::FORBIDDEN));
        assert!(!is_client_error(StatusCode::NOT_FOUND));
        assert!(!is_client_error(StatusCode::TOO_MANY_REQUESTS));
        assert!(!is_client_error(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_client_error(StatusCode::BAD_GATEWAY));
    }
}
