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
    if let Some(data_uri) = url.strip_prefix("data:") {
        if let Some((meta, data)) = data_uri.split_once(',') {
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
            let name = call.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = call.pointer("/function/arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let input = serde_json::from_str::<Value>(arguments)
                .unwrap_or_else(|_| Value::String(arguments.to_string()));
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

/// Converts an OpenAI `tool` role message into an Anthropic `user` message with
/// a single `tool_result` content block.
fn openai_tool_to_anthropic(msg: &Value) -> Value {
    let tool_use_id = msg.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
    let result_content = msg
        .get("content")
        .cloned()
        .unwrap_or_else(|| Value::String(String::new()));
    serde_json::json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": result_content
        }]
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
            let name = tool.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let mut t = serde_json::Map::new();
            t.insert("name".to_string(), Value::String(name.to_string()));
            if let Some(desc) = tool.pointer("/function/description").and_then(|v| v.as_str()) {
                if !desc.is_empty() {
                    t.insert("description".to_string(), Value::String(desc.to_string()));
                }
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
                anthropic_obj.insert(
                    "tool_choice".to_string(),
                    serde_json::json!({ "type": t }),
                );
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
                "tool" => anthropic_messages.push(openai_tool_to_anthropic(msg)),
                _ => {
                    let content = openai_content_to_anthropic(
                        msg.get("content").unwrap_or(&Value::String(String::new())),
                    );
                    anthropic_messages.push(serde_json::json!({
                        "role": role,
                        "content": content
                    }));
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

/// Converts an OpenAI `image_url` content part to a Gemini content part. Base64
/// `data:` URIs become `inlineData`; public URLs become `fileData` (Gemini
/// fetches publicly accessible URIs server-side).
fn openai_image_to_gemini_part(url: &str) -> Option<Value> {
    if url.is_empty() {
        return None;
    }
    if let Some(data_uri) = url.strip_prefix("data:") {
        if let Some((meta, data)) = data_uri.split_once(',') {
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
    let mut tool_call_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

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
                    let tool_call_id = msg.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = tool_call_names.get(tool_call_id).cloned().unwrap_or_default();
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
                    let parts = openai_content_to_gemini_parts(
                        msg.get("content").unwrap_or(&Value::String(String::new())),
                    );
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
            let name = tool.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let mut f = serde_json::Map::new();
            f.insert("name".to_string(), Value::String(name.to_string()));
            if let Some(desc) = tool.pointer("/function/description").and_then(|v| v.as_str()) {
                if !desc.is_empty() {
                    f.insert("description".to_string(), Value::String(desc.to_string()));
                }
            }
            if let Some(params) = tool.pointer("/function/parameters") {
                if !params.is_null() {
                    f.insert("parameters".to_string(), params.clone());
                }
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
                    cfg.insert(
                        "allowedFunctionNames".to_string(),
                        serde_json::json!([fname]),
                    );
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
    if let Some(usage_obj) = val.get("usage") {
        let prompt_tokens = usage_obj
            .get("prompt_tokens")
            .or_else(|| usage_obj.get("input_tokens"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let completion_tokens = usage_obj
            .get("completion_tokens")
            .or_else(|| usage_obj.get("output_tokens"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let total_tokens = usage_obj
            .get("total_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(prompt_tokens + completion_tokens);

        Some(Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        })
    } else {
        None
    }
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
        assert_eq!(
            anthropic.get("tool_choice").unwrap().get("type").unwrap(),
            "auto"
        );

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
        let decls = tools[0].get("functionDeclarations").unwrap().as_array().unwrap();
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
        assert_eq!(
            model_parts[0].pointer("/functionCall/name").unwrap(),
            "get_weather"
        );
        assert_eq!(
            model_parts[0].pointer("/functionCall/args/city").unwrap(),
            "Paris"
        );
        // tool message maps to functionResponse with the resolved function name
        let user_parts = contents[2].get("parts").unwrap().as_array().unwrap();
        assert_eq!(
            user_parts[0].pointer("/functionResponse/name").unwrap(),
            "get_weather"
        );
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
        assert_eq!(
            content[1].pointer("/source/type").unwrap(),
            "url"
        );
        assert_eq!(
            content[1].pointer("/source/url").unwrap(),
            "https://example.com/a.png"
        );
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
}
