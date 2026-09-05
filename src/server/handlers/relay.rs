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
    // path 形如 /v1beta/models/{model}:generateContent[?alt=sse]
    let path = uri.path();
    let model = path
        .split("/models/")
        .nth(1)
        .and_then(|rest| rest.split([':', '/']).next())
        .unwrap_or("")
        .trim();
    if model.is_empty() {
        return crate::server::resp::error(
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid request: missing model in URL path",
        );
    }
    forward_relay(
        state,
        auth_ctx,
        ApiFormat::GeminiGenerate,
        Some(model.to_string()),
        uri,
        headers,
        body,
    )
    .await
}
