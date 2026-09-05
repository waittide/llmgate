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

/// 将 messages 中纯文本的 content 数组扁平化为字符串；含多模态部分（图片等）的数组保持原样。
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
        let mut all_text = true;
        for part in parts {
            match part.get("type").and_then(Value::as_str) {
                Some("text") | Some("input_text") | Some("output_text") => {
                    text.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
                }
                _ => {
                    all_text = false;
                    break;
                }
            }
        }
        if all_text && !parts.is_empty() {
            msg_obj.insert("content".to_string(), Value::String(text));
        }
    }
}

/// 将 Chat Completions 的 function tool 转换为 Responses API 的扁平结构。
fn convert_chat_tool_to_responses(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }

    tool.pointer("/function/name")?;

    let mut flat = serde_json::Map::new();
    flat.insert("type".to_string(), Value::String("function".to_string()));
    flat.insert(
        "name".to_string(),
        tool.pointer("/function/name")
            .cloned()
            .unwrap_or(Value::Null),
    );
    if let Some(description) = tool.pointer("/function/description") {
        flat.insert("description".to_string(), description.clone());
    }
    if let Some(parameters) = tool.pointer("/function/parameters") {
        flat.insert("parameters".to_string(), parameters.clone());
    }

    Some(Value::Object(flat))
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
                                let mut text = String::new();
                                let mut text_only = true;
                                for part in parts {
                                    let ptype = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    match ptype {
                                        "input_text" | "text" | "output_text" => {
                                            text.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
                                        }
                                        "input_image" | "image_url" => {
                                            text_only = false;
                                            if let Some(url) = part.get("image_url") {
                                                converted_parts.push(serde_json::json!({
                                                    "type": "image_url",
                                                    "image_url": url
                                                }));
                                            }
                                        }
                                        _ => {
                                            // 未知 part 类型原样保留，避免内容静默丢失
                                            text_only = false;
                                            converted_parts.push(part.clone());
                                        }
                                    }
                                }
                                if text_only {
                                    Value::String(text)
                                } else {
                                    let mut merged = Vec::new();
                                    if !text.is_empty() {
                                        merged.push(serde_json::json!({ "type": "text", "text": text }));
                                    }
                                    merged.extend(converted_parts);
                                    Value::Array(merged)
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
        // instructions 是 Responses 请求的顶层系统提示，转换为首条 system 消息
        if let Some(instructions) = obj.get("instructions").and_then(Value::as_str) {
            let instructions = instructions.trim();
            if !instructions.is_empty() {
                messages.insert(0, serde_json::json!({ "role": "system", "content": instructions }));
            }
        }
        obj.remove("instructions");
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
    let mut input: Vec<Value> = Vec::new();
    let mut instructions: Option<String> = None;

    if let Some(messages) = obj.remove("messages").and_then(|v| v.as_array().cloned()) {
        for msg in messages {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
            match role {
                // system 消息并入 instructions（Responses 的系统提示形态）
                "system" => {
                    let text = msg.get("content").and_then(Value::as_str).unwrap_or("");
                    if !text.is_empty() {
                        instructions = Some(match instructions {
                            Some(existing) => format!("{existing}\n\n{text}"),
                            None => text.to_string(),
                        });
                    }
                }
                // 工具结果 → function_call_output item
                "tool" => {
                    let call_id = msg
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let content = msg
                        .get("content")
                        .cloned()
                        .unwrap_or(Value::String(String::new()));
                    let output = match content {
                        Value::String(s) => Value::String(s),
                        other => Value::String(serde_json::to_string(&other).unwrap_or_default()),
                    };
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output
                    }));
                }
                _ => {
                    // content 归一：user 方向统一 input_text part；assistant 保留字符串
                    let is_user = role != "assistant";
                    let message_item = match msg.get("content") {
                        Some(Value::String(text)) if !text.is_empty() => {
                            let content = if is_user {
                                serde_json::json!([{ "type": "input_text", "text": text }])
                            } else {
                                serde_json::json!(text)
                            };
                            Some(serde_json::json!({ "type": "message", "role": role, "content": content }))
                        }
                        Some(Value::Array(parts)) => {
                            let converted: Vec<Value> = parts
                                .iter()
                                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                                    Some("text") | Some("input_text") | Some("output_text") => {
                                        let text_type = if is_user { "input_text" } else { "output_text" };
                                        Some(serde_json::json!({
                                            "type": text_type,
                                            "text": part.get("text").cloned().unwrap_or(Value::String(String::new()))
                                        }))
                                    }
                                    Some("image_url") | Some("input_image") => {
                                        let url = part
                                            .pointer("/image_url/url")
                                            .or_else(|| part.get("image_url"))
                                            .cloned()
                                            .unwrap_or(Value::Null);
                                        Some(serde_json::json!({ "type": "input_image", "image_url": url }))
                                    }
                                    _ => None,
                                })
                                .collect();
                            (!converted.is_empty())
                                .then(|| serde_json::json!({ "type": "message", "role": role, "content": converted }))
                        }
                        _ => None,
                    };
                    // 助手文本在先，tool_calls 剥离为随后的 function_call item，
                    // 保持 Responses 对话流中文本先于其引发的调用这一时序
                    if let Some(item) = message_item {
                        input.push(item);
                    }
                    if role == "assistant"
                        && let Some(calls) = msg.get("tool_calls").and_then(Value::as_array)
                    {
                        for call in calls {
                            input.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": call.get("id").and_then(Value::as_str).unwrap_or(""),
                                "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or(""),
                                "arguments": call.pointer("/function/arguments").cloned()
                                    .unwrap_or(Value::String("{}".to_string()))
                            }));
                        }
                    }
                }
            }
        }
    }

    // Chat 嵌套 tools/function 结构扁平化为 Responses 形态，否则 Responses 上游 400 校验失败
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        *tools = tools
            .iter()
            .filter_map(convert_chat_tool_to_responses)
            .collect();
    }
    if let Some(choice) = obj.get_mut("tool_choice")
        && let Some(name) = choice.pointer("/function/name").and_then(Value::as_str)
    {
        *choice = serde_json::json!({ "type": "function", "name": name });
    }

    if let Some(instructions) = instructions {
        obj.insert("instructions".to_string(), Value::String(instructions));
    }
    obj.insert("input".to_string(), Value::Array(input));
    Value::Object(obj)
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
        let val: Value = serde_json::from_str(data_str).ok()?;

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
            } else if let Some(thinking) = val.pointer("/delta/thinking").and_then(|t| t.as_str()) {
                // 扩展思考的 thinking_delta 转发为 reasoning_content（社区事实标准）：
                // 既让支持该字段的客户端渲染思考过程，也保证转换流持续有字节流动，
                // 避免 StreamFinalizer 的停滞超时误杀长思考生成。
                let chunk = serde_json::json!({
                    "id": chatcmpl_id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "delta": { "reasoning_content": thinking },
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

/// 将 Gemini generateContent 请求载荷转换为 OpenAI Chat Completions 请求载荷。
pub fn convert_gemini_to_openai_chat_request(val: &Value) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(model) = val.get("model") {
        obj.insert("model".to_string(), model.clone());
    }

    let mut messages: Vec<Value> = Vec::new();
    // systemInstruction → system 消息
    if let Some(sys) = val
        .get("system_instruction")
        .or_else(|| val.get("systemInstruction"))
    {
        let text: String = sys
            .pointer("/parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if !text.is_empty() {
            messages.push(serde_json::json!({ "role": "system", "content": text }));
        }
    }

    // Gemini 的 functionResponse 只带函数名；记录 functionCall 生成的 call_id，
    // 让对应 tool 消息能引用同一 id（OpenAI 上游强校验 tool_call_id 匹配）
    let mut call_ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(contents) = val.get("contents").and_then(Value::as_array) {
        for content in contents {
            let role = content
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let parts = content.get("parts").and_then(Value::as_array);
            let mut text_parts: Vec<Value> = Vec::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            for part in parts.into_iter().flatten() {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    text_parts.push(serde_json::json!({ "type": "text", "text": text }));
                } else if let Some(call) = part.get("functionCall") {
                    let name = call.get("name").and_then(Value::as_str).unwrap_or("");
                    let args = call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    let call_id = format!("call_{}", nanoid::nanoid!(20));
                    call_ids.insert(name.to_string(), call_id.clone());
                    tool_calls.push(serde_json::json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
                        }
                    }));
                } else if let Some(resp) = part.get("functionResponse") {
                    let name = resp.get("name").and_then(Value::as_str).unwrap_or("");
                    let call_id = call_ids
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| format!("call_{}", nanoid::nanoid!(20)));
                    let content = match resp.get("response") {
                        Some(Value::String(s)) => Value::String(s.clone()),
                        Some(other) => Value::String(serde_json::to_string(other).unwrap_or_default()),
                        None => Value::String(String::new()),
                    };
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": content
                    }));
                } else if let Some(image) = part
                    .get("inlineData")
                    .or_else(|| part.get("inline_data"))
                    .or_else(|| part.get("fileData"))
                    .or_else(|| part.get("file_data"))
                {
                    let url = if let Some(data) = image.get("data").and_then(Value::as_str) {
                        format!(
                            "data:{};base64,{}",
                            image
                                .get("mimeType")
                                .and_then(Value::as_str)
                                .unwrap_or("image/png"),
                            data
                        )
                    } else {
                        image
                            .get("fileUri")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    };
                    if !url.is_empty() {
                        text_parts.push(serde_json::json!({ "type": "image_url", "image_url": { "url": url } }));
                    }
                }
            }
            let chat_role = if role == "model" { "assistant" } else { role };
            if !text_parts.is_empty() || !tool_calls.is_empty() {
                let content =
                    if text_parts.len() == 1 && text_parts[0].get("type").and_then(Value::as_str) == Some("text") {
                        text_parts[0].get("text").cloned().unwrap_or(Value::Null)
                    } else if text_parts.is_empty() && !tool_calls.is_empty() {
                        Value::Null
                    } else {
                        Value::Array(text_parts)
                    };
                let mut message = serde_json::json!({ "role": chat_role, "content": content });
                if !tool_calls.is_empty() {
                    message["tool_calls"] = Value::Array(tool_calls);
                }
                messages.push(message);
            }
        }
    }
    obj.insert("messages".to_string(), Value::Array(messages));

    // generationConfig → temperature/top_p/max_tokens 等
    if let Some(config) = val
        .get("generation_config")
        .or_else(|| val.get("generationConfig"))
    {
        if let Some(temp) = config.get("temperature") {
            obj.insert("temperature".to_string(), temp.clone());
        }
        if let Some(top_p) = config.get("topP").or_else(|| config.get("top_p")) {
            obj.insert("top_p".to_string(), top_p.clone());
        }
        if let Some(max) = config
            .get("maxOutputTokens")
            .or_else(|| config.get("max_output_tokens"))
        {
            obj.insert("max_tokens".to_string(), max.clone());
        }
        if let Some(stop) = config
            .get("stopSequences")
            .or_else(|| config.get("stop_sequences"))
        {
            obj.insert("stop".to_string(), stop.clone());
        }
        if let Some(freq) = config
            .get("frequencyPenalty")
            .or_else(|| config.get("frequency_penalty"))
        {
            obj.insert("frequency_penalty".to_string(), freq.clone());
        }
        if let Some(pres) = config
            .get("presencePenalty")
            .or_else(|| config.get("presence_penalty"))
        {
            obj.insert("presence_penalty".to_string(), pres.clone());
        }
        if let Some(seed) = config.get("seed") {
            obj.insert("seed".to_string(), seed.clone());
        }
    }

    // tools.functionDeclarations → Chat 嵌套 function 结构
    if let Some(tools) = val.get("tools").and_then(Value::as_array) {
        let mut chat_tools = Vec::new();
        for tool in tools {
            if let Some(decls) = tool.get("functionDeclarations").and_then(Value::as_array) {
                for decl in decls {
                    let mut function = serde_json::Map::new();
                    if let Some(name) = decl.get("name") {
                        function.insert("name".to_string(), name.clone());
                    }
                    if let Some(desc) = decl.get("description") {
                        function.insert("description".to_string(), desc.clone());
                    }
                    if let Some(params) = decl.get("parameters") {
                        function.insert("parameters".to_string(), params.clone());
                    }
                    chat_tools.push(serde_json::json!({ "type": "function", "function": function }));
                }
            }
        }
        if !chat_tools.is_empty() {
            obj.insert("tools".to_string(), Value::Array(chat_tools));
        }
    }

    // toolConfig.functionCallingConfig → tool_choice
    if let Some(config) = val
        .pointer("/toolConfig/functionCallingConfig")
        .or_else(|| val.pointer("/tool_config/functionCallingConfig"))
    {
        let mode = config.get("mode").and_then(Value::as_str).unwrap_or("AUTO");
        let choice = match mode {
            "NONE" => serde_json::json!("none"),
            "ANY" => match config
                .get("allowedFunctionNames")
                .and_then(Value::as_array)
                .and_then(|names| names.first())
            {
                Some(name) => serde_json::json!({
                    "type": "function",
                    "function": { "name": name }
                }),
                None => serde_json::json!("required"),
            },
            _ => serde_json::json!("auto"),
        };
        obj.insert("tool_choice".to_string(), choice);
    }

    Value::Object(obj)
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
                    // Gemini 规定 functionResponse.response 必须是 JSON Object：
                    // 字符串或非对象内容统一包装为 {"result": ...}，避免上游 400 校验失败
                    let raw = msg
                        .get("content")
                        .cloned()
                        .unwrap_or_else(|| Value::String(String::new()));
                    let response = match raw {
                        response @ Value::Object(_) => response,
                        Value::String(text) => match serde_json::from_str::<Value>(&text) {
                            Ok(parsed @ Value::Object(_)) => parsed,
                            Ok(parsed) => serde_json::json!({ "result": parsed }),
                            Err(_) => serde_json::json!({ "result": text }),
                        },
                        other => serde_json::json!({ "result": other }),
                    };
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
    // 多个 functionCall 在同一 chunk 内按出现顺序分配从 0 递增的 index；
    // id 由 (name, args) 哈希确定性生成，同一调用跨 chunk 内容一致则 id 稳定
    let mut tool_calls: Vec<Value> = Vec::new();
    if let Some(parts) = parts {
        for part in parts {
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
            let index = tool_calls.len() as u64;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            name.hash(&mut hasher);
            arguments_json.hash(&mut hasher);
            tool_calls.push(serde_json::json!({
                "index": index,
                "id": format!("call_{:016x}", hasher.finish()),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments_json
                }
            }));
        }
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
    fn test_convert_gemini_function_response_object() {
        let openai_req = serde_json::json!({
            "model": "gemini-2.5-pro",
            "messages": [
                { "role": "user", "content": "Check weather" },
                { "role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_1", "type": "function",
                    "function": { "name": "get_weather", "arguments": "{\"city\":\"Paris\"}" }
                }] },
                { "role": "tool", "tool_call_id": "call_1", "content": "24°C, sunny" }
            ]
        });
        let gemini = convert_openai_to_gemini(&openai_req);
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        let resp = contents[2]
            .pointer("/parts/0/functionResponse/response")
            .unwrap();
        // 字符串工具结果必须包装为 JSON Object，否则 Gemini 报 400
        assert!(resp.is_object());
        assert_eq!(resp.get("result").unwrap(), "24°C, sunny");
    }

    #[test]
    fn test_convert_gemini_function_response_json_passthrough() {
        let openai_req = serde_json::json!({
            "model": "gemini-2.5-pro",
            "messages": [
                { "role": "user", "content": "Check weather" },
                { "role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_1", "type": "function",
                    "function": { "name": "get_weather", "arguments": "{\"city\":\"Paris\"}" }
                }] },
                { "role": "tool", "tool_call_id": "call_1", "content": "{\"temperature_c\": 24, \"condition\": \"sunny\"}" }
            ]
        });
        let gemini = convert_openai_to_gemini(&openai_req);
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        let resp = contents[2]
            .pointer("/parts/0/functionResponse/response")
            .unwrap();
        assert_eq!(resp.get("temperature_c").unwrap(), 24);
        assert_eq!(resp.get("condition").unwrap(), "sunny");
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
        // user content 归一为 input_text part 数组（Responses 规范形态）
        assert_eq!(input[0].pointer("/content/0/type").unwrap(), "input_text");
        assert_eq!(input[0].pointer("/content/0/text").unwrap(), "Hello world");
    }

    #[test]
    fn test_convert_chat_to_responses_assistant_text_before_calls() {
        // 同一条 assistant 消息既有文本又有 tool_calls 时，文本 item 必须先于 function_call item
        let chat = serde_json::json!({
            "model": "m",
            "messages": [
                { "role": "assistant", "content": "让我查一下", "tool_calls": [
                    { "id": "call_1", "type": "function",
                      "function": { "name": "bash", "arguments": "{\"cmd\":\"ls\"}" } }
                ]}
            ]
        });
        let responses = convert_chat_completions_to_responses(&chat);
        let input = responses.get("input").unwrap().as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0].get("type").unwrap(), "message");
        // assistant 字符串 content 保留原样
        assert_eq!(input[0].pointer("/content").unwrap(), "让我查一下");
        assert_eq!(input[1].get("type").unwrap(), "function_call");
        assert_eq!(input[1].get("name").unwrap(), "bash");
    }

    #[test]
    fn test_convert_chat_tools_to_responses_flat_structure() {
        let chat_req = serde_json::json!({
            "model": "gpt-5.6-sol",
            "messages": [{ "role": "user", "content": "Weather?" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": { "type": "object", "properties": { "city": { "type": "string" } } }
                }
            }],
            "tool_choice": { "type": "function", "function": { "name": "get_weather" } }
        });
        let responses = convert_chat_completions_to_responses(&chat_req);
        // tools 必须扁平化为 Responses 形态：{type, name, description, parameters}
        let tools = responses.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.get("type").unwrap(), "function");
        assert_eq!(tool.get("name").unwrap(), "get_weather");
        assert!(tool.get("function").is_none());
        assert_eq!(tool.pointer("/parameters/properties/city/type").unwrap(), "string");
        // tool_choice 同步扁平化：{"type":"function","name":...}
        let choice = responses.get("tool_choice").unwrap();
        assert_eq!(choice.get("type").unwrap(), "function");
        assert_eq!(choice.get("name").unwrap(), "get_weather");
        assert!(choice.get("function").is_none());
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
    fn test_anthropic_sse_event_to_openai_chunk_thinking() {
        let event = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"thinking hard\"}}\n\n";
        let chunk = anthropic_sse_event_to_openai_chunk(event, "claude-3-5-sonnet", 1700000000, "chatcmpl-test");
        assert!(chunk.is_some());
        let chunk_str = chunk.unwrap();
        assert!(chunk_str.contains("\"reasoning_content\":\"thinking hard\""));
        assert!(!chunk_str.contains("\"content\""));
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
    }

    #[test]
    fn test_convert_chat_to_responses_full_mapping() {
        let chat = serde_json::json!({
            "model": "m",
            "messages": [
                { "role": "system", "content": "系统提示" },
                { "role": "user", "content": "你好" },
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "call_1", "type": "function",
                      "function": { "name": "bash", "arguments": "{\"cmd\":\"ls\"}" } }
                ]},
                { "role": "tool", "tool_call_id": "call_1", "content": "输出" }
            ]
        });
        let responses = convert_chat_completions_to_responses(&chat);
        // system 并入 instructions，而非 input item
        assert_eq!(responses.get("instructions").unwrap(), "系统提示");
        assert!(responses.get("messages").is_none());
        let input = responses.get("input").unwrap().as_array().unwrap();
        assert_eq!(input.len(), 3);
        // user 字符串 content → input_text part 数组
        assert_eq!(input[0].get("role").unwrap(), "user");
        assert_eq!(input[0].pointer("/content/0/type").unwrap(), "input_text");
        assert_eq!(input[0].pointer("/content/0/text").unwrap(), "你好");
        // assistant.tool_calls → function_call item
        assert_eq!(input[1].get("type").unwrap(), "function_call");
        assert_eq!(input[1].get("call_id").unwrap(), "call_1");
        assert_eq!(input[1].get("name").unwrap(), "bash");
        // tool → function_call_output item
        assert_eq!(input[2].get("type").unwrap(), "function_call_output");
        assert_eq!(input[2].get("output").unwrap(), "输出");
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
    fn test_convert_gemini_to_openai_chat_request() {
        let gemini = serde_json::json!({
            "contents": [
                { "role": "user", "parts": [{ "text": "Check weather" }] },
                { "role": "model", "parts": [{ "functionCall": { "name": "get_weather", "args": { "city": "Paris" } } }] },
                { "role": "user", "parts": [{ "functionResponse": { "name": "get_weather", "response": { "temp": 24 } } }] }
            ],
            "systemInstruction": { "parts": [{ "text": "You are helpful" }] },
            "tools": [{ "functionDeclarations": [{ "name": "get_weather", "description": "Get weather", "parameters": { "type": "object" } }] }],
            "generationConfig": { "temperature": 0.5, "maxOutputTokens": 128 }
        });
        let chat = convert_gemini_to_openai_chat_request(&gemini);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages[0].get("role").unwrap(), "system");
        assert_eq!(messages[1].pointer("/content").unwrap(), "Check weather");
        assert_eq!(
            messages[2].pointer("/tool_calls/0/function/name").unwrap(),
            "get_weather"
        );
        assert_eq!(messages[3].get("role").unwrap(), "tool");
        assert_eq!(chat.pointer("/tools/0/function/name").unwrap(), "get_weather");
        assert_eq!(chat.get("max_tokens").unwrap(), 128);
    }

    #[test]
    fn test_convert_gemini_tool_response_reuses_call_id_and_unwraps_string() {
        let gemini = serde_json::json!({
            "contents": [
                { "role": "model", "parts": [{ "functionCall": { "name": "get_weather", "args": { "city": "Paris" } } }] },
                { "role": "user", "parts": [{ "functionResponse": { "name": "get_weather", "response": "sunny, 24C" } }] }
            ]
        });
        let chat = convert_gemini_to_openai_chat_request(&gemini);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        let call_id = messages[0]
            .pointer("/tool_calls/0/id")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(
            messages[1]
                .pointer("/tool_call_id")
                .unwrap()
                .as_str()
                .unwrap(),
            call_id
        );
        // 字符串 response 不能被二次 JSON 编码成带引号的转义串
        assert_eq!(messages[1].pointer("/content").unwrap(), "sunny, 24C");
    }

    #[test]
    fn test_convert_gemini_multimodal_and_generation_config() {
        let gemini = serde_json::json!({
            "contents": [
                { "role": "user", "parts": [
                    { "text": "what is this" },
                    { "inlineData": { "mimeType": "image/png", "data": "aGVsbG8=" } }
                ] }
            ],
            "generationConfig": {
                "stopSequences": ["END"],
                "frequencyPenalty": 0.2,
                "presencePenalty": 0.3,
                "seed": 7
            }
        });
        let chat = convert_gemini_to_openai_chat_request(&gemini);
        let parts = chat
            .pointer("/messages/0/content")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            parts[1].pointer("/image_url/url").unwrap(),
            "data:image/png;base64,aGVsbG8="
        );
        assert_eq!(chat.pointer("/stop/0").unwrap(), "END");
        assert_eq!(chat.get("frequency_penalty").unwrap(), 0.2);
        assert_eq!(chat.get("presence_penalty").unwrap(), 0.3);
        assert_eq!(chat.get("seed").unwrap(), 7);
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
        assert_eq!(output[1].get("type").unwrap(), "message");
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
    fn test_gemini_sse_tool_calls_sequential_index_and_stable_id() {
        let gemini_event = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"a\",\"args\":{\"x\":1}}},{\"functionCall\":{\"name\":\"b\",\"args\":{\"y\":2}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";
        let chunk =
            gemini_sse_event_to_openai_chunk(gemini_event, "gemini-1.5-flash", 1700000000, "chatcmpl-gemini").unwrap();
        // 两个调用 index 分别为 0、1，且 id 确定性生成（不等于彼此）
        assert!(chunk.contains("\"index\":0"));
        assert!(chunk.contains("\"index\":1"));
        assert!(!chunk.contains("\"index\":2"));
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

    #[test]
    fn test_convert_responses_merges_text_parts_and_instructions() {
        let responses_req = serde_json::json!({
            "model": "m",
            "instructions": "系统提示",
            "input": [
                { "type": "message", "role": "user", "content": [
                    { "type": "input_text", "text": "part1 " },
                    { "type": "input_text", "text": "part2" }
                ]},
                { "type": "message", "role": "user", "content": [
                    { "type": "input_text", "text": "看图" },
                    { "type": "input_image", "image_url": "https://x/y.png" }
                ]}
            ]
        });
        let chat = convert_responses_to_chat_completions(&responses_req);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].get("role").unwrap(), "system");
        assert_eq!(messages[0].get("content").unwrap(), "系统提示");
        assert_eq!(messages[1].get("content").unwrap(), "part1 part2");
        let parts = messages[2].get("content").unwrap().as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].pointer("/text").unwrap(), "看图");
        // instructions 已消费，不再残留于转发体
        assert!(chat.get("instructions").is_none());
    }
}
