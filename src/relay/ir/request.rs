//! 请求方向的边界转换：4 个 parser（原始格式 → IR）与 4 个 emitter（IR → 目标格式）。

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{ContentPart, ImageSource, Message, RelayRequest, Role, Sampling, Tool, ToolChoice};

// ---------- 通用小工具 ----------

fn as_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn owned_str(v: Option<&Value>) -> Option<String> {
    as_str(v).map(str::to_string)
}

fn non_null(v: Option<&Value>) -> Option<Value> {
    v.filter(|v| !v.is_null()).cloned()
}

/// Chat/Anthropic 的 stop（字符串或数组）→ IR 停止序列。
fn parse_stop(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// 将 system 文本块以 separator 连接（跳过空段）。
fn join_system_text(parts: &[ContentPart], sep: &str) -> String {
    let texts: Vec<&str> = parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text, .. } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.join(sep)
}

/// 工具结果统一渲染为字符串（Chat/Responses 目标口径）。
fn render_result_string(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// 解析 Chat 形态的 arguments（JSON 字符串）为 Value，失败时保留原始字符串。
fn arguments_from_string(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone())),
        Some(other) => other.clone(),
        None => json!({}),
    }
}

// ---------- Parsers ----------

pub(super) fn parse_chat(val: &Value) -> Option<RelayRequest> {
    let obj = val.as_object()?;
    let mut sampling = Sampling {
        temperature: obj.get("temperature").cloned().filter(|v| !v.is_null()),
        top_p: obj.get("top_p").cloned().filter(|v| !v.is_null()),
        max_tokens: obj
            .get("max_tokens")
            .or_else(|| obj.get("max_completion_tokens"))
            .cloned()
            .filter(|v| !v.is_null()),
        stop: parse_stop(obj.get("stop")),
        seed: obj.get("seed").cloned().filter(|v| !v.is_null()),
        frequency_penalty: obj
            .get("frequency_penalty")
            .cloned()
            .filter(|v| !v.is_null()),
        presence_penalty: obj
            .get("presence_penalty")
            .cloned()
            .filter(|v| !v.is_null()),
        reasoning_effort: owned_str(obj.get("reasoning_effort")),
        verbosity: owned_str(obj.get("verbosity")),
        ..Default::default()
    };
    if let Some(top_k) = non_null(obj.get("top_k")) {
        // Chat 无 top_k；来源若为 Gemini 转换载荷可能携带，保留给支持的 emitter
        sampling.top_k = Some(top_k);
    }
    let mut req = RelayRequest {
        model: owned_str(obj.get("model")).unwrap_or_default(),
        stream: obj.get("stream").and_then(Value::as_bool).unwrap_or(false),
        sampling,
        parallel_tool_calls: obj.get("parallel_tool_calls").and_then(Value::as_bool),
        store: obj.get("store").and_then(Value::as_bool),
        service_tier: non_null(obj.get("service_tier")),
        metadata: non_null(obj.get("metadata")),
        safety_identifier: owned_str(obj.get("safety_identifier")),
        prompt_cache_key: owned_str(obj.get("prompt_cache_key")),
        user: owned_str(obj.get("user")),
        ..Default::default()
    };

    // function 名称映射：tool 消息只有 tool_call_id，Gemini functionResponse 需要函数名
    let mut call_names: HashMap<String, String> = HashMap::new();
    if let Some(messages) = obj.get("messages").and_then(Value::as_array) {
        for msg in messages {
            let role_str = as_str(msg.get("role")).unwrap_or("user");
            if role_str == "system" {
                push_system_text(&mut req.system, msg.get("content"));
                continue;
            }
            let role = match role_str {
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let mut parts = parse_chat_content(msg.get("content"));
            if role == Role::Assistant {
                if let Some(reasoning) = as_str(msg.get("reasoning_content")).filter(|s| !s.is_empty()) {
                    parts.insert(
                        0,
                        ContentPart::Thinking {
                            text:      reasoning.to_string(),
                            signature: None,
                        },
                    );
                }
                if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let id = owned_str(call.get("id")).unwrap_or_default();
                        let name = owned_str(call.pointer("/function/name")).unwrap_or_default();
                        let arguments = arguments_from_string(call.pointer("/function/arguments"));
                        call_names.insert(id.clone(), name.clone());
                        parts.push(ContentPart::ToolCall { id, name, arguments });
                    }
                }
            }
            if role == Role::Tool {
                let call_id = owned_str(msg.get("tool_call_id")).unwrap_or_default();
                let content = msg
                    .get("content")
                    .cloned()
                    .unwrap_or_else(|| Value::String(String::new()));
                let name = call_names.get(&call_id).cloned();
                parts = vec![ContentPart::ToolResult { call_id, name, content }];
            }
            req.messages.push(Message { role, content: parts });
        }
    }

    req.tools = parse_chat_tools(obj.get("tools"));
    req.tool_choice = parse_chat_tool_choice(obj.get("tool_choice"));
    Some(req)
}

/// Chat content（字符串或块数组）→ IR parts。
fn parse_chat_content(content: Option<&Value>) -> Vec<ContentPart> {
    let mut parts = Vec::new();
    match content {
        Some(Value::String(s)) if !s.is_empty() => {
            parts.push(ContentPart::Text {
                text:          s.clone(),
                cache_control: None,
            });
        }
        Some(Value::Array(items)) => {
            for part in items {
                match as_str(part.get("type")) {
                    Some("text") | Some("input_text") | Some("output_text") => parts.push(ContentPart::Text {
                        text:          owned_str(part.get("text")).unwrap_or_default(),
                        cache_control: non_null(part.get("cache_control")),
                    }),
                    Some("image_url") => {
                        if let Some(url) =
                            as_str(part.pointer("/image_url/url")).or_else(|| as_str(part.get("image_url")))
                        {
                            parts.push(image_from_url(url));
                        }
                    }
                    // 未知块原样保留，避免内容静默丢失
                    _ => parts.push(ContentPart::Unknown(part.clone())),
                }
            }
        }
        _ => {}
    }
    parts
}

/// URL 形态图像归一：`data:` URI 解析为 inline base64，其余保持 URL。
fn image_from_url(url: &str) -> ContentPart {
    if let Some(data_uri) = url.strip_prefix("data:")
        && let Some((meta, data)) = data_uri.split_once(',')
    {
        return ContentPart::Image(ImageSource::InlineBase64 {
            media_type: meta
                .split(';')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("image/png")
                .to_string(),
            data:       data.to_string(),
        });
    }
    ContentPart::Image(ImageSource::Url(url.to_string()))
}

/// system 文本作为独立部件保存；连接分隔符由各 emitter 按目标口径决定。
fn push_system_text(system: &mut Vec<ContentPart>, content: Option<&Value>) {
    let text = match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|p| as_str(p.get("text")))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if !text.is_empty() {
        system.push(ContentPart::Text {
            text,
            cache_control: None,
        });
    }
}
fn parse_chat_tools(tools: Option<&Value>) -> Vec<Tool> {
    let Some(Value::Array(tools)) = tools else {
        return Vec::new();
    };
    tools
        .iter()
        .filter(|tool| as_str(tool.get("type")) == Some("function"))
        .filter_map(|tool| {
            let name = owned_str(tool.pointer("/function/name"))?;
            Some(Tool {
                name,
                description: owned_str(tool.pointer("/function/description")),
                parameters: tool
                    .pointer("/function/parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            })
        })
        .collect()
}

fn parse_chat_tool_choice(choice: Option<&Value>) -> Option<ToolChoice> {
    match choice {
        Some(Value::String(s)) => Some(match s.as_str() {
            "none" => ToolChoice::None,
            "required" => ToolChoice::Required,
            _ => ToolChoice::Auto,
        }),
        Some(Value::Object(o)) => owned_str(o.get("function").and_then(|f| f.get("name"))).map(ToolChoice::Tool),
        _ => None,
    }
}

pub(super) fn parse_responses(val: &Value) -> Option<RelayRequest> {
    let obj = val.as_object()?;
    let mut req = RelayRequest {
        model: owned_str(obj.get("model")).unwrap_or_default(),
        stream: obj.get("stream").and_then(Value::as_bool).unwrap_or(false),
        sampling: Sampling {
            temperature: non_null(obj.get("temperature")),
            top_p: non_null(obj.get("top_p")),
            max_tokens: non_null(obj.get("max_output_tokens")),
            reasoning_effort: as_str(val.pointer("/reasoning/effort"))
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            verbosity: as_str(val.pointer("/text/verbosity"))
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            ..Default::default()
        },
        parallel_tool_calls: obj.get("parallel_tool_calls").and_then(Value::as_bool),
        store: obj.get("store").and_then(Value::as_bool),
        service_tier: non_null(obj.get("service_tier")),
        metadata: non_null(obj.get("metadata")),
        safety_identifier: owned_str(obj.get("safety_identifier")),
        prompt_cache_key: owned_str(obj.get("prompt_cache_key")),
        ..Default::default()
    };

    if let Some(instructions) = as_str(obj.get("instructions"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req.system.push(ContentPart::Text {
            text:          instructions.to_string(),
            cache_control: None,
        });
    }

    req.tools = obj
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter(|tool| as_str(tool.get("type")) == Some("function"))
                .filter_map(|tool| {
                    let name = owned_str(tool.get("name"))?;
                    Some(Tool {
                        name,
                        description: owned_str(tool.get("description")),
                        parameters: tool
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    req.tool_choice = match obj.get("tool_choice") {
        Some(Value::String(s)) => Some(match s.as_str() {
            "none" => ToolChoice::None,
            "required" => ToolChoice::Required,
            _ => ToolChoice::Auto,
        }),
        Some(o @ Value::Object(_)) => owned_str(o.get("name")).map(ToolChoice::Tool),
        _ => None,
    };

    // Responses 允许 input 直接为字符串
    let items: Vec<Value> = match obj.get("input") {
        Some(Value::String(text)) => vec![json!({
            "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": text }]
        })],
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };

    // 生成的 call_id 栈：function_call_output 缺失 call_id 时按序回退复用
    let mut call_lookup: Vec<String> = Vec::new();

    // 连续 assistant 输出项（文本 + 函数调用）聚合为单条消息
    let mut pending_assistant: Option<Message> = None;
    for item in &items {
        match as_str(item.get("type")).unwrap_or("") {
            "function_call" => {
                let call_id = owned_str(item.get("call_id"))
                    .or_else(|| owned_str(item.get("id")))
                    .unwrap_or_else(|| format!("call_{}", nanoid::nanoid!(20)));
                call_lookup.push(call_id.clone());
                let message = pending_assistant.get_or_insert_with(|| Message {
                    role:    Role::Assistant,
                    content: Vec::new(),
                });
                message.content.push(ContentPart::ToolCall {
                    id:        call_id,
                    name:      owned_str(item.get("name")).unwrap_or_default(),
                    arguments: arguments_from_string(item.get("arguments")),
                });
            }
            "function_call_output" => {
                if let Some(assistant) = pending_assistant.take() {
                    req.messages.push(assistant);
                }
                let call_id = owned_str(item.get("call_id"))
                    .or_else(|| owned_str(item.get("id")))
                    .or_else(|| call_lookup.pop())
                    .unwrap_or_default();
                let content = match item.get("output") {
                    Some(Value::String(s)) => Value::String(s.clone()),
                    Some(other) => Value::String(serde_json::to_string(other).unwrap_or_default()),
                    None => Value::String(String::new()),
                };
                req.messages.push(Message {
                    role:    Role::Tool,
                    content: vec![ContentPart::ToolResult {
                        call_id,
                        name: None,
                        content,
                    }],
                });
            }
            // reasoning / web_search_call 等无法映射的项跳过，避免空 user 消息污染上下文
            "reasoning" => {}
            "message" | "" => {
                let role_str = as_str(item.get("role")).unwrap_or("user");
                let role = if role_str == "assistant" {
                    Role::Assistant
                } else {
                    Role::User
                };
                let mut parts = Vec::new();
                match item.get("content") {
                    Some(Value::String(text)) if !text.is_empty() => {
                        parts.push(ContentPart::Text {
                            text:          text.clone(),
                            cache_control: None,
                        });
                    }
                    Some(Value::Array(items)) => {
                        for part in items {
                            match as_str(part.get("type")) {
                                Some("input_text") | Some("text") | Some("output_text") => {
                                    parts.push(ContentPart::Text {
                                        text:          owned_str(part.get("text")).unwrap_or_default(),
                                        cache_control: non_null(part.get("cache_control")),
                                    });
                                }
                                Some("input_image") | Some("image_url") => {
                                    if let Some(url) = as_str(part.pointer("/image_url/url"))
                                        .or_else(|| as_str(part.get("image_url")))
                                        .filter(|u| !u.is_empty())
                                    {
                                        parts.push(image_from_url(url));
                                    }
                                }
                                // 未知 part 原样保留，避免内容静默丢失
                                _ => parts.push(ContentPart::Unknown(part.clone())),
                            }
                        }
                    }
                    _ => {}
                }
                if role == Role::Assistant {
                    let message = pending_assistant.get_or_insert_with(|| Message {
                        role:    Role::Assistant,
                        content: Vec::new(),
                    });
                    message.content.extend(parts);
                } else {
                    if let Some(assistant) = pending_assistant.take() {
                        req.messages.push(assistant);
                    }
                    req.messages.push(Message { role, content: parts });
                }
            }
            _ => {}
        }
    }
    if let Some(assistant) = pending_assistant.take() {
        req.messages.push(assistant);
    }
    Some(req)
}

pub(super) fn parse_anthropic(val: &Value) -> Option<RelayRequest> {
    let obj = val.as_object()?;
    let mut req = RelayRequest {
        model: owned_str(obj.get("model")).unwrap_or_default(),
        stream: obj.get("stream").and_then(Value::as_bool).unwrap_or(false),
        sampling: Sampling {
            temperature: non_null(obj.get("temperature")),
            top_p: non_null(obj.get("top_p")),
            top_k: non_null(obj.get("top_k")),
            max_tokens: non_null(obj.get("max_tokens")),
            stop: parse_stop(obj.get("stop_sequences")),
            ..Default::default()
        },
        user: obj
            .get("metadata")
            .and_then(|m| owned_str(m.get("user_id"))),
        ..Default::default()
    };

    match obj.get("system") {
        Some(Value::String(s)) if !s.is_empty() => {
            req.system.push(ContentPart::Text {
                text:          s.clone(),
                cache_control: None,
            });
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                if as_str(block.get("type")) == Some("text")
                    && let Some(text) = as_str(block.get("text"))
                {
                    req.system.push(ContentPart::Text {
                        text:          text.to_string(),
                        cache_control: non_null(block.get("cache_control")),
                    });
                }
            }
        }
        _ => {}
    }

    let mut call_names: HashMap<String, String> = HashMap::new();
    if let Some(messages) = obj.get("messages").and_then(Value::as_array) {
        for msg in messages {
            let role = if as_str(msg.get("role")) == Some("assistant") {
                Role::Assistant
            } else {
                Role::User
            };
            let mut parts = Vec::new();
            let flush = |parts: &mut Vec<ContentPart>, role: Role, req: &mut RelayRequest| {
                if !parts.is_empty() {
                    req.messages.push(Message {
                        role,
                        content: std::mem::take(parts),
                    });
                }
            };
            match msg.get("content") {
                Some(Value::String(s)) if !s.is_empty() => {
                    parts.push(ContentPart::Text {
                        text:          s.clone(),
                        cache_control: None,
                    });
                }
                Some(Value::Array(blocks)) => {
                    for block in blocks {
                        match as_str(block.get("type")) {
                            Some("text") => parts.push(ContentPart::Text {
                                text:          owned_str(block.get("text")).unwrap_or_default(),
                                cache_control: non_null(block.get("cache_control")),
                            }),
                            Some("image") => {
                                if let Some(image) = parse_anthropic_image(block.get("source")) {
                                    parts.push(ContentPart::Image(image));
                                }
                            }
                            Some("tool_use") => {
                                let id = owned_str(block.get("id")).unwrap_or_default();
                                let name = owned_str(block.get("name")).unwrap_or_default();
                                call_names.insert(id.clone(), name.clone());
                                parts.push(ContentPart::ToolCall {
                                    id,
                                    name,
                                    arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                                });
                            }
                            Some("tool_result") => {
                                // 与 chat/gemini parser 统一：工具结果拆为独立 Tool 消息，
                                // 否则 emit 侧（chat/responses/gemini）会丢弃 tool_result
                                flush(&mut parts, role, &mut req);
                                let call_id = owned_str(block.get("tool_use_id")).unwrap_or_default();
                                req.messages.push(Message {
                                    role:    Role::Tool,
                                    content: vec![ContentPart::ToolResult {
                                        call_id,
                                        name: call_names
                                            .get(
                                                owned_str(block.get("tool_use_id"))
                                                    .unwrap_or_default()
                                                    .as_str(),
                                            )
                                            .cloned(),
                                        content: block
                                            .get("content")
                                            .cloned()
                                            .unwrap_or_else(|| Value::String(String::new())),
                                    }],
                                });
                            }
                            // 扩展思考块转为 Thinking（签名保留）
                            Some("thinking") => parts.push(ContentPart::Thinking {
                                text:      owned_str(block.get("thinking")).unwrap_or_default(),
                                signature: owned_str(block.get("signature")),
                            }),
                            // 未知块丢弃（与既有转换行为一致）
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            flush(&mut parts, role, &mut req);
        }
    }

    req.tools = obj
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = owned_str(tool.get("name"))?;
                    Some(Tool {
                        name,
                        description: owned_str(tool.get("description")),
                        parameters: tool
                            .get("input_schema")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    req.tool_choice = match obj.get("tool_choice") {
        Some(o @ Value::Object(_)) => match as_str(o.get("type")) {
            Some("tool") => owned_str(o.get("name")).map(ToolChoice::Tool),
            Some("any") => Some(ToolChoice::Required),
            Some("none") => Some(ToolChoice::None),
            _ => Some(ToolChoice::Auto),
        },
        _ => None,
    };
    if obj
        .get("tool_choice")
        .and_then(|c| c.get("disable_parallel_tool_use"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        req.parallel_tool_calls = Some(false);
    }
    Some(req)
}

fn parse_anthropic_image(source: Option<&Value>) -> Option<ImageSource> {
    let source = source?;
    if as_str(source.get("type")) == Some("base64") {
        return Some(ImageSource::InlineBase64 {
            media_type: owned_str(source.get("media_type")).unwrap_or_else(|| "image/png".to_string()),
            data:       owned_str(source.get("data")).unwrap_or_default(),
        });
    }
    owned_str(source.get("url")).map(ImageSource::Url)
}

pub(super) fn parse_gemini(val: &Value) -> Option<RelayRequest> {
    let obj = val.as_object()?;
    let mut req = RelayRequest {
        model: owned_str(obj.get("model")).unwrap_or_default(),
        stream: obj.get("stream").and_then(Value::as_bool).unwrap_or(false),
        ..Default::default()
    };

    if let Some(sys) = obj
        .get("system_instruction")
        .or_else(|| obj.get("systemInstruction"))
    {
        for part in sys
            .pointer("/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = as_str(part.get("text")) {
                req.system.push(ContentPart::Text {
                    text:          text.to_string(),
                    cache_control: None,
                });
            }
        }
    }

    // functionResponse 只带函数名；记录 functionCall 生成的 call_id 供回填
    let mut call_ids: HashMap<String, String> = HashMap::new();
    if let Some(contents) = obj.get("contents").and_then(Value::as_array) {
        for content in contents {
            let role = if as_str(content.get("role")) == Some("model") {
                Role::Assistant
            } else {
                Role::User
            };
            let mut parts = Vec::new();
            for part in content
                .get("parts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(text) = as_str(part.get("text")) {
                    parts.push(ContentPart::Text {
                        text:          text.to_string(),
                        cache_control: None,
                    });
                } else if let Some(call) = part.get("functionCall") {
                    let name = owned_str(call.get("name")).unwrap_or_default();
                    let call_id = format!("call_{}", nanoid::nanoid!(20));
                    call_ids.insert(name.clone(), call_id.clone());
                    parts.push(ContentPart::ToolCall {
                        id: call_id,
                        name,
                        arguments: call.get("args").cloned().unwrap_or_else(|| json!({})),
                    });
                } else if let Some(resp) = part.get("functionResponse") {
                    let name = owned_str(resp.get("name")).unwrap_or_default();
                    let call_id = call_ids
                        .get(&name)
                        .cloned()
                        .unwrap_or_else(|| format!("call_{}", nanoid::nanoid!(20)));
                    let content = match resp.get("response") {
                        Some(Value::String(s)) => Value::String(s.clone()),
                        Some(other) => Value::String(serde_json::to_string(other).unwrap_or_default()),
                        None => Value::String(String::new()),
                    };
                    // 工具结果在该 content 内立即成条，保持先于其后 assistant 消息
                    req.messages.push(Message {
                        role:    Role::Tool,
                        content: vec![ContentPart::ToolResult {
                            call_id,
                            name: Some(name),
                            content,
                        }],
                    });
                } else if let Some(image) = part
                    .get("inlineData")
                    .or_else(|| part.get("inline_data"))
                    .map(|image| ImageSource::InlineBase64 {
                        media_type: owned_str(image.get("mimeType")).unwrap_or_else(|| "image/png".to_string()),
                        data:       owned_str(image.get("data")).unwrap_or_default(),
                    })
                    .or_else(|| {
                        part.get("fileData")
                            .or_else(|| part.get("file_data"))
                            .and_then(|file| owned_str(file.get("fileUri")))
                            .map(ImageSource::Url)
                    })
                {
                    parts.push(ContentPart::Image(image));
                }
            }
            if !parts.is_empty() {
                req.messages.push(Message { role, content: parts });
            }
        }
    }

    if let Some(config) = obj
        .get("generation_config")
        .or_else(|| obj.get("generationConfig"))
    {
        req.sampling.temperature = non_null(config.get("temperature"));
        req.sampling.top_p = config
            .get("topP")
            .or_else(|| config.get("top_p"))
            .cloned()
            .filter(|v| !v.is_null());
        req.sampling.top_k = config
            .get("topK")
            .or_else(|| config.get("top_k"))
            .cloned()
            .filter(|v| !v.is_null());
        req.sampling.max_tokens = config
            .get("maxOutputTokens")
            .or_else(|| config.get("max_output_tokens"))
            .cloned()
            .filter(|v| !v.is_null());
        req.sampling.stop = parse_stop(
            config
                .get("stopSequences")
                .or_else(|| config.get("stop_sequences")),
        );
        req.sampling.frequency_penalty = config
            .get("frequencyPenalty")
            .or_else(|| config.get("frequency_penalty"))
            .cloned()
            .filter(|v| !v.is_null());
        req.sampling.presence_penalty = config
            .get("presencePenalty")
            .or_else(|| config.get("presence_penalty"))
            .cloned()
            .filter(|v| !v.is_null());
        req.sampling.seed = non_null(config.get("seed"));
    }

    req.tools = obj
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("functionDeclarations")?.as_array())
                .flatten()
                .filter_map(|decl| {
                    let name = owned_str(decl.get("name"))?;
                    Some(Tool {
                        name,
                        description: owned_str(decl.get("description")),
                        parameters: decl.get("parameters").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    req.tool_choice = val
        .pointer("/toolConfig/functionCallingConfig")
        .or_else(|| val.pointer("/tool_config/functionCallingConfig"))
        .map(|config| match as_str(config.get("mode")).unwrap_or("AUTO") {
            "NONE" => ToolChoice::None,
            "ANY" => config
                .pointer("/allowedFunctionNames/0")
                .and_then(Value::as_str)
                .map(|name| ToolChoice::Tool(name.to_string()))
                .unwrap_or(ToolChoice::Required),
            _ => ToolChoice::Auto,
        });
    Some(req)
}

// ---------- Emitters ----------
fn tool_choice_string(choice: &ToolChoice) -> &'static str {
    match choice {
        ToolChoice::None => "none",
        ToolChoice::Required => "required",
        _ => "auto",
    }
}

pub(super) fn emit_chat(req: &RelayRequest) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("model".into(), Value::String(req.model.clone()));
    obj.insert("stream".into(), Value::Bool(req.stream));
    let s = &req.sampling;
    if let Some(v) = s.temperature.clone() {
        obj.insert("temperature".into(), v);
    }
    if let Some(v) = s.top_p.clone() {
        obj.insert("top_p".into(), v);
    }
    // Chat 无 top_k，不支持即剥离
    if let Some(v) = s.max_tokens.clone() {
        obj.insert("max_tokens".into(), v);
    }
    if !s.stop.is_empty() {
        obj.insert("stop".into(), json!(s.stop));
    }
    if let Some(v) = s.seed.clone() {
        obj.insert("seed".into(), v);
    }
    if let Some(v) = s.frequency_penalty.clone() {
        obj.insert("frequency_penalty".into(), v);
    }
    if let Some(v) = s.presence_penalty.clone() {
        obj.insert("presence_penalty".into(), v);
    }
    if let Some(v) = s.reasoning_effort.clone() {
        obj.insert("reasoning_effort".into(), Value::String(v));
    }
    if let Some(v) = s.verbosity.clone() {
        obj.insert("verbosity".into(), Value::String(v));
    }
    if let Some(v) = req.parallel_tool_calls {
        obj.insert("parallel_tool_calls".into(), Value::Bool(v));
    }
    if let Some(v) = req.store {
        obj.insert("store".into(), Value::Bool(v));
    }
    if let Some(v) = req.service_tier.clone() {
        obj.insert("service_tier".into(), v);
    }
    if let Some(v) = req.metadata.clone() {
        obj.insert("metadata".into(), v);
    }
    if let Some(v) = req.safety_identifier.clone() {
        obj.insert("safety_identifier".into(), Value::String(v));
    }
    if let Some(v) = req.prompt_cache_key.clone() {
        obj.insert("prompt_cache_key".into(), Value::String(v));
    }
    if let Some(v) = req.user.clone() {
        obj.insert("user".into(), Value::String(v));
    }

    let mut messages = Vec::new();
    let system_text = join_system_text(&req.system, "\n");
    if !system_text.is_empty() {
        messages.push(json!({ "role": "system", "content": system_text }));
    }
    for msg in &req.messages {
        match msg.role {
            Role::Tool => {
                let Some(ContentPart::ToolResult { call_id, content, .. }) = msg.content.first() else {
                    continue;
                };
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": render_result_string(content)
                }));
            }
            Role::User | Role::Assistant => {
                let mut text = String::new();
                let mut blocks: Vec<Value> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                let mut mixed = false;
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text: t, .. } => text.push_str(t),
                        ContentPart::Image(image) => {
                            mixed = true;
                            blocks.push(json!({
                                "type": "image_url",
                                "image_url": { "url": render_image_url(image) }
                            }));
                        }
                        ContentPart::ToolCall { id, name, arguments } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string())
                                }
                            }));
                        }
                        ContentPart::Unknown(raw) => {
                            mixed = true;
                            blocks.push(raw.clone());
                        }
                        // Thinking/ToolResult 在 Chat 请求目标无对应形态，丢弃（与既有行为一致）
                        _ => {}
                    }
                }
                let content = if mixed {
                    let mut merged = Vec::new();
                    if !text.is_empty() {
                        merged.push(json!({ "type": "text", "text": text }));
                    }
                    merged.extend(blocks);
                    Value::Array(merged)
                } else if text.is_empty() && !tool_calls.is_empty() {
                    Value::Null
                } else {
                    Value::String(text)
                };
                let mut message = json!({ "role": render_role(msg.role), "content": content });
                if !tool_calls.is_empty() {
                    message["tool_calls"] = Value::Array(tool_calls);
                }
                messages.push(message);
            }
        }
    }
    obj.insert("messages".into(), Value::Array(messages));

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|tool| {
            let mut function = json!({ "name": tool.name });
            if let Some(description) = &tool.description {
                function["description"] = Value::String(description.clone());
            }
            function["parameters"] = if tool.parameters.is_null() {
                json!({ "type": "object", "properties": {} })
            } else {
                tool.parameters.clone()
            };
            json!({ "type": "function", "function": function })
        })
        .collect();
    if !tools.is_empty() {
        obj.insert("tools".into(), Value::Array(tools));
    }
    if let Some(choice) = &req.tool_choice {
        obj.insert(
            "tool_choice".into(),
            match choice {
                ToolChoice::Tool(name) => json!({ "type": "function", "function": { "name": name } }),
                other => Value::String(tool_choice_string(other).to_string()),
            },
        );
    }
    Value::Object(obj)
}

fn render_role(role: Role) -> &'static str {
    match role {
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::User => "user",
    }
}

/// 图像统一为 URL 形态；inline base64 拼回 data: URI（Chat/Responses 无独立 inline 形态）。
fn render_image_url(image: &ImageSource) -> String {
    match image {
        ImageSource::Url(url) => url.clone(),
        ImageSource::InlineBase64 { media_type, data } => format!("data:{media_type};base64,{data}"),
    }
}

pub(super) fn emit_responses(req: &RelayRequest) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("model".into(), Value::String(req.model.clone()));
    obj.insert("stream".into(), Value::Bool(req.stream));
    let s = &req.sampling;
    if let Some(v) = s.temperature.clone() {
        obj.insert("temperature".into(), v);
    }
    if let Some(v) = s.top_p.clone() {
        obj.insert("top_p".into(), v);
    }
    if let Some(v) = s.max_tokens.clone() {
        obj.insert("max_output_tokens".into(), v);
    }
    if let Some(effort) = s.reasoning_effort.clone() {
        obj.insert("reasoning".into(), json!({ "effort": effort }));
    }
    if let Some(verbosity) = s.verbosity.clone() {
        obj.insert("text".into(), json!({ "verbosity": verbosity }));
    }
    if let Some(v) = req.parallel_tool_calls {
        obj.insert("parallel_tool_calls".into(), Value::Bool(v));
    }
    if let Some(v) = req.store {
        obj.insert("store".into(), Value::Bool(v));
    }
    if let Some(v) = req.service_tier.clone() {
        obj.insert("service_tier".into(), v);
    }
    if let Some(v) = req.metadata.clone() {
        obj.insert("metadata".into(), v);
    }
    if let Some(v) = req.safety_identifier.clone() {
        obj.insert("safety_identifier".into(), Value::String(v));
    }
    if let Some(v) = req.prompt_cache_key.clone() {
        obj.insert("prompt_cache_key".into(), Value::String(v));
    }

    let instructions = join_system_text(&req.system, "\n\n");
    if !instructions.is_empty() {
        obj.insert("instructions".into(), Value::String(instructions));
    }

    let mut input = Vec::new();
    for msg in &req.messages {
        match msg.role {
            Role::Tool => {
                let Some(ContentPart::ToolResult { call_id, content, .. }) = msg.content.first() else {
                    continue;
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": render_result_string(content)
                }));
            }
            Role::User | Role::Assistant => {
                let text_type = if msg.role == Role::Assistant {
                    "output_text"
                } else {
                    "input_text"
                };
                let mut content_parts: Vec<Value> = Vec::new();
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } if !text.is_empty() => {
                            content_parts.push(json!({ "type": text_type, "text": text }));
                        }
                        ContentPart::Image(image) => {
                            content_parts.push(json!({ "type": "input_image", "image_url": render_image_url(image) }));
                        }
                        _ => {}
                    }
                }
                if msg.role == Role::Assistant {
                    // 文本 item 在先，tool_calls 剥离为随后的 function_call item，
                    // 保持 Responses 对话流中文本先于其引发的调用这一时序
                    match content_parts.len() {
                        0 => {}
                        1 => input.push(json!({
                            "type": "message", "role": "assistant",
                            "content": content_parts[0].get("text").cloned().unwrap_or(Value::Null)
                        })),
                        _ => input.push(json!({ "type": "message", "role": "assistant", "content": content_parts })),
                    }
                    for part in &msg.content {
                        if let ContentPart::ToolCall { id, name, arguments } = part {
                            input.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string())
                            }));
                        }
                    }
                } else if !content_parts.is_empty() {
                    input.push(json!({ "type": "message", "role": "user", "content": content_parts }));
                }
            }
        }
    }
    obj.insert("input".into(), Value::Array(input));

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|tool| {
            let mut flat = json!({ "type": "function", "name": tool.name });
            if let Some(description) = &tool.description {
                flat["description"] = Value::String(description.clone());
            }
            flat["parameters"] = if tool.parameters.is_null() {
                json!({ "type": "object", "properties": {} })
            } else {
                tool.parameters.clone()
            };
            flat
        })
        .collect();
    if !tools.is_empty() {
        obj.insert("tools".into(), Value::Array(tools));
    }
    if let Some(choice) = &req.tool_choice {
        obj.insert(
            "tool_choice".into(),
            match choice {
                ToolChoice::Tool(name) => json!({ "type": "function", "name": name }),
                other => Value::String(tool_choice_string(other).to_string()),
            },
        );
    }
    Value::Object(obj)
}

pub(super) fn emit_anthropic(req: &RelayRequest) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("model".into(), Value::String(req.model.clone()));
    obj.insert("stream".into(), Value::Bool(req.stream));
    // Anthropic 必需 max_tokens；请求缺失时兜底
    obj.insert(
        "max_tokens".into(),
        req.sampling
            .max_tokens
            .clone()
            .unwrap_or_else(|| json!(4096)),
    );
    if let Some(v) = req.sampling.temperature.clone() {
        obj.insert("temperature".into(), v);
    }
    if let Some(v) = req.sampling.top_p.clone() {
        obj.insert("top_p".into(), v);
    }
    if let Some(v) = req.sampling.top_k.clone() {
        obj.insert("top_k".into(), v);
    }
    if !req.sampling.stop.is_empty() {
        obj.insert("stop_sequences".into(), json!(req.sampling.stop));
    }

    let system_text = join_system_text(&req.system, "\n");
    if !system_text.is_empty() {
        obj.insert("system".into(), Value::String(system_text));
    }

    let mut messages: Vec<Value> = Vec::new();
    // 连续 user 消息合并；tool_result 挂到最近的 user 消息（Anthropic 约束）
    for msg in &req.messages {
        match msg.role {
            Role::Tool => {
                let Some(ContentPart::ToolResult { call_id, content, .. }) = msg.content.first() else {
                    continue;
                };
                attach_tool_result(
                    &mut messages,
                    json!({ "type": "tool_result", "tool_use_id": call_id, "content": content }),
                );
            }
            Role::User | Role::Assistant => {
                let content = render_anthropic_content(&msg.content);
                if msg.role == Role::User
                    && let Some(last) = messages.last_mut()
                    && last.get("role").and_then(Value::as_str) == Some("user")
                {
                    let new_blocks = match &content {
                        Value::String(s) => vec![json!({ "type": "text", "text": s })],
                        Value::Array(arr) => arr.clone(),
                        other => vec![other.clone()],
                    };
                    if let Some(existing) = last.get_mut("content") {
                        let prev = existing.take();
                        let mut all = match prev {
                            Value::Array(arr) => arr,
                            Value::String(s) => vec![json!({ "type": "text", "text": s })],
                            other => vec![other],
                        };
                        all.extend(new_blocks);
                        *existing = Value::Array(all);
                    } else {
                        last["content"] = Value::Array(new_blocks);
                    }
                    continue;
                }
                let role = if msg.role == Role::Assistant {
                    "assistant"
                } else {
                    "user"
                };
                messages.push(json!({ "role": role, "content": content }));
            }
        }
    }
    obj.insert("messages".into(), Value::Array(messages));

    let tools: Vec<Value> = req
        .tools
        .iter()
        .filter(|tool| !tool.name.is_empty())
        .map(|tool| {
            let mut t = json!({ "name": tool.name });
            if let Some(description) = tool.description.as_deref().filter(|d| !d.is_empty()) {
                t["description"] = Value::String(description.to_string());
            }
            t["input_schema"] = if tool.parameters.is_null() {
                json!({ "type": "object", "properties": {} })
            } else {
                tool.parameters.clone()
            };
            t
        })
        .collect();
    if !tools.is_empty() {
        obj.insert("tools".into(), Value::Array(tools));
    }
    if let Some(choice) = &req.tool_choice {
        obj.insert(
            "tool_choice".into(),
            match choice {
                ToolChoice::Auto => json!({ "type": "auto" }),
                ToolChoice::None => json!({ "type": "none" }),
                ToolChoice::Required => json!({ "type": "any" }),
                ToolChoice::Tool(name) => json!({ "type": "tool", "name": name }),
            },
        );
    }
    Value::Object(obj)
}

/// 渲染消息内容：纯文本（含空）折叠为字符串，其余渲染为块数组。
fn render_anthropic_content(parts: &[ContentPart]) -> Value {
    if parts.iter().all(|p| matches!(p, ContentPart::Text { .. })) {
        let joined: String = parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text, .. } => text.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>()
            .join("");
        return Value::String(joined);
    }
    let mut blocks = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text { text, cache_control } => {
                let mut block = json!({ "type": "text", "text": text });
                if let Some(cache_control) = cache_control {
                    block["cache_control"] = cache_control.clone();
                }
                blocks.push(block);
            }
            ContentPart::Image(image) => {
                if let Some(block) = render_anthropic_image(image) {
                    blocks.push(block);
                }
            }
            ContentPart::Thinking { text, signature } => {
                let mut block = json!({ "type": "thinking", "thinking": text });
                if let Some(signature) = signature {
                    block["signature"] = Value::String(signature.clone());
                }
                blocks.push(block);
            }
            ContentPart::ToolCall { id, name, arguments } => {
                blocks.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": arguments
                }));
            }
            ContentPart::ToolResult { call_id, content, .. } => {
                blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": content
                }));
            }
            ContentPart::Unknown(_) => {}
        }
    }
    Value::Array(blocks)
}

fn render_anthropic_image(image: &ImageSource) -> Option<Value> {
    Some(match image {
        ImageSource::InlineBase64 { media_type, data } => json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data }
        }),
        ImageSource::Url(url) if !url.is_empty() => {
            json!({ "type": "image", "source": { "type": "url", "url": url } })
        }
        _ => return None,
    })
}

fn attach_tool_result(messages: &mut Vec<Value>, result: Value) {
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
        && let Some(content) = last.get_mut("content")
    {
        let existing = content.take();
        let mut all = match existing {
            Value::Array(arr) => arr,
            Value::String(s) => vec![json!({ "type": "text", "text": s })],
            other => vec![other],
        };
        all.push(result);
        *content = Value::Array(all);
        return;
    }
    messages.push(json!({ "role": "user", "content": [result] }));
}

pub(super) fn emit_gemini(req: &RelayRequest) -> Value {
    let mut obj = serde_json::Map::new();
    let mut contents = Vec::new();
    for msg in &req.messages {
        match msg.role {
            Role::Tool => {
                let Some(ContentPart::ToolResult { name, content, .. }) = msg.content.first() else {
                    continue;
                };
                contents.push(json!({
                    "role": "user",
                    "parts": [{ "functionResponse": {
                        "name": name.clone().unwrap_or_default(),
                        "response": render_gemini_result(content)
                    } }]
                }));
            }
            Role::User | Role::Assistant => {
                let mut parts = Vec::new();
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } => parts.push(json!({ "text": text })),
                        ContentPart::Image(image) => {
                            if let Some(rendered) = render_gemini_image(image) {
                                parts.push(rendered);
                            }
                        }
                        ContentPart::ToolCall { name, arguments, .. } => {
                            parts.push(json!({ "functionCall": { "name": name, "args": arguments } }));
                        }
                        // Thinking/Unknown 在 Gemini 请求目标无对应形态，丢弃（与既有行为一致）
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": if msg.role == Role::Assistant { "model" } else { "user" },
                        "parts": parts
                    }));
                }
            }
        }
    }
    obj.insert("contents".into(), Value::Array(contents));

    let system_text = join_system_text(&req.system, "\n");
    if !system_text.is_empty() {
        obj.insert(
            "system_instruction".into(),
            json!({ "parts": [{ "text": system_text }] }),
        );
    }

    let mut gen_config = serde_json::Map::new();
    let s = &req.sampling;
    if let Some(v) = s.temperature.clone() {
        gen_config.insert("temperature".into(), v);
    }
    if let Some(v) = s.top_p.clone() {
        gen_config.insert("topP".into(), v);
    }
    if let Some(v) = s.top_k.clone() {
        gen_config.insert("topK".into(), v);
    }
    if let Some(v) = s.max_tokens.clone() {
        gen_config.insert("maxOutputTokens".into(), v);
    }
    if !s.stop.is_empty() {
        gen_config.insert("stopSequences".into(), json!(s.stop));
    }
    if let Some(v) = s.frequency_penalty.clone() {
        gen_config.insert("frequencyPenalty".into(), v);
    }
    if let Some(v) = s.presence_penalty.clone() {
        gen_config.insert("presencePenalty".into(), v);
    }
    if let Some(v) = s.seed.clone() {
        gen_config.insert("seed".into(), v);
    }
    if !gen_config.is_empty() {
        obj.insert("generationConfig".into(), Value::Object(gen_config));
    }

    let declarations: Vec<Value> = req
        .tools
        .iter()
        .filter(|tool| !tool.name.is_empty())
        .map(|tool| {
            let mut f = json!({ "name": tool.name });
            if let Some(description) = tool.description.as_deref().filter(|d| !d.is_empty()) {
                f["description"] = Value::String(description.to_string());
            }
            if !tool.parameters.is_null() {
                f["parameters"] = tool.parameters.clone();
            }
            f
        })
        .collect();
    if !declarations.is_empty() {
        obj.insert("tools".into(), json!([{ "functionDeclarations": declarations }]));
    }
    if let Some(choice) = &req.tool_choice {
        let mut cfg = serde_json::Map::new();
        match choice {
            ToolChoice::None => {
                cfg.insert("mode".into(), Value::String("NONE".into()));
            }
            ToolChoice::Required => {
                cfg.insert("mode".into(), Value::String("ANY".into()));
            }
            ToolChoice::Tool(name) => {
                cfg.insert("mode".into(), Value::String("ANY".into()));
                cfg.insert("allowedFunctionNames".into(), json!([name]));
            }
            ToolChoice::Auto => {
                cfg.insert("mode".into(), Value::String("AUTO".into()));
            }
        }
        obj.insert("toolConfig".into(), json!({ "functionCallingConfig": cfg }));
    }
    Value::Object(obj)
}

/// Gemini functionResponse.response 必须是 JSON Object：
/// 字符串/非对象统一包装为 {"result": ...}，避免上游 400。
fn render_gemini_result(content: &Value) -> Value {
    match content {
        response @ Value::Object(_) => response.clone(),
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(parsed @ Value::Object(_)) => parsed,
            Ok(parsed) => json!({ "result": parsed }),
            Err(_) => json!({ "result": text }),
        },
        other => json!({ "result": other }),
    }
}

fn render_gemini_image(image: &ImageSource) -> Option<Value> {
    Some(match image {
        ImageSource::InlineBase64 { media_type, data } => {
            json!({ "inlineData": { "mimeType": media_type, "data": data } })
        }
        ImageSource::Url(url) if !url.is_empty() => json!({ "fileData": { "fileUri": url } }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::super::{emit_request, parse_request};
    use crate::relay::protocol::ApiFormat;

    fn convert(src: ApiFormat, req: &Value, dst: ApiFormat) -> Value {
        let ir = parse_request(src, req).unwrap();
        emit_request(&ir, dst)
    }

    #[test]
    fn chat_to_anthropic_basic() {
        let openai = json!({
            "model": "claude-3-5-sonnet",
            "messages": [
                { "role": "system", "content": "You are a helpful assistant" },
                { "role": "user", "content": "Hello world" }
            ],
            "temperature": 0.7,
            "max_tokens": 2048,
            "stop": ["END"]
        });
        let anthropic = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::AnthropicMessage);
        assert_eq!(anthropic.get("model").unwrap(), "claude-3-5-sonnet");
        assert_eq!(anthropic.get("system").unwrap(), "You are a helpful assistant");
        assert_eq!(anthropic.get("max_tokens").unwrap(), 2048);
        assert_eq!(anthropic.pointer("/stop_sequences/0").unwrap(), "END");
        let messages = anthropic.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].get("role").unwrap(), "user");
        assert_eq!(messages[0].get("content").unwrap(), "Hello world");
    }

    #[test]
    fn chat_to_gemini_basic() {
        let openai = json!({
            "model": "gemini-1.5-flash",
            "messages": [
                { "role": "system", "content": "Think step by step" },
                { "role": "user", "content": "Hi" }
            ],
            "temperature": 0.5,
            "stop": "STOP_STR",
            "frequency_penalty": 0.1,
            "presence_penalty": 0.2,
            "seed": 9
        });
        let gemini = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::GeminiGenerate);
        assert!(gemini.get("system_instruction").is_some());
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].get("role").unwrap(), "user");
        assert_eq!(gemini.pointer("/generationConfig/stopSequences/0").unwrap(), "STOP_STR");
        assert_eq!(
            gemini
                .pointer("/generationConfig/frequencyPenalty")
                .unwrap(),
            0.1
        );
        assert_eq!(gemini.pointer("/generationConfig/presencePenalty").unwrap(), 0.2);
        assert_eq!(gemini.pointer("/generationConfig/seed").unwrap(), 9);
    }

    #[test]
    fn chat_tools_to_anthropic() {
        let openai = json!({
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
        let anthropic = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::AnthropicMessage);
        let tools = anthropic.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools[0].get("name").unwrap(), "get_weather");

        let messages = anthropic.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        let assistant = messages[1].as_object().unwrap();
        let blocks = assistant.get("content").unwrap().as_array().unwrap();
        assert_eq!(blocks[0].get("type").unwrap(), "tool_use");
        assert_eq!(blocks[0].get("id").unwrap(), "call_abc");
        assert_eq!(blocks[0].get("name").unwrap(), "get_weather");
        assert_eq!(blocks[0].pointer("/input/city").unwrap(), "Paris");
        let tool_msg = messages[2].as_object().unwrap();
        assert_eq!(tool_msg.get("role").unwrap(), "user");
        let result = tool_msg.get("content").unwrap().as_array().unwrap();
        assert_eq!(result[0].get("type").unwrap(), "tool_result");
        assert_eq!(result[0].get("tool_use_id").unwrap(), "call_abc");
    }

    #[test]
    fn consecutive_tool_results_merge_into_one_user_message() {
        let openai = json!({
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
        let anthropic = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::AnthropicMessage);
        let messages = anthropic.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].get("role").unwrap(), "user");
        let tool_msg_content = messages[2].get("content").unwrap().as_array().unwrap();
        assert_eq!(tool_msg_content.len(), 2);
        assert_eq!(tool_msg_content[0].get("tool_use_id").unwrap(), "call_1");
        assert_eq!(tool_msg_content[1].get("tool_use_id").unwrap(), "call_2");
    }

    #[test]
    fn chat_to_gemini_tool_calls() {
        let openai = json!({
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
        let gemini = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::GeminiGenerate);
        let tools = gemini.get("tools").unwrap().as_array().unwrap();
        let decls = tools[0]
            .get("functionDeclarations")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(decls[0].get("name").unwrap(), "get_weather");
        let cfg = gemini
            .pointer("/toolConfig/functionCallingConfig/mode")
            .unwrap();
        assert_eq!(cfg, "ANY");
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[1].get("role").unwrap(), "model");
        let model_parts = contents[1].get("parts").unwrap().as_array().unwrap();
        assert_eq!(model_parts[0].pointer("/functionCall/name").unwrap(), "get_weather");
        assert_eq!(model_parts[0].pointer("/functionCall/args/city").unwrap(), "Paris");
        let user_parts = contents[2].get("parts").unwrap().as_array().unwrap();
        assert_eq!(user_parts[0].pointer("/functionResponse/name").unwrap(), "get_weather");
    }

    #[test]
    fn gemini_function_response_wraps_non_object() {
        let openai = json!({
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
        let gemini = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::GeminiGenerate);
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        let resp = contents[2]
            .pointer("/parts/0/functionResponse/response")
            .unwrap();
        assert!(resp.is_object());
        assert_eq!(resp.get("result").unwrap(), "24°C, sunny");
    }

    #[test]
    fn gemini_function_response_json_passthrough() {
        let openai = json!({
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
        let gemini = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::GeminiGenerate);
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        let resp = contents[2]
            .pointer("/parts/0/functionResponse/response")
            .unwrap();
        assert_eq!(resp.get("temperature_c").unwrap(), 24);
        assert_eq!(resp.get("condition").unwrap(), "sunny");
    }

    #[test]
    fn chat_anthropic_vision() {
        let openai = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "What is in this image?" },
                    { "type": "image_url", "image_url": { "url": "https://example.com/a.png" } }
                ]
            }]
        });
        let anthropic = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::AnthropicMessage);
        let content = anthropic
            .pointer("/messages/0/content")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(content[0].get("type").unwrap(), "text");
        assert_eq!(content[1].get("type").unwrap(), "image");
        assert_eq!(content[1].pointer("/source/type").unwrap(), "url");
        assert_eq!(content[1].pointer("/source/url").unwrap(), "https://example.com/a.png");
    }

    #[test]
    fn chat_gemini_vision() {
        let openai = json!({
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
        let gemini = convert(ApiFormat::OpenaiCompletion, &openai, ApiFormat::GeminiGenerate);
        let parts = gemini
            .pointer("/contents/0/parts")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(parts[1].pointer("/inlineData/mimeType").unwrap(), "image/png");
        assert_eq!(parts[1].pointer("/inlineData/data").unwrap(), "iVBORw0KGgo=");
    }

    #[test]
    fn responses_to_chat_basic() {
        let responses = json!({
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
        let chat = convert(ApiFormat::OpenaiResponse, &responses, ApiFormat::OpenaiCompletion);
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
    fn responses_tool_calls_aggregate_into_single_assistant() {
        let responses = json!({
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
        let chat = convert(ApiFormat::OpenaiResponse, &responses, ApiFormat::OpenaiCompletion);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].get("content").unwrap(), "Let me check.");
        assert_eq!(messages[1].pointer("/tool_calls/0/function/name").unwrap(), "read_file");
        assert_eq!(messages[1].pointer("/tool_calls/0/id").unwrap(), "call_1");
        assert_eq!(messages[2].get("role").unwrap(), "tool");
        assert_eq!(messages[2].get("tool_call_id").unwrap(), "call_1");
        assert_eq!(messages[2].get("content").unwrap(), "127.0.0.1 localhost");
    }

    #[test]
    fn responses_missing_call_id_generates_stable_fallback() {
        let responses = json!({
            "model": "test-model",
            "input": [
                { "type": "message", "role": "assistant", "content": [] },
                { "type": "function_call", "id": "fc_abc", "name": "read_file",
                  "arguments": "{\"path\":\"/etc/hosts\"}" },
                { "type": "function_call_output", "id": "fc_abc", "output": { "ok": true } }
            ]
        });
        let chat = convert(ApiFormat::OpenaiResponse, &responses, ApiFormat::OpenaiCompletion);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        let call_id = messages[0]
            .pointer("/tool_calls/0/id")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(!call_id.is_empty());
        assert_eq!(messages[1].get("tool_call_id").unwrap(), call_id);
        let content = messages[1].get("content").unwrap().as_str().unwrap();
        assert!(content.contains("\"ok\""));
    }

    #[test]
    fn responses_string_input_and_item_hygiene() {
        let responses = json!({
            "model": "m",
            "input": "你好",
            "max_output_tokens": 512,
            "reasoning": { "effort": "high", "summary": "auto" },
            "previous_response_id": "resp_prev",
            "truncation": "auto",
            "background": false,
            "store": true,
            "temperature": 0.7
        });
        let chat = convert(ApiFormat::OpenaiResponse, &responses, ApiFormat::OpenaiCompletion);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].pointer("/content").unwrap(), "你好");
        assert_eq!(chat.get("max_tokens").unwrap(), 512);
        assert_eq!(chat.get("reasoning_effort").unwrap(), "high");
        assert_eq!(chat.get("temperature").unwrap(), 0.7);
        assert_eq!(chat.get("store").unwrap(), true);
        for key in [
            "max_output_tokens",
            "reasoning",
            "previous_response_id",
            "truncation",
            "background",
        ] {
            assert!(chat.get(key).is_none(), "{key} 应被剥离");
        }
    }

    #[test]
    fn responses_skips_reasoning_items() {
        let responses = json!({
            "model": "m",
            "input": [
                { "type": "message", "role": "user",
                  "content": [{ "type": "input_text", "text": "问题" }] },
                { "type": "reasoning", "id": "rs_1", "summary": [] },
                { "type": "message", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "回答" }] }
            ]
        });
        let chat = convert(ApiFormat::OpenaiResponse, &responses, ApiFormat::OpenaiCompletion);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].pointer("/content").unwrap(), "回答");
    }

    #[test]
    fn chat_to_responses_field_hygiene() {
        let chat = json!({
            "model": "m",
            "messages": [{ "role": "user", "content": "Hi" }],
            "max_tokens": 1024,
            "stream_options": { "include_usage": true },
            "frequency_penalty": 0.5,
            "presence_penalty": 0.2,
            "seed": 42,
            "stop": ["END"],
            "n": 1,
            "user": "u1",
            "store": true,
            "temperature": 0.6
        });
        let responses = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse);
        assert_eq!(responses.get("max_output_tokens").unwrap(), 1024);
        assert_eq!(responses.get("temperature").unwrap(), 0.6);
        assert_eq!(responses.get("store").unwrap(), true);
        for key in [
            "max_tokens",
            "stream_options",
            "frequency_penalty",
            "presence_penalty",
            "seed",
            "stop",
            "n",
            "user",
        ] {
            assert!(responses.get(key).is_none(), "{key} 应被剥离");
        }
    }

    #[test]
    fn chat_to_responses_basic() {
        let chat = json!({
            "model": "gpt-5.6-sol",
            "messages": [{ "role": "user", "content": "Hello world" }]
        });
        let responses = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse);
        assert!(responses.get("messages").is_none());
        let input = responses.get("input").unwrap().as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].get("role").unwrap(), "user");
        assert_eq!(input[0].pointer("/content/0/type").unwrap(), "input_text");
        assert_eq!(input[0].pointer("/content/0/text").unwrap(), "Hello world");
    }

    #[test]
    fn chat_to_responses_assistant_text_before_calls() {
        let chat = json!({
            "model": "m",
            "messages": [
                { "role": "assistant", "content": "回答",
                  "tool_calls": [{ "id": "call_1", "type": "function",
                    "function": { "name": "bash", "arguments": "{}" } }] }
            ]
        });
        let responses = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse);
        let input = responses.get("input").unwrap().as_array().unwrap();
        assert_eq!(input[0].get("type").unwrap(), "message");
        assert_eq!(input[1].get("type").unwrap(), "function_call");
    }

    #[test]
    fn chat_tools_to_responses_flat_structure() {
        let chat = json!({
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
        let responses = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse);
        let tools = responses.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.get("type").unwrap(), "function");
        assert_eq!(tool.get("name").unwrap(), "get_weather");
        assert!(tool.get("function").is_none());
        assert_eq!(tool.pointer("/parameters/properties/city/type").unwrap(), "string");
        let choice = responses.get("tool_choice").unwrap();
        assert_eq!(choice.get("type").unwrap(), "function");
        assert_eq!(choice.get("name").unwrap(), "get_weather");
        assert!(choice.get("function").is_none());
    }

    #[test]
    fn chat_to_responses_full_mapping() {
        let chat = json!({
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
        let responses = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse);
        assert_eq!(responses.get("instructions").unwrap(), "系统提示");
        assert!(responses.get("messages").is_none());
        let input = responses.get("input").unwrap().as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0].get("role").unwrap(), "user");
        assert_eq!(input[0].pointer("/content/0/type").unwrap(), "input_text");
        assert_eq!(input[0].pointer("/content/0/text").unwrap(), "你好");
        assert_eq!(input[1].get("type").unwrap(), "function_call");
        assert_eq!(input[1].get("call_id").unwrap(), "call_1");
        assert_eq!(input[1].get("name").unwrap(), "bash");
        assert_eq!(input[2].get("type").unwrap(), "function_call_output");
        assert_eq!(input[2].get("output").unwrap(), "输出");
    }

    #[test]
    fn gemini_to_chat_basic() {
        let gemini = json!({
            "contents": [
                { "role": "user", "parts": [{ "text": "Check weather" }] },
                { "role": "model", "parts": [{ "functionCall": { "name": "get_weather", "args": { "city": "Paris" } } }] },
                { "role": "user", "parts": [{ "functionResponse": { "name": "get_weather", "response": { "temp": 24 } } }] }
            ],
            "systemInstruction": { "parts": [{ "text": "You are helpful" }] },
            "tools": [{ "functionDeclarations": [{ "name": "get_weather", "description": "Get weather", "parameters": { "type": "object" } }] }],
            "generationConfig": { "temperature": 0.5, "maxOutputTokens": 128 }
        });
        let chat = convert(ApiFormat::GeminiGenerate, &gemini, ApiFormat::OpenaiCompletion);
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
    fn gemini_tool_response_reuses_call_id_and_unwraps_string() {
        let gemini = json!({
            "contents": [
                { "role": "model", "parts": [{ "functionCall": { "name": "get_weather", "args": { "city": "Paris" } } }] },
                { "role": "user", "parts": [{ "functionResponse": { "name": "get_weather", "response": "sunny, 24C" } }] }
            ]
        });
        let chat = convert(ApiFormat::GeminiGenerate, &gemini, ApiFormat::OpenaiCompletion);
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
        assert_eq!(messages[1].pointer("/content").unwrap(), "sunny, 24C");
    }

    #[test]
    fn gemini_multimodal_and_generation_config() {
        let gemini = json!({
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
        let chat = convert(ApiFormat::GeminiGenerate, &gemini, ApiFormat::OpenaiCompletion);
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
    fn anthropic_to_chat_basic() {
        let anthropic = json!({
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
        let openai = convert(ApiFormat::AnthropicMessage, &anthropic, ApiFormat::OpenaiCompletion);
        assert_eq!(openai.pointer("/messages/0/role").unwrap(), "system");
        assert_eq!(openai.pointer("/messages/1/content").unwrap(), "Hello");
        assert_eq!(openai.pointer("/tools/0/function/name").unwrap(), "read_file");
        assert_eq!(openai.pointer("/tools/0/function/parameters/type").unwrap(), "object");
    }

    #[test]
    fn anthropic_request_field_hygiene() {
        let anthropic = json!({
            "model": "m",
            "max_tokens": 512,
            "system": "s",
            "messages": [{ "role": "user", "content": "Hi" }],
            "stop_sequences": ["END", "!"],
            "metadata": { "user_id": "user-9" },
            "tool_choice": { "type": "tool", "name": "read_file", "disable_parallel_tool_use": true },
            "top_k": 40,
            "thinking": { "type": "enabled", "budget_tokens": 2048 }
        });
        let openai = convert(ApiFormat::AnthropicMessage, &anthropic, ApiFormat::OpenaiCompletion);
        assert_eq!(openai.pointer("/stop/0").unwrap(), "END");
        assert_eq!(openai.get("user").unwrap(), "user-9");
        assert_eq!(openai.get("parallel_tool_calls").unwrap(), false);
        assert_eq!(openai.pointer("/tool_choice/function/name").unwrap(), "read_file");
        for key in ["stop_sequences", "metadata", "top_k", "thinking"] {
            assert!(openai.get(key).is_none(), "{key} 应被剥离");
        }
    }

    #[test]
    fn responses_merges_text_parts_and_instructions() {
        let responses = json!({
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
        let chat = convert(ApiFormat::OpenaiResponse, &responses, ApiFormat::OpenaiCompletion);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].get("role").unwrap(), "system");
        assert_eq!(messages[0].get("content").unwrap(), "系统提示");
        assert_eq!(messages[1].get("content").unwrap(), "part1 part2");
        let parts = messages[2].get("content").unwrap().as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].pointer("/text").unwrap(), "看图");
        assert!(chat.get("instructions").is_none());
    }

    #[test]
    fn anthropic_tool_round_trip_to_chat() {
        // 回归：anthropic 工具回合转 Chat 时 tool_result 曾被内联在 user 消息里
        // 并被 emit 丢弃，DeepSeek 报 "insufficient tool messages following
        // tool_calls message"
        let anthropic = json!({
            "model": "m",
            "max_tokens": 512,
            "messages": [
                { "role": "user", "content": "What's the weather?" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": { "city": "Paris" } }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "23C sunny" }
                ]}
            ],
            "tools": [{ "name": "get_weather", "input_schema": { "type": "object" } }]
        });
        let chat = convert(ApiFormat::AnthropicMessage, &anthropic, ApiFormat::OpenaiCompletion);
        let messages = chat.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].pointer("/tool_calls/0/id").unwrap(), "toolu_1");
        assert_eq!(messages[2].get("role").unwrap(), "tool");
        assert_eq!(messages[2].get("tool_call_id").unwrap(), "toolu_1");
        assert_eq!(messages[2].get("content").unwrap(), "23C sunny");
    }
}
