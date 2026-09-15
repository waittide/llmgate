use serde::Deserialize;
use serde_json::Value;

use crate::{model::ModelPrice, relay::state::Usage};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    OpenaiCompletion,
    OpenaiResponse,
    AnthropicMessage,
    GeminiGenerate,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ChatRequestMetadata {
    pub model:      String,
    pub stream:     bool,
    /// 本轮请求的会话标识：由网关按「调用方身份 + 会话前缀」推导一次，渠道开启
    /// 「分配会话标识」时写入上游请求头。不来自请求体，故反序列化时跳过。
    #[serde(skip)]
    pub session_id: String,
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
                if v.is_null() {
                    // 值为 null 约定为删除该字段：用于剥离上游不接受的客户端专有字段
                    // （如 GLM 等严格校验上游拒绝的 OpenAI prompt_cache_key/store/include）
                    obj.remove(k);
                } else {
                    obj.insert(k.clone(), v.clone());
                }
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
            .or_else(|| usage_obj.pointer("/input_tokens_details/cached_tokens"))
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
    fn param_override_null_deletes_field() {
        let body = br#"{"model":"gpt-x","prompt_cache_key":"abc","store":false,"messages":[]}"#;

        // null 值 → 删除字段（剥离严格上游拒绝的客户端专有字段）
        let out = rewrite_request_body_in_json(body, "glm-5.3-flash", Some(r#"{"prompt_cache_key": null}"#));
        let val: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(val.get("model").and_then(Value::as_str), Some("glm-5.3-flash"));
        assert!(val.get("prompt_cache_key").is_none());
        assert_eq!(val.get("store").and_then(Value::as_bool), Some(false));

        // 非 null 值 → 保持原有合并语义
        let out2 = rewrite_request_body_in_json(body, "m", Some(r#"{"temperature": 0.5}"#));
        let val2: Value = serde_json::from_slice(&out2).unwrap();
        assert_eq!(val2.get("temperature").and_then(Value::as_f64), Some(0.5));
    }
}
