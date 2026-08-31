use crate::{
    op::AppState,
    relay::{forward_relay, protocol::ApiFormat},
    server::middleware::AuthContext,
};
use axum::{
    extract::{Extension, State},
    response::Response,
};
use bytes::Bytes;
use std::sync::Arc;

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    body: Bytes,
) -> Response {
    forward_relay(state, auth_ctx, ApiFormat::OpenaiCompletion, body).await
}

pub async fn responses(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    body: Bytes,
) -> Response {
    forward_relay(state, auth_ctx, ApiFormat::OpenAIResponse, body).await
}

pub async fn messages(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
    body: Bytes,
) -> Response {
    forward_relay(state, auth_ctx, ApiFormat::AnthropicMessage, body).await
}
