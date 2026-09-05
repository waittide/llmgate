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
    pub supported_models:    Vec<String>,
    pub is_anthropic_header: bool,
}

/// 从 Authorization 头提取 JWT（token 仅经请求头传输）。
fn extract_admin_token(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers.get(header::AUTHORIZATION)?;
    let auth_str = auth_header.to_str().ok()?;
    let stripped = auth_str.strip_prefix("Bearer ")?;
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 从请求中提取 API Key，优先级：x-api-key > Authorization Bearer >
/// x-goog-api-key（Gemini 原生头）> key 查询参数（Gemini 原生 ?key=）。
/// 返回 (密钥, 是否 Anthropic x-api-key 头)。
fn extract_api_key(headers: &HeaderMap, query: Option<&str>) -> Option<(String, bool)> {
    if let Some(x_key) = headers.get("x-api-key")
        && let Ok(key) = x_key.to_str()
        && !key.trim().is_empty()
    {
        return Some((key.trim().to_string(), true));
    }

    if let Some(auth_header) = headers.get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(stripped) = auth_str.strip_prefix("Bearer ")
        && !stripped.trim().is_empty()
    {
        return Some((stripped.trim().to_string(), false));
    }

    if let Some(goog_key) = headers.get("x-goog-api-key")
        && let Ok(key) = goog_key.to_str()
        && !key.trim().is_empty()
    {
        return Some((key.trim().to_string(), false));
    }

    query.and_then(extract_query_key).map(|key| (key, false))
}

/// 从查询串中取 `key` 参数值。
fn extract_query_key(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|pair| match pair.split_once('=') {
            Some(("key", value)) if !value.trim().is_empty() => Some(value.trim().to_string()),
            _ => None,
        })
}

pub async fn admin_auth_middleware(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let token = match extract_admin_token(req.headers()) {
        Some(t) => t,
        None => return error(StatusCode::UNAUTHORIZED, "鉴权失败"),
    };

    let secret = state.jwt_secret.read().clone();
    match verify_jwt(&token, &secret) {
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

    let api_key_obj = match get_api_key_by_key(&state, &api_key_str) {
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

    let supported_models: Vec<String> = if api_key_obj.supported_models.trim().is_empty() {
        Vec::new()
    } else {
        api_key_obj
            .supported_models
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let auth_context = AuthContext {
        api_key: api_key_obj,
        supported_models,
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
        assert_eq!(extract_admin_token(&headers), Some("jwt-token".to_string()));
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
            Some(("sk-anthropic".to_string(), true))
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer sk-bearer"));
        headers.insert("x-goog-api-key", HeaderValue::from_static("goog-key"));
        assert_eq!(
            extract_api_key(&headers, Some("key=query-key")),
            Some(("sk-bearer".to_string(), false))
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", HeaderValue::from_static(" goog-key "));
        assert_eq!(
            extract_api_key(&headers, Some("key=query-key")),
            Some(("goog-key".to_string(), false))
        );
    }

    #[test]
    fn test_extract_api_key_gemini_query_param() {
        assert_eq!(
            extract_api_key(&HeaderMap::new(), Some("key=abc123")),
            Some(("abc123".to_string(), false))
        );
        assert_eq!(
            extract_api_key(&HeaderMap::new(), Some("alt=sse&key=abc123")),
            Some(("abc123".to_string(), false))
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
