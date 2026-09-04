use serde::Deserialize;
use serde_json::Value;

use crate::{model::ModelPrice, relay::state::Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenaiCompletion,
    OpenaiResponse,
    AnthropicMessage,
}

#[derive(Debug, Deserialize)]
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

        if let Ok(bytes) = serde_json::to_vec(&val) {
            return bytes;
        }
    }
    body.to_vec()
}

/// 将 Responses API 的 function tool 转换为 Chat Completions 的嵌套结构。
fn convert_responses_tool_to_chat(tool: &Value) -> Option<Value> {
    let tool_type = tool
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if tool_type != "function" {
        return None;
    }

    if tool.pointer("/function/name").is_some() {
        return Some(tool.clone());
    }

    let name = tool.get("name").and_then(Value::as_str)?;
    let mut function = serde_json::Map::new();
    function.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(description) = tool.get("description") {
        function.insert("description".to_string(), description.clone());
    }
    function.insert(
        "parameters".to_string(),
        tool.get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} })),
    );

    Some(serde_json::json!({
        "type": "function",
        "function": function
    }))
}

/// 将 OpenAI Responses API 请求载荷（包含 `input: [...]`）转换为 Chat Completions 请求。
pub fn convert_responses_to_chat_completions(val: &Value) -> Value {
    let mut obj = match val.as_object() {
        Some(o) => o.clone(),
        None => return val.clone(),
    };

    // 移除 Responses API 专属字段，避免在标准 Chat Completions 上引发 400 Bad Request
    obj.remove("include");
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        *tools = tools
            .iter()
            .filter_map(convert_responses_tool_to_chat)
            .collect();
    }

    if let Some(choice) = obj.get_mut("tool_choice")
        && let Some(name) = choice.get("name").and_then(Value::as_str)
    {
        *choice = serde_json::json!({
            "type": "function",
            "function": { "name": name }
        });
    }

    if let Some(input) = obj.remove("input") {
        let mut messages: Vec<Value> = Vec::new();
        // 将连续的 assistant 输出项（消息文本 + 函数调用）聚合到单条 Chat Completions 消息中，
        // 保证 tool calls 正确附加在生成它们的 assistant 回合内。
        let mut current_assistant: Option<serde_json::Map<String, Value>> = None;
        // 生成的 function_call 标识符映射表，当 `call_id` 缺失时记录并在对应的 `function_call_output` 中复用
        let mut call_lookup: Vec<(String, String)> = Vec::new();

        if let Some(items) = input.as_array() {
            for item in items {
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
                match item_type {
                    "function_call" => {
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| item.get("id").and_then(Value::as_str).map(str::to_string))
                            .unwrap_or_else(|| format!("call_{}", nanoid::nanoid!(20)));
                        let raw_id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        call_lookup.push((raw_id, call_id.clone()));
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let arguments = item
                            .get("arguments")
                            .cloned()
                            .unwrap_or(Value::String("{}".to_string()));
                        let call = serde_json::json!({
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        });

                        let entry = current_assistant.get_or_insert_with(serde_json::Map::new);
                        entry.insert("role".to_string(), Value::String("assistant".to_string()));
                        if !entry.contains_key("content") {
                            entry.insert("content".to_string(), Value::Null);
                        }
                        let calls = entry
                            .entry("tool_calls".to_string())
                            .or_insert_with(|| Value::Array(vec![]));
                        if let Some(arr) = calls.as_array_mut() {
                            arr.push(call);
                        }
                    }
                    "function_call_output" => {
                        // 工具结果输出将结束当前 assistant 轮次
                        if let Some(assistant) = current_assistant.take() {
                            messages.push(Value::Object(assistant));
                        }
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| item.get("id").and_then(Value::as_str).map(str::to_string))
                            .or_else(|| call_lookup.pop().map(|(_, used)| used));
                        let content = match item.get("output").cloned() {
                            Some(Value::String(s)) => Value::String(s),
                            Some(other) => Value::String(serde_json::to_string(&other).unwrap_or_default()),
                            None => Value::String(String::new()),
                        };
                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call_id.unwrap_or_default(),
                            "content": content
                        }));
                    }
                    _ => {
                        let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                        let content_val = match item.get("content") {
                            Some(Value::Array(parts)) => {
                                let mut converted_parts = Vec::new();
                                for part in parts {
                                    let ptype = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if ptype == "input_text" || ptype == "text" || ptype == "output_text" {
                                        if let Some(text) = part.get("text") {
                                            converted_parts.push(serde_json::json!({
                                                "type": "text",
                                                "text": text
                                            }));
                                        }
                                    } else if (ptype == "input_image" || ptype == "image_url")
                                        && let Some(url) = part.get("image_url")
                                    {
                                        converted_parts.push(serde_json::json!({
                                            "type": "image_url",
                                            "image_url": url
                                        }));
                                    }
                                }
                                if converted_parts.len() == 1
                                    && converted_parts[0].get("type").and_then(|t| t.as_str()) == Some("text")
                                {
                                    converted_parts[0]
                                        .get("text")
                                        .cloned()
                                        .unwrap_or(Value::Array(converted_parts))
                                } else {
                                    Value::Array(converted_parts)
                                }
                            }
                            Some(other) => other.clone(),
                            None => Value::String(String::new()),
                        };

                        if role == "assistant" {
                            // 助手输出文本属于当前 assistant 轮次，合并至当前消息中
                            let entry = current_assistant.get_or_insert_with(serde_json::Map::new);
                            entry.insert("role".to_string(), Value::String("assistant".to_string()));
                            match entry.get("content") {
                                Some(Value::Null) => {
                                    entry.insert("content".to_string(), content_val);
                                }
                                Some(_) => {
                                    // 追加至已有的文本内容之后
                                    let existing = entry.get("content").and_then(Value::as_str).unwrap_or("");
                                    let combined = if existing.is_empty() {
                                        content_val.as_str().unwrap_or("").to_string()
                                    } else {
                                        format!("{existing}{}", content_val.as_str().unwrap_or(""))
                                    };
                                    entry.insert("content".to_string(), Value::String(combined));
                                }
                                None => {
                                    entry.insert("content".to_string(), content_val);
                                }
                            }
                        } else {
                            // user 消息（或非 assistant 项）将结束当前 assistant 轮次
                            if let Some(assistant) = current_assistant.take() {
                                messages.push(Value::Object(assistant));
                            }
                            messages.push(serde_json::json!({
                                "role": role,
                                "content": content_val
                            }));
                        }
                    }
                }
            }
        }
        if let Some(assistant) = current_assistant.take() {
            messages.push(Value::Object(assistant));
        }
        obj.insert("messages".to_string(), Value::Array(messages));
    }

    Value::Object(obj)
}

/// 将 OpenAI Chat Completions 请求载荷（包含 `messages: [...]`）转换为 OpenAI Responses 请求载荷（包含 `input: [...]`）。
pub fn convert_chat_completions_to_responses(val: &Value) -> Value {
    let mut obj = match val.as_object() {
        Some(o) => o.clone(),
        None => return val.clone(),
    };

    if let Some(messages) = obj.remove("messages") {
        obj.insert("input".to_string(), messages);
    }

    Value::Object(obj)
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
    serde_json::json!({
        "id": format!("resp_{}", nanoid::nanoid!(24)),
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "completed_at": chrono::Utc::now().timestamp(),
        "status": "completed",
        "model": model_name,
        "output": output,
        "error": null,
        "incomplete_details": null,
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

/// 将 Anthropic Messages 请求转换为 OpenAI Chat Completions 请求。
pub fn convert_anthropic_to_openai_chat_request(val: &Value) -> Value {
    let mut obj = match val.as_object() {
        Some(obj) => obj.clone(),
        None => return val.clone(),
    };
    let mut messages = Vec::new();

    if let Some(system) = obj.remove("system") {
        let text = extract_text_content(&system);
        if !text.is_empty() {
            messages.push(serde_json::json!({ "role": "system", "content": text }));
        }
    }

    if let Some(Value::Array(input_messages)) = obj.remove("messages") {
        for message in input_messages {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let content = message.get("content").unwrap_or(&Value::Null);
            if let Some(parts) = content.as_array() {
                let mut normal_parts = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => normal_parts.push(serde_json::json!({
                            "type": "text",
                            "text": part.get("text").cloned().unwrap_or(Value::String(String::new()))
                        })),
                        Some("image") => {
                            if let Some(source) = part.get("source") {
                                let url = if source.get("type").and_then(Value::as_str) == Some("base64") {
                                    format!(
                                        "data:{};base64,{}",
                                        source.get("media_type").and_then(Value::as_str).unwrap_or("image/png"),
                                        source.get("data").and_then(Value::as_str).unwrap_or("")
                                    )
                                } else {
                                    source.get("url").and_then(Value::as_str).unwrap_or("").to_string()
                                };
                                normal_parts.push(serde_json::json!({
                                    "type": "image_url",
                                    "image_url": { "url": url }
                                }));
                            }
                        }
                        Some("tool_use") => tool_calls.push(serde_json::json!({
                            "id": part.get("id").cloned().unwrap_or(Value::String(String::new())),
                            "type": "function",
                            "function": {
                                "name": part.get("name").cloned().unwrap_or(Value::String(String::new())),
                                "arguments": serde_json::to_string(part.get("input").unwrap_or(&Value::Null)).unwrap_or_else(|_| "{}".to_string())
                            }
                        })),
                        Some("tool_result") => tool_results.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": part.get("tool_use_id").cloned().unwrap_or(Value::String(String::new())),
                            "content": part.get("content").cloned().unwrap_or(Value::String(String::new()))
                        })),
                        _ => {}
                    }
                }

                if !normal_parts.is_empty() || !tool_calls.is_empty() {
                    let content = if normal_parts.len() == 1
                        && normal_parts[0].get("type").and_then(Value::as_str) == Some("text")
                    {
                        normal_parts[0]
                            .get("text")
                            .cloned()
                            .unwrap_or(Value::String(String::new()))
                    } else {
                        Value::Array(normal_parts)
                    };
                    let mut converted = serde_json::json!({ "role": role, "content": content });
                    if !tool_calls.is_empty() {
                        converted["tool_calls"] = Value::Array(tool_calls);
                    }
                    messages.push(converted);
                }
                messages.extend(tool_results);
            } else {
                messages.push(serde_json::json!({ "role": role, "content": content }));
            }
        }
    }
    obj.insert("messages".to_string(), Value::Array(messages));

    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        *tools = tools
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.clone();
                Some(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": tool.get("description").cloned().unwrap_or(Value::String(String::new())),
                        "parameters": tool.get("input_schema").cloned().unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }))
                    }
                }))
            })
            .collect();
    }
    if let Some(choice) = obj.get_mut("tool_choice") {
        match choice.get("type").and_then(Value::as_str) {
            Some("tool") => {
                let name = choice
                    .get("name")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                *choice = serde_json::json!({ "type": "function", "function": { "name": name } });
            }
            Some("any") => *choice = Value::String("required".to_string()),
            Some("none") => *choice = Value::String("none".to_string()),
            _ => *choice = Value::String("auto".to_string()),
        }
    }

    Value::Object(obj)
}
/// 从 OpenAI `content`（字符串或内容块数组）中提取纯文本。
fn extract_text_content(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 将 OpenAI `image_url` 内容块转换为 Anthropic `image` 块。
/// Base64 `data:` URI 转为 `source.type = "base64"`；公开 URL 则以 `source.type = "url"` 交由 Anthropic 处理。
fn openai_image_to_anthropic(url: &str) -> Option<Value> {
    if url.is_empty() {
        return None;
    }
    if let Some(data_uri) = url.strip_prefix("data:")
        && let Some((meta, data)) = data_uri.split_once(',')
    {
        let media_type = meta
            .split(';')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("image/png")
            .to_string();
        return Some(serde_json::json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data }
        }));
    }
    Some(serde_json::json!({
        "type": "image",
        "source": { "type": "url", "url": url }
    }))
}

/// 将 OpenAI `content` 转换为 Anthropic 的 `content` 结构（字符串或块数组）。
fn openai_content_to_anthropic(content: &Value) -> Value {
    match content {
        Value::String(s) => Value::String(s.clone()),
        Value::Array(parts) => {
            let mut blocks = Vec::new();
            for part in parts {
                let ptype = part.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                match ptype {
                    "text" => {
                        if let Some(text) = part.get("text") {
                            blocks.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                    }
                    "image_url" => {
                        if let Some(url) = part.pointer("/image_url/url").and_then(|u| u.as_str())
                            && let Some(block) = openai_image_to_anthropic(url)
                        {
                            blocks.push(block);
                        }
                    }
                    _ => {
                        if let Some(text) = part.get("text") {
                            blocks.push(serde_json::json!({ "type": "text", "text": text }));
                        }
                    }
                }
            }
            match blocks.len() {
                0 => Value::String(String::new()),
                1 => blocks.remove(0),
                _ => Value::Array(blocks),
            }
        }
        _ => Value::String(String::new()),
    }
}

/// 将 OpenAI 助手消息（内容 + 可选的 `tool_calls`）转换为包含 `tool_use` 块的 Anthropic 助手消息。
fn openai_assistant_to_anthropic(msg: &Value) -> Value {
    let mut blocks = Vec::new();
    if let Some(content) = msg.get("content") {
        let converted = openai_content_to_anthropic(content);
        let is_empty = converted.as_str().map(|s| s.is_empty()).unwrap_or(false);
        if !is_empty {
            blocks.push(converted);
        }
    }
    if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let input =
                serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| Value::String(arguments.to_string()));
            blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }
    serde_json::json!({ "role": "assistant", "content": Value::Array(blocks) })
}

/// 将 OpenAI `tool` 角色消息转换为 Anthropic `tool_result` 内容块。
fn openai_tool_to_anthropic_block(msg: &Value) -> Value {
    let tool_use_id = msg
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let result_content = msg
        .get("content")
        .cloned()
        .unwrap_or_else(|| Value::String(String::new()));
    serde_json::json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": result_content
    })
}

/// 将 OpenAI /chat/completions 请求载荷转换为 Anthropic /messages 请求载荷。
pub fn convert_openai_to_anthropic(val: &Value) -> Value {
    let mut anthropic_obj = serde_json::Map::new();

    if let Some(model) = val.get("model") {
        anthropic_obj.insert("model".to_string(), model.clone());
    }
    if let Some(stream) = val.get("stream") {
        anthropic_obj.insert("stream".to_string(), stream.clone());
    }
    if let Some(max_tokens) = val
        .get("max_tokens")
        .or_else(|| val.get("max_completion_tokens"))
    {
        anthropic_obj.insert("max_tokens".to_string(), max_tokens.clone());
    } else {
        anthropic_obj.insert("max_tokens".to_string(), serde_json::json!(4096));
    }
    if let Some(temp) = val.get("temperature") {
        anthropic_obj.insert("temperature".to_string(), temp.clone());
    }
    if let Some(top_p) = val.get("top_p") {
        anthropic_obj.insert("top_p".to_string(), top_p.clone());
    }

    // `tools`：OpenAI 将每个工具包装为 `{type, function:{...}}`；Anthropic 使用扁平的
    // `{name, description, input_schema}` 对象。
    if let Some(tools) = val.get("tools").and_then(|v| v.as_array()) {
        let mut anthropic_tools = Vec::new();
        for tool in tools {
            let name = tool
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let mut t = serde_json::Map::new();
            t.insert("name".to_string(), Value::String(name.to_string()));
            if let Some(desc) = tool
                .pointer("/function/description")
                .and_then(|v| v.as_str())
                && !desc.is_empty()
            {
                t.insert("description".to_string(), Value::String(desc.to_string()));
            }
            let input_schema = tool
                .pointer("/function/parameters")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }));
            t.insert("input_schema".to_string(), input_schema);
            anthropic_tools.push(Value::Object(t));
        }
        if !anthropic_tools.is_empty() {
            anthropic_obj.insert("tools".to_string(), Value::Array(anthropic_tools));
        }
    }

    // `tool_choice`：OpenAI 为 "auto"/"none"/"required" 或函数对象；
    // Anthropic 使用 `type: auto|none|any|tool` 的对象。
    if let Some(choice) = val.get("tool_choice") {
        match choice {
            Value::String(s) => {
                let t = match s.as_str() {
                    "none" => "none",
                    "required" => "any",
                    _ => "auto",
                };
                anthropic_obj.insert("tool_choice".to_string(), serde_json::json!({ "type": t }));
            }
            Value::Object(o) => {
                if let Some(fname) = o
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                {
                    anthropic_obj.insert(
                        "tool_choice".to_string(),
                        serde_json::json!({ "type": "tool", "name": fname }),
                    );
                }
            }
            _ => {}
        }
    }

    let mut system_prompt = String::new();
    let mut anthropic_messages = Vec::new();

    if let Some(messages) = val.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            match role {
                "system" => {
                    let text = extract_text_content(msg.get("content").unwrap_or(&Value::Null));
                    if !text.is_empty() {
                        if !system_prompt.is_empty() {
                            system_prompt.push('\n');
                        }
                        system_prompt.push_str(&text);
                    }
                }
                "assistant" => anthropic_messages.push(openai_assistant_to_anthropic(msg)),
                "tool" => {
                    let block = openai_tool_to_anthropic_block(msg);
                    if let Some(last_msg) = anthropic_messages.last_mut()
                        && last_msg.get("role").and_then(Value::as_str) == Some("user")
                    {
                        match last_msg.get_mut("content") {
                            Some(Value::Array(arr)) => arr.push(block),
                            Some(existing_content) => {
                                let prev = existing_content.take();
                                let prev_block = match prev {
                                    Value::String(s) => serde_json::json!({ "type": "text", "text": s }),
                                    other => other,
                                };
                                *existing_content = Value::Array(vec![prev_block, block]);
                            }
                            None => {
                                if let Some(obj) = last_msg.as_object_mut() {
                                    obj.insert("content".to_string(), Value::Array(vec![block]));
                                }
                            }
                        }
                    } else {
                        anthropic_messages.push(serde_json::json!({
                            "role": "user",
                            "content": [block]
                        }));
                    }
                }
                _ => {
                    let content =
                        openai_content_to_anthropic(msg.get("content").unwrap_or(&Value::String(String::new())));
                    if role == "user"
                        && let Some(last_msg) = anthropic_messages.last_mut()
                        && last_msg.get("role").and_then(Value::as_str) == Some("user")
                    {
                        let new_blocks = match content {
                            Value::Array(arr) => arr,
                            Value::String(s) => vec![serde_json::json!({ "type": "text", "text": s })],
                            other => vec![other],
                        };
                        match last_msg.get_mut("content") {
                            Some(Value::Array(arr)) => arr.extend(new_blocks),
                            Some(existing_content) => {
                                let prev = existing_content.take();
                                let mut blocks = match prev {
                                    Value::Array(arr) => arr,
                                    Value::String(s) => vec![serde_json::json!({ "type": "text", "text": s })],
                                    other => vec![other],
                                };
                                blocks.extend(new_blocks);
                                *existing_content = Value::Array(blocks);
                            }
                            None => {
                                if let Some(obj) = last_msg.as_object_mut() {
                                    obj.insert("content".to_string(), Value::Array(new_blocks));
                                }
                            }
                        }
                    } else {
                        anthropic_messages.push(serde_json::json!({
                            "role": role,
                            "content": content
                        }));
                    }
                }
            }
        }
    }

    if !system_prompt.is_empty() {
        anthropic_obj.insert("system".to_string(), Value::String(system_prompt));
    }
    anthropic_obj.insert("messages".to_string(), Value::Array(anthropic_messages));

    Value::Object(anthropic_obj)
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
    let mut tool_calls = Vec::new();

    if let Some(arr) = val.get("content").and_then(|c| c.as_array()) {
        for item in arr {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                        text_parts.push(t);
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

/// 将 Anthropic SSE 事件字符串转换为对应的 OpenAI Chat Completion SSE chunk 行。
pub fn anthropic_sse_event_to_openai_chunk(
    event_str: &str,
    model_name: &str,
    created: i64,
    chatcmpl_id: &str,
) -> Option<String> {
    let mut event_type = "";
    let mut data_str = "";

    for line in event_str.lines() {
        let trimmed = line.trim();
        if let Some(et) = trimmed.strip_prefix("event:") {
            event_type = et.trim();
        } else if let Some(d) = trimmed.strip_prefix("data:") {
            data_str = d.trim();
        }
    }

    if data_str.is_empty() {
        return None;
    }

    let val: serde_json::Value = serde_json::from_str(data_str).ok()?;

    match event_type {
        "message_start" => {
            if let Some(usage) = val.pointer("/message/usage") {
                let prompt_tokens = usage
                    .get("input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let completion_tokens = usage
                    .get("output_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let total_tokens = prompt_tokens + completion_tokens;
                let chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": null
                    }],
                    "usage": {
                        "prompt_tokens": prompt_tokens,
                        "completion_tokens": completion_tokens,
                        "total_tokens": total_tokens
                    }
                });
                return Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
            }
        }
        "content_block_start" => {
            let index = val.get("index").and_then(Value::as_u64).unwrap_or(0);
            if let Some(cb) = val.get("content_block")
                && cb.get("type").and_then(Value::as_str) == Some("tool_use")
            {
                let id = cb.get("id").and_then(Value::as_str).unwrap_or("");
                let name = cb.get("name").and_then(Value::as_str).unwrap_or("");
                let chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": index,
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": ""
                                }
                            }]
                        },
                        "finish_reason": null
                    }]
                });
                return Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
            }
        }
        "content_block_delta" => {
            let index = val.get("index").and_then(Value::as_u64).unwrap_or(0);
            if let Some(text) = val.pointer("/delta/text").and_then(|t| t.as_str()) {
                let chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "delta": { "content": text },
                        "finish_reason": null
                    }]
                });
                return Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
            } else if let Some(partial) = val.pointer("/delta/partial_json").and_then(|t| t.as_str()) {
                let chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": index,
                                "function": {
                                    "arguments": partial
                                }
                            }]
                        },
                        "finish_reason": null
                    }]
                });
                return Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
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
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": finish_reason
                }]
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
            return Some(format!(
                "data: {}\n\n",
                serde_json::to_string(&chunk).unwrap_or_default()
            ));
        }
        "message_stop" => {
            return Some("data: [DONE]\n\n".to_string());
        }
        _ => {}
    }

    None
}

/// 将 OpenAI `image_url` 内容块转换为 Gemini 格式。Base64 `data:` URI 转换为 `inlineData`；公开 URL 转换为 `fileData`。
fn openai_image_to_gemini_part(url: &str) -> Option<Value> {
    if url.is_empty() {
        return None;
    }
    if let Some(data_uri) = url.strip_prefix("data:")
        && let Some((meta, data)) = data_uri.split_once(',')
    {
        let mime_type = meta
            .split(';')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("image/png")
            .to_string();
        return Some(serde_json::json!({
            "inlineData": { "mimeType": mime_type, "data": data }
        }));
    }
    Some(serde_json::json!({
        "fileData": { "fileUri": url }
    }))
}

/// 将 OpenAI `content` 转换为 Gemini 的 `parts` 内容块。
fn openai_content_to_gemini_parts(content: &Value) -> Vec<Value> {
    let mut parts = Vec::new();
    match content {
        Value::String(s) => parts.push(serde_json::json!({ "text": s })),
        Value::Array(items) => {
            for item in items {
                let ptype = item.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                match ptype {
                    "text" => {
                        if let Some(text) = item.get("text") {
                            parts.push(serde_json::json!({ "text": text }));
                        }
                    }
                    "image_url" => {
                        if let Some(url) = item.pointer("/image_url/url").and_then(|u| u.as_str())
                            && let Some(part) = openai_image_to_gemini_part(url)
                        {
                            parts.push(part);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    parts
}

/// 将 OpenAI /chat/completions 请求载荷转换为 Google Gemini generateContent 请求载荷。
pub fn convert_openai_to_gemini(val: &Value) -> Value {
    let mut contents = Vec::new();
    let mut system_instruction = None;
    // OpenAI 的 `tool` 消息仅携带 `tool_call_id`；记录其与函数名称的映射，供 Gemini 的 `functionResponse` 填充对应函数名
    let mut tool_call_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    if let Some(messages) = val.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            match role {
                "system" => {
                    let text = extract_text_content(msg.get("content").unwrap_or(&Value::Null));
                    if !text.is_empty() {
                        system_instruction = Some(serde_json::json!({
                            "parts": [{ "text": text }]
                        }));
                    }
                }
                "assistant" => {
                    let mut parts = Vec::new();
                    parts.extend(openai_content_to_gemini_parts(
                        msg.get("content").unwrap_or(&Value::Null),
                    ));
                    if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                        for call in calls {
                            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = call
                                .pointer("/function/name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let arguments = call
                                .pointer("/function/arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or("{}");
                            if !id.is_empty() && !name.is_empty() {
                                tool_call_names.insert(id.to_string(), name.to_string());
                            }
                            let args = serde_json::from_str::<Value>(arguments)
                                .unwrap_or_else(|_| Value::String(arguments.to_string()));
                            parts.push(serde_json::json!({
                                "functionCall": { "name": name, "args": args }
                            }));
                        }
                    }
                    contents.push(serde_json::json!({
                        "role": "model",
                        "parts": parts
                    }));
                }
                "tool" => {
                    let tool_call_id = msg
                        .get("tool_call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let name = tool_call_names
                        .get(tool_call_id)
                        .cloned()
                        .unwrap_or_default();
                    let response = msg
                        .get("content")
                        .cloned()
                        .unwrap_or_else(|| Value::String(String::new()));
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": { "name": name, "response": response }
                        }]
                    }));
                }
                _ => {
                    let parts =
                        openai_content_to_gemini_parts(msg.get("content").unwrap_or(&Value::String(String::new())));
                    let gemini_role = if role == "assistant" { "model" } else { "user" };
                    contents.push(serde_json::json!({
                        "role": gemini_role,
                        "parts": parts
                    }));
                }
            }
        }
    }

    let mut gemini_obj = serde_json::Map::new();
    gemini_obj.insert("contents".to_string(), Value::Array(contents));
    if let Some(sys) = system_instruction {
        gemini_obj.insert("system_instruction".to_string(), sys);
    }

    // `tools`: 将 OpenAI `{type:"function", function:{name,...}}` 转换为 Gemini 的 `functionDeclarations` 列表
    if let Some(tools) = val.get("tools").and_then(|v| v.as_array()) {
        let mut funcs = Vec::new();
        for tool in tools {
            let name = tool
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let mut f = serde_json::Map::new();
            f.insert("name".to_string(), Value::String(name.to_string()));
            if let Some(desc) = tool
                .pointer("/function/description")
                .and_then(|v| v.as_str())
                && !desc.is_empty()
            {
                f.insert("description".to_string(), Value::String(desc.to_string()));
            }
            if let Some(params) = tool.pointer("/function/parameters")
                && !params.is_null()
            {
                f.insert("parameters".to_string(), params.clone());
            }
            funcs.push(Value::Object(f));
        }
        if !funcs.is_empty() {
            gemini_obj.insert(
                "tools".to_string(),
                serde_json::json!([{ "functionDeclarations": funcs }]),
            );
        }
    }

    // `tool_choice`: 将 OpenAI "auto"/"none"/"required" 或指定函数映射到 Gemini 的 `toolConfig.functionCallingConfig`
    if let Some(choice) = val.get("tool_choice") {
        let mut cfg = serde_json::Map::new();
        match choice {
            Value::String(s) => {
                let mode = match s.as_str() {
                    "none" => "NONE",
                    "required" => "ANY",
                    _ => "AUTO",
                };
                cfg.insert("mode".to_string(), Value::String(mode.to_string()));
            }
            Value::Object(o) => {
                if let Some(fname) = o
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                {
                    cfg.insert("mode".to_string(), Value::String("ANY".to_string()));
                    cfg.insert("allowedFunctionNames".to_string(), serde_json::json!([fname]));
                }
            }
            _ => {}
        }
        if !cfg.is_empty() {
            gemini_obj.insert(
                "toolConfig".to_string(),
                serde_json::json!({ "functionCallingConfig": cfg }),
            );
        }
    }

    let mut gen_config = serde_json::Map::new();
    if let Some(temp) = val.get("temperature") {
        gen_config.insert("temperature".to_string(), temp.clone());
    }
    if let Some(top_p) = val.get("top_p") {
        gen_config.insert("topP".to_string(), top_p.clone());
    }
    if let Some(max_tokens) = val
        .get("max_tokens")
        .or_else(|| val.get("max_completion_tokens"))
    {
        gen_config.insert("maxOutputTokens".to_string(), max_tokens.clone());
    }
    if !gen_config.is_empty() {
        gemini_obj.insert("generationConfig".to_string(), Value::Object(gen_config));
    }

    Value::Object(gemini_obj)
}

fn gemini_finish_reason(reason: &str) -> &str {
    match reason {
        "MAX_TOKENS" => "length",
        "STOP" => "stop",
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

/// 将单个 Gemini 原生 SSE 事件转换为 OpenAI Chat Completion SSE chunk 行。
pub fn gemini_sse_event_to_openai_chunk(
    event_str: &str,
    model_name: &str,
    created: i64,
    chatcmpl_id: &str,
) -> Option<String> {
    let data = event_str
        .lines()
        .find_map(|line| line.trim().strip_prefix("data:").map(str::trim))?;
    let val: Value = serde_json::from_str(data).ok()?;
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
                "index": 0,
                "id": format!("call_{}", nanoid::nanoid!(20)),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string())
                }
            }))
        })
        .collect();

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

pub fn calculate_cost(prompt_tokens: i64, completion_tokens: i64, price: Option<&ModelPrice>) -> f64 {
    match price {
        Some(p) => {
            // 价格按每 1M token 计算
            let input_cost = (prompt_tokens as f64 / 1_000_000.0) * p.input;
            let output_cost = (completion_tokens as f64 / 1_000_000.0) * p.output;
            input_cost + output_cost
        }
        None => 0.0,
    }
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

        let total_tokens = usage_obj
            .get("total_tokens")
            .or_else(|| usage_obj.get("totalTokenCount"))
            .and_then(|v| v.as_i64())
            .unwrap_or(prompt_tokens + completion_tokens);

        return Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
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
        let total_tokens = usage_obj
            .get("totalTokenCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(prompt_tokens + completion_tokens);
        return Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_openai_to_anthropic() {
        let openai_req = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "messages": [
                { "role": "system", "content": "You are a helpful assistant" },
                { "role": "user", "content": "Hello world" }
            ],
            "temperature": 0.7,
            "max_tokens": 2048
        });

        let anthropic_req = convert_openai_to_anthropic(&openai_req);
        assert_eq!(anthropic_req.get("model").unwrap(), "claude-3-5-sonnet");
        assert_eq!(anthropic_req.get("system").unwrap(), "You are a helpful assistant");
        assert_eq!(anthropic_req.get("max_tokens").unwrap(), 2048);

        let messages = anthropic_req.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].get("role").unwrap(), "user");
        assert_eq!(messages[0].get("content").unwrap(), "Hello world");
    }

    #[test]
    fn test_convert_openai_to_gemini() {
        let openai_req = serde_json::json!({
            "model": "gemini-1.5-flash",
            "messages": [
                { "role": "system", "content": "Think step by step" },
                { "role": "user", "content": "Hi" }
            ],
            "temperature": 0.5
        });

        let gemini_req = convert_openai_to_gemini(&openai_req);
        assert!(gemini_req.get("system_instruction").is_some());
        let contents = gemini_req.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].get("role").unwrap(), "user");
    }

    #[test]
    fn test_convert_anthropic_tool_calls() {
        let openai_req = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "messages": [
                { "role": "user", "content": "What's the weather in Paris?" },
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": { "name": "get_weather", "arguments": "{\"city\":\"Paris\"}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "call_abc", "content": "23°C sunny" }
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
                }
            }],
            "tool_choice": "auto"
        });

        let anthropic = convert_openai_to_anthropic(&openai_req);
        let tools = anthropic.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools[0].get("name").unwrap(), "get_weather");
        assert!(tools[0].get("input_schema").is_some());
        assert_eq!(anthropic.get("tool_choice").unwrap().get("type").unwrap(), "auto");

        let messages = anthropic.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        // assistant -> tool_use 块
        let assistant = messages[1].as_object().unwrap();
        let blocks = assistant.get("content").unwrap().as_array().unwrap();
        assert_eq!(blocks[0].get("type").unwrap(), "tool_use");
        assert_eq!(blocks[0].get("id").unwrap(), "call_abc");
        assert_eq!(blocks[0].get("name").unwrap(), "get_weather");
        assert_eq!(blocks[0].pointer("/input/city").unwrap(), "Paris");
        // tool 角色 -> 携带 tool_result 的 user 消息
        let tool_msg = messages[2].as_object().unwrap();
        assert_eq!(tool_msg.get("role").unwrap(), "user");
        let result = tool_msg.get("content").unwrap().as_array().unwrap();
        assert_eq!(result[0].get("type").unwrap(), "tool_result");
        assert_eq!(result[0].get("tool_use_id").unwrap(), "call_abc");
    }

    #[test]
    fn test_convert_anthropic_consecutive_tool_and_user_calls() {
        let openai_req = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "messages": [
                { "role": "user", "content": "What's the weather in Paris and London?" },
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": { "name": "get_weather", "arguments": "{\"city\":\"Paris\"}" }
                        },
                        {
                            "id": "call_2",
                            "type": "function",
                            "function": { "name": "get_weather", "arguments": "{\"city\":\"London\"}" }
                        }
                    ]
                },
                { "role": "tool", "tool_call_id": "call_1", "content": "23°C sunny" },
                { "role": "tool", "tool_call_id": "call_2", "content": "15°C rainy" }
            ]
        });

        let anthropic = convert_openai_to_anthropic(&openai_req);
        let messages = anthropic.get("messages").unwrap().as_array().unwrap();
        // 必须恰好 3 条消息：user -> assistant -> user（已合并 tool 结果）
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].get("role").unwrap(), "user");
        assert_eq!(messages[1].get("role").unwrap(), "assistant");
        assert_eq!(messages[2].get("role").unwrap(), "user");

        let tool_msg_content = messages[2].get("content").unwrap().as_array().unwrap();
        assert_eq!(tool_msg_content.len(), 2);
        assert_eq!(tool_msg_content[0].get("type").unwrap(), "tool_result");
        assert_eq!(tool_msg_content[0].get("tool_use_id").unwrap(), "call_1");
        assert_eq!(tool_msg_content[1].get("type").unwrap(), "tool_result");
        assert_eq!(tool_msg_content[1].get("tool_use_id").unwrap(), "call_2");
    }

    #[test]
    fn test_convert_gemini_tool_calls() {
        let openai_req = serde_json::json!({
            "model": "gemini-1.5-flash",
            "messages": [
                { "role": "user", "content": "What's the weather in Paris?" },
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": { "name": "get_weather", "arguments": "{\"city\":\"Paris\"}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "call_abc", "content": "23°C sunny" }
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
                }
            }],
            "tool_choice": "required"
        });

        let gemini = convert_openai_to_gemini(&openai_req);
        let tools = gemini.get("tools").unwrap().as_array().unwrap();
        let decls = tools[0]
            .get("functionDeclarations")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(decls[0].get("name").unwrap(), "get_weather");

        let cfg = gemini
            .get("toolConfig")
            .unwrap()
            .pointer("/functionCallingConfig/mode")
            .unwrap();
        assert_eq!(cfg, "ANY");

        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        // user -> model(functionCall) -> user(functionResponse)
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[1].get("role").unwrap(), "model");
        let model_parts = contents[1].get("parts").unwrap().as_array().unwrap();
        assert_eq!(model_parts[0].pointer("/functionCall/name").unwrap(), "get_weather");
        assert_eq!(model_parts[0].pointer("/functionCall/args/city").unwrap(), "Paris");
        // tool 消息映射为 functionResponse，函数名取自解析出的映射
        let user_parts = contents[2].get("parts").unwrap().as_array().unwrap();
        assert_eq!(user_parts[0].pointer("/functionResponse/name").unwrap(), "get_weather");
    }

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
        let chunk = gemini_sse_event_to_openai_chunk(event, "gemini-2.5-pro", 1700000000, "chatcmpl-test").unwrap();
        assert!(chunk.contains("chat.completion.chunk"));
        assert!(chunk.contains("Hello"));
    }
    #[test]
    fn test_convert_anthropic_vision() {
        let openai_req = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "What is in this image?" },
                    { "type": "image_url", "image_url": { "url": "https://example.com/a.png" } }
                ]
            }]
        });

        let anthropic = convert_openai_to_anthropic(&openai_req);
        let messages = anthropic.get("messages").unwrap().as_array().unwrap();
        let content = messages[0].get("content").unwrap().as_array().unwrap();
        assert_eq!(content[0].get("type").unwrap(), "text");
        assert_eq!(content[1].get("type").unwrap(), "image");
        assert_eq!(content[1].pointer("/source/type").unwrap(), "url");
        assert_eq!(content[1].pointer("/source/url").unwrap(), "https://example.com/a.png");
    }

    #[test]
    fn test_convert_gemini_vision() {
        let openai_req = serde_json::json!({
            "model": "gemini-1.5-flash",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "What is in this image?" },
                    {
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,iVBORw0KGgo=" }
                    }
                ]
            }]
        });

        let gemini = convert_openai_to_gemini(&openai_req);
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        let parts = contents[0].get("parts").unwrap().as_array().unwrap();
        assert_eq!(parts[1].pointer("/inlineData/mimeType").unwrap(), "image/png");
        assert_eq!(parts[1].pointer("/inlineData/data").unwrap(), "iVBORw0KGgo=");
    }

    #[test]
    fn test_convert_responses_to_chat_completions() {
        let responses_req = serde_json::json!({
            "model": "deepseek-v4-flash",
            "include": ["reasoning.encrypted_content"],
            "tools": [{
                "type": "function",
                "name": "read_file",
                "description": "Read a file",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                },
                "strict": false
            }],
            "tool_choice": { "type": "function", "name": "read_file" },
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "Hello world" }]
            }]
        });

        let chat = convert_responses_to_chat_completions(&responses_req);
        assert!(chat.get("include").is_none());
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].get("role").unwrap(), "user");
        assert_eq!(messages[0].get("content").unwrap(), "Hello world");
        assert_eq!(chat.pointer("/tools/0/function/name").unwrap(), "read_file");
        assert_eq!(chat.pointer("/tools/0/function/parameters/type").unwrap(), "object");
        assert_eq!(chat.pointer("/tool_choice/function/name").unwrap(), "read_file");
        assert!(chat.pointer("/tools/0/name").is_none());
    }

    #[test]
    fn test_convert_responses_tool_calls_aggregate_into_single_assistant() {
        let responses_req = serde_json::json!({
            "model": "test-model",
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "Hi" }] },
                { "type": "message", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "Let me check." }] },
                { "type": "function_call", "call_id": "call_1", "name": "read_file",
                  "arguments": "{\"path\":\"/etc/hosts\"}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "127.0.0.1 localhost" }
            ]
        });
        let chat = convert_responses_to_chat_completions(&responses_req);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        // user + 单条 assistant（text + tool_calls）+ tool
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].get("role").unwrap(), "user");
        assert_eq!(messages[1].get("role").unwrap(), "assistant");
        assert_eq!(messages[1].get("content").unwrap(), "Let me check.");
        assert_eq!(messages[1].pointer("/tool_calls/0/function/name").unwrap(), "read_file");
        assert_eq!(messages[1].pointer("/tool_calls/0/id").unwrap(), "call_1");
        assert_eq!(messages[2].get("role").unwrap(), "tool");
        assert_eq!(messages[2].get("tool_call_id").unwrap(), "call_1");
        assert_eq!(messages[2].get("content").unwrap(), "127.0.0.1 localhost");
    }

    #[test]
    fn test_convert_responses_missing_call_id_generates_stable_fallback() {
        let responses_req = serde_json::json!({
            "model": "test-model",
            "input": [
                { "type": "message", "role": "assistant", "content": [] },
                { "type": "function_call", "id": "fc_abc", "name": "read_file",
                  "arguments": "{\"path\":\"/etc/hosts\"}" },
                { "type": "function_call_output", "id": "fc_abc", "output": { "ok": true } }
            ]
        });
        let chat = convert_responses_to_chat_completions(&responses_req);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        let call_id = messages[0]
            .pointer("/tool_calls/0/id")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(!call_id.is_empty());
        assert_eq!(messages[1].get("tool_call_id").unwrap(), call_id);
        // 对象输出应序列化为 JSON 字符串
        let content = messages[1].get("content").unwrap().as_str().unwrap();
        assert!(content.contains("\"ok\""));
    }

    #[test]
    fn test_convert_chat_completions_to_responses() {
        let chat_req = serde_json::json!({
            "model": "gpt-5.6-sol",
            "messages": [{
                "role": "user",
                "content": "Hello world"
            }]
        });

        let responses = convert_chat_completions_to_responses(&chat_req);
        assert!(responses.get("messages").is_none());
        let input = responses.get("input").unwrap().as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].get("role").unwrap(), "user");
        assert_eq!(input[0].get("content").unwrap(), "Hello world");
    }

    #[test]
    fn test_anthropic_sse_event_to_openai_chunk() {
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n";
        let chunk = anthropic_sse_event_to_openai_chunk(event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test");
        assert!(chunk.is_some());
        let chunk_str = chunk.unwrap();
        assert!(chunk_str.starts_with("data: "));
        assert!(chunk_str.contains("Hello"));
        assert!(chunk_str.contains("chatcmpl-test"));

        let stop_event = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let stop_chunk =
            anthropic_sse_event_to_openai_chunk(stop_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test");
        assert_eq!(stop_chunk, Some("data: [DONE]\n\n".to_string()));
    }

    #[test]
    fn test_anthropic_sse_event_to_openai_chunk_tool_calls() {
        let start_event = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_test123\",\"name\":\"get_weather\",\"input\":{}}}\n\n";
        let chunk1 =
            anthropic_sse_event_to_openai_chunk(start_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test").unwrap();
        assert!(chunk1.contains("call_test123"));
        assert!(chunk1.contains("get_weather"));

        let delta_event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}\n\n";
        let chunk2 =
            anthropic_sse_event_to_openai_chunk(delta_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test").unwrap();
        assert!(chunk2.contains("Paris"));

        let message_delta =
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n";
        let chunk3 =
            anthropic_sse_event_to_openai_chunk(message_delta, "claude-3-5-sonnet", 1700000000, "chatcmpl-test")
                .unwrap();
        assert!(chunk3.contains("tool_calls"));
    }

    #[test]
    fn test_anthropic_sse_event_message_start_usage() {
        let start_event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":42,\"output_tokens\":0}}}\n\n";
        let chunk =
            anthropic_sse_event_to_openai_chunk(start_event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test").unwrap();
        assert!(chunk.contains("\"prompt_tokens\":42"));
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
    fn test_gemini_sse_event_to_openai_chunk_tool_calls() {
        let gemini_event = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"calc\",\"args\":{\"x\":1}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";
        let chunk =
            gemini_sse_event_to_openai_chunk(gemini_event, "gemini-1.5-flash", 1700000000, "chatcmpl-gemini").unwrap();
        assert!(chunk.contains("calc"));
        assert!(chunk.contains("tool_calls"));
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
    fn test_convert_anthropic_to_openai_chat_request() {
        let anthropic = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "system": "Follow instructions",
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "Hello" }]
            }],
            "tools": [{
                "name": "read_file",
                "description": "Read a file",
                "input_schema": { "type": "object", "properties": { "path": { "type": "string" } } }
            }]
        });
        let openai = convert_anthropic_to_openai_chat_request(&anthropic);
        assert_eq!(openai.pointer("/messages/0/role").unwrap(), "system");
        assert_eq!(openai.pointer("/messages/1/content").unwrap(), "Hello");
        assert_eq!(openai.pointer("/tools/0/function/name").unwrap(), "read_file");
        assert_eq!(openai.pointer("/tools/0/function/parameters/type").unwrap(), "object");
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
}
