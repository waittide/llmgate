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

    let mut system_prompt = String::new();
    let mut anthropic_messages = Vec::new();

    if let Some(messages) = val.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg
                .get("content")
                .cloned()
                .unwrap_or(Value::String(String::new()));

            if role == "system" {
                if let Some(s) = content.as_str() {
                    if !system_prompt.is_empty() {
                        system_prompt.push('\n');
                    }
                    system_prompt.push_str(s);
                }
            } else {
                let mut m = serde_json::Map::new();
                m.insert("role".to_string(), Value::String(role.to_string()));
                m.insert("content".to_string(), content);
                anthropic_messages.push(Value::Object(m));
            }
        }
    }

    if !system_prompt.is_empty() {
        anthropic_obj.insert("system".to_string(), Value::String(system_prompt));
    }
    anthropic_obj.insert("messages".to_string(), Value::Array(anthropic_messages));

    Value::Object(anthropic_obj)
}

/// Convert OpenAI /chat/completions payload to Google Gemini generateContent payload.
pub fn convert_openai_to_gemini(val: &Value) -> Value {
    let mut contents = Vec::new();
    let mut system_instruction = None;

    if let Some(messages) = val.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let text_content = match msg.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };

            if role == "system" {
                system_instruction = Some(serde_json::json!({
                    "parts": [{ "text": text_content }]
                }));
            } else {
                let gemini_role = if role == "assistant" { "model" } else { "user" };
                contents.push(serde_json::json!({
                    "role": gemini_role,
                    "parts": [{ "text": text_content }]
                }));
            }
        }
    }

    let mut gemini_obj = serde_json::Map::new();
    gemini_obj.insert("contents".to_string(), Value::Array(contents));
    if let Some(sys) = system_instruction {
        gemini_obj.insert("system_instruction".to_string(), sys);
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
