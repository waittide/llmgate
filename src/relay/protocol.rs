use serde::Deserialize;
use serde_json::Value;

use crate::{model::LLMPrice, relay::state::Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenaiCompletion,
    OpenAIResponse,
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

/// Converts a Responses API function tool into Chat Completions' nested shape.
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

/// Converts an OpenAI Responses API payload (containing `input: [...]`)
pub fn convert_responses_to_chat_completions(val: &Value) -> Value {
    let mut obj = match val.as_object() {
        Some(o) => o.clone(),
        None => return val.clone(),
    };

    // Remove Responses API-only fields that cause 400 Bad Request on standard Chat Completions
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
        // Accumulate consecutive assistant output items (message text + function calls)
        // into a single Chat Completions assistant message, so tool calls stay
        // attached to the assistant turn that produced them.
        let mut current_assistant: Option<serde_json::Map<String, Value>> = None;
        // Generated function_call identifiers, keyed by the item's raw id when
        // `call_id` is absent; reused for the matching `function_call_output`.
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
                        // A tool result terminates the current assistant turn.
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
                            // Assistant output text belongs to the current assistant
                            // turn; merge it into the ongoing assistant message so
                            // text and following tool calls stay in one message.
                            let entry = current_assistant.get_or_insert_with(serde_json::Map::new);
                            entry.insert("role".to_string(), Value::String("assistant".to_string()));
                            match entry.get("content") {
                                Some(Value::Null) => {
                                    entry.insert("content".to_string(), content_val);
                                }
                                Some(_) => {
                                    // Append to existing text content.
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
                            // A user message (or non-assistant item) terminates the
                            // current assistant turn.
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

/// Converts an OpenAI Chat Completions payload (containing `messages: [...]`)
/// into an OpenAI Responses payload (containing `input: [...]`).
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

/// Converts a non-streaming Chat Completion response into an OpenAI Responses response.
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

/// Converts a non-streaming Chat Completion response into an Anthropic Message response.
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

/// Converts a non-streaming Responses response into an Anthropic Message response.
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

/// Converts an Anthropic Messages request into an OpenAI Chat Completions request.
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
/// Extracts plain text from an OpenAI `content` value (string or content array).
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

/// Converts an OpenAI `image_url` content part to an Anthropic `image` block.
/// Base64 `data:` URIs become `source.type = "base64"`; public URLs are handed
/// to Anthropic as `source.type = "url"` (Anthropic fetches them server-side).
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

/// Converts an OpenAI `content` value (string or content array with text/image)
/// into Anthropic's `content` shape: a string, a single block, or a block array.
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

/// Converts an OpenAI assistant message (content + optional `tool_calls`) into
/// an Anthropic assistant message with `tool_use` content blocks.
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

/// Converts an OpenAI `tool` role message into an Anthropic `tool_result` content block.
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

/// Convert OpenAI /chat/completions payload to Anthropic /messages payload.
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

    // `tools`: OpenAI wraps each tool as `{type, function:{...}}`; Anthropic uses
    // a flat `{name, description, input_schema}` object.
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

    // `tool_choice`: OpenAI "auto"/"none"/"required" or a function object;
    // Anthropic uses an object with `type: auto|none|any|tool`.
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

/// Converts a non-streaming Anthropic message response into an OpenAI Chat Completion response.
pub fn convert_anthropic_to_openai_chat_response(val: &Value, model_name: &str) -> Value {
    let text = val
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let prompt_tokens = val
        .pointer("/usage/input_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let completion_tokens = val
        .pointer("/usage/output_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    serde_json::json!({
        "id": format!("chatcmpl-{}", nanoid::nanoid!(24)),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text
            },
            "finish_reason": val.get("stop_reason").and_then(|r| r.as_str()).unwrap_or("stop")
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    })
}

/// Translates an Anthropic SSE event string into corresponding OpenAI Chat Completion SSE chunk lines.
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
        "content_block_delta" => {
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
            }
        }
        "message_delta" => {
            let finish_reason = val
                .pointer("/delta/stop_reason")
                .and_then(|s| s.as_str())
                .unwrap_or("stop");
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
            // Preserve Anthropic output_tokens so the StreamFinalizer can recover
            // real completion token counts instead of falling back to estimates.
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

/// Converts an OpenAI `image_url` content part to a Gemini content part. Base64
/// `data:` URIs become `inlineData`; public URLs become `fileData` (Gemini
/// fetches publicly accessible URIs server-side).
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

/// Converts an OpenAI `content` value (string or content array) into Gemini
/// content `parts`.
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

/// Convert OpenAI /chat/completions payload to Google Gemini generateContent payload.
pub fn convert_openai_to_gemini(val: &Value) -> Value {
    let mut contents = Vec::new();
    let mut system_instruction = None;
    // OpenAI `tool` messages only carry `tool_call_id`; remember the mapping to
    // the function name so Gemini `functionResponse` can name the function.
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

    // `tools`: OpenAI `{type:"function", function:{name,...}}` becomes a Gemini
    // `functionDeclarations` entry.
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

    // `tool_choice`: OpenAI "auto"/"none"/"required" or a function object maps to
    // Gemini `toolConfig.functionCallingConfig`.
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

/// Converts a native Gemini generateContent response into an OpenAI Chat Completion response.
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
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }

    let finish_reason = candidate
        .and_then(|c| c.get("finishReason"))
        .and_then(Value::as_str)
        .map(gemini_finish_reason);
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

/// Translates one native Gemini SSE event into an OpenAI Chat Completion SSE chunk.
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
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();
    let finish_reason = candidate
        .get("finishReason")
        .and_then(Value::as_str)
        .map(gemini_finish_reason);

    if text.is_empty() && finish_reason.is_none() {
        return None;
    }

    let mut chunk = serde_json::json!({
        "id": chatcmpl_id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model_name,
        "choices": [{
            "index": 0,
            "delta": if text.is_empty() { serde_json::json!({}) } else { serde_json::json!({ "content": text }) },
            "finish_reason": finish_reason
        }]
    });
    // Preserve Gemini usageMetadata so the StreamFinalizer can recover real token
    // counts instead of falling back to estimates.
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

pub fn calculate_cost(prompt_tokens: i64, completion_tokens: i64, price: Option<&LLMPrice>) -> f64 {
    match price {
        Some(p) => {
            // Price is per 1M tokens
            let input_cost = (prompt_tokens as f64 / 1_000_000.0) * p.input;
            let output_cost = (completion_tokens as f64 / 1_000_000.0) * p.output;
            input_cost + output_cost
        }
        None => 0.0,
    }
}

pub fn extract_usage_from_json(val: &Value) -> Option<Usage> {
    // OpenAI-style `usage` object: prompt_tokens / completion_tokens / total_tokens
    // (also accepted via the Responses aliases input_tokens / output_tokens).
    if let Some(usage_obj) = val.get("usage") {
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

    // Gemini-style top-level `usageMetadata` object.
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
        // assistant -> tool_use block
        let assistant = messages[1].as_object().unwrap();
        let blocks = assistant.get("content").unwrap().as_array().unwrap();
        assert_eq!(blocks[0].get("type").unwrap(), "tool_use");
        assert_eq!(blocks[0].get("id").unwrap(), "call_abc");
        assert_eq!(blocks[0].get("name").unwrap(), "get_weather");
        assert_eq!(blocks[0].pointer("/input/city").unwrap(), "Paris");
        // tool role -> user message with tool_result
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
        // Must be exactly 3 messages: user -> assistant -> user (merged tool results)
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
        // tool message maps to functionResponse with the resolved function name
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
        // user + single assistant (text + tool_calls) + tool
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
        // object output should be serialized to a JSON string
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
