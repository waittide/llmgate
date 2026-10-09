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

/// 无需渠道配置、默认透传给上游的客户端 Header。
///
/// 这里放的是传输层与内容协商头：客户端直连上游时本就由客户端自己决定，
/// 网关原样转发才能让上游看到与直连一致的行为（压缩编码协商、UA 识别等）。
/// 鉴权类头名不在其列——它们由渠道 key 独占，只能经渠道透传名单显式放行。
///
/// `connection` 是 HTTP/1.1 专属的逐跳头：HTTP/2 判定其非法（RFC 9113 §8.2.2），
/// h2 客户端会以 MalformedHeaders 直接拒绝整个请求。因此只在确定走 HTTP/1.1 的
/// 明文 http 目标上透传；https 目标可能经 ALPN 协商成 h2，一律不带。
fn is_builtin_passthrough(name: &str, http1_only: bool) -> bool {
    match name {
        "accept" | "accept-encoding" | "user-agent" | "openai-organization" | "openai-project" => true,
        "connection" => http1_only,
        _ => name.starts_with("anthropic-beta"),
    }
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

    // 2. 透传客户端 Header：内置白名单 + 渠道透传名单（客户端值原样转发，
    //    网关不推导也不改写；鉴权类头名由渠道 key 独占，见 allows_passthrough）
    //    只有明文 http 目标确定走 HTTP/1.1（reqwest 不会对其发起 h2c 升级）
    let http1_only = channel
        .base_url
        .trim()
        .to_ascii_lowercase()
        .starts_with("http://");
    if let Some(client_hdrs) = client_headers {
        for (name, value) in client_hdrs.iter() {
            // 解析后的 Header 名已是小写，无需再分配一个小写副本
            let name_str = name.as_str();
            if is_builtin_passthrough(name_str, http1_only) || channel.allows_passthrough(name_str) {
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

    /// 内置白名单只覆盖传输层与内容协商头：鉴权、路由、实体元数据头一律不默认上行。
    #[test]
    fn builtin_passthrough_covers_transport_headers_only() {
        for name in [
            "accept",
            "accept-encoding",
            "connection",
            "user-agent",
            "anthropic-beta",
            "anthropic-beta-prompt-caching-2024-07-31",
            "openai-organization",
            "openai-project",
        ] {
            assert!(is_builtin_passthrough(name, true), "{name} 应默认透传");
        }
        for name in [
            "authorization",
            "x-api-key",
            "host",
            "content-length",
            "content-type",
            "x-unlisted",
        ] {
            assert!(!is_builtin_passthrough(name, true), "{name} 不应默认透传");
        }
    }

    /// 逐跳头 `connection` 只在确定走 HTTP/1.1 的目标上透传：h2 会以非法头拒收整个请求。
    #[test]
    fn connection_header_is_not_forwarded_to_http2_targets() {
        assert!(is_builtin_passthrough("connection", true));
        assert!(!is_builtin_passthrough("connection", false));
        assert!(is_builtin_passthrough("user-agent", false));
    }

    /// 渠道透传名单命中的客户端 Header 原样上行；名单外的客户端 Header 仍被丢弃，
    /// 鉴权类头名即使写进名单也不能覆盖渠道 key。
    #[test]
    fn channel_passthrough_forwards_only_listed_client_headers() {
        use reqwest::header::HeaderValue;

        let channel = Channel {
            channel_type: ChannelProvider::OpenaiCompletion,
            key: "sk-ch".to_string(),
            passthrough_header: vec!["x-opencode-session".to_string()],
            ..Default::default()
        };
        let mut client_headers = HeaderMap::new();
        client_headers.insert("x-opencode-session", HeaderValue::from_static("client-session-abc"));
        client_headers.insert("x-unlisted", HeaderValue::from_static("should-be-dropped"));
        client_headers.insert("authorization", HeaderValue::from_static("Bearer client-key"));
        client_headers.insert("user-agent", HeaderValue::from_static("omp/18.8.6"));
        client_headers.insert("accept-encoding", HeaderValue::from_static("gzip, deflate, br, zstd"));

        let headers = build_upstream_headers(&channel, "sk-ch", Some(&client_headers)).unwrap();
        assert_eq!(headers.get("x-opencode-session").unwrap(), "client-session-abc");
        assert!(headers.get("x-unlisted").is_none(), "名单外的客户端头必须丢弃");
        assert_eq!(
            headers.get("authorization").unwrap(),
            "Bearer sk-ch",
            "鉴权头必须仍是渠道 key"
        );

        // 名单项大小写不敏感
        let mut upper = channel.clone();
        upper.passthrough_header = vec!["X-Opencode-Session".to_string()];
        let headers = build_upstream_headers(&upper, "sk-ch", Some(&client_headers)).unwrap();
        assert_eq!(headers.get("x-opencode-session").unwrap(), "client-session-abc");

        // 鉴权类头名写进名单也不透传
        let mut reserved = channel.clone();
        reserved.passthrough_header = vec!["Authorization".to_string(), "x-api-key".to_string()];
        let headers = build_upstream_headers(&reserved, "sk-ch", Some(&client_headers)).unwrap();
        assert_eq!(headers.get("authorization").unwrap(), "Bearer sk-ch");

        // 空名单：客户端头一个都不上行
        let headers = build_upstream_headers(&Channel::default(), "sk-ch", Some(&client_headers)).unwrap();
        assert!(headers.get("x-opencode-session").is_none());
        assert_eq!(
            headers.get("user-agent").unwrap(),
            "omp/18.8.6",
            "传输层头属于内置白名单，无需渠道配置"
        );
        assert_eq!(headers.get("accept-encoding").unwrap(), "gzip, deflate, br, zstd");
    }
}
