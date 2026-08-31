use crate::model::{Channel, ChannelProvider};
use anyhow::{anyhow, Result};
use reqwest::{header::HeaderMap, Client, Proxy, RequestBuilder};
use std::time::Duration;

pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

pub fn get_client_for_channel(channel: &Channel) -> Result<Client> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .tcp_nodelay(true);

    if let Some(proxy_str) = &channel.channel_proxy
        && !proxy_str.trim().is_empty()
    {
        builder = builder.proxy(Proxy::all(proxy_str)?);
    }

    Ok(builder.build()?)
}

pub fn build_upstream_request(
    client: &Client,
    channel: &Channel,
    endpoint: &str, // e.g. "/chat/completions", "/messages", "/models"
    body: Option<Vec<u8>>,
    timeout_secs: u64,
) -> Result<RequestBuilder> {
    let base_url = normalize_base_url(&channel.base_url);
    let target_url = format!("{}{}", base_url, endpoint);

    let mut headers = HeaderMap::new();
    match channel.channel_type {
        ChannelProvider::Anthropic => {
            headers.insert(
                "x-api-key",
                channel
                    .key
                    .parse()
                    .map_err(|_| anyhow!("Invalid api key header"))?,
            );
            headers.insert(
                "anthropic-version",
                "2023-06-01"
                    .parse()
                    .map_err(|_| anyhow!("Invalid anthropic-version header"))?,
            );
        }
        ChannelProvider::Gemini => {
            headers.insert(
                "x-goog-api-key",
                channel
                    .key
                    .parse()
                    .map_err(|_| anyhow!("Invalid gemini key header"))?,
            );
        }
        _ => {
            headers.insert(
                "Authorization",
                format!("Bearer {}", channel.key)
                    .parse()
                    .map_err(|_| anyhow!("Invalid auth header"))?,
            );
        }
    }

    headers.insert(
        "Content-Type",
        "application/json".parse().map_err(|_| anyhow!("Invalid content-type"))?,
    );

    // Apply custom headers
    for h in &channel.custom_header {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(h.header_key.as_bytes()),
            h.header_value.parse(),
        ) {
            headers.insert(name, val);
        }
    }

    let mut req_builder = if body.is_some() {
        client.post(&target_url)
    } else {
        client.get(&target_url)
    };

    req_builder = req_builder
        .headers(headers)
        .timeout(Duration::from_secs(timeout_secs));

    if let Some(b) = body {
        req_builder = req_builder.body(b);
    }

    Ok(req_builder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_base_url() {
        assert_eq!(normalize_base_url("https://api.openai.com"), "https://api.openai.com");
        assert_eq!(normalize_base_url("https://api.openai.com/"), "https://api.openai.com");
        assert_eq!(normalize_base_url("https://api.openai.com///"), "https://api.openai.com");
        assert_eq!(normalize_base_url("https://custom.gateway.com/v1"), "https://custom.gateway.com/v1");
        assert_eq!(normalize_base_url("https://custom.gateway.com/v1/"), "https://custom.gateway.com/v1");
        assert_eq!(normalize_base_url("https://custom.gateway.com/api"), "https://custom.gateway.com/api");
    }
}
