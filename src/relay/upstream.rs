use std::{fmt::Write as _, sync::LazyLock, time::Duration};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use reqwest::{Client, RequestBuilder, header::HeaderMap};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    model::{Channel, ChannelProvider},
    utils::truncate_utf8,
};

/// 会话指纹的取材上限：首条 user 消息超过此长度只取前缀，避免为超长首轮
/// （整段粘贴、大文件）重复付出全量哈希与拷贝的代价。
const SESSION_SEED_LIMIT: usize = 8 * 1024;

/// 会话 ID 长度（十六进制字符）：128 位散列足以区分会话，且不撑大请求头。
const SESSION_ID_LEN: usize = 32;

/// 采信客户端自带会话值的长度上限，超长值不采信，退回推导值。
const CLIENT_SESSION_LIMIT: usize = 128;

pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

/// 推导请求级会话 ID：`salt` 为调用方身份（API Key ID），`body` 为客户端请求体。
///
/// 前缀取首条 user 消息：编码类客户端每一轮都会重放同一段历史，该消息在整个
/// 会话期内保持不变，同一会话的每轮请求因此得到同一 ID；不同会话的首条消息不同，
/// ID 自然区分。请求形状无法识别或没有 user 消息时，退化为按身份的固定 ID，
/// 仍然满足「稳定」要求，只是同身份的这类请求会共用一个会话。
pub fn resolve_session_id(salt: i64, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.to_le_bytes());
    if let Some(seed) = session_seed(body) {
        hasher.update([0x1F]);
        hasher.update(seed.as_bytes());
    }
    let digest = hasher.finalize();
    let mut id = String::with_capacity(SESSION_ID_LEN);
    for byte in digest.iter().take(SESSION_ID_LEN / 2) {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

/// 客户端自带同名会话头时取其值，便于 Hermes、Kilo Code 等已自行管理会话边界的
/// 客户端沿用真实会话；空值、非 ASCII 或超长值不采信，退回推导值。
fn client_session_value<'a>(client_headers: Option<&'a HeaderMap>, name: &str) -> Option<&'a str> {
    let value = client_headers?.get(name)?.to_str().ok()?.trim();
    (!value.is_empty() && value.len() <= CLIENT_SESSION_LIMIT).then_some(value)
}

/// 会话前缀：首条 user 消息的文本，兼容 Chat Completions / Anthropic Messages
/// （`messages`）、Responses API（`input`）与 Gemini GenerateContent（`contents`）。
fn session_seed(body: &[u8]) -> Option<String> {
    let val: Value = serde_json::from_slice(body).ok()?;
    if let Some(text) = first_user_text(val.get("messages")) {
        return Some(text);
    }
    if let Some(text) = gemini_first_user_text(val.get("contents")) {
        return Some(text);
    }
    match val.get("input")? {
        Value::String(text) => text_seed(text),
        Value::Array(items) => items
            .iter()
            .find(|item| item.get("role").and_then(Value::as_str) == Some("user"))
            .and_then(|item| content_text(item.get("content"))),
        _ => None,
    }
}

fn first_user_text(messages: Option<&Value>) -> Option<String> {
    messages?
        .as_array()?
        .iter()
        .find(|msg| msg.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|msg| content_text(msg.get("content")))
}

/// Gemini：`contents[]` 中 role 为 user 的 parts 文本。
fn gemini_first_user_text(contents: Option<&Value>) -> Option<String> {
    contents?
        .as_array()?
        .iter()
        .find(|item| item.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|item| content_text(item.get("parts")))
}

/// 取出消息文本：字符串直接使用；内容块数组只拼接其中的 `text` 字段
/// （Anthropic 文本块、OpenAI 多模态块）。纯图片等无文本消息返回 None。
fn content_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(text) => text_seed(text),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                    text.push_str(part_text);
                }
                if text.len() >= SESSION_SEED_LIMIT {
                    break;
                }
            }
            text_seed(&text)
        }
        _ => None,
    }
}

/// 截断到取材上限的非空文本；空文本不构成指纹（如纯工具结果轮次）。
fn text_seed(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| truncate_utf8(text, SESSION_SEED_LIMIT).to_string())
}

static SHARED_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .tcp_nodelay(true)
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(32)
        .build()
        .expect("初始化共享 HTTP 客户端失败")
});

pub fn get_client() -> &'static Client {
    &SHARED_HTTP_CLIENT
}

pub fn build_upstream_request(
    client: &Client,
    channel: &Channel,
    endpoint: &str, // 例如 "/chat/completions", "/messages", "/models"
    body: Option<Bytes>,
    timeout_secs: Option<u64>,
    headers: HeaderMap,
) -> Result<RequestBuilder> {
    let base_url = normalize_base_url(&channel.base_url);
    let target_url = format!("{}{}", base_url, endpoint);

    let mut req_builder = if body.is_some() {
        client.post(&target_url)
    } else {
        client.get(&target_url)
    };

    req_builder = req_builder.headers(headers);

    if let Some(timeout_secs) = timeout_secs {
        req_builder = req_builder.timeout(Duration::from_secs(timeout_secs));
    }

    if let Some(b) = body {
        req_builder = req_builder.body(b);
    }

    Ok(req_builder)
}

pub fn build_upstream_headers(
    channel: &Channel,
    api_key: &str,
    client_headers: Option<&HeaderMap>,
    session_id: &str,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    match channel.channel_type {
        ChannelProvider::Anthropic => {
            headers.insert(
                "x-api-key",
                api_key
                    .parse()
                    .map_err(|_| anyhow!("无效的 api key 请求头"))?,
            );
            headers.insert(
                "anthropic-version",
                "2023-06-01"
                    .parse()
                    .map_err(|_| anyhow!("无效的 anthropic-version 请求头"))?,
            );
        }
        ChannelProvider::Gemini => {
            headers.insert(
                "x-goog-api-key",
                api_key
                    .parse()
                    .map_err(|_| anyhow!("无效的 gemini key 请求头"))?,
            );
        }
        ChannelProvider::OpenaiCompletion | ChannelProvider::OpenaiResponses => {
            headers.insert(
                "Authorization",
                format!("Bearer {}", api_key)
                    .parse()
                    .map_err(|_| anyhow!("无效的鉴权请求头"))?,
            );
        }
    }

    headers.insert(
        "Content-Type",
        "application/json"
            .parse()
            .map_err(|_| anyhow!("无效的 content-type"))?,
    );

    // 会话标识：渠道开启「分配会话标识」后，每轮请求以配置的头名携带会话 ID，
    // 客户端自带同名头（Hermes、Kilo Code 等）则沿用其值。空 ID 不写入 ——
    // `/models` 探测这类无会话语义的请求复用同一构建函数。
    if let Some(name) = channel.session_header_name()
        && !session_id.is_empty()
    {
        let value = client_session_value(client_headers, name).unwrap_or(session_id);
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| anyhow!("无效的会话标识头名"))?,
            value.parse().map_err(|_| anyhow!("无效的会话标识请求头"))?,
        );
    }

    // 2. 透传客户端特定的业务 Header（如 anthropic-beta, openai-organization）
    if let Some(client_hdrs) = client_headers {
        for (name, value) in client_hdrs.iter() {
            let name_str = name.as_str().to_lowercase();
            if name_str.starts_with("anthropic-beta")
                || name_str == "openai-organization"
                || name_str == "openai-project"
            {
                headers.insert(name.clone(), value.clone());
            }
        }
    }

    // 3. 应用渠道自定义 Header（最高优先级，覆盖默认值或客户端透传头）
    for h in &channel.custom_header {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(h.header_key.as_bytes()),
            h.header_value.parse(),
        ) {
            headers.insert(name, val);
        }
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_base_url() {
        assert_eq!(normalize_base_url("https://api.openai.com"), "https://api.openai.com");
        assert_eq!(normalize_base_url("https://api.openai.com/"), "https://api.openai.com");
        assert_eq!(
            normalize_base_url("https://api.openai.com///"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_base_url("https://custom.gateway.com/v1"),
            "https://custom.gateway.com/v1"
        );
        assert_eq!(
            normalize_base_url("https://custom.gateway.com/v1/"),
            "https://custom.gateway.com/v1"
        );
        assert_eq!(
            normalize_base_url("https://custom.gateway.com/api"),
            "https://custom.gateway.com/api"
        );
    }

    #[test]
    fn test_build_upstream_headers_passthrough_anthropic_beta() {
        use reqwest::header::HeaderValue;
        let channel = Channel {
            channel_type: ChannelProvider::Anthropic,
            key: "sk-ant-test".to_string(),
            ..Default::default()
        };
        let mut client_headers = HeaderMap::new();
        client_headers.insert("anthropic-beta", HeaderValue::from_static("prompt-caching-2024-07-31"));
        client_headers.insert("authorization", HeaderValue::from_static("Bearer client-key"));

        let headers = build_upstream_headers(&channel, "sk-ant-test", Some(&client_headers), "").unwrap();
        // 客户端传入的 anthropic-beta 应被正常透传
        assert_eq!(headers.get("anthropic-beta").unwrap(), "prompt-caching-2024-07-31");
        // x-api-key 鉴权头必须强制使用渠道 key（此处为显式传入的 api_key 参数），不能被客户端覆盖
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-test");
    }

    /// 未开启「分配会话标识」的普通渠道。
    fn openai_channel() -> Channel {
        Channel {
            channel_type: ChannelProvider::OpenaiCompletion,
            key: "sk-ch".to_string(),
            ..Default::default()
        }
    }

    /// 开启「分配会话标识」的渠道。
    fn session_channel(name: &str) -> Channel {
        Channel {
            channel_type: ChannelProvider::OpenaiCompletion,
            key: "sk-ch".to_string(),
            session_header: name.to_string(),
            ..Default::default()
        }
    }

    /// 渠道开启会话标识后，上游请求必须带上该头与推导出的会话 ID（OpenCode Go
    /// 这类上游缺失时直接 400）；未开启或没有会话 ID 时不得凭空写入。
    #[test]
    fn upstream_request_carries_session_header_when_enabled() {
        let body = br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#;
        let session = resolve_session_id(7, body);
        let channel = session_channel("x-opencode-session");
        let headers = build_upstream_headers(&channel, "sk-ch", None, &session).unwrap();
        assert_eq!(headers.get("x-opencode-session").unwrap(), session.as_str());

        // 未开启：不写会话头
        let headers = build_upstream_headers(&openai_channel(), "sk-ch", None, &session).unwrap();
        assert!(headers.get("x-opencode-session").is_none());
        // 空会话 ID（如 /models 探测）不写会话头
        let headers = build_upstream_headers(&channel, "sk-ch", None, "").unwrap();
        assert!(headers.get("x-opencode-session").is_none());
        // 头名非法（含空格 / 非 ASCII）视为未开启，坏配置不能拖垮请求构建
        for bad in ["x session id", "会话标识", "   "] {
            let headers = build_upstream_headers(&session_channel(bad), "sk-ch", None, &session).unwrap();
            assert_eq!(
                headers.len(),
                build_upstream_headers(&openai_channel(), "sk-ch", None, "")
                    .unwrap()
                    .len()
            );
        }
    }

    /// 客户端自带同名会话头时沿用其值（它更清楚会话边界），空值则退回推导值。
    #[test]
    fn client_session_header_wins_over_derived_id() {
        use reqwest::header::HeaderValue;
        let channel = session_channel("x-opencode-session");
        let mut client_headers = HeaderMap::new();
        client_headers.insert("x-opencode-session", HeaderValue::from_static("client-session-1"));
        let headers = build_upstream_headers(&channel, "sk-ch", Some(&client_headers), "derived").unwrap();
        assert_eq!(headers.get("x-opencode-session").unwrap(), "client-session-1");

        // 客户端发的是别的会话头名：不采信，用推导值
        let mut other_name = HeaderMap::new();
        other_name.insert("x-session-id", HeaderValue::from_static("client-session-2"));
        let headers = build_upstream_headers(&channel, "sk-ch", Some(&other_name), "derived").unwrap();
        assert_eq!(headers.get("x-opencode-session").unwrap(), "derived");
    }

    /// 同一会话的每一轮请求重放同一前缀，必须得到同一会话 ID；不同会话必须区分。
    #[test]
    fn session_id_is_stable_per_conversation() {
        let first_turn = r#"{"model":"m","messages":[{"role":"user","content":"写一个快排"}]}"#;
        let later_turn = r#"{"model":"m","messages":[{"role":"user","content":"写一个快排"},{"role":"assistant","content":"..."},{"role":"user","content":"再优化一下"}]}"#;
        let other_conversation = r#"{"model":"m","messages":[{"role":"user","content":"写一个归并"}]}"#;

        let id = resolve_session_id(7, first_turn.as_bytes());
        assert_eq!(id.len(), SESSION_ID_LEN);
        assert_eq!(id, resolve_session_id(7, later_turn.as_bytes()));
        assert_ne!(id, resolve_session_id(7, other_conversation.as_bytes()));
        // 身份（密钥）不同即不同会话
        assert_ne!(id, resolve_session_id(8, first_turn.as_bytes()));
    }

    /// 四种入站协议的请求形状都要能取到会话前缀。
    #[test]
    fn session_seed_covers_every_request_shape() {
        let expected = resolve_session_id(7, br#"{"messages":[{"role":"user","content":"hello"}]}"#);
        let shapes = [
            // Anthropic：内容块数组，图片块无文本
            r#"{"model":"m","messages":[{"role":"user","content":[{"type":"text","text":"hello"},{"type":"image","source":{}}]}]}"#,
            // Responses：顶层 input 字符串
            r#"{"model":"m","input":"hello"}"#,
            // Gemini：contents[].parts[]
            r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#,
        ];
        for shape in shapes {
            assert_eq!(resolve_session_id(7, shape.as_bytes()), expected, "{shape}");
        }

        // 无会话前缀（非 JSON、无 user 消息）时退化为按身份的固定 ID，仍然稳定
        let no_prefix = resolve_session_id(7, b"not-json");
        assert_eq!(no_prefix.len(), SESSION_ID_LEN);
        assert_eq!(no_prefix, resolve_session_id(7, b""));
    }
}
