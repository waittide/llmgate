use crate::{
    model::LLMPrice,
    relay::state::Usage,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenaiCompletion,
    OpenAIResponse,
    AnthropicMessage,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequestMetadata {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub stream: bool,
}

pub fn parse_request_metadata(body: &[u8]) -> Option<ChatRequestMetadata> {
    serde_json::from_slice(body).ok()
}

pub fn rewrite_model_in_json(body: &[u8], new_model: &str) -> Vec<u8> {
    if let Ok(mut val) = serde_json::from_slice::<Value>(body)
        && let Some(obj) = val.as_object_mut()
    {
        obj.insert("model".to_string(), Value::String(new_model.to_string()));
        if let Ok(bytes) = serde_json::to_vec(&val) {
            return bytes;
        }
    }
    body.to_vec()
}

pub fn calculate_cost(
    prompt_tokens: i64,
    completion_tokens: i64,
    price: Option<&LLMPrice>,
) -> f64 {
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
