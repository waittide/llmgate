use std::{sync::LazyLock, time::Duration};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use reqwest::{Client, RequestBuilder, header::HeaderMap};

use crate::model::{Channel, ChannelProvider};

pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
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
    api_key: &str,
    endpoint: &str, // 例如 "/chat/completions", "/messages", "/models"
    body: Option<Bytes>,
    timeout_secs: Option<u64>,
    client_headers: Option<&HeaderMap>,
) -> Result<RequestBuilder> {
    let base_url = normalize_base_url(&channel.base_url);
    let target_url = format!("{}{}", base_url, endpoint);

    let headers = build_upstream_headers(channel, api_key, client_headers)?;
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

        let headers = build_upstream_headers(&channel, "sk-ant-test", Some(&client_headers)).unwrap();
        // 客户端传入的 anthropic-beta 应被正常透传
        assert_eq!(headers.get("anthropic-beta").unwrap(), "prompt-caching-2024-07-31");
        // x-api-key 鉴权头必须强制使用渠道 key（此处为显式传入的 api_key 参数），不能被客户端覆盖
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-test");
    }
}
