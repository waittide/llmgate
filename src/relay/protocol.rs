use serde::Deserialize;
use serde_json::Value;

use crate::{
    model::ModelPrice,
    relay::{state::Usage, stream::SseChunkConverter},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenaiCompletion,
    OpenaiResponse,
    AnthropicMessage,
    GeminiGenerate,
}

#[derive(Debug, Default, Deserialize)]
pub struct ChatRequestMetadata {
    #[serde(default)]
    pub model:  String,
    #[serde(default)]
    pub stream: bool,
}

pub fn parse_request_metadata(body: &[u8]) -> Option<ChatRequestMetadata> {
    serde_json::from_slice(body).ok()
}

pub fn rewrite_request_body_in_json(body: &[u8], new_model: &str, param_override: Option<&str>) -> Vec<u8> {
    if let Ok(mut val) = serde_json::from_slice::<Value>(body)
        && let Some(obj) = val.as_object_mut()
    {
        obj.insert("model".to_string(), Value::String(new_model.to_string()));

        if let Some(override_str) = param_override
            && !override_str.trim().is_empty()
            && let Ok(override_val) = serde_json::from_str::<Value>(override_str)
            && let Some(override_obj) = override_val.as_object()
        {
            for (k, v) in override_obj {
                obj.insert(k.clone(), v.clone());
            }
        }

        flatten_text_content_arrays(obj);

        if let Ok(bytes) = serde_json::to_vec(&val) {
            return bytes;
        }
    }
    body.to_vec()
}

/// 将 messages 中恰好为 `{type, text}` 的纯文本 content 数组扁平化为字符串。
/// 其余块（多模态、cache_control、citations 等附加字段）保持数组原样，
/// 避免合并静默丢弃块级字段。
fn flatten_text_content_arrays(obj: &mut serde_json::Map<String, Value>) {
    let Some(messages) = obj.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for msg in messages {
        let Some(msg_obj) = msg.as_object_mut() else {
            continue;
        };
        let Some(parts) = msg_obj.get("content").and_then(Value::as_array) else {
            continue;
        };
        let mut text = String::new();
        let mut all_pure_text = true;
        for part in parts {
            let is_pure_text = matches!(
                part.get("type").and_then(Value::as_str),
                Some("text") | Some("input_text") | Some("output_text")
            ) && part
                .as_object()
                .is_some_and(|part_obj| part_obj.keys().all(|key| key == "type" || key == "text"));
            if !is_pure_text {
                all_pure_text = false;
                break;
            }
            text.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
        }
        if all_pure_text && !parts.is_empty() {
            msg_obj.insert("content".to_string(), Value::String(text));
        }
    }
}

/// 将 OpenAI Responses 非流式响应转换为 Chat Completion 响应结构。
pub fn convert_responses_result_to_chat_response(val: &Value, model_name: &str) -> Value {
    let mut content = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    if let Some(output) = val.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    // content 兼容字符串与 part 数组两种形态
                    match item.get("content") {
                        Some(Value::String(text)) => content.push_str(text),
                        Some(Value::Array(parts)) => {
                            for part in parts {
                                if part.get("type").and_then(Value::as_str) == Some("output_text")
                                    && let Some(text) = part.get("text").and_then(Value::as_str)
                                {
                                    content.push_str(text);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some("function_call") => {
                    tool_calls.push(serde_json::json!({
                        "id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                        "type": "function",
                        "function": {
                            "name": item.get("name").and_then(Value::as_str).unwrap_or(""),
                            "arguments": item.get("arguments").cloned()
                                .unwrap_or(Value::String("{}".to_string()))
                        }
                    }));
                }
                _ => {}
            }
        }
    }

    let message = if tool_calls.is_empty() {
        serde_json::json!({ "role": "assistant", "content": content })
    } else {
        serde_json::json!({
            "role": "assistant",
            "content": if content.is_empty() { Value::Null } else { Value::String(content) },
            "tool_calls": tool_calls
        })
    };
    // finish_reason 精细映射：截断状态优先于工具调用与正常停止
    let finish_reason = if val.get("status").and_then(Value::as_str) == Some("incomplete") {
        "length"
    } else if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    let id = val
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("chatcmpl-generated");
    serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason }],
        "usage": extract_usage_from_json(val).unwrap_or_default()
    })
}

/// Responses → Chat 有状态转换器：维护 item_id → Chat 工具序号的映射，
/// 保证 `tool_calls[].index` 严格从 0 递增（OpenAI SDK 流式解析的前提），
/// 而非直接透传可能跳变的 Responses `output_index`。
#[derive(Default)]
pub struct ResponsesToChatChunkConverter {
    tool_indexes: std::collections::HashMap<String, u64>,
    next_index:   u64,
}

impl SseChunkConverter for ResponsesToChatChunkConverter {
    fn convert_event(&mut self, event_str: &str, model_name: &str, created: i64, chatcmpl_id: &str) -> Option<String> {
        let (event_type, data_str) = crate::relay::parse_sse_event(event_str);
        if data_str.is_empty() {
            return None;
        }
        let val: Value = serde_json::from_str(&data_str).ok()?;

        let chunk_with = |delta: Value, finish: Value| -> String {
            let chunk = serde_json::json!({
                "id": chatcmpl_id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model_name,
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
            });
            format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap_or_default())
        };

        match event_type {
            "response.output_text.delta" => {
                let text = val.get("delta").and_then(Value::as_str).unwrap_or("");
                (!text.is_empty()).then(|| chunk_with(serde_json::json!({ "content": text }), Value::Null))
            }
            // function_call 项的元信息一次性下发（Responses 的 arguments 非分片起始）
            "response.output_item.added" => {
                if val.pointer("/item/type").and_then(Value::as_str) != Some("function_call") {
                    return None;
                }
                let item_id = val
                    .pointer("/item/id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let index = self.next_index;
                self.next_index += 1;
                self.tool_indexes.insert(item_id.to_string(), index);
                let call_id = val
                    .pointer("/item/call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let name = val
                    .pointer("/item/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let arguments = val
                    .pointer("/item/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                (!name.is_empty()).then(|| {
                    chunk_with(
                        serde_json::json!({ "tool_calls": [{
                            "index": index, "id": call_id, "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        }]}),
                        Value::Null,
                    )
                })
            }
            "response.function_call_arguments.delta" => {
                let delta = val.get("delta").and_then(Value::as_str).unwrap_or("");
                let item_id = val.get("item_id").and_then(Value::as_str).unwrap_or("");
                let index = self.tool_indexes.get(item_id).copied().unwrap_or(0);
                (!delta.is_empty()).then(|| {
                    chunk_with(
                        serde_json::json!({ "tool_calls": [{
                            "index": index, "function": { "arguments": delta }
                        }]}),
                        Value::Null,
                    )
                })
            }
            "response.completed" => {
                let has_function_call = val
                    .pointer("/output")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
                    })
                    .unwrap_or(false);
                let finish = if has_function_call { "tool_calls" } else { "stop" };
                let mut chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": finish }]
                });
                // 保留真实 usage，让 StreamFinalizer 捕获
                if let Some(usage) = extract_usage_from_json(&val) {
                    chunk["usage"] = serde_json::json!({
                        "prompt_tokens": usage.prompt_tokens,
                        "completion_tokens": usage.completion_tokens,
                        "total_tokens": usage.total_tokens
                    });
                }
                Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ))
            }
            _ => None,
        }
    }
}

/// 将非流式 Chat Completion 响应转换为 OpenAI Responses 响应结构。
pub fn convert_openai_chat_to_responses_response(val: &Value, model_name: &str) -> Value {
    let choice = val.pointer("/choices/0");
    let message = choice.and_then(|choice| choice.get("message"));
    let text = message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut output = Vec::new();
    // reasoning_content（思考链）转为 reasoning 输出项，保留给支持该形态的客户端
    if let Some(reasoning) = message
        .and_then(|message| message.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
    {
        output.push(serde_json::json!({
            "id": format!("rs_{}", nanoid::nanoid!(24)),
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": reasoning }]
        }));
    }
    if !text.is_empty() {
        output.push(serde_json::json!({
            "id": format!("msg_{}", nanoid::nanoid!(24)),
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text, "annotations": [], "logprobs": [] }]
        }));
    }
    if let Some(tool_calls) = message
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for call in tool_calls {
            output.push(serde_json::json!({
                "id": format!("fc_{}", nanoid::nanoid!(20)),
                "type": "function_call",
                "status": "completed",
                "call_id": call.get("id").cloned().unwrap_or(Value::String(format!("call_{}", nanoid::nanoid!(20)))),
                "name": call.pointer("/function/name").cloned().unwrap_or(Value::String(String::new())),
                "arguments": call.pointer("/function/arguments").cloned().unwrap_or(Value::String("{}".to_string()))
            }));
        }
    }

    let input_tokens = val
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = val
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let total_tokens = val
        .pointer("/usage/total_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(input_tokens + output_tokens);
    let truncated = choice
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .is_some_and(|reason| reason == "length");
    let (status, incomplete_details) = if truncated {
        ("incomplete", serde_json::json!({ "reason": "max_output_tokens" }))
    } else {
        ("completed", Value::Null)
    };
    serde_json::json!({
        "id": format!("resp_{}", nanoid::nanoid!(24)),
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "completed_at": chrono::Utc::now().timestamp(),
        "status": status,
        "model": model_name,
        "output": output,
        "error": null,
        "incomplete_details": incomplete_details,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "total_tokens": total_tokens
        }
    })
}

fn anthropic_stop_reason(reason: Option<&str>) -> &str {
    match reason {
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        _ => "end_turn",
    }
}

/// 将非流式 Chat Completion 响应转换为 Anthropic Message 响应结构。
pub fn convert_openai_chat_to_anthropic_response(val: &Value, model_name: &str) -> Value {
    let choice = val.pointer("/choices/0");
    let message = choice.and_then(|choice| choice.get("message"));
    let mut content = Vec::new();
    // 思考链（reasoning_content）转为 Anthropic thinking 块；其余场景保持原语义
    if let Some(reasoning) = message
        .and_then(|message| message.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
    {
        content.push(serde_json::json!({ "type": "thinking", "thinking": reasoning }));
    }
    if let Some(text) = message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        content.push(serde_json::json!({ "type": "text", "text": text }));
    }
    if let Some(tool_calls) = message
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for call in tool_calls {
            let arguments = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            content.push(serde_json::json!({
                "type": "tool_use",
                "id": call.get("id").cloned().unwrap_or(Value::String(format!("toolu_{}", nanoid::nanoid!(20)))),
                "name": call.pointer("/function/name").cloned().unwrap_or(Value::String(String::new())),
                "input": arguments
            }));
        }
    }
    // Anthropic SDK 要求助手 content 至少含一个块；全空时补空文本块兜底
    if content.is_empty() {
        content.push(serde_json::json!({ "type": "text", "text": "" }));
    }
    let input_tokens = val
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = val
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    serde_json::json!({
        "id": format!("msg_{}", nanoid::nanoid!(24)),
        "type": "message",
        "role": "assistant",
        "model": model_name,
        "content": content,
        "stop_reason": anthropic_stop_reason(choice.and_then(|choice| choice.get("finish_reason")).and_then(Value::as_str)),
        "stop_sequence": null,
        "usage": { "input_tokens": input_tokens, "output_tokens": output_tokens }
    })
}

/// 将非流式 Responses 响应转换为 Anthropic Message 响应结构。
pub fn convert_responses_to_anthropic_response(val: &Value, model_name: &str) -> Value {
    let mut content = Vec::new();
    if let Some(output) = val.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("reasoning") => {
                    // Responses 的 reasoning 输出项摘要保留为 Anthropic thinking 块
                    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                        let text: String = summary
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect();
                        if !text.is_empty() {
                            content.push(serde_json::json!({ "type": "thinking", "thinking": text }));
                        }
                    }
                }
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                content.push(serde_json::json!({ "type": "text", "text": text }));
                            }
                        }
                    }
                }
                Some("function_call") => {
                    let input = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                        .unwrap_or_else(|| serde_json::json!({}));
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": item.get("call_id").cloned().or_else(|| item.get("id").cloned()).unwrap_or(Value::String(format!("toolu_{}", nanoid::nanoid!(20)))),
                        "name": item.get("name").cloned().unwrap_or(Value::String(String::new())),
                        "input": input
                    }));
                }
                _ => {}
            }
        }
    }
    let input_tokens = val
        .pointer("/usage/input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = val
        .pointer("/usage/output_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    serde_json::json!({
        "id": format!("msg_{}", nanoid::nanoid!(24)),
        "type": "message",
        "role": "assistant",
        "model": model_name,
        "content": content,
        "stop_reason": if content.iter().any(|item| item.get("type").and_then(Value::as_str) == Some("tool_use")) { "tool_use" } else { "end_turn" },
        "stop_sequence": null,
        "usage": { "input_tokens": input_tokens, "output_tokens": output_tokens }
    })
}

fn anthropic_to_openai_finish_reason(reason: Option<&str>) -> &str {
    match reason {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        Some("end_turn") | Some("stop_sequence") => "stop",
        Some(other) => other,
        None => "stop",
    }
}

/// 将非流式 Anthropic message 响应转换为 OpenAI Chat Completion 响应。
pub fn convert_anthropic_to_openai_chat_response(val: &Value, model_name: &str) -> Value {
    let mut text_parts = Vec::new();
    let mut thinking_parts = Vec::new();
    let mut tool_calls = Vec::new();

    if let Some(arr) = val.get("content").and_then(|c| c.as_array()) {
        for item in arr {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                        text_parts.push(t);
                    }
                }
                Some("thinking") => {
                    // 扩展思考块转为 reasoning_content（社区事实标准），不再静默丢弃
                    if let Some(t) = item.get("thinking").and_then(|t| t.as_str()) {
                        thinking_parts.push(t);
                    }
                }
                Some("tool_use") => {
                    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let input = item.get("input").unwrap_or(&Value::Null);
                    let arguments = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());
                    tool_calls.push(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments
                        }
                    }));
                }
                _ => {}
            }
        }
    }
    let text = text_parts.join("");
    let thinking = thinking_parts.join("");

    let prompt_tokens = val
        .pointer("/usage/input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let completion_tokens = val
        .pointer("/usage/output_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let stop_reason = val.get("stop_reason").and_then(|r| r.as_str());
    let finish_reason = if !tool_calls.is_empty() && stop_reason == Some("tool_use") {
        "tool_calls"
    } else {
        anthropic_to_openai_finish_reason(stop_reason)
    };

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": if text.is_empty() && !tool_calls.is_empty() { Value::Null } else { Value::String(text) }
    });
    if !thinking.is_empty() {
        message["reasoning_content"] = Value::String(thinking);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }

    serde_json::json!({
        "id": format!("chatcmpl-{}", nanoid::nanoid!(24)),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    })
}

/// Anthropic SSE → OpenAI Chat Completion SSE 的有状态转换器。
/// Anthropic 的 tool_use 块索引是内容块序号（thinking/text 块在前时非 0 起始），
/// 必须重映射为严格从 0 递增的 `tool_calls[].index`（OpenAI SDK 流式解析的前提）。
#[derive(Default)]
pub struct AnthropicToChatChunkConverter {
    tool_indexes: std::collections::HashMap<u64, u64>,
}

impl AnthropicToChatChunkConverter {
    /// 分配（或复用）内容块索引对应的 tool_calls 序号。
    fn tool_index(&mut self, block_index: u64) -> u64 {
        let next = self.tool_indexes.len() as u64;
        *self.tool_indexes.entry(block_index).or_insert(next)
    }
}

impl SseChunkConverter for AnthropicToChatChunkConverter {
    fn convert_event(&mut self, event_str: &str, model_name: &str, created: i64, chatcmpl_id: &str) -> Option<String> {
        let (event_type, data_str) = crate::relay::parse_sse_event(event_str);
        if data_str.is_empty() {
            return None;
        }
        let val: serde_json::Value = serde_json::from_str(&data_str).ok()?;

        let chunk_with = |delta: Value, finish: Value| -> String {
            let chunk = serde_json::json!({
                "id": chatcmpl_id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model_name,
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
            });
            format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap_or_default())
        };

        match event_type {
            "message_start" => {
                let usage = val.pointer("/message/usage")?;
                let prompt_tokens = usage
                    .get("input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let completion_tokens = usage
                    .get("output_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": null }],
                    "usage": {
                        "prompt_tokens": prompt_tokens,
                        "completion_tokens": completion_tokens,
                        "total_tokens": prompt_tokens + completion_tokens
                    }
                });
                Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ))
            }
            "content_block_start" => {
                let block_index = val.get("index").and_then(Value::as_u64).unwrap_or(0);
                let cb = val.get("content_block")?;
                if cb.get("type").and_then(Value::as_str) != Some("tool_use") {
                    return None;
                }
                let tool_index = self.tool_index(block_index);
                let id = cb.get("id").and_then(Value::as_str).unwrap_or("");
                let name = cb.get("name").and_then(Value::as_str).unwrap_or("");
                Some(chunk_with(
                    serde_json::json!({ "tool_calls": [{
                        "index": tool_index, "id": id, "type": "function",
                        "function": { "name": name, "arguments": "" }
                    }]}),
                    Value::Null,
                ))
            }
            "content_block_delta" => {
                if let Some(text) = val.pointer("/delta/text").and_then(|t| t.as_str()) {
                    Some(chunk_with(serde_json::json!({ "content": text }), Value::Null))
                } else if let Some(thinking) = val.pointer("/delta/thinking").and_then(|t| t.as_str()) {
                    // 扩展思考的 thinking_delta 转发为 reasoning_content（社区事实标准）：
                    // 既让支持该字段的客户端渲染思考过程，也保证转换流持续有字节流动，
                    // 避免 StreamFinalizer 的停滞超时误杀长思考生成。
                    Some(chunk_with(
                        serde_json::json!({ "reasoning_content": thinking }),
                        Value::Null,
                    ))
                } else if let Some(partial) = val.pointer("/delta/partial_json").and_then(|t| t.as_str()) {
                    let block_index = val.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let tool_index = self.tool_index(block_index);
                    Some(chunk_with(
                        serde_json::json!({ "tool_calls": [{
                            "index": tool_index, "function": { "arguments": partial }
                        }]}),
                        Value::Null,
                    ))
                } else {
                    None
                }
            }
            "message_delta" => {
                let stop_reason = val.pointer("/delta/stop_reason").and_then(|s| s.as_str());
                let finish_reason = anthropic_to_openai_finish_reason(stop_reason);
                let mut chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": finish_reason }]
                });
                // 保留 Anthropic 的 output_tokens，让 StreamFinalizer 能恢复
                // 真实的补全 token 数，而不是退化为估算值。
                if let Some(usage) = val.get("usage") {
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    if output_tokens > 0 {
                        chunk["usage"] = serde_json::json!({ "completion_tokens": output_tokens });
                    }
                }
                Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ))
            }
            "message_stop" => Some("data: [DONE]\n\n".to_string()),
            _ => None,
        }
    }
}

fn openai_chat_finish_to_gemini(reason: Option<&str>) -> &str {
    match reason {
        Some("length") => "MAX_TOKENS",
        Some("content_filter") => "SAFETY",
        _ => "STOP",
    }
}

/// 将非流式 Chat Completion 响应转换为 Gemini generateContent 响应。
pub fn convert_openai_chat_to_gemini_response(val: &Value, model_name: &str) -> Value {
    let choice = val.pointer("/choices/0");
    let message = choice.and_then(|choice| choice.get("message"));
    let mut parts = Vec::new();
    if let Some(reasoning) = message
        .and_then(|message| message.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
    {
        parts.push(serde_json::json!({ "text": reasoning }));
    }
    if let Some(text) = message
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        parts.push(serde_json::json!({ "text": text }));
    }
    if let Some(tool_calls) = message
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for call in tool_calls {
            let args = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            parts.push(serde_json::json!({
                "functionCall": {
                    "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or(""),
                    "args": args
                }
            }));
        }
    }
    if parts.is_empty() {
        parts.push(serde_json::json!({ "text": "" }));
    }
    let prompt_tokens = val
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completion_tokens = val
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    serde_json::json!({
        "candidates": [{
            "content": { "role": "model", "parts": parts },
            "finishReason": openai_chat_finish_to_gemini(
                choice.and_then(|choice| choice.get("finish_reason")).and_then(Value::as_str),
            )
        }],
        "usageMetadata": {
            "promptTokenCount": prompt_tokens,
            "candidatesTokenCount": completion_tokens,
            "totalTokenCount": val
                .pointer("/usage/total_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(prompt_tokens + completion_tokens)
        },
        "modelVersion": model_name
    })
}

fn gemini_finish_reason(reason: &str) -> &str {
    match reason {
        "MAX_TOKENS" => "length",
        "STOP" => "stop",
        // 安全过滤 / 违禁词 / 复述拦截等阻断场景标记为 content_filter，
        // 让客户端能区分正常结束与内容被拦截
        "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "RECITATION" => "content_filter",
        "MALFORMED_FUNCTION_CALL" => "stop",
        _ => "stop",
    }
}

/// 将 Gemini 原生 generateContent 响应转换为 OpenAI Chat Completion 响应。
pub fn convert_gemini_to_openai_chat_response(val: &Value, model_name: &str) -> Value {
    let candidate = val.pointer("/candidates/0");
    let parts = candidate
        .and_then(|c| c.pointer("/content/parts"))
        .and_then(Value::as_array);
    let text = parts
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();
    let tool_calls: Vec<Value> = parts
        .into_iter()
        .flatten()
        .filter_map(|part| {
            let call = part.get("functionCall")?;
            let name = call.get("name").and_then(Value::as_str)?;
            let arguments = call
                .get("args")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            Some(serde_json::json!({
                "id": format!("call_{}", nanoid::nanoid!(20)),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string())
                }
            }))
        })
        .collect();

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": text
    });
    let has_tools = !tool_calls.is_empty();
    if has_tools {
        message["tool_calls"] = Value::Array(tool_calls);
    }

    let mut finish_reason = candidate
        .and_then(|c| c.get("finishReason"))
        .and_then(Value::as_str)
        .map(gemini_finish_reason);
    if has_tools && finish_reason == Some("stop") {
        finish_reason = Some("tool_calls");
    }
    let prompt_tokens = val
        .pointer("/usageMetadata/promptTokenCount")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completion_tokens = val
        .pointer("/usageMetadata/candidatesTokenCount")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let total_tokens = val
        .pointer("/usageMetadata/totalTokenCount")
        .and_then(Value::as_i64)
        .unwrap_or(prompt_tokens + completion_tokens);

    serde_json::json!({
        "id": format!("chatcmpl-{}", nanoid::nanoid!(24)),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens
        }
    })
}

/// Gemini 原生 SSE → OpenAI Chat Completion SSE 的有状态转换器。
/// Gemini 的 functionCall 无分片流式；同一调用的 (name, args) 跨 chunk 一致，
/// 以确定性哈希 id 去重并分配严格从 0 递增的 `tool_calls[].index`，
/// 避免不同调用分散在多个 chunk 时 index 全为 0 被客户端合并成坏调用。
#[derive(Default)]
pub struct GeminiToChatChunkConverter {
    tool_indexes: std::collections::HashMap<String, u64>,
}

impl GeminiToChatChunkConverter {
    /// 分配（或复用）确定性 call_id 对应的 tool_calls 序号。
    fn tool_index(&mut self, call_id: &str) -> u64 {
        let next = self.tool_indexes.len() as u64;
        *self.tool_indexes.entry(call_id.to_string()).or_insert(next)
    }
}

impl SseChunkConverter for GeminiToChatChunkConverter {
    fn convert_event(&mut self, event_str: &str, model_name: &str, created: i64, chatcmpl_id: &str) -> Option<String> {
        let (_, data) = crate::relay::parse_sse_event(event_str);
        if data.is_empty() {
            return None;
        }
        let val: Value = serde_json::from_str(&data).ok()?;
        let candidate = val.pointer("/candidates/0")?;
        let parts = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array);
        let text = parts
            .map(|p| {
                p.iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let mut tool_calls: Vec<Value> = Vec::new();
        for part in parts.into_iter().flatten() {
            let Some(call) = part.get("functionCall") else {
                continue;
            };
            let Some(name) = call.get("name").and_then(Value::as_str) else {
                continue;
            };
            let arguments = call
                .get("args")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let arguments_json = serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
            let call_id = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                name.hash(&mut hasher);
                arguments_json.hash(&mut hasher);
                format!("call_{:016x}", hasher.finish())
            };
            let index = self.tool_index(&call_id);
            tool_calls.push(serde_json::json!({
                "index": index,
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments_json
                }
            }));
        }
        let mut finish_reason = candidate
            .get("finishReason")
            .and_then(Value::as_str)
            .map(gemini_finish_reason);
        if !tool_calls.is_empty() && finish_reason == Some("stop") {
            finish_reason = Some("tool_calls");
        }

        if text.is_empty() && tool_calls.is_empty() && finish_reason.is_none() && val.get("usageMetadata").is_none() {
            return None;
        }

        let mut delta = serde_json::Map::new();
        if !text.is_empty() {
            delta.insert("content".to_string(), Value::String(text));
        }
        if !tool_calls.is_empty() {
            delta.insert("tool_calls".to_string(), Value::Array(tool_calls));
        }

        let mut chunk = serde_json::json!({
            "id": chatcmpl_id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model_name,
            "choices": [{
                "index": 0,
                "delta": Value::Object(delta),
                "finish_reason": finish_reason
            }]
        });
        // 保留 Gemini usageMetadata，使 StreamFinalizer 能获得真实 Token 用量而非估算
        if let Some(usage) = val.get("usageMetadata") {
            let prompt_tokens = usage
                .get("promptTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let completion_tokens = usage
                .get("candidatesTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let total_tokens = usage
                .get("totalTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(prompt_tokens + completion_tokens);
            chunk["usage"] = serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": total_tokens
            });
        }
        Some(format!(
            "data: {}\n\n",
            serde_json::to_string(&chunk).unwrap_or_default()
        ))
    }
}

/// 按缓存拆分计算输入侧费用：未命中部分按输入价，缓存读/写按各自价目（每 1M token）。
/// prompt_tokens 为总输入口径（含缓存命中与写入，OpenAI 语义），缓存部分从中扣除。
pub fn calculate_input_cost(
    prompt_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    price: Option<&ModelPrice>,
) -> f64 {
    let Some(p) = price else {
        return 0.0;
    };
    let cache_read = cache_read_tokens.max(0).min(prompt_tokens.max(0)) as f64;
    let cache_write = cache_write_tokens.max(0).min(prompt_tokens.max(0)) as f64;
    let fresh_prompt = (prompt_tokens.max(0) - cache_read as i64 - cache_write as i64).max(0) as f64;
    fresh_prompt / 1_000_000.0 * p.input
        + cache_read / 1_000_000.0 * p.cache_read
        + cache_write / 1_000_000.0 * p.cache_write
}

/// 按缓存拆分计算请求总费用（输入侧 + 输出侧）。
pub fn calculate_cost(
    prompt_tokens: i64,
    completion_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    price: Option<&ModelPrice>,
) -> f64 {
    let output_cost = completion_tokens as f64 / 1_000_000.0 * price.map(|p| p.output).unwrap_or(0.0);
    calculate_input_cost(prompt_tokens, cache_read_tokens, cache_write_tokens, price) + output_cost
}

pub fn extract_usage_from_json(val: &Value) -> Option<Usage> {
    // OpenAI 风格的 `usage` 对象：prompt_tokens / completion_tokens / total_tokens
    // （兼容 Responses 的 input_tokens / output_tokens 别名）。
    let usage_candidate = val
        .get("usage")
        .or_else(|| val.pointer("/response/usage"))
        .or_else(|| val.pointer("/message/usage"));
    if let Some(usage_obj) = usage_candidate {
        let prompt_tokens = usage_obj
            .get("prompt_tokens")
            .or_else(|| usage_obj.get("input_tokens"))
            .or_else(|| usage_obj.get("promptTokenCount"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let completion_tokens = usage_obj
            .get("completion_tokens")
            .or_else(|| usage_obj.get("output_tokens"))
            .or_else(|| usage_obj.get("candidatesTokenCount"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        // 缓存读/写：OpenAI/Gemini 的缓存 token 是 prompt 子集；Anthropic 的
        // cache_*_input_tokens 不含在 input_tokens 内，需并入以统一“总输入”口径。
        let cache_read_tokens = usage_obj
            .get("cache_read_input_tokens")
            .or_else(|| usage_obj.pointer("/prompt_tokens_details/cached_tokens"))
            .or_else(|| usage_obj.get("cachedContentTokenCount"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache_write_tokens = usage_obj
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let is_anthropic_usage = usage_obj.get("cache_read_input_tokens").is_some()
            || usage_obj.get("cache_creation_input_tokens").is_some();
        let prompt_tokens = if is_anthropic_usage {
            prompt_tokens + cache_read_tokens + cache_write_tokens
        } else {
            prompt_tokens
        };

        let total_tokens = usage_obj
            .get("total_tokens")
            .or_else(|| usage_obj.get("totalTokenCount"))
            .and_then(|v| v.as_i64())
            .unwrap_or(prompt_tokens + completion_tokens);

        return Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cache_read_tokens,
            cache_write_tokens,
        });
    }

    // Gemini 风格的顶层 `usageMetadata` 对象。
    if let Some(usage_obj) = val.get("usageMetadata") {
        let prompt_tokens = usage_obj
            .get("promptTokenCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let completion_tokens = usage_obj
            .get("candidatesTokenCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache_read_tokens = usage_obj
            .get("cachedContentTokenCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let total_tokens = usage_obj
            .get("totalTokenCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(prompt_tokens + completion_tokens);
        return Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cache_read_tokens,
            cache_write_tokens: 0,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_gemini_to_openai_chat_response() {
        let gemini = serde_json::json!({
            "candidates": [{
                "content": { "role": "model", "parts": [{ "text": "Hello from Gemini" }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 4,
                "candidatesTokenCount": 3,
                "totalTokenCount": 7
            }
        });
        let openai = convert_gemini_to_openai_chat_response(&gemini, "gemini-2.5-pro");
        assert_eq!(
            openai.pointer("/choices/0/message/content").unwrap(),
            "Hello from Gemini"
        );
        assert_eq!(openai.pointer("/choices/0/finish_reason").unwrap(), "stop");
        assert_eq!(openai.pointer("/usage/total_tokens").unwrap(), 7);
    }

    #[test]
    fn test_gemini_sse_event_to_openai_chunk() {
        let event = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n";
        let mut converter = GeminiToChatChunkConverter::default();
        let chunk = converter
            .convert_event(event, "gemini-2.5-pro", 1700000000, "chatcmpl-test")
            .unwrap();
        assert!(chunk.contains("chat.completion.chunk"));
        assert!(chunk.contains("Hello"));
    }
    #[test]
    fn test_anthropic_sse_event_to_openai_chunk() {
        let mut converter = AnthropicToChatChunkConverter::default();
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n";
        let chunk_str = converter
            .convert_event(event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test")
            .unwrap();
        assert!(chunk_str.starts_with("data: "));
        assert!(chunk_str.contains("Hello"));
        assert!(chunk_str.contains("chatcmpl-test"));

        let stop_event = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let stop_chunk = converter.convert_event(stop_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test");
        assert_eq!(stop_chunk, Some("data: [DONE]\n\n".to_string()));
    }

    #[test]
    fn test_anthropic_sse_event_to_openai_chunk_thinking() {
        let mut converter = AnthropicToChatChunkConverter::default();
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"thinking hard\"}}\n\n";
        let chunk_str = converter
            .convert_event(event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test")
            .unwrap();
        assert!(chunk_str.contains("\"reasoning_content\":\"thinking hard\""));
        assert!(!chunk_str.contains("\"content\""));
    }

    #[test]
    fn test_anthropic_sse_event_to_openai_chunk_tool_calls() {
        let mut converter = AnthropicToChatChunkConverter::default();
        let start_event = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_test123\",\"name\":\"get_weather\",\"input\":{}}}\n\n";
        let chunk1 = converter
            .convert_event(start_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test")
            .unwrap();
        assert!(chunk1.contains("call_test123"));
        assert!(chunk1.contains("get_weather"));

        let delta_event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}\n\n";
        let chunk2 = converter
            .convert_event(delta_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test")
            .unwrap();
        assert!(chunk2.contains("Paris"));
    }

    #[test]
    fn test_anthropic_sse_tool_index_remaps_to_sequential() {
        // thinking(0)/text(1) 块在前时，tool_use 的块索引是 2+；
        // 转换后 tool_calls[].index 必须归一为 0、1
        let mut converter = AnthropicToChatChunkConverter::default();
        let start = |index: u64, id: &str| {
            format!(
                "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":{index},\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"calc\",\"input\":{{}}}}}}\n\n"
            )
        };
        let chunk1 = converter
            .convert_event(&start(2, "call_a"), "m", 1, "cc")
            .unwrap();
        let chunk2 = converter
            .convert_event(&start(3, "call_b"), "m", 1, "cc")
            .unwrap();
        assert!(chunk1.contains("\"index\":0"));
        assert!(chunk2.contains("\"index\":1"));

        let delta = |index: u64, args: &str| {
            format!(
                "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":{index},\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{args}\"}}}}\n\n"
            )
        };
        let d1 = converter
            .convert_event(&delta(2, "1"), "m", 1, "cc")
            .unwrap();
        let d2 = converter
            .convert_event(&delta(3, "2"), "m", 1, "cc")
            .unwrap();
        assert!(d1.contains("\"index\":0"));
        assert!(d2.contains("\"index\":1"));
    }

    #[test]
    fn test_responses_to_chat_chunk_converter() {
        let mut converter = ResponsesToChatChunkConverter::default();
        let event =
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n";
        let chunk = converter.convert_event(event, "m", 1, "cc").unwrap();
        assert!(chunk.contains("\"content\":\"你好\""));

        let completed = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":9,\"output_tokens\":3,\"total_tokens\":12}}}\n\n";
        let chunk = converter.convert_event(completed, "m", 1, "cc").unwrap();
        assert!(chunk.contains("\"finish_reason\":\"stop\""));
        assert!(chunk.contains("\"prompt_tokens\":9"));
    }

    #[test]
    fn test_responses_to_chat_chunk_tool_indexes_sequential() {
        // 两个并行 function_call 的 output_index 为 1、2（文本占 0），
        // 转 Chat 后 tool_calls.index 必须归一为 0、1
        let mut converter = ResponsesToChatChunkConverter::default();
        let added = "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"fc_a\",\"type\":\"function_call\",\"call_id\":\"call_a\",\"name\":\"a\",\"arguments\":\"{}\"}}\n\n";
        let chunk = converter.convert_event(added, "m", 1, "cc").unwrap();
        assert!(chunk.contains("\"index\":0"));
        let delta = "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_a\",\"delta\":\"{\\\"x\\\":1}\"}\n\n";
        let chunk = converter.convert_event(delta, "m", 1, "cc").unwrap();
        assert!(chunk.contains("\"index\":0"));

        let added = "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":2,\"item\":{\"id\":\"fc_b\",\"type\":\"function_call\",\"call_id\":\"call_b\",\"name\":\"b\",\"arguments\":\"{}\"}}\n\n";
        let chunk = converter.convert_event(added, "m", 1, "cc").unwrap();
        assert!(chunk.contains("\"index\":1"));
    }

    #[test]
    fn test_convert_openai_chat_to_gemini_response() {
        let chat = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Answer", "tool_calls": [{
                    "id": "call_1", "type": "function",
                    "function": { "name": "calc", "arguments": "{\"x\":1}" }
                }] },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7 }
        });
        let gemini = convert_openai_chat_to_gemini_response(&chat, "gemini-2.5-pro");
        let parts = gemini
            .pointer("/candidates/0/content/parts")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(parts[0].get("text").unwrap(), "Answer");
        assert_eq!(parts[1].pointer("/functionCall/name").unwrap(), "calc");
        assert_eq!(parts[1].pointer("/functionCall/args/x").unwrap(), 1);
        assert_eq!(gemini.pointer("/candidates/0/finishReason").unwrap(), "STOP");
        assert_eq!(gemini.pointer("/usageMetadata/promptTokenCount").unwrap(), 3);
    }

    #[test]
    fn test_convert_responses_result_to_chat_response() {
        let result = serde_json::json!({
            "id": "resp_1",
            "status": "completed",
            "output": [
                { "type": "message", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "答案" }] }
            ],
            "usage": { "input_tokens": 8, "output_tokens": 2, "total_tokens": 10 }
        });
        let chat = convert_responses_result_to_chat_response(&result, "m");
        assert_eq!(chat.get("object").unwrap(), "chat.completion");
        let choice = chat.pointer("/choices/0").unwrap();
        assert_eq!(choice.pointer("/message/content").unwrap(), "答案");
        assert_eq!(choice.get("finish_reason").unwrap(), "stop");
        assert_eq!(chat.pointer("/usage/prompt_tokens").unwrap(), 8);
    }

    #[test]
    fn test_convert_responses_result_string_content_and_incomplete() {
        // content 为字符串形态时同样提取文本
        let string_content = serde_json::json!({
            "id": "resp_2",
            "status": "completed",
            "output": [ { "type": "message", "role": "assistant", "content": "直接文本" } ],
            "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
        });
        let chat = convert_responses_result_to_chat_response(&string_content, "m");
        assert_eq!(chat.pointer("/choices/0/message/content").unwrap(), "直接文本");

        // status=incomplete（token 截断）映射为 finish_reason=length
        let truncated = serde_json::json!({
            "id": "resp_3",
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "output": [],
            "usage": { "input_tokens": 1, "output_tokens": 0, "total_tokens": 1 }
        });
        let chat = convert_responses_result_to_chat_response(&truncated, "m");
        assert_eq!(chat.pointer("/choices/0/finish_reason").unwrap(), "length");
    }

    #[test]
    fn test_anthropic_sse_event_message_start_usage() {
        let start_event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":42,\"output_tokens\":0}}}\n\n";
        let mut converter = AnthropicToChatChunkConverter::default();
        let chunk = converter
            .convert_event(start_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test")
            .unwrap();
        assert!(chunk.contains("\"prompt_tokens\":42"));
    }
    #[test]
    fn test_convert_chat_to_responses_incomplete_status() {
        let chat = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "被截断" },
                "finish_reason": "length"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 6, "total_tokens": 11 }
        });
        let resp = convert_openai_chat_to_responses_response(&chat, "m");
        assert_eq!(resp.get("status").unwrap(), "incomplete");
        assert_eq!(resp.pointer("/incomplete_details/reason").unwrap(), "max_output_tokens");
    }

    #[test]
    fn test_gemini_finish_reason_content_filter() {
        let gemini = serde_json::json!({
            "candidates": [{
                "content": { "role": "model", "parts": [] },
                "finishReason": "SAFETY"
            }]
        });
        let openai = convert_gemini_to_openai_chat_response(&gemini, "gemini-2.5-pro");
        assert_eq!(openai.pointer("/choices/0/finish_reason").unwrap(), "content_filter");
    }

    #[test]
    fn test_extract_usage_nested_response() {
        let responses_json = serde_json::json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 15,
                    "output_tokens": 30,
                    "total_tokens": 45
                }
            }
        });
        let usage = extract_usage_from_json(&responses_json).unwrap();
        assert_eq!(usage.prompt_tokens, 15);
        assert_eq!(usage.completion_tokens, 30);
        assert_eq!(usage.total_tokens, 45);
    }

    #[test]
    fn test_rewrite_flattens_text_content_arrays() {
        let body = serde_json::json!({
            "model": "m",
            "messages": [
                { "role": "system", "content": "sys" },
                { "role": "user", "content": [
                    { "type": "text", "text": "part1 " },
                    { "type": "text", "text": "part2" }
                ]},
                { "role": "user", "content": [
                    { "type": "text", "text": "看图" },
                    { "type": "image_url", "image_url": { "url": "https://x/y.png" } }
                ]}
            ]
        });
        let out = rewrite_request_body_in_json(&serde_json::to_vec(&body).unwrap(), "m2", None);
        let val: Value = serde_json::from_slice(&out).unwrap();
        let messages = val.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages[0].get("content").unwrap(), "sys");
        assert_eq!(messages[1].get("content").unwrap(), "part1 part2");
        assert!(messages[2].get("content").unwrap().is_array());
        assert_eq!(val.get("model").unwrap(), "m2");
    }

    #[test]
    fn test_rewrite_keeps_cache_control_arrays() {
        // 带 cache_control 的文本块承载缓存断点，不能被扁平化丢弃
        let body = serde_json::json!({
            "model": "m",
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": "缓存块", "cache_control": { "type": "ephemeral" } }
                ]}
            ]
        });
        let out = rewrite_request_body_in_json(&serde_json::to_vec(&body).unwrap(), "m2", None);
        let val: Value = serde_json::from_slice(&out).unwrap();
        let content = val.pointer("/messages/0/content").unwrap();
        assert!(content.is_array());
        assert_eq!(content.pointer("/0/text").unwrap(), "缓存块");
        assert!(content.pointer("/0/cache_control/type").is_some());
    }

    #[test]
    fn test_rewrite_keeps_text_blocks_with_extra_fields() {
        // 附带 citations 等块级字段的文本块合并会静默丢字段，必须保持数组原样
        let body = serde_json::json!({
            "model": "m",
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": "引用内容", "citations": [{ "type": "web_search_result_location" }] },
                    { "type": "text", "text": "普通文本" }
                ]}
            ]
        });
        let out = rewrite_request_body_in_json(&serde_json::to_vec(&body).unwrap(), "m2", None);
        let val: Value = serde_json::from_slice(&out).unwrap();
        let content = val
            .pointer("/messages/0/content")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(
            content[0].pointer("/citations/0/type").unwrap(),
            "web_search_result_location"
        );
        assert_eq!(content[1].get("text").unwrap(), "普通文本");
    }

    #[test]
    fn test_convert_anthropic_to_openai_chat_response_tool_calls() {
        let anthropic_resp = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Checking weather" },
                { "type": "tool_use", "id": "call_abc", "name": "get_weather", "input": { "city": "Paris" } }
            ],
            "model": "claude-3-5-sonnet",
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 10, "output_tokens": 20 }
        });

        let openai = convert_anthropic_to_openai_chat_response(&anthropic_resp, "claude-3-5-sonnet");
        assert_eq!(openai.pointer("/choices/0/finish_reason").unwrap(), "tool_calls");
        assert_eq!(
            openai
                .pointer("/choices/0/message/tool_calls/0/id")
                .unwrap(),
            "call_abc"
        );
        assert_eq!(
            openai
                .pointer("/choices/0/message/tool_calls/0/function/name")
                .unwrap(),
            "get_weather"
        );
    }

    #[test]
    fn test_convert_anthropic_thinking_to_reasoning_content() {
        let anthropic_resp = serde_json::json!({
            "id": "msg_t",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "thinking", "thinking": "深入思考中" },
                { "type": "text", "text": "答案" }
            ],
            "model": "claude-4.5",
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 5, "output_tokens": 6 }
        });
        let openai = convert_anthropic_to_openai_chat_response(&anthropic_resp, "claude-4.5");
        assert_eq!(
            openai
                .pointer("/choices/0/message/reasoning_content")
                .unwrap(),
            "深入思考中"
        );
        assert_eq!(openai.pointer("/choices/0/message/content").unwrap(), "答案");
    }

    #[test]
    fn test_convert_chat_to_anthropic_preserves_reasoning_and_empty_content() {
        // reasoning_content 转为 thinking 块，且 content 不为空数组
        let chat = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "", "reasoning_content": "推理过程" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2 }
        });
        let anthropic = convert_openai_chat_to_anthropic_response(&chat, "m");
        assert_eq!(anthropic.pointer("/content/0/type").unwrap(), "thinking");
        assert_eq!(anthropic.pointer("/content/0/thinking").unwrap(), "推理过程");

        // 全空响应兜底：content 至少含一个空文本块，不产生空数组
        let chat_empty = serde_json::json!({
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 0, "completion_tokens": 0 }
        });
        let anthropic = convert_openai_chat_to_anthropic_response(&chat_empty, "m");
        let content = anthropic.get("content").unwrap().as_array().unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_convert_responses_to_anthropic_preserves_reasoning_summary() {
        let resp = serde_json::json!({
            "output": [
                { "id": "rs_1", "type": "reasoning", "summary": [{ "type": "summary_text", "text": "摘要思考" }] },
                { "id": "msg_1", "type": "message", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "正文" }] }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 4 }
        });
        let anthropic = convert_responses_to_anthropic_response(&resp, "m");
        assert_eq!(anthropic.pointer("/content/0/type").unwrap(), "thinking");
        assert_eq!(anthropic.pointer("/content/0/thinking").unwrap(), "摘要思考");
        assert_eq!(anthropic.pointer("/content/1/type").unwrap(), "text");
    }

    #[test]
    fn test_convert_chat_to_responses_preserves_reasoning() {
        let chat = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "答案", "reasoning_content": "思考" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3 }
        });
        let resp = convert_openai_chat_to_responses_response(&chat, "m");
        let output = resp.get("output").unwrap().as_array().unwrap();
        assert_eq!(output[0].get("type").unwrap(), "reasoning");
        assert_eq!(output[0].pointer("/summary/0/text").unwrap(), "思考");
    }

    #[test]
    fn test_gemini_sse_event_to_openai_chunk_tool_calls() {
        let gemini_event = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"calc\",\"args\":{\"x\":1}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";
        let mut converter = GeminiToChatChunkConverter::default();
        let chunk = converter
            .convert_event(gemini_event, "gemini-1.5-flash", 1700000000, "chatcmpl-gemini")
            .unwrap();
        assert!(chunk.contains("calc"));
        assert!(chunk.contains("tool_calls"));
    }

    #[test]
    fn test_gemini_sse_tool_calls_sequential_index_and_stable_id() {
        let gemini_event = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"a\",\"args\":{\"x\":1}}},{\"functionCall\":{\"name\":\"b\",\"args\":{\"y\":2}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";
        let mut converter = GeminiToChatChunkConverter::default();
        let chunk = converter
            .convert_event(gemini_event, "gemini-1.5-flash", 1700000000, "chatcmpl-gemini")
            .unwrap();
        // 两个调用 index 分别为 0、1，且 id 确定性生成（不等于彼此）
        assert!(chunk.contains("\"index\":0"));
        assert!(chunk.contains("\"index\":1"));
        assert!(!chunk.contains("\"index\":2"));
    }

    #[test]
    fn test_gemini_sse_tool_calls_across_chunks_get_distinct_indexes() {
        // 两次 functionCall 分散在不同 chunk 时 index 也必须递增，不能全为 0
        let mut converter = GeminiToChatChunkConverter::default();
        let call = |name: &str| {
            let event = serde_json::json!({
                "candidates": [{
                    "content": {
                        "parts": [{ "functionCall": { "name": name, "args": { "x": 1 } } }],
                        "role": "model"
                    }
                }]
            });
            format!("data: {}\n\n", event)
        };
        let chunk1 = converter.convert_event(&call("a"), "m", 1, "cc").unwrap();
        let chunk2 = converter.convert_event(&call("b"), "m", 1, "cc").unwrap();
        assert!(chunk1.contains("\"index\":0"));
        assert!(chunk2.contains("\"index\":1"));
    }
    #[test]
    fn test_convert_anthropic_to_openai_chat_response() {
        let anthropic_resp = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hi from Claude" }],
            "model": "claude-3-5-sonnet",
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 12, "output_tokens": 8 }
        });

        let openai = convert_anthropic_to_openai_chat_response(&anthropic_resp, "claude-3-5-sonnet");
        assert_eq!(openai.get("model").unwrap(), "claude-3-5-sonnet");
        let choices = openai.get("choices").unwrap().as_array().unwrap();
        assert_eq!(choices[0].pointer("/message/content").unwrap(), "Hi from Claude");
        assert_eq!(openai.pointer("/usage/prompt_tokens").unwrap(), 12);
        assert_eq!(openai.pointer("/usage/completion_tokens").unwrap(), 8);
    }

    #[test]
    fn test_convert_openai_chat_to_responses_response() {
        let chat = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hello" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3 }
        });
        let response = convert_openai_chat_to_responses_response(&chat, "test-model");
        assert_eq!(response.get("object").unwrap(), "response");
        assert_eq!(response.pointer("/output/0/content/0/text").unwrap(), "Hello");
        assert_eq!(response.pointer("/usage/total_tokens").unwrap(), 3);
    }
    #[test]
    fn test_convert_openai_chat_to_anthropic_response() {
        let chat = serde_json::json!({
            "choices": [{ "message": { "content": "Hello" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 2, "completion_tokens": 1 }
        });
        let response = convert_openai_chat_to_anthropic_response(&chat, "test-model");
        assert_eq!(response.get("type").unwrap(), "message");
        assert_eq!(response.pointer("/content/0/text").unwrap(), "Hello");
        assert_eq!(response.get("stop_reason").unwrap(), "end_turn");
    }

    #[test]
    fn test_extract_usage_anthropic_cache_normalization() {
        // Anthropic 的 input_tokens 不含缓存部分：归一化后 prompt_tokens 应为总输入
        let start = serde_json::json!({
            "type": "message_start",
            "message": { "usage": {
                "input_tokens": 10,
                "cache_read_input_tokens": 100,
                "cache_creation_input_tokens": 5,
                "output_tokens": 0
            }}
        });
        let usage = extract_usage_from_json(&start).unwrap();
        assert_eq!(usage.prompt_tokens, 115);
        assert_eq!(usage.cache_read_tokens, 100);
        assert_eq!(usage.cache_write_tokens, 5);
        assert_eq!(usage.total_tokens, 115);
    }

    #[test]
    fn test_extract_usage_openai_cached_tokens() {
        // OpenAI 的 cached_tokens 是 prompt_tokens 的子集，不改变总量口径
        let chunk = serde_json::json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 7,
                "total_tokens": 107,
                "prompt_tokens_details": { "cached_tokens": 80 }
            }
        });
        let usage = extract_usage_from_json(&chunk).unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.cache_read_tokens, 80);
        assert_eq!(usage.cache_write_tokens, 0);
    }

    #[test]
    fn test_calculate_cost_cache_split() {
        let price = ModelPrice {
            input:       3.0,
            output:      15.0,
            cache_read:  0.3,
            cache_write: 3.75,
        };
        // prompt=115（fresh 10 + 缓存读 100 + 缓存写 5），completion=7
        let cost = calculate_cost(115, 7, 100, 5, Some(&price));
        let expected = 10.0 / 1e6 * 3.0 + 100.0 / 1e6 * 0.3 + 5.0 / 1e6 * 3.75 + 7.0 / 1e6 * 15.0;
        assert!((cost - expected).abs() < 1e-12);
        // 无价目时恒为 0
        assert_eq!(calculate_cost(115, 7, 100, 5, None), 0.0);
    }
}
