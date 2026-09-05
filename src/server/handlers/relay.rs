use std::sync::Arc;

use axum::{
    extract::{Extension, State},
    http::{HeaderMap, Uri},
    response::Response,
};
use bytes::Bytes;

use crate::{
    op::AppState,
    relay::{forward_relay, protocol::ApiFormat},
    server::middleware::AuthContext,
};

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_relay(state, auth_ctx, ApiFormat::OpenaiCompletion, None, uri, headers, body).await
}

pub async fn responses(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_relay(state, auth_ctx, ApiFormat::OpenaiResponse, None, uri, headers, body).await
}

pub async fn messages(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_relay(state, auth_ctx, ApiFormat::AnthropicMessage, None, uri, headers, body).await
}

/// Gemini 原生客户端入口：`/v1beta/models/{model}:generateContent` 与
/// `/v1beta/models/{model}:streamGenerateContent`（可带 ?alt=sse）。
/// 模型名与流式标志取自 URL path，请求体为 generateContent 载荷（无 model 字段）。
pub async fn generate_content(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(model) = parse_gemini_model(uri.path()) else {
        return crate::server::resp::error(axum::http::StatusCode::NOT_FOUND, "Not Found");
    };
    forward_relay(
        state,
        auth_ctx,
        ApiFormat::GeminiGenerate,
        Some(model),
        uri,
        headers,
        body,
    )
    .await
}

/// 解析 Gemini 入口 path 中 `/models/` 后的 `{model}:{action}` 段。
/// 仅接受 generateContent / streamGenerateContent 动作；模型为空或动作未知时返回 None。
fn parse_gemini_model(path: &str) -> Option<String> {
    let model_action = path.split("/models/").nth(1)?;
    let (model, action) = model_action.split_once(':')?;
    let model = model.trim();
    if model.is_empty() || model.contains('/') {
        return None;
    }
    matches!(action, "generateContent" | "streamGenerateContent").then(|| model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_model_accepts_both_actions() {
        assert_eq!(
            parse_gemini_model("/v1beta/models/gemini-2.5-pro:generateContent"),
            Some("gemini-2.5-pro".to_string())
        );
        assert_eq!(
            parse_gemini_model("/v1beta/models/gemini-2.5-flash:streamGenerateContent"),
            Some("gemini-2.5-flash".to_string())
        );
    }

    #[test]
    fn test_parse_gemini_model_rejects_unknown_action_and_bad_segment() {
        assert_eq!(parse_gemini_model("/v1beta/models/gemini-2.5-pro:countTokens"), None);
        assert_eq!(parse_gemini_model("/v1beta/models/:generateContent"), None);
        assert_eq!(parse_gemini_model("/v1beta/models/gemini-2.5-pro"), None);
        assert_eq!(parse_gemini_model("/v1/chat/completions"), None);
    }
}
