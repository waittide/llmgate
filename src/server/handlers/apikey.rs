use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, State},
    response::Response,
};
use serde_json::json;

use crate::{
    model::APIKey,
    op::{AppState, apikey, stats::get_stats_api_key},
    server::{
        middleware::AuthContext,
        resp::{internal_error, msg_or_500, ok_or_500, success, success_msg},
    },
};

pub async fn list_api_keys(State(state): State<AppState>) -> Response {
    success(apikey::list_api_keys(&state))
}

pub async fn create_api_key(State(state): State<AppState>, Json(mut payload): Json<APIKey>) -> Response {
    match apikey::create_api_key(&state, &mut payload).await {
        Ok(_) => success(payload),
        Err(e) => internal_error(e),
    }
}

pub async fn update_api_key(State(state): State<AppState>, Json(payload): Json<APIKey>) -> Response {
    ok_or_500(apikey::update_api_key(&state, &payload).await)
}

pub async fn delete_api_key(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    msg_or_500(apikey::delete_api_key(&state, id).await, "API 密钥已删除")
}

pub async fn get_api_key_stats_by_id(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
) -> Response {
    let stats = get_stats_api_key(&state, auth_ctx.api_key.id);
    // 白名单为空时 supports_model 恒真，等价于列出全部模型
    let models_str = crate::op::group::list_model_names(&state)
        .into_iter()
        .filter(|m| auth_ctx.api_key.supports_model(m))
        .collect::<Vec<_>>()
        .join(", ");

    let mut info = auth_ctx.api_key.clone();
    info.supported_models = models_str;

    success(json!({
        "stats": stats,
        "info": info
    }))
}

pub async fn login_api_key(Extension(_auth_ctx): Extension<Arc<AuthContext>>) -> Response {
    success_msg("登录成功")
}
