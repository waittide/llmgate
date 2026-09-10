//! 流式方向的 IR：上游 SSE 事件 → `StreamEvent` → 客户端格式字节。
//!
//! `EventParser`（上游侧）与 `EventSink`（客户端侧）解耦：
//! `tool_calls[].index` 从 0 严格递增的不变量由 ChatChunkSink 单点维护。

use std::{
    collections::{HashMap, HashSet, VecDeque},
    pin::Pin,
    task::{Context, Poll},
};

use axum::http::StatusCode;
use bytes::Bytes;
use futures::Stream;
use serde_json::{Value, json};

use super::{
    StopReason, anthropic_stop_reason, anthropic_usage, chat_finish_reason, chat_usage, gemini_call_id,
    gemini_finish_reason, gemini_usage, responses_status, responses_usage, stop_reason_from_anthropic,
    stop_reason_from_chat, stop_reason_from_gemini, stop_reason_from_responses,
};
use crate::relay::{
    SseBuffer, parse_sse_event,
    protocol::{ApiFormat, extract_usage_from_json},
    state::Usage,
};

/// 流式事件的中间定义。
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    /// 工具调用开始；`arguments` 为该事件内已知的初始参数（可为空串）
    ToolCallStart {
        call_id:   String,
        name:      String,
        arguments: String,
    },
    ToolCallDelta {
        call_id:   String,
        arguments: String,
    },
    Usage(Usage),
    Finish {
        reason: StopReason,
    },
    /// 上游流内错误（HTTP 200 但事件体是错误信封）：sink 先向客户端发出协议错误事件，
    /// 泵随后以 Err 终止传输——绝不能把它当成正常结束，否则客户端会拿到「空但成功」的结果。
    Error {
        message: String,
    },
}

/// 事件体是错误信封时取出可读消息。
///
/// 只认顶层 `error`（四种协议的错误信封形状一致：OpenAI/Anthropic/Gemini 均为
/// `{"error": {...}}`，Anthropic 另有 `type: "error"`），避免把正常事件里
/// 恰好同名的字段误判为错误。
fn stream_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if error.is_null() {
        return None;
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::trim)
        .filter(|message| !message.is_empty());
    Some(message.unwrap_or("上游流内错误").to_string())
}

/// 上游 SSE 事件 → IR 事件。
pub trait EventParser {
    fn parse(&mut self, event: &str) -> Vec<StreamEvent>;
    /// 上游是否已显式发送终止事件（决定 EOF 时是否补发 Finish）
    fn finished(&self) -> bool;
}

/// IR 事件 → 客户端格式字节。
pub trait EventSink {
    fn handle(&mut self, event: &StreamEvent) -> Vec<Bytes>;
    /// 流收尾：对未终止的流补发终态字节（chat 为 finish chunk + [DONE]）
    fn finish(&mut self) -> Vec<Bytes>;
}

/// 字段级合并用量：非零值优先（例如 Anthropic 的 input/output 分属两个事件）。
fn merge_usage(existing: &mut Option<Usage>, incoming: Usage) {
    let merged = match existing.take() {
        Some(old) => Usage {
            prompt_tokens:      if old.prompt_tokens > 0 {
                old.prompt_tokens
            } else {
                incoming.prompt_tokens
            },
            completion_tokens:  if old.completion_tokens > 0 {
                old.completion_tokens
            } else {
                incoming.completion_tokens
            },
            total_tokens:       if old.total_tokens > 0 {
                old.total_tokens
            } else {
                incoming.total_tokens
            },
            cache_read_tokens:  if old.cache_read_tokens > 0 {
                old.cache_read_tokens
            } else {
                incoming.cache_read_tokens
            },
            cache_write_tokens: if old.cache_write_tokens > 0 {
                old.cache_write_tokens
            } else {
                incoming.cache_write_tokens
            },
        },
        None => incoming,
    };
    *existing = Some(merged);
}

// ---------- 上游 Parsers ----------

/// OpenAI Chat Completions SSE。
#[derive(Default)]
pub struct ChatEventParser {
    tool_calls: HashMap<u64, String>,
    finished:   bool,
}

impl EventParser for ChatEventParser {
    fn parse(&mut self, event: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        for data in event
            .lines()
            .filter_map(|line| line.trim().strip_prefix("data:").map(str::trim))
        {
            if data == "[DONE]" {
                if !self.finished {
                    self.finished = true;
                    events.push(StreamEvent::Finish {
                        reason: StopReason::Stop,
                    });
                }
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(message) = stream_error_message(&value) {
                self.finished = true;
                events.push(StreamEvent::Error { message });
                break;
            }
            if let Some(usage) = extract_usage_from_json(&value) {
                events.push(StreamEvent::Usage(usage));
            }
            let Some(choice) = value.pointer("/choices/0") else {
                continue;
            };
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                events.push(StreamEvent::TextDelta { text: text.to_string() });
            }
            if let Some(thinking) = delta.get("reasoning_content").and_then(Value::as_str)
                && !thinking.is_empty()
            {
                events.push(StreamEvent::ThinkingDelta {
                    text: thinking.to_string(),
                });
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let initial = call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if let Some(call_id) = self.tool_calls.get(&index) {
                        if !initial.is_empty() {
                            events.push(StreamEvent::ToolCallDelta {
                                call_id:   call_id.clone(),
                                arguments: initial.to_string(),
                            });
                        }
                        continue;
                    }
                    let call_id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{}", nanoid::nanoid!(20)));
                    self.tool_calls.insert(index, call_id.clone());
                    events.push(StreamEvent::ToolCallStart {
                        call_id,
                        name: call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        arguments: initial.to_string(),
                    });
                }
            }
            let finishing = choice
                .get("finish_reason")
                .is_some_and(|reason| !reason.is_null());
            if finishing && !self.finished {
                self.finished = true;
                events.push(StreamEvent::Finish {
                    reason: stop_reason_from_chat(choice.get("finish_reason").and_then(Value::as_str)),
                });
            }
        }
        events
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

/// OpenAI Responses SSE。
#[derive(Default)]
pub struct ResponsesEventParser {
    item_to_call: HashMap<String, String>,
    finished:     bool,
}

impl EventParser for ResponsesEventParser {
    fn parse(&mut self, event: &str) -> Vec<StreamEvent> {
        let (event_type, data) = parse_sse_event(event);
        if data.is_empty() {
            return Vec::new();
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return Vec::new();
        };
        if let Some(message) = stream_error_message(&value) {
            self.finished = true;
            return vec![StreamEvent::Error { message }];
        }
        match event_type {
            "response.output_text.delta" => value
                .get("delta")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(|t| vec![StreamEvent::TextDelta { text: t.to_string() }])
                .unwrap_or_default(),
            "response.reasoning_summary_text.delta" => value
                .get("delta")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(|t| vec![StreamEvent::ThinkingDelta { text: t.to_string() }])
                .unwrap_or_default(),
            "response.output_item.added" => {
                let item = value.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return Vec::new();
                }
                let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| item_id.to_string());
                if !item_id.is_empty() {
                    self.item_to_call
                        .insert(item_id.to_string(), call_id.clone());
                }
                vec![StreamEvent::ToolCallStart {
                    call_id,
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                }]
            }
            "response.function_call_arguments.delta" => {
                let item_id = value.get("item_id").and_then(Value::as_str).unwrap_or("");
                let call_id = self
                    .item_to_call
                    .get(item_id)
                    .cloned()
                    .unwrap_or_else(|| item_id.to_string());
                value
                    .get("delta")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .map(|t| {
                        vec![StreamEvent::ToolCallDelta {
                            call_id,
                            arguments: t.to_string(),
                        }]
                    })
                    .unwrap_or_default()
            }
            // 截断/过滤同样以终态事件收尾：漏掉 response.incomplete 会把它当未知事件
            // 丢弃，EOF 时被补成 Stop，客户端的截断原因就此丢失。
            "response.completed" | "response.incomplete" => {
                let mut events = Vec::new();
                if let Some(usage) = extract_usage_from_json(&value) {
                    events.push(StreamEvent::Usage(usage));
                }
                self.finished = true;
                let reason = stop_reason_from_responses(
                    value.pointer("/response/status").and_then(Value::as_str),
                    value
                        .pointer("/response/incomplete_details/reason")
                        .and_then(Value::as_str),
                );
                events.push(StreamEvent::Finish { reason });
                events
            }
            _ => Vec::new(),
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

/// Anthropic Messages SSE。
#[derive(Default)]
pub struct AnthropicEventParser {
    blocks:   HashMap<u64, String>,
    finished: bool,
}

impl EventParser for AnthropicEventParser {
    fn parse(&mut self, event: &str) -> Vec<StreamEvent> {
        let (event_type, data) = parse_sse_event(event);
        if data.is_empty() {
            return Vec::new();
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return Vec::new();
        };
        if let Some(message) = stream_error_message(&value) {
            self.finished = true;
            return vec![StreamEvent::Error { message }];
        }
        match event_type {
            "message_start" => extract_usage_from_json(&value)
                .map(StreamEvent::Usage)
                .into_iter()
                .collect(),
            "content_block_start" => {
                let block = value.get("content_block").unwrap_or(&Value::Null);
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    return Vec::new();
                }
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.blocks.insert(index, call_id.clone());
                vec![StreamEvent::ToolCallStart {
                    call_id,
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments: String::new(),
                }]
            }
            "content_block_delta" => {
                if let Some(text) = value.pointer("/delta/text").and_then(Value::as_str) {
                    return vec![StreamEvent::TextDelta { text: text.to_string() }];
                }
                if let Some(thinking) = value.pointer("/delta/thinking").and_then(Value::as_str) {
                    return vec![StreamEvent::ThinkingDelta {
                        text: thinking.to_string(),
                    }];
                }
                if let Some(partial) = value.pointer("/delta/partial_json").and_then(Value::as_str) {
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let call_id = self
                        .blocks
                        .get(&index)
                        .cloned()
                        .unwrap_or_else(|| index.to_string());
                    return vec![StreamEvent::ToolCallDelta {
                        call_id,
                        arguments: partial.to_string(),
                    }];
                }
                Vec::new()
            }
            "message_delta" => {
                let mut events = Vec::new();
                if let Some(usage) = extract_usage_from_json(&value) {
                    events.push(StreamEvent::Usage(usage));
                }
                self.finished = true;
                events.push(StreamEvent::Finish {
                    reason: stop_reason_from_anthropic(value.pointer("/delta/stop_reason").and_then(Value::as_str)),
                });
                events
            }
            // message_stop 的 Finish 已在 message_delta 发出
            "message_stop" => {
                self.finished = true;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

/// Google Gemini streamGenerateContent SSE。
#[derive(Default)]
pub struct GeminiEventParser {
    seen:     HashSet<String>,
    finished: bool,
}

impl EventParser for GeminiEventParser {
    fn parse(&mut self, event: &str) -> Vec<StreamEvent> {
        let Some(data) = event
            .lines()
            .find_map(|line| line.trim().strip_prefix("data:").map(str::trim))
            .filter(|d| !d.is_empty())
        else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(message) = stream_error_message(&value) {
            self.finished = true;
            return vec![StreamEvent::Error { message }];
        }
        let mut events = Vec::new();
        let parts = value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array);
        let text: String = parts
            .map(|p| {
                p.iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        if !text.is_empty() {
            events.push(StreamEvent::TextDelta { text });
        }
        for part in parts.into_iter().flatten() {
            let Some(call) = part.get("functionCall") else {
                continue;
            };
            let Some(name) = call.get("name").and_then(Value::as_str) else {
                continue;
            };
            let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
            let arguments_json = serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
            let call_id = gemini_call_id(name, &arguments_json);
            if !self.seen.insert(call_id.clone()) {
                continue;
            }
            events.push(StreamEvent::ToolCallStart {
                call_id,
                name: name.to_string(),
                arguments: arguments_json,
            });
        }
        if let Some(usage) = extract_usage_from_json(&value) {
            events.push(StreamEvent::Usage(usage));
        }
        if let Some(finish) = value
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            self.finished = true;
            events.push(StreamEvent::Finish {
                reason: stop_reason_from_gemini(Some(finish)),
            });
        }
        events
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

// ---------- 客户端 Sinks ----------

fn chunk_bytes(chunk: &Value) -> Bytes {
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(chunk).unwrap_or_default()
    ))
}

fn event_bytes(event_type: &str, data: &Value) -> Bytes {
    Bytes::from(format!(
        "event: {event_type}\ndata: {}\n\n",
        serde_json::to_string(data).unwrap_or_default()
    ))
}

/// OpenAI Chat Completions chunk 流。
pub struct ChatChunkSink {
    chatcmpl_id: String,
    created:     i64,
    model:       String,
    tool_calls:  HashMap<String, u64>,
    usage:       Option<Usage>,
    finished:    bool,
}

impl ChatChunkSink {
    pub fn new(model: String) -> Self {
        Self {
            chatcmpl_id: format!("chatcmpl-{}", nanoid::nanoid!(24)),
            created: chrono::Utc::now().timestamp(),
            model,
            tool_calls: HashMap::new(),
            usage: None,
            finished: false,
        }
    }

    fn tool_index(&mut self, call_id: &str) -> u64 {
        let next = self.tool_calls.len() as u64;
        *self.tool_calls.entry(call_id.to_string()).or_insert(next)
    }

    fn emit_finish(&mut self, reason: &StopReason) -> Vec<Bytes> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        self.finish_chunk(chat_finish_reason(reason, !self.tool_calls.is_empty()))
    }

    fn finish_chunk(&mut self, finish_reason: String) -> Vec<Bytes> {
        self.finished = true;
        let mut chunk = json!({
            "id": self.chatcmpl_id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{ "index": 0, "delta": {}, "finish_reason": finish_reason }]
        });
        if let Some(usage) = &self.usage {
            chunk["usage"] = chat_usage(usage);
        }
        vec![chunk_bytes(&chunk), Bytes::from_static(b"data: [DONE]\n\n")]
    }
}

impl EventSink for ChatChunkSink {
    fn handle(&mut self, event: &StreamEvent) -> Vec<Bytes> {
        match event {
            StreamEvent::TextDelta { text } => {
                let chunk = json!({
                    "id": self.chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": self.created,
                    "model": self.model,
                    "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": null }]
                });
                vec![chunk_bytes(&chunk)]
            }
            StreamEvent::ThinkingDelta { text } => {
                // 扩展思考转发为 reasoning_content（社区事实标准），同时保持流上有字节流动
                let chunk = json!({
                    "id": self.chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": self.created,
                    "model": self.model,
                    "choices": [{ "index": 0, "delta": { "reasoning_content": text }, "finish_reason": null }]
                });
                vec![chunk_bytes(&chunk)]
            }
            StreamEvent::ToolCallStart {
                call_id,
                name,
                arguments,
            } => {
                let index = self.tool_index(call_id);
                let chunk = json!({
                    "id": self.chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": self.created,
                    "model": self.model,
                    "choices": [{
                        "index": 0,
                        "delta": { "tool_calls": [{
                            "index": index, "id": call_id, "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        }] },
                        "finish_reason": null
                    }]
                });
                vec![chunk_bytes(&chunk)]
            }
            StreamEvent::ToolCallDelta { call_id, arguments } => {
                let index = self.tool_index(call_id);
                let chunk = json!({
                    "id": self.chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": self.created,
                    "model": self.model,
                    "choices": [{
                        "index": 0,
                        "delta": { "tool_calls": [{ "index": index, "function": { "arguments": arguments } }] },
                        "finish_reason": null
                    }]
                });
                vec![chunk_bytes(&chunk)]
            }
            StreamEvent::Usage(usage) => {
                merge_usage(&mut self.usage, usage.clone());
                Vec::new()
            }
            StreamEvent::Error { message } => {
                self.finished = true;
                // 不发 [DONE]：连接随即被中止，客户端据 error 字段报错而非当作正常结束
                vec![chunk_bytes(&error_envelope(ApiFormat::OpenaiCompletion, message))]
            }
            StreamEvent::Finish { reason } => self.emit_finish(reason),
        }
    }

    fn finish(&mut self) -> Vec<Bytes> {
        self.emit_finish(&StopReason::Stop)
    }
}

/// 流内错误的客户端信封：与 HTTP 错误路径共用同一份构造逻辑。
fn error_envelope(format: ApiFormat, message: &str) -> Value {
    crate::relay::error::envelope(format, StatusCode::BAD_GATEWAY, message, None)
}

/// OpenAI Responses SSE 流。
pub struct ResponsesSink {
    response_id:    String,
    model:          String,
    created_at:     i64,
    sequence:       u64,
    started:        bool,
    reasoning:      Option<OutputItem>,
    /// 已发出的推理摘要文本：终态对象的 summary 必须与 summary_text.delta 一致
    reasoning_text: String,
    message:        Option<OutputItem>,
    text:           String,
    tools:          std::collections::BTreeMap<String, PendingToolCall>,
    next_output:    usize,
    usage:          Option<Usage>,
    completed:      bool,
    pending:        VecDeque<Bytes>,
}

/// 已开始的 Responses 输出项（输出索引按开始顺序从 0 分配）。
#[derive(Clone)]
struct OutputItem {
    item_id:      String,
    output_index: usize,
}

/// Responses sink 中累积中的函数调用。
struct PendingToolCall {
    item_id:      String,
    name:         String,
    output_index: usize,
    arguments:    String,
}

impl ResponsesSink {
    pub fn new(model: String) -> Self {
        let mut sink = Self {
            response_id: format!("resp_{}", nanoid::nanoid!(24)),
            model,
            created_at: chrono::Utc::now().timestamp(),
            sequence: 0,
            started: false,
            reasoning: None,
            reasoning_text: String::new(),
            message: None,
            text: String::new(),
            tools: std::collections::BTreeMap::new(),
            next_output: 0,
            usage: None,
            completed: false,
            pending: VecDeque::new(),
        };
        sink.start_response();
        sink
    }

    fn alloc_output(&mut self) -> usize {
        let index = self.next_output;
        self.next_output += 1;
        index
    }

    fn push_event(&mut self, event_type: &str, mut data: Value) {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("type".to_string(), Value::String(event_type.to_string()));
            obj.insert("sequence_number".to_string(), json!(self.sequence));
        }
        self.sequence += 1;
        self.pending.push_back(event_bytes(event_type, &data));
    }

    fn start_response(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.push_event(
            "response.created",
            json!({
                "response": {
                    "id": self.response_id,
                    "object": "response",
                    "created_at": self.created_at,
                    "completed_at": null,
                    "status": "in_progress",
                    "model": self.model,
                    "output": [],
                    "error": null,
                    "incomplete_details": null,
                    "usage": null
                }
            }),
        );
    }

    fn start_message(&mut self) {
        if self.message.is_some() {
            return;
        }
        let item_id = format!("msg_{}", nanoid::nanoid!(24));
        let output_index = self.alloc_output();
        self.message = Some(OutputItem {
            item_id: item_id.clone(),
            output_index,
        });
        self.push_event(
            "response.output_item.added",
            json!({
                "output_index": output_index,
                "item": {
                    "id": item_id,
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": []
                }
            }),
        );
        self.push_event(
            "response.content_part.added",
            json!({
                "item_id": item_id,
                "output_index": output_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": "", "annotations": [], "logprobs": [] }
            }),
        );
    }

    fn start_reasoning(&mut self) {
        if self.reasoning.is_some() {
            return;
        }
        let item_id = format!("rs_{}", nanoid::nanoid!(24));
        let output_index = self.alloc_output();
        self.reasoning = Some(OutputItem {
            item_id: item_id.clone(),
            output_index,
        });
        self.push_event(
            "response.output_item.added",
            json!({
                "output_index": output_index,
                "item": { "id": item_id, "type": "reasoning", "status": "in_progress", "summary": [] }
            }),
        );
        self.push_event(
            "response.reasoning_summary_part.added",
            json!({
                "item_id": item_id,
                "output_index": output_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": "" }
            }),
        );
    }

    fn complete(&mut self, reason: &StopReason) {
        if self.completed {
            return;
        }
        self.completed = true;
        // 终态 output 按 output_index 排序，与增量事件的顺序一致
        let mut items: Vec<(usize, Value)> = Vec::new();

        if let Some(OutputItem { item_id, output_index }) = self.reasoning.clone() {
            let item = json!({
                "id": item_id,
                "type": "reasoning",
                "status": "completed",
                "summary": [{
                    "type": "summary_text",
                    "text": self.reasoning_text
                }]
            });
            self.push_event(
                "response.output_item.done",
                json!({ "output_index": output_index, "item": item }),
            );
            items.push((output_index, item));
        }

        if let Some(OutputItem { item_id, output_index }) = self.message.clone() {
            self.push_event(
                "response.output_text.done",
                json!({
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "text": self.text,
                    "logprobs": []
                }),
            );
            self.push_event(
                "response.content_part.done",
                json!({
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": self.text, "annotations": [], "logprobs": [] }
                }),
            );
            let message = json!({
                "id": item_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": self.text, "annotations": [], "logprobs": [] }]
            });
            self.push_event(
                "response.output_item.done",
                json!({ "output_index": output_index, "item": message }),
            );
            items.push((output_index, message));
        }

        let tool_keys: Vec<String> = self.tools.keys().cloned().collect();
        for call_id in tool_keys {
            let PendingToolCall {
                item_id,
                name,
                output_index,
                arguments,
            } = self.tools.remove(&call_id).expect("key exists");
            self.push_event(
                "response.function_call_arguments.done",
                json!({
                    "item_id": item_id,
                    "output_index": output_index,
                    "arguments": arguments
                }),
            );
            let item = json!({
                "id": item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": arguments
            });
            self.push_event(
                "response.output_item.done",
                json!({ "output_index": output_index, "item": item }),
            );
            items.push((output_index, item));
        }

        items.sort_by_key(|(index, _)| *index);
        let output: Vec<Value> = items.into_iter().map(|(_, item)| item).collect();
        let usage = self.usage.clone().unwrap_or_default();
        let (status, incomplete_details) = responses_status(reason);
        self.push_event(
            if status == "incomplete" {
                "response.incomplete"
            } else {
                "response.completed"
            },
            json!({
                "response": {
                    "id": self.response_id,
                    "object": "response",
                    "created_at": self.created_at,
                    "completed_at": chrono::Utc::now().timestamp(),
                    "status": status,
                    "model": self.model,
                    "output": output,
                    "error": null,
                    "incomplete_details": incomplete_details,
                    "usage": responses_usage(&usage)
                }
            }),
        );
    }
}

impl EventSink for ResponsesSink {
    fn handle(&mut self, event: &StreamEvent) -> Vec<Bytes> {
        match event {
            StreamEvent::TextDelta { text } => {
                self.start_message();
                let Some(OutputItem { item_id, output_index }) = self.message.clone() else {
                    return Vec::new();
                };
                self.text.push_str(text);
                self.push_event(
                    "response.output_text.delta",
                    json!({
                        "item_id": item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "delta": text,
                        "logprobs": []
                    }),
                );
            }
            StreamEvent::ThinkingDelta { text } => {
                self.start_reasoning();
                let Some(OutputItem { item_id, output_index }) = self.reasoning.clone() else {
                    return Vec::new();
                };
                self.reasoning_text.push_str(text);
                self.push_event(
                    "response.reasoning_summary_text.delta",
                    json!({
                        "item_id": item_id,
                        "output_index": output_index,
                        "summary_index": 0,
                        "delta": text
                    }),
                );
            }
            StreamEvent::ToolCallStart {
                call_id,
                name,
                arguments,
            } => {
                let item_id = format!("fc_{}", nanoid::nanoid!(20));
                let output_index = self.alloc_output();
                self.tools.insert(
                    call_id.clone(),
                    PendingToolCall {
                        item_id: item_id.clone(),
                        name: name.clone(),
                        output_index,
                        arguments: arguments.clone(),
                    },
                );
                self.push_event(
                    "response.output_item.added",
                    json!({
                        "output_index": output_index,
                        "item": {
                            "id": item_id,
                            "type": "function_call",
                            "status": "in_progress",
                            "call_id": call_id,
                            "name": name,
                            "arguments": ""
                        }
                    }),
                );
                if !arguments.is_empty() {
                    self.push_event(
                        "response.function_call_arguments.delta",
                        json!({
                            "item_id": item_id,
                            "output_index": output_index,
                            "delta": arguments
                        }),
                    );
                }
            }
            StreamEvent::ToolCallDelta { call_id, arguments } => {
                let Some(PendingToolCall {
                    item_id,
                    output_index,
                    arguments: accumulated,
                    ..
                }) = self.tools.get_mut(call_id)
                else {
                    return Vec::new();
                };
                accumulated.push_str(arguments);
                let item_id = item_id.clone();
                let output_index = *output_index;
                self.push_event(
                    "response.function_call_arguments.delta",
                    json!({
                        "item_id": item_id,
                        "output_index": output_index,
                        "delta": arguments
                    }),
                );
            }
            StreamEvent::Usage(usage) => {
                merge_usage(&mut self.usage, usage.clone());
            }
            StreamEvent::Error { message } => {
                self.completed = true;
                let mut data = error_envelope(ApiFormat::OpenaiResponse, message);
                if let Some(obj) = data.as_object_mut() {
                    obj.insert("type".to_string(), Value::String("response.failed".to_string()));
                    obj.insert("sequence_number".to_string(), json!(self.sequence));
                }
                self.sequence += 1;
                self.pending
                    .push_back(event_bytes("response.failed", &data));
            }
            StreamEvent::Finish { reason } => self.complete(reason),
        }
        self.pending.drain(..).collect()
    }

    fn finish(&mut self) -> Vec<Bytes> {
        self.complete(&StopReason::Stop);
        self.pending.drain(..).collect()
    }
}

/// Anthropic Messages SSE 流。
pub struct AnthropicSink {
    model:            String,
    message_id:       String,
    text_started:     bool,
    text_block:       usize,
    thinking_started: bool,
    thinking_block:   usize,
    next_block_index: usize,
    tools:            HashMap<String, usize>,
    usage:            Option<Usage>,
    completed:        bool,
    pending:          VecDeque<Bytes>,
}

impl AnthropicSink {
    pub fn new(model: String) -> Self {
        let mut sink = Self {
            model,
            message_id: format!("msg_{}", nanoid::nanoid!(24)),
            text_started: false,
            text_block: 0,
            thinking_started: false,
            thinking_block: 0,
            next_block_index: 0,
            tools: HashMap::new(),
            usage: None,
            completed: false,
            pending: VecDeque::new(),
        };
        sink.push(
            "message_start",
            json!({
                "message": {
                    "id": sink.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": sink.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                }
            }),
        );
        sink
    }

    fn push(&mut self, event: &str, mut data: Value) {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("type".to_string(), Value::String(event.to_string()));
        }
        self.pending.push_back(event_bytes(event, &data));
    }

    fn start_block(&mut self, index: usize, block: Value) {
        self.push("content_block_start", json!({ "index": index, "content_block": block }));
    }

    fn complete(&mut self, reason: &StopReason) {
        if self.completed {
            return;
        }
        self.completed = true;
        if self.thinking_started {
            self.push(
                "content_block_delta",
                json!({
                    "index": self.thinking_block,
                    "delta": { "type": "signature_delta", "signature": "" }
                }),
            );
            self.push("content_block_stop", json!({ "index": self.thinking_block }));
        }
        if self.text_started {
            self.push("content_block_stop", json!({ "index": self.text_block }));
        }
        let mut tool_indexes: Vec<usize> = self.tools.values().copied().collect();
        tool_indexes.sort_unstable();
        for index in tool_indexes {
            self.push("content_block_stop", json!({ "index": index }));
        }
        let stop_reason = anthropic_stop_reason(reason, !self.tools.is_empty());
        // message_start 里的 input_tokens 恒为 0（构造时尚未知）；在此补发真实值
        let usage = self.usage.clone().unwrap_or_default();
        self.push(
            "message_delta",
            json!({
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": anthropic_usage(&usage)
            }),
        );
        self.push("message_stop", json!({}));
    }
}

impl EventSink for AnthropicSink {
    fn handle(&mut self, event: &StreamEvent) -> Vec<Bytes> {
        match event {
            StreamEvent::TextDelta { text } => {
                if !self.text_started {
                    self.text_started = true;
                    // 与思考块共用递增分配器：思考先行时它已占 0，正文不能硬编码 0
                    self.text_block = self.next_block_index;
                    self.next_block_index += 1;
                    self.start_block(self.text_block, json!({ "type": "text", "text": "" }));
                }
                self.push(
                    "content_block_delta",
                    json!({ "index": self.text_block, "delta": { "type": "text_delta", "text": text } }),
                );
            }
            StreamEvent::ThinkingDelta { text } => {
                if !self.thinking_started {
                    self.thinking_started = true;
                    self.thinking_block = self.next_block_index;
                    self.next_block_index += 1;
                    // signature 是 Anthropic thinking 块的必填字段；转译场景没有真实签名，
                    // 补空串以满足客户端的类型校验（伪造签名会在回放时被上游拒绝）
                    self.start_block(
                        self.thinking_block,
                        json!({ "type": "thinking", "thinking": "", "signature": "" }),
                    );
                }
                self.push(
                    "content_block_delta",
                    json!({
                        "index": self.thinking_block,
                        "delta": { "type": "thinking_delta", "thinking": text }
                    }),
                );
            }
            StreamEvent::ToolCallStart {
                call_id,
                name,
                arguments,
            } => {
                let block_index = self.next_block_index;
                self.next_block_index += 1;
                self.tools.insert(call_id.clone(), block_index);
                self.start_block(
                    block_index,
                    json!({ "type": "tool_use", "id": call_id, "name": name, "input": {} }),
                );
                // 首个 delta 可能已带参数（有上游一次性下发完整 arguments）；
                // 丢弃它们会让客户端拿到 input 为空的 tool_use，按空参数调用工具
                if !arguments.is_empty() {
                    self.push(
                        "content_block_delta",
                        json!({
                            "index": block_index,
                            "delta": { "type": "input_json_delta", "partial_json": arguments }
                        }),
                    );
                }
            }
            StreamEvent::ToolCallDelta { call_id, arguments } => {
                let Some(block_index) = self.tools.get(call_id).copied() else {
                    return self.pending.drain(..).collect();
                };
                self.push(
                    "content_block_delta",
                    json!({
                        "index": block_index,
                        "delta": { "type": "input_json_delta", "partial_json": arguments }
                    }),
                );
            }
            StreamEvent::Usage(usage) => {
                merge_usage(&mut self.usage, usage.clone());
            }
            StreamEvent::Error { message } => {
                self.completed = true;
                // Anthropic 的流内错误事件形状：event: error + type: "error" 信封
                self.pending.push_back(event_bytes(
                    "error",
                    &error_envelope(ApiFormat::AnthropicMessage, message),
                ));
            }
            StreamEvent::Finish { reason } => self.complete(reason),
        }
        self.pending.drain(..).collect()
    }

    fn finish(&mut self) -> Vec<Bytes> {
        self.complete(&StopReason::Stop);
        self.pending.drain(..).collect()
    }
}

/// Google Gemini streamGenerateContent SSE 流。
pub struct GeminiSink {
    model:     String,
    tools:     HashMap<String, PendingGeminiCall>,
    usage:     Option<Usage>,
    completed: bool,
    pending:   VecDeque<Bytes>,
}

/// Gemini sink 中累积中的函数调用（finish 时一次性补发完整调用）。
struct PendingGeminiCall {
    name:      String,
    arguments: String,
}

impl GeminiSink {
    pub fn new(model: String) -> Self {
        Self {
            model,
            tools: HashMap::new(),
            usage: None,
            completed: false,
            pending: VecDeque::new(),
        }
    }

    fn push(&mut self, mut data: Value) {
        data["modelVersion"] = Value::String(self.model.clone());
        self.pending.push_back(chunk_bytes(&data));
    }

    /// 终态 chunk：补发累积的完整 functionCall 与真实用量；completed 防止重复发射。
    fn emit_final(&mut self, finish_reason: &str) {
        if self.completed {
            return;
        }
        self.completed = true;
        let mut parts = Vec::new();
        for call in self.tools.values() {
            let parsed: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
            parts.push(json!({ "functionCall": { "name": call.name, "args": parsed } }));
        }
        let usage = self.usage.clone().unwrap_or_default();
        self.push(json!({
            "candidates": [{
                "content": { "role": "model", "parts": parts },
                "finishReason": finish_reason
            }],
            "usageMetadata": gemini_usage(&usage)
        }));
    }
}

impl EventSink for GeminiSink {
    fn handle(&mut self, event: &StreamEvent) -> Vec<Bytes> {
        match event {
            StreamEvent::TextDelta { text } => {
                self.push(json!({
                    "candidates": [{ "content": { "role": "model", "parts": [{ "text": text }] }, "finishReason": null }]
                }));
            }
            StreamEvent::ToolCallStart {
                call_id,
                name,
                arguments,
            } => {
                self.tools.insert(
                    call_id.clone(),
                    PendingGeminiCall {
                        name:      name.clone(),
                        arguments: arguments.clone(),
                    },
                );
            }
            StreamEvent::ToolCallDelta { call_id, arguments } => {
                if let Some(call) = self.tools.get_mut(call_id) {
                    call.arguments.push_str(arguments);
                }
            }
            StreamEvent::Usage(usage) => {
                merge_usage(&mut self.usage, usage.clone());
            }
            StreamEvent::Finish { reason } => self.emit_final(gemini_finish_reason(reason)),
            StreamEvent::Error { message } => {
                self.completed = true;
                self.push(error_envelope(ApiFormat::GeminiGenerate, message));
            }
            // ThinkingDelta 在 Gemini 请求目标无对应形态，丢弃（与既有行为一致）
            StreamEvent::ThinkingDelta { .. } => {}
        }
        self.pending.drain(..).collect()
    }

    fn finish(&mut self) -> Vec<Bytes> {
        // 上游未发终止事件（如直接断流）时按正常结束补发终态 chunk
        self.emit_final("STOP");
        self.pending.drain(..).collect()
    }
}

// ---------- 泵 ----------

/// 将上游 SSE 字节流解析为 IR 事件再发射为客户端格式字节流。
pub struct StreamTranslate<S, P: EventParser, K: EventSink> {
    inner:    S,
    sse:      SseBuffer,
    parser:   P,
    sink:     K,
    pending:  VecDeque<Bytes>,
    finished: bool,
    /// 上游流内错误消息；帧已交付后在下一轮 poll 终止传输
    failed:   Option<String>,
}

impl<S, P: EventParser, K: EventSink> StreamTranslate<S, P, K> {
    pub fn new(inner: S, parser: P, sink: K) -> Self {
        Self {
            inner,
            sse: SseBuffer::default(),
            parser,
            sink,
            pending: VecDeque::new(),
            finished: false,
            failed: None,
        }
    }
}

impl<S, P, K> Stream for StreamTranslate<S, P, K>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
    P: EventParser + Unpin,
    K: EventSink + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        loop {
            if let Some(chunk) = this.pending.pop_front() {
                return Poll::Ready(Some(Ok(chunk)));
            }
            // 流内错误帧已交付，此时才终止传输：先让客户端读到错误再断开
            if let Some(message) = this.failed.take() {
                this.finished = true;
                return Poll::Ready(Some(Err(std::io::Error::other(message))));
            }
            if this.finished {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.sse.feed(&bytes);
                    while let Some(event) = this.sse.next_event() {
                        for ev in this.parser.parse(&event) {
                            if let StreamEvent::Error { message } = &ev {
                                this.failed = Some(message.clone());
                            }
                            this.pending.extend(this.sink.handle(&ev));
                        }
                        if this.failed.is_some() {
                            // 上游已报错：其后的事件一律不再处理
                            break;
                        }
                    }
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => {
                    this.finished = true;
                    // 上游在最后一个事件后直接断流（缺空行）时，残留字节仍是事件，
                    // 通常是承载 finish 与 usage 的那一个，不能当作空。
                    if let Some(tail) = this.sse.flush() {
                        for ev in this.parser.parse(&tail) {
                            this.pending.extend(this.sink.handle(&ev));
                        }
                    }
                    if !this.parser.finished() {
                        let tail = this.sink.handle(&StreamEvent::Finish {
                            reason: StopReason::Stop,
                        });
                        this.pending.extend(tail);
                    }
                    let tail = this.sink.finish();
                    this.pending.extend(tail);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// 按上游格式构造 parser，按客户端格式构造 sink。
pub fn translate<S>(
    inner: S,
    upstream: crate::relay::protocol::ApiFormat,
    client: crate::relay::protocol::ApiFormat,
    model: String,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static,
{
    use crate::relay::protocol::ApiFormat;
    match (upstream, client) {
        (ApiFormat::OpenaiCompletion, _) => {
            let sink = match client {
                ApiFormat::OpenaiResponse => DynSink::Responses(ResponsesSink::new(model)),
                ApiFormat::AnthropicMessage => DynSink::Anthropic(AnthropicSink::new(model)),
                ApiFormat::GeminiGenerate => DynSink::Gemini(GeminiSink::new(model)),
                _ => unreachable!("同格式直通不进入翻译泵"),
            };
            Box::pin(StreamTranslate::new(inner, ChatEventParser::default(), sink))
        }
        (ApiFormat::OpenaiResponse, _) => {
            let sink = match client {
                ApiFormat::OpenaiCompletion => DynSink::Chat(ChatChunkSink::new(model)),
                ApiFormat::AnthropicMessage => DynSink::Anthropic(AnthropicSink::new(model)),
                ApiFormat::GeminiGenerate => DynSink::Gemini(GeminiSink::new(model)),
                _ => unreachable!("同格式直通不进入翻译泵"),
            };
            Box::pin(StreamTranslate::new(inner, ResponsesEventParser::default(), sink))
        }
        (ApiFormat::AnthropicMessage, _) => {
            let sink = match client {
                ApiFormat::OpenaiCompletion => DynSink::Chat(ChatChunkSink::new(model)),
                ApiFormat::OpenaiResponse => DynSink::Responses(ResponsesSink::new(model)),
                ApiFormat::GeminiGenerate => DynSink::Gemini(GeminiSink::new(model)),
                _ => unreachable!("同格式直通不进入翻译泵"),
            };
            Box::pin(StreamTranslate::new(inner, AnthropicEventParser::default(), sink))
        }
        (ApiFormat::GeminiGenerate, _) => {
            let sink = match client {
                ApiFormat::OpenaiCompletion => DynSink::Chat(ChatChunkSink::new(model)),
                ApiFormat::OpenaiResponse => DynSink::Responses(ResponsesSink::new(model)),
                ApiFormat::AnthropicMessage => DynSink::Anthropic(AnthropicSink::new(model)),
                _ => unreachable!("同格式直通不进入翻译泵"),
            };
            Box::pin(StreamTranslate::new(inner, GeminiEventParser::default(), sink))
        }
    }
}

enum DynSink {
    Chat(ChatChunkSink),
    Responses(ResponsesSink),
    Anthropic(AnthropicSink),
    Gemini(GeminiSink),
}

impl EventSink for DynSink {
    fn handle(&mut self, event: &StreamEvent) -> Vec<Bytes> {
        match self {
            DynSink::Chat(sink) => sink.handle(event),
            DynSink::Responses(sink) => sink.handle(event),
            DynSink::Anthropic(sink) => sink.handle(event),
            DynSink::Gemini(sink) => sink.handle(event),
        }
    }

    fn finish(&mut self) -> Vec<Bytes> {
        match self {
            DynSink::Chat(sink) => sink.finish(),
            DynSink::Responses(sink) => sink.finish(),
            DynSink::Anthropic(sink) => sink.finish(),
            DynSink::Gemini(sink) => sink.finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};

    use super::*;
    use crate::relay::protocol::ApiFormat;

    /// 跨格式翻译整流并聚合输出（同格式直通不走泵，不在此覆盖）。
    async fn run(upstream: ApiFormat, client: ApiFormat, chunks: Vec<impl AsRef<str>>) -> String {
        let chunks: Vec<String> = chunks.into_iter().map(|c| c.as_ref().to_owned()).collect();
        let stream = stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok::<Bytes, std::io::Error>(Bytes::from(c))),
        );
        translate(stream, upstream, client, "m".to_string())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(String::new(), |mut out, chunk| {
                out.push_str(&String::from_utf8_lossy(&chunk));
                out
            })
    }

    /// 上游最后一个事件没有以空行收尾（EOF 截断）时，不得丢弃该事件里的内容。
    #[tokio::test]
    async fn unterminated_trailing_event_is_not_dropped() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"tail\"}}]}"],
        )
        .await;
        assert!(out.contains("tail"));
    }

    /// 跨格式翻译整流：返回（已发出的字节，传输错误消息）。
    /// 与 `run` 的差别只在于不 panic——流内错误必须以 Err 终止传输。
    async fn run_checked(
        upstream: ApiFormat,
        client: ApiFormat,
        chunks: Vec<impl AsRef<str>>,
    ) -> (String, Option<String>) {
        let chunks: Vec<String> = chunks.into_iter().map(|c| c.as_ref().to_owned()).collect();
        let stream = stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok::<Bytes, std::io::Error>(Bytes::from(c))),
        );
        let mut out = String::new();
        let mut error = None;
        for item in translate(stream, upstream, client, "m".to_string())
            .collect::<Vec<_>>()
            .await
        {
            match item {
                Ok(chunk) => out.push_str(&String::from_utf8_lossy(&chunk)),
                Err(e) => error = Some(e.to_string()),
            }
        }
        (out, error)
    }

    #[tokio::test]
    async fn gemini_text_to_chat() {
        let out = run(
            ApiFormat::GeminiGenerate,
            ApiFormat::OpenaiCompletion,
            vec![
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
                "data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("chat.completion.chunk"));
        assert!(out.contains("Hello"));
        assert!(out.contains("data: [DONE]"));
    }

    #[tokio::test]
    async fn anthropic_text_to_chat() {
        let out = run(
            ApiFormat::AnthropicMessage,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        )
        .await;
        assert!(out.contains("Hello"));
        assert!(out.contains("data: [DONE]"));
    }

    #[tokio::test]
    async fn anthropic_thinking_to_chat_reasoning_content() {
        let out = run(
            ApiFormat::AnthropicMessage,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"thinking hard\"}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"reasoning_content\":\"thinking hard\""));
    }

    #[tokio::test]
    async fn anthropic_tool_calls_to_chat() {
        let out = run(
            ApiFormat::AnthropicMessage,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_test123\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}\n\n",
            ],
        )
        .await;
        assert!(out.contains("call_test123"));
        assert!(out.contains("get_weather"));
        assert!(out.contains("Paris"));
    }

    #[tokio::test]
    async fn anthropic_tool_index_remaps_to_sequential() {
        // thinking/text 块在前时 tool_use 块索引非 0 起始；转 Chat 后必须归一为 0、1
        let start = |index: u64, id: &str| {
            format!(
                "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":{index},\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"calc\",\"input\":{{}}}}}}\n\n"
            )
        };
        let delta = |index: u64, args: &str| {
            format!(
                "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":{index},\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{args}\"}}}}\n\n"
            )
        };
        let out = run(
            ApiFormat::AnthropicMessage,
            ApiFormat::OpenaiCompletion,
            vec![&start(2, "call_a"), &start(3, "call_b"), &delta(2, "1"), &delta(3, "2")],
        )
        .await;
        assert!(out.contains("\"index\":0"));
        assert!(out.contains("\"index\":1"));
    }

    #[tokio::test]
    async fn anthropic_message_start_usage_merged_into_finish_chunk() {
        let out = run(
            ApiFormat::AnthropicMessage,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":42,\"output_tokens\":0}}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"prompt_tokens\":42"));
        assert!(out.contains("\"completion_tokens\":9"));
    }

    #[tokio::test]
    async fn responses_text_to_chat() {
        let out = run(
            ApiFormat::OpenaiResponse,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n",
                "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":9,\"output_tokens\":3,\"total_tokens\":12}}}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"content\":\"你好\""));
        assert!(out.contains("\"finish_reason\":\"stop\""));
        assert!(out.contains("\"prompt_tokens\":9"));
    }

    #[tokio::test]
    async fn responses_tool_indexes_sequential() {
        // 两个并行 function_call 的 output_index 为 1、2（文本占 0），
        // 转 Chat 后 tool_calls.index 必须归一为 0、1
        let out = run(
            ApiFormat::OpenaiResponse,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"fc_a\",\"type\":\"function_call\",\"call_id\":\"call_a\",\"name\":\"a\",\"arguments\":\"{}\"}}\n\n",
                "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_a\",\"delta\":\"{\\\"x\\\":1}\"}\n\n",
                "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":2,\"item\":{\"id\":\"fc_b\",\"type\":\"function_call\",\"call_id\":\"call_b\",\"name\":\"b\",\"arguments\":\"{}\"}}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"index\":0"));
        assert!(out.contains("\"index\":1"));
    }

    /// 上游在首个 tool_call delta 里就带上了参数（部分上游一次性下发完整 JSON）：
    /// 这些参数必须送达 Anthropic 客户端，否则 tool_use 的 input 为空、工具被错误调用。
    #[tokio::test]
    async fn chat_tool_arguments_in_first_delta_reach_anthropic_client() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"calc\",\"arguments\":\"{\\\"x\\\":1}\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("input_json_delta"));
        assert!(out.contains("partial_json"));
        // 参数原文必须完整出现（SSE JSON 里转义为 {\"x\":1}）
        assert!(out.contains("{\\\"x\\\":1}"), "首个 delta 的参数必须原样送达: {out}");
    }

    #[tokio::test]
    async fn chat_to_responses_events() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::OpenaiResponse,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            ],
        )
        .await;
        assert!(out.contains("event: response.created"));
        assert!(out.contains("event: response.output_text.delta"));
        assert!(out.contains("\"delta\":\"Hello\""));
        assert_eq!(out.matches("event: response.completed").count(), 1);
    }

    /// 截断的上游流在 Responses 客户端侧必须以 `response.incomplete` 收尾，
    /// 不能报 `response.completed` 却带着 `status: incomplete`。
    /// 转译出的 Anthropic thinking 块必须字段完整：起始块带 signature，
    /// 关闭前补 signature_delta，便于客户端按类型解析并在下一轮回传。
    #[tokio::test]
    async fn chat_reasoning_to_anthropic_thinking_block_has_signature() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想一下\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"答\"},\"finish_reason\":\"stop\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"type\":\"thinking\""));
        assert!(out.contains("\"signature\":\"\""));
        assert!(out.contains("signature_delta"));
        assert!(out.contains("\"thinking\":\"想一下\""));
    }

    /// 思考块先行时正文块不得复用 index 0：两个 content_block_start 撞同一索引、
    /// 且该索引上收到两次 content_block_stop，客户端解析会串块或直接报错。
    #[tokio::test]
    async fn anthropic_thinking_first_does_not_collide_with_text_block_index() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"答\"},\"finish_reason\":\"stop\"}]}\n\n",
            ],
        )
        .await;
        // 思考块 0，正文块 1：块起始索引互不相同
        assert!(
            out.contains("\"content_block\":{\"signature\":\"\",\"thinking\":\"\",\"type\":\"thinking\"},\"index\":0"),
            "{out}"
        );
        assert!(
            out.contains("\"content_block\":{\"text\":\"\",\"type\":\"text\"},\"index\":1"),
            "{out}"
        );
        // 每个索引恰好一次 content_block_stop
        for index in [0, 1] {
            assert_eq!(
                out.matches(&format!("\"index\":{index},\"type\":\"content_block_stop\""))
                    .count(),
                1,
                "index {index} 的 content_block_stop 次数不为 1: {out}"
            );
        }
    }

    #[tokio::test]
    async fn chat_truncated_to_responses_incomplete_event() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::OpenaiResponse,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"part\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("event: response.incomplete"));
        assert!(out.contains("\"reason\":\"max_output_tokens\""));
        assert_eq!(out.matches("event: response.completed").count(), 0);
    }

    /// 上游 Responses 流以 `response.incomplete` 收尾时，截断原因必须传到客户端，
    /// 不能被当作未知事件丢弃、再在 EOF 补成正常结束。
    #[tokio::test]
    async fn responses_incomplete_event_reaches_chat_client() {
        let out = run(
            ApiFormat::OpenaiResponse,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"part\"}\n\n",
                "event: response.incomplete\ndata: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":2,\"total_tokens\":3}}}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"finish_reason\":\"length\""), "{out}");
        assert!(out.contains("\"completion_tokens\":2"), "用量不得丢失: {out}");
        assert_eq!(out.matches("[DONE]").count(), 1, "终态只能发一次: {out}");

        // 内容过滤同属未完成，必须映射为 content_filter 而不是截断
        let filtered = run(
            ApiFormat::OpenaiResponse,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: response.incomplete\ndata: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\",\"incomplete_details\":{\"reason\":\"content_filter\"},\"output\":[]}}\n\n",
            ],
        )
        .await;
        assert!(filtered.contains("\"finish_reason\":\"content_filter\""), "{filtered}");
    }

    #[tokio::test]
    async fn chat_to_anthropic_events() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            ],
        )
        .await;
        assert!(out.contains("event: message_start"));
        assert!(out.contains("text_delta"));
        assert!(out.contains("Hello"));
        assert!(out.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn chat_usage_forwarded_to_anthropic_message_delta() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":7,\"total_tokens\":12}}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"input_tokens\":5"));
        assert!(out.contains("\"output_tokens\":7"));
    }

    #[tokio::test]
    async fn responses_to_anthropic_events() {
        let out = run(
            ApiFormat::OpenaiResponse,
            ApiFormat::AnthropicMessage,
            vec![
                "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello from responses\"}\n\n",
                "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n",
            ],
        )
        .await;
        assert!(out.contains("Hello from responses"));
        assert!(out.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn chat_text_and_tools_to_gemini() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::GeminiGenerate,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"calc\",\"arguments\":\"{\\\"x\\\":\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4,\"total_tokens\":7}}\n\ndata: [DONE]\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"functionCall\":{\"args\":{\"x\":1},\"name\":\"calc\"}"));
        assert!(out.contains("\"finishReason\":\"STOP\""));
        assert!(out.contains("\"promptTokenCount\":3"));
        // 终态 chunk 只出现一次
        assert_eq!(out.matches("\"finishReason\":\"STOP\"").count(), 1);
        assert_eq!(out.matches("\"functionCall\"").count(), 1);
    }

    #[tokio::test]
    async fn chat_length_maps_to_gemini_max_tokens() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::GeminiGenerate,
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"length\"}]}\n\n"],
        )
        .await;
        assert!(out.contains("\"finishReason\":\"MAX_TOKENS\""));
    }

    #[tokio::test]
    async fn chat_done_maps_to_gemini_stop() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::GeminiGenerate,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
                "data: [DONE]\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"finishReason\":\"STOP\""));
    }

    #[tokio::test]
    async fn gemini_tool_calls_across_chunks_get_distinct_indexes() {
        let call = |name: &str| {
            let event = json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "functionCall": { "name": name, "args": { "x": 1 } } }],
                        "role": "model"
                    }
                }]
            });
            format!("data: {}\n\n", event)
        };
        let out = run(
            ApiFormat::GeminiGenerate,
            ApiFormat::OpenaiCompletion,
            vec![&call("a"), &call("b")],
        )
        .await;
        assert!(out.contains("\"index\":0"));
        assert!(out.contains("\"index\":1"));
    }

    #[tokio::test]
    async fn gemini_tool_calls_same_chunk_sequential() {
        let event = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"a\",\"args\":{\"x\":1}}},{\"functionCall\":{\"name\":\"b\",\"args\":{\"y\":2}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";
        let out = run(ApiFormat::GeminiGenerate, ApiFormat::OpenaiCompletion, vec![event]).await;
        assert!(out.contains("\"index\":0"));
        assert!(out.contains("\"index\":1"));
        assert!(!out.contains("\"index\":2"));
    }

    /// 上游 200 但流中途发错误事件：必须向客户端发出协议错误事件并以 Err 终止传输，
    /// 绝不能补一个「正常结束」（客户端会把空结果当成模型没话说）。
    #[tokio::test]
    async fn chat_stream_error_reaches_anthropic_client_and_terminates() {
        let (out, error) = run_checked(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
                "data: {\"error\":{\"message\":\"upstream exploded\",\"type\":\"server_error\"}}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"never\"},\"finish_reason\":\"stop\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("partial"), "错误前的增量应已送达: {out}");
        assert!(out.contains("event: error"), "必须发出 Anthropic 错误事件: {out}");
        assert!(out.contains("upstream exploded"), "错误消息必须透出: {out}");
        assert!(!out.contains("never"), "错误之后的事件不得再处理: {out}");
        assert!(!out.contains("message_stop"), "错误流不得补正常结束: {out}");
        assert_eq!(error.as_deref(), Some("upstream exploded"));
    }

    /// 同一场景下 Chat 客户端拿到 OpenAI 信封，且不发 [DONE]。
    #[tokio::test]
    async fn chat_stream_error_reaches_chat_client_without_done() {
        let (out, error) = run_checked(
            ApiFormat::GeminiGenerate,
            ApiFormat::OpenaiCompletion,
            vec![
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}\n\n",
                "data: {\"error\":{\"code\":429,\"message\":\"quota exceeded\"}}\n\n",
            ],
        )
        .await;
        assert!(out.contains("hi"));
        assert!(out.contains("quota exceeded"));
        assert!(out.contains("\"error\""));
        assert!(!out.contains("[DONE]"), "错误流不得发 [DONE]: {out}");
        assert_eq!(error.as_deref(), Some("quota exceeded"));
    }

    /// Responses 客户端收到 response.failed 事件。
    #[tokio::test]
    async fn stream_error_reaches_responses_client_as_failed_event() {
        let (out, error) = run_checked(
            ApiFormat::OpenaiCompletion,
            ApiFormat::OpenaiResponse,
            vec!["data: {\"error\":{\"message\":\"boom\"}}\n\n"],
        )
        .await;
        assert!(out.contains("event: response.failed"), "{out}");
        assert!(out.contains("boom"));
        assert!(!out.contains("response.completed"));
        assert_eq!(error.as_deref(), Some("boom"));
    }

    /// 跨格式流式的缓存用量必须双向无损：上游 Anthropic 的缓存命中/写入经 Chat 客户端
    /// 出来仍是 OpenAI 的 cached_tokens，计费才拿得到缓存折扣。
    #[tokio::test]
    async fn anthropic_cache_usage_survives_to_chat_client() {
        let (out, error) = run_checked(
            ApiFormat::AnthropicMessage,
            ApiFormat::OpenaiCompletion,
            vec![
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"usage\":{\"input_tokens\":20,\"cache_read_input_tokens\":80,\"cache_creation_input_tokens\":10,\"output_tokens\":0}}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        )
        .await;
        assert_eq!(error, None);
        // 总输入 = 20 + 80 + 10（IR 口径），缓存明细单列
        assert!(out.contains("\"prompt_tokens\":110"), "{out}");
        assert!(out.contains("\"cached_tokens\":80"), "{out}");
        assert!(out.contains("\"completion_tokens\":5"), "{out}");
    }

    /// 反方向：上游 Chat 的 cached_tokens 转给 Anthropic 客户端时，input_tokens 必须
    /// 只含未命中缓存的部分，缓存量单列——否则客户端自行相加会重复计入。
    #[tokio::test]
    async fn chat_cache_usage_survives_to_anthropic_client() {
        let (out, error) = run_checked(
            ApiFormat::OpenaiCompletion,
            ApiFormat::AnthropicMessage,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}],\"usage\":{\"prompt_tokens\":110,\"completion_tokens\":5,\"total_tokens\":115,\"prompt_tokens_details\":{\"cached_tokens\":80}}}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            ],
        )
        .await;
        assert_eq!(error, None);
        assert!(out.contains("\"input_tokens\":30"), "input 只含未命中部分: {out}");
        assert!(out.contains("\"cache_read_input_tokens\":80"), "{out}");
        assert!(out.contains("\"output_tokens\":5"), "{out}");
    }

    /// Responses 终态对象必须与增量事件自洽：推理摘要不能是空的（增量里发过文本），
    /// 且 reasoning 项要出现在 output[] 里（缺项会让客户端回看时丢失思考链）。
    #[tokio::test]
    async fn chat_reasoning_to_responses_keeps_summary_in_final_object() {
        let out = run(
            ApiFormat::OpenaiCompletion,
            ApiFormat::OpenaiResponse,
            vec![
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先想一想\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"答案\"},\"finish_reason\":\"stop\"}]}\n\n",
            ],
        )
        .await;
        assert!(out.contains("\"type\":\"summary_text\""), "{out}");
        assert!(out.contains("\"text\":\"先想一想\""), "{out}");
        // reasoning 项进入终态 output，且排在 message 之前（output_index 顺序）
        let completed = out
            .lines()
            .find(|line| line.contains("\"object\":\"response\"") && line.contains("\"status\":\"completed\""))
            .expect("缺少 response.completed");
        let value: Value = serde_json::from_str(completed.trim_start_matches("data: ")).unwrap();
        let output = value
            .pointer("/response/output")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(output[0].get("type").unwrap(), "reasoning");
        assert_eq!(output[1].get("type").unwrap(), "message");
        assert_eq!(output[0].pointer("/summary/0/text").unwrap(), "先想一想");
    }
}
