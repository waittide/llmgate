//! 响应方向的边界转换：4 个 parser（原始格式 → IR）与 4 个 emitter（IR → 目标格式）。

use serde_json::{Value, json};

use super::{
    ContentPart, RelayResponse, anthropic_stop_reason, anthropic_usage, chat_finish_reason, chat_usage,
    gemini_call_id, gemini_finish_reason, gemini_usage, responses_status, responses_usage, stop_reason_from_anthropic,
    stop_reason_from_chat, stop_reason_from_gemini, stop_reason_from_responses,
};
use crate::relay::{protocol::extract_usage_from_json, state::Usage};

fn as_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn owned_str(v: Option<&Value>) -> Option<String> {
    as_str(v).map(str::to_string)
}

/// 解析 Chat 形态的 arguments（JSON 字符串）为 Value，失败时保留原始字符串。
fn arguments_from_string(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone())),
        Some(other) => other.clone(),
        None => json!({}),
    }
}

fn usage_of(val: &Value) -> Usage {
    extract_usage_from_json(val).unwrap_or_default()
}

// ---------- Parsers ----------

pub(super) fn parse_chat(val: &Value, model_fallback: &str) -> Option<RelayResponse> {
    let message = val.pointer("/choices/0/message")?;
    let mut content = Vec::new();
    // 思考链（reasoning_content）位于正文之前（社区事实标准字段语义）
    if let Some(reasoning) = as_str(message.get("reasoning_content")).filter(|s| !s.is_empty()) {
        content.push(ContentPart::Thinking {
            text:      reasoning.to_string(),
            signature: None,
        });
    }
    if let Some(text) = as_str(message.get("content")).filter(|t| !t.is_empty()) {
        content.push(ContentPart::Text {
            text:          text.to_string(),
            cache_control: None,
        });
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            content.push(ContentPart::ToolCall {
                id:        owned_str(call.get("id")).unwrap_or_default(),
                name:      owned_str(call.pointer("/function/name")).unwrap_or_default(),
                arguments: arguments_from_string(call.pointer("/function/arguments")),
            });
        }
    }
    let stop_reason = stop_reason_from_chat(as_str(val.pointer("/choices/0/finish_reason")));
    Some(RelayResponse {
        model: owned_str(val.get("model")).unwrap_or_else(|| model_fallback.to_string()),
        content,
        stop_reason,
        usage: usage_of(val),
    })
}

pub(super) fn parse_responses(val: &Value, model_fallback: &str) -> Option<RelayResponse> {
    let mut content = Vec::new();
    if let Some(output) = val.get("output").and_then(Value::as_array) {
        for item in output {
            match as_str(item.get("type")) {
                Some("reasoning") => {
                    let text: String = item
                        .get("summary")
                        .and_then(Value::as_array)
                        .map(|summary| {
                            summary
                                .iter()
                                .filter_map(|part| as_str(part.get("text")))
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();
                    if !text.is_empty() {
                        content.push(ContentPart::Thinking { text, signature: None });
                    }
                }
                Some("message") => match item.get("content") {
                    Some(Value::String(text)) if !text.is_empty() => {
                        content.push(ContentPart::Text {
                            text:          text.clone(),
                            cache_control: None,
                        });
                    }
                    Some(Value::Array(parts)) => {
                        for part in parts {
                            if let Some(text) = as_str(part.get("text")).filter(|t| !t.is_empty()) {
                                content.push(ContentPart::Text {
                                    text:          text.to_string(),
                                    cache_control: None,
                                });
                            }
                        }
                    }
                    _ => {}
                },
                Some("function_call") => {
                    content.push(ContentPart::ToolCall {
                        id:        owned_str(item.get("call_id"))
                            .or_else(|| owned_str(item.get("id")))
                            .unwrap_or_default(),
                        name:      owned_str(item.get("name")).unwrap_or_default(),
                        arguments: arguments_from_string(item.get("arguments")),
                    });
                }
                _ => {}
            }
        }
    }
    // status=incomplete（截断/内容过滤）→ 对应 StopReason，与 responses_status 互为逆映射
    let stop_reason = stop_reason_from_responses(
        as_str(val.get("status")),
        val.pointer("/incomplete_details/reason")
            .and_then(Value::as_str),
    );
    Some(RelayResponse {
        model: owned_str(val.get("model")).unwrap_or_else(|| model_fallback.to_string()),
        content,
        stop_reason,
        usage: usage_of(val),
    })
}

pub(super) fn parse_anthropic(val: &Value, model_fallback: &str) -> Option<RelayResponse> {
    let mut content = Vec::new();
    if let Some(blocks) = val.get("content").and_then(Value::as_array) {
        for block in blocks {
            match as_str(block.get("type")) {
                Some("text") => {
                    if let Some(text) = as_str(block.get("text"))
                        && !text.is_empty()
                    {
                        content.push(ContentPart::Text {
                            text:          text.to_string(),
                            cache_control: None,
                        });
                    }
                }
                Some("thinking") => {
                    if let Some(text) = as_str(block.get("thinking"))
                        && !text.is_empty()
                    {
                        content.push(ContentPart::Thinking {
                            text:      text.to_string(),
                            signature: owned_str(block.get("signature")),
                        });
                    }
                }
                Some("tool_use") => {
                    content.push(ContentPart::ToolCall {
                        id:        owned_str(block.get("id")).unwrap_or_default(),
                        name:      owned_str(block.get("name")).unwrap_or_default(),
                        arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                    });
                }
                _ => {}
            }
        }
    }
    let stop_reason = stop_reason_from_anthropic(as_str(val.get("stop_reason")));
    Some(RelayResponse {
        model: owned_str(val.get("model")).unwrap_or_else(|| model_fallback.to_string()),
        content,
        stop_reason,
        usage: usage_of(val),
    })
}

pub(super) fn parse_gemini(val: &Value, model_fallback: &str) -> Option<RelayResponse> {
    let mut content = Vec::new();
    let parts = val
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array);
    for part in parts.into_iter().flatten() {
        if let Some(text) = as_str(part.get("text")) {
            if !text.is_empty() {
                content.push(ContentPart::Text {
                    text:          text.to_string(),
                    cache_control: None,
                });
            }
        } else if let Some(call) = part.get("functionCall")
            && let Some(name) = owned_str(call.get("name"))
        {
            let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
            // (name, args) 哈希确定性生成 id，跨转换可复现
            let call_id = gemini_call_id(&name, &serde_json::to_string(&arguments).unwrap_or_default());
            content.push(ContentPart::ToolCall {
                id: call_id,
                name,
                arguments,
            });
        }
    }
    let stop_reason = stop_reason_from_gemini(as_str(val.pointer("/candidates/0/finishReason")));
    Some(RelayResponse {
        model: owned_str(val.get("modelVersion")).unwrap_or_else(|| model_fallback.to_string()),
        content,
        stop_reason,
        usage: usage_of(val),
    })
}

// ---------- Emitters ----------

pub(super) fn emit_chat(resp: &RelayResponse) -> Value {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls = Vec::new();
    for part in &resp.content {
        match part {
            ContentPart::Text { text: t, .. } => text.push_str(t),
            ContentPart::Thinking { text: t, .. } => thinking.push_str(t),
            ContentPart::ToolCall { id, name, arguments } => tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string())
                }
            })),
            _ => {}
        }
    }
    let message = if tool_calls.is_empty() {
        json!({ "role": "assistant", "content": text })
    } else {
        json!({
            "role": "assistant",
            "content": if text.is_empty() { Value::Null } else { Value::String(text) },
            "tool_calls": tool_calls
        })
    };
    let mut message = message;
    if !thinking.is_empty() {
        message["reasoning_content"] = Value::String(thinking);
    }
    let finish_reason = chat_finish_reason(&resp.stop_reason, !tool_calls.is_empty());
    let usage = &resp.usage;
    json!({
        "id": format!("chatcmpl-{}", nanoid::nanoid!(24)),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": resp.model,
        "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason }],
        "usage": chat_usage(usage)
    })
}

pub(super) fn emit_responses(resp: &RelayResponse) -> Value {
    let mut output = Vec::new();
    for part in &resp.content {
        match part {
            // 思考链转为 reasoning 输出项，保留给支持该形态的客户端
            ContentPart::Thinking { text, .. } => {
                output.push(json!({
                    "id": format!("rs_{}", nanoid::nanoid!(24)),
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": text }]
                }));
            }
            ContentPart::Text { text, .. } if !text.is_empty() => {
                output.push(json!({
                    "id": format!("msg_{}", nanoid::nanoid!(24)),
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": text, "annotations": [], "logprobs": [] }]
                }));
            }
            ContentPart::ToolCall { id, name, arguments } => {
                output.push(json!({
                    "id": format!("fc_{}", nanoid::nanoid!(20)),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": id,
                    "name": name,
                    "arguments": serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string())
                }));
            }
            _ => {}
        }
    }
    let (status, incomplete_details) = responses_status(&resp.stop_reason);
    let usage = &resp.usage;
    json!({
        "id": format!("resp_{}", nanoid::nanoid!(24)),
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "completed_at": chrono::Utc::now().timestamp(),
        "status": status,
        "model": resp.model,
        "output": output,
        "error": null,
        "incomplete_details": incomplete_details,
        "usage": responses_usage(usage)
    })
}

pub(super) fn emit_anthropic(resp: &RelayResponse) -> Value {
    let mut content = Vec::new();
    for part in &resp.content {
        match part {
            // signature 是 Anthropic thinking 块的必填字段；无签名时补空串，
            // 保证客户端按类型解析不报错
            ContentPart::Thinking { text, signature } => {
                content.push(json!({
                    "type": "thinking",
                    "thinking": text,
                    "signature": signature.clone().unwrap_or_default()
                }));
            }
            ContentPart::Text { text, .. } if !text.is_empty() => {
                content.push(json!({ "type": "text", "text": text }));
            }
            ContentPart::ToolCall { id, name, arguments } => {
                content.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": arguments
                }));
            }
            _ => {}
        }
    }
    // Anthropic SDK 要求助手 content 至少含一个块；全空时补空文本块兜底
    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }
    let has_tool_calls = resp
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::ToolCall { .. }));
    let stop_reason = anthropic_stop_reason(&resp.stop_reason, has_tool_calls);
    let usage = &resp.usage;
    json!({
        "id": format!("msg_{}", nanoid::nanoid!(24)),
        "type": "message",
        "role": "assistant",
        "model": resp.model,
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": anthropic_usage(usage)
    })
}

pub(super) fn emit_gemini(resp: &RelayResponse) -> Value {
    let mut parts = Vec::new();
    for part in &resp.content {
        match part {
            ContentPart::Thinking { text, .. } if !text.is_empty() => {
                parts.push(json!({ "text": text }));
            }
            ContentPart::Text { text, .. } if !text.is_empty() => {
                parts.push(json!({ "text": text }));
            }
            ContentPart::ToolCall { name, arguments, .. } => {
                parts.push(json!({ "functionCall": { "name": name, "args": arguments } }));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "text": "" }));
    }
    let finish_reason = gemini_finish_reason(&resp.stop_reason);
    let usage = &resp.usage;
    json!({
        "candidates": [{
            "content": { "role": "model", "parts": parts },
            "finishReason": finish_reason
        }],
        "usageMetadata": gemini_usage(usage),
        "modelVersion": resp.model
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::super::{emit_response, parse_response};
    use crate::relay::protocol::ApiFormat;

    fn convert(src: ApiFormat, resp: &Value, dst: ApiFormat, model: &str) -> Value {
        let ir = parse_response(src, resp, model).unwrap();
        emit_response(&ir, dst)
    }

    #[test]
    fn gemini_to_chat_response() {
        let gemini = json!({
            "candidates": [{
                "content": { "role": "model", "parts": [{ "text": "Hello from Gemini" }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": { "promptTokenCount": 4, "candidatesTokenCount": 3, "totalTokenCount": 7 }
        });
        let openai = convert(
            ApiFormat::GeminiGenerate,
            &gemini,
            ApiFormat::OpenaiCompletion,
            "gemini-2.5-pro",
        );
        assert_eq!(
            openai.pointer("/choices/0/message/content").unwrap(),
            "Hello from Gemini"
        );
        assert_eq!(openai.pointer("/choices/0/finish_reason").unwrap(), "stop");
        assert_eq!(openai.pointer("/usage/total_tokens").unwrap(), 7);
    }

    #[test]
    fn chat_to_gemini_response() {
        let chat = json!({
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
        let gemini = convert(
            ApiFormat::OpenaiCompletion,
            &chat,
            ApiFormat::GeminiGenerate,
            "gemini-2.5-pro",
        );
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
    fn responses_to_chat_response() {
        let result = json!({
            "id": "resp_1",
            "status": "completed",
            "output": [
                { "type": "message", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "答案" }] }
            ],
            "usage": { "input_tokens": 8, "output_tokens": 2, "total_tokens": 10 }
        });
        let chat = convert(ApiFormat::OpenaiResponse, &result, ApiFormat::OpenaiCompletion, "m");
        assert_eq!(chat.get("object").unwrap(), "chat.completion");
        let choice = chat.pointer("/choices/0").unwrap();
        assert_eq!(choice.pointer("/message/content").unwrap(), "答案");
        assert_eq!(choice.get("finish_reason").unwrap(), "stop");
        assert_eq!(chat.pointer("/usage/prompt_tokens").unwrap(), 8);
    }

    #[test]
    fn responses_string_content_and_incomplete() {
        let string_content = json!({
            "id": "resp_2",
            "status": "completed",
            "output": [ { "type": "message", "role": "assistant", "content": "直接文本" } ],
            "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
        });
        let chat = convert(
            ApiFormat::OpenaiResponse,
            &string_content,
            ApiFormat::OpenaiCompletion,
            "m",
        );
        assert_eq!(chat.pointer("/choices/0/message/content").unwrap(), "直接文本");

        let truncated = json!({
            "id": "resp_3",
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "output": [],
            "usage": { "input_tokens": 1, "output_tokens": 0, "total_tokens": 1 }
        });
        let chat = convert(ApiFormat::OpenaiResponse, &truncated, ApiFormat::OpenaiCompletion, "m");
        assert_eq!(chat.pointer("/choices/0/finish_reason").unwrap(), "length");

        // 内容过滤也是 incomplete，但理由必须区分：客户端据此判断是重试还是改内容
        let filtered = json!({
            "id": "resp_4",
            "status": "incomplete",
            "incomplete_details": { "reason": "content_filter" },
            "output": [],
            "usage": { "input_tokens": 1, "output_tokens": 0, "total_tokens": 1 }
        });
        let chat = convert(ApiFormat::OpenaiResponse, &filtered, ApiFormat::OpenaiCompletion, "m");
        assert_eq!(chat.pointer("/choices/0/finish_reason").unwrap(), "content_filter");

        // 无明细的 incomplete 仍按截断处理
        let no_detail = json!({ "status": "incomplete", "output": [] });
        let chat = convert(ApiFormat::OpenaiResponse, &no_detail, ApiFormat::OpenaiCompletion, "m");
        assert_eq!(chat.pointer("/choices/0/finish_reason").unwrap(), "length");
    }

    #[test]
    fn chat_to_responses_incomplete_status() {
        let chat = json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "被截断" },
                "finish_reason": "length"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 6, "total_tokens": 11 }
        });
        let resp = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse, "m");
        assert_eq!(resp.get("status").unwrap(), "incomplete");
        assert_eq!(resp.pointer("/incomplete_details/reason").unwrap(), "max_output_tokens");

        // 内容过滤同属未完成，理由与截断区分开
        let filtered = json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "" },
                "finish_reason": "content_filter"
            }]
        });
        let resp = convert(ApiFormat::OpenaiCompletion, &filtered, ApiFormat::OpenaiResponse, "m");
        assert_eq!(resp.get("status").unwrap(), "incomplete");
        assert_eq!(resp.pointer("/incomplete_details/reason").unwrap(), "content_filter");
    }

    #[test]
    fn gemini_finish_reason_content_filter() {
        let gemini = json!({
            "candidates": [{
                "content": { "role": "model", "parts": [] },
                "finishReason": "SAFETY"
            }]
        });
        let openai = convert(
            ApiFormat::GeminiGenerate,
            &gemini,
            ApiFormat::OpenaiCompletion,
            "gemini-2.5-pro",
        );
        assert_eq!(openai.pointer("/choices/0/finish_reason").unwrap(), "content_filter");
    }

    #[test]
    fn anthropic_to_chat_tool_calls() {
        let anthropic_resp = json!({
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
        let openai = convert(
            ApiFormat::AnthropicMessage,
            &anthropic_resp,
            ApiFormat::OpenaiCompletion,
            "claude-3-5-sonnet",
        );
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
    fn truncated_tool_call_keeps_max_tokens_stop_reason() {
        // 截断发生在工具参数只生成了一半时：stop_reason 必须是 max_tokens，
        // 报 tool_use 会让客户端把残缺 JSON 当作完整调用执行。
        let chat = json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "call_1", "type": "function",
                      "function": { "name": "calc", "arguments": "{\"x\":" } }
                ]},
                "finish_reason": "length"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2 }
        });
        let anthropic = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::AnthropicMessage, "m");
        assert_eq!(anthropic.pointer("/stop_reason").unwrap(), "max_tokens");

        // 正常结束且有工具调用时仍报 tool_use
        let done = json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "call_1", "type": "function",
                      "function": { "name": "calc", "arguments": "{\"x\":1}" } }
                ]},
                "finish_reason": "tool_calls"
            }]
        });
        let anthropic = convert(ApiFormat::OpenaiCompletion, &done, ApiFormat::AnthropicMessage, "m");
        assert_eq!(anthropic.pointer("/stop_reason").unwrap(), "tool_use");
    }

    #[test]
    fn anthropic_thinking_to_reasoning_content() {
        let anthropic_resp = json!({
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
        let openai = convert(
            ApiFormat::AnthropicMessage,
            &anthropic_resp,
            ApiFormat::OpenaiCompletion,
            "claude-4.5",
        );
        assert_eq!(
            openai
                .pointer("/choices/0/message/reasoning_content")
                .unwrap(),
            "深入思考中"
        );
        assert_eq!(openai.pointer("/choices/0/message/content").unwrap(), "答案");
    }

    #[test]
    fn chat_to_anthropic_preserves_reasoning_and_empty_content() {
        let chat = json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "", "reasoning_content": "推理过程" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2 }
        });
        let anthropic = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::AnthropicMessage, "m");
        assert_eq!(anthropic.pointer("/content/0/type").unwrap(), "thinking");
        assert_eq!(anthropic.pointer("/content/0/thinking").unwrap(), "推理过程");
        // 无真实签名时补空串：Anthropic 的 thinking 块 signature 必填
        assert_eq!(anthropic.pointer("/content/0/signature").unwrap(), "");

        let chat_empty = json!({
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 0, "completion_tokens": 0 }
        });
        let anthropic = convert(
            ApiFormat::OpenaiCompletion,
            &chat_empty,
            ApiFormat::AnthropicMessage,
            "m",
        );
        let content = anthropic.get("content").unwrap().as_array().unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn responses_to_anthropic_preserves_reasoning_summary() {
        let resp = json!({
            "output": [
                { "id": "rs_1", "type": "reasoning", "summary": [{ "type": "summary_text", "text": "摘要思考" }] },
                { "id": "msg_1", "type": "message", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "正文" }] }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 4 }
        });
        let anthropic = convert(ApiFormat::OpenaiResponse, &resp, ApiFormat::AnthropicMessage, "m");
        assert_eq!(anthropic.pointer("/content/0/type").unwrap(), "thinking");
        assert_eq!(anthropic.pointer("/content/0/thinking").unwrap(), "摘要思考");
        assert_eq!(anthropic.pointer("/content/1/type").unwrap(), "text");
    }

    #[test]
    fn chat_to_responses_preserves_reasoning() {
        let chat = json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "答案", "reasoning_content": "思考" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3 }
        });
        let resp = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse, "m");
        let output = resp.get("output").unwrap().as_array().unwrap();
        assert_eq!(output[0].get("type").unwrap(), "reasoning");
        assert_eq!(output[0].pointer("/summary/0/text").unwrap(), "思考");
    }

    #[test]
    fn anthropic_to_chat_response() {
        let anthropic_resp = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hi from Claude" }],
            "model": "claude-3-5-sonnet",
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 12, "output_tokens": 8 }
        });
        let openai = convert(
            ApiFormat::AnthropicMessage,
            &anthropic_resp,
            ApiFormat::OpenaiCompletion,
            "claude-3-5-sonnet",
        );
        assert_eq!(openai.get("model").unwrap(), "claude-3-5-sonnet");
        let choices = openai.get("choices").unwrap().as_array().unwrap();
        assert_eq!(choices[0].pointer("/message/content").unwrap(), "Hi from Claude");
        assert_eq!(openai.pointer("/usage/prompt_tokens").unwrap(), 12);
        assert_eq!(openai.pointer("/usage/completion_tokens").unwrap(), 8);
    }

    #[test]
    fn chat_to_responses_response() {
        let chat = json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hello" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3 }
        });
        let response = convert(
            ApiFormat::OpenaiCompletion,
            &chat,
            ApiFormat::OpenaiResponse,
            "test-model",
        );
        assert_eq!(response.get("object").unwrap(), "response");
        assert_eq!(response.pointer("/output/0/content/0/text").unwrap(), "Hello");
        assert_eq!(response.pointer("/usage/total_tokens").unwrap(), 3);
    }

    #[test]
    fn chat_to_anthropic_response() {
        let chat = json!({
            "choices": [{ "message": { "content": "Hello" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 2, "completion_tokens": 1 }
        });
        let response = convert(
            ApiFormat::OpenaiCompletion,
            &chat,
            ApiFormat::AnthropicMessage,
            "test-model",
        );
        assert_eq!(response.get("type").unwrap(), "message");
        assert_eq!(response.pointer("/content/0/text").unwrap(), "Hello");
        assert_eq!(response.get("stop_reason").unwrap(), "end_turn");
    }

    #[test]
    fn cache_usage_round_trips_between_protocols() {
        // Anthropic → Chat：缓存明细落到 prompt_tokens_details
        let anthropic = json!({
            "model": "claude-3-5-sonnet",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "ok" }],
            "usage": {
                "input_tokens": 20,
                "cache_read_input_tokens": 80,
                "cache_creation_input_tokens": 10,
                "output_tokens": 5
            }
        });
        let chat = convert(
            ApiFormat::AnthropicMessage,
            &anthropic,
            ApiFormat::OpenaiCompletion,
            "m",
        );
        assert_eq!(chat.pointer("/usage/prompt_tokens").unwrap(), 110);
        assert_eq!(
            chat.pointer("/usage/prompt_tokens_details/cached_tokens")
                .unwrap(),
            80
        );

        // Chat → Anthropic：input_tokens 拆出未命中部分，缓存单列（相加即总输入）
        let anthropic = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::AnthropicMessage, "m");
        assert_eq!(anthropic.pointer("/usage/input_tokens").unwrap(), 30);
        assert_eq!(anthropic.pointer("/usage/cache_read_input_tokens").unwrap(), 80);
        assert_eq!(anthropic.pointer("/usage/output_tokens").unwrap(), 5);
    }

    #[test]
    fn responses_cache_usage_uses_input_details() {
        let chat = json!({
            "choices": [{ "message": { "content": "ok" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 4,
                "total_tokens": 54,
                "prompt_tokens_details": { "cached_tokens": 30 }
            }
        });
        let responses = convert(ApiFormat::OpenaiCompletion, &chat, ApiFormat::OpenaiResponse, "m");
        assert_eq!(
            responses
                .pointer("/usage/input_tokens_details/cached_tokens")
                .unwrap(),
            30
        );
        // 回读：Responses 的 cached_tokens 必须被用量提取识别（否则计费丢折扣）
        assert_eq!(
            crate::relay::protocol::extract_usage_from_json(&responses)
                .unwrap()
                .cache_read_tokens,
            30
        );
    }
}
