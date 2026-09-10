use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::Response,
};

use crate::{
    model::APIKey,
    op::{AppState, apikey::get_api_key_by_key, stats::get_stats_api_key},
    server::{auth::verify_jwt, resp::error},
};

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub api_key:             APIKey,
    pub is_anthropic_header: bool,
}

/// 按 RFC 6750 提取 Bearer token；scheme 大小写不敏感（部分客户端发送小写 `bearer`）。
fn strip_bearer_token(auth_str: &str) -> Option<&str> {
    let (scheme, rest) = auth_str.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let trimmed = rest.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// 从 Authorization 头提取 JWT（token 仅经请求头传输）。
fn extract_admin_token(headers: &HeaderMap) -> Option<&str> {
    let auth_header = headers.get(header::AUTHORIZATION)?;
    strip_bearer_token(auth_header.to_str().ok()?)
}

/// 从请求中提取 API Key，优先级：x-api-key > Authorization Bearer >
/// x-goog-api-key（Gemini 原生头）> key 查询参数（Gemini 原生 ?key=）。
/// 返回 (密钥, 是否 Anthropic x-api-key 头)。
///
/// 返回值借用自请求头/查询串，鉴权命中与否都不分配：无效密钥本就常来常往。
fn extract_api_key<'a>(headers: &'a HeaderMap, query: Option<&'a str>) -> Option<(&'a str, bool)> {
    if let Some(x_key) = headers.get("x-api-key")
        && let Ok(key) = x_key.to_str()
        && !key.trim().is_empty()
    {
        return Some((key.trim(), true));
    }

    if let Some(auth_header) = headers.get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(stripped) = strip_bearer_token(auth_str)
    {
        return Some((stripped, false));
    }

    if let Some(goog_key) = headers.get("x-goog-api-key")
        && let Ok(key) = goog_key.to_str()
        && !key.trim().is_empty()
    {
        return Some((key.trim(), false));
    }

    query.and_then(extract_query_key).map(|key| (key, false))
}

/// 从查询串中取 `key` 参数值。
fn extract_query_key(query: &str) -> Option<&str> {
    query
        .split('&')
        .find_map(|pair| match pair.split_once('=') {
            Some(("key", value)) if !value.trim().is_empty() => Some(value.trim()),
            _ => None,
        })
}

pub async fn admin_auth_middleware(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let Some(token) = extract_admin_token(req.headers()) else {
        return error(StatusCode::UNAUTHORIZED, "鉴权失败");
    };

    // 读锁内直接校验：secret 仅在此借用，不做每请求的 String 克隆
    let verified = {
        let secret = state.jwt_secret.read();
        verify_jwt(token, secret.as_str())
    };
    match verified {
        Ok(claims) => {
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        Err(_) => error(StatusCode::UNAUTHORIZED, "鉴权失败"),
    }
}

pub async fn api_key_auth_middleware(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let Some((api_key_str, is_anthropic_header)) = extract_api_key(req.headers(), req.uri().query()) else {
        return error(StatusCode::UNAUTHORIZED, "鉴权失败");
    };

    let api_key_obj = match get_api_key_by_key(&state, api_key_str) {
        Ok(k) => k,
        Err(_) => return error(StatusCode::UNAUTHORIZED, "鉴权失败"),
    };

    if !api_key_obj.enabled {
        return error(StatusCode::UNAUTHORIZED, "API 密钥已被禁用");
    }

    let now_sec = chrono::Utc::now().timestamp();
    if api_key_obj.expire_at > 0 && api_key_obj.expire_at < now_sec {
        return error(StatusCode::UNAUTHORIZED, "API 密钥已过期");
    }

    let stats = get_stats_api_key(&state, api_key_obj.id);
    let total_cost = stats.metrics.input_cost + stats.metrics.output_cost;
    if api_key_obj.max_cost > 0.0 && total_cost >= api_key_obj.max_cost {
        return error(StatusCode::UNAUTHORIZED, "API 密钥已达到费用上限");
    }

    let auth_context = AuthContext {
        api_key: api_key_obj,
        is_anthropic_header,
    };

    req.extensions_mut().insert(Arc::new(auth_context));
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn test_extract_admin_token_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer jwt-token"));
        assert_eq!(extract_admin_token(&headers), Some("jwt-token"));
    }

    #[test]
    fn test_extract_admin_token_rejects_blank_and_missing() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert_eq!(extract_admin_token(&headers), None);
        assert_eq!(extract_admin_token(&HeaderMap::new()), None);
    }

    #[test]
    fn test_extract_api_key_priority_order() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-anthropic"));
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer sk-bearer"));
        headers.insert("x-goog-api-key", HeaderValue::from_static("goog-key"));
        assert_eq!(
            extract_api_key(&headers, Some("key=query-key")),
            Some(("sk-anthropic", true))
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer sk-bearer"));
        headers.insert("x-goog-api-key", HeaderValue::from_static("goog-key"));
        assert_eq!(
            extract_api_key(&headers, Some("key=query-key")),
            Some(("sk-bearer", false))
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", HeaderValue::from_static(" goog-key "));
        assert_eq!(
            extract_api_key(&headers, Some("key=query-key")),
            Some(("goog-key", false))
        );
    }

    #[test]
    fn test_extract_api_key_gemini_query_param() {
        assert_eq!(
            extract_api_key(&HeaderMap::new(), Some("key=abc123")),
            Some(("abc123", false))
        );
        assert_eq!(
            extract_api_key(&HeaderMap::new(), Some("alt=sse&key=abc123")),
            Some(("abc123", false))
        );
        assert_eq!(extract_api_key(&HeaderMap::new(), None), None);
        assert_eq!(extract_api_key(&HeaderMap::new(), Some("alt=sse")), None);
        assert_eq!(extract_api_key(&HeaderMap::new(), Some("key=")), None);
    }

    #[test]
    fn test_extract_api_key_rejects_blank_values() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("   "));
        assert_eq!(extract_api_key(&headers, None), None);

        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", HeaderValue::from_static(" "));
        assert_eq!(extract_api_key(&headers, None), None);
    }
}
