use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::Response,
};
use serde_json::json;

use crate::{
    model::APIKey,
    op::{AppState, apikey, stats::get_stats_api_key},
    server::{
        middleware::AuthContext,
        resp::{error, success, success_msg},
    },
};

pub async fn list_api_keys(State(state): State<AppState>) -> Response {
    success(apikey::list_api_keys(&state))
}

pub async fn create_api_key(State(state): State<AppState>, Json(mut payload): Json<APIKey>) -> Response {
    match apikey::create_api_key(&state, &mut payload).await {
        Ok(_) => success(payload),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn update_api_key(State(state): State<AppState>, Json(payload): Json<APIKey>) -> Response {
    match apikey::update_api_key(&state, &payload).await {
        Ok(k) => success(k),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn delete_api_key(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    match apikey::delete_api_key(&state, id).await {
        Ok(_) => success_msg("api key deleted"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn get_api_key_stats_by_id(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
) -> Response {
    let stats = get_stats_api_key(&state, auth_ctx.api_key.id);
    let all_models = crate::op::group::list_model_names(&state);
    let models_str = if auth_ctx.api_key.supported_models.is_empty() {
        all_models.join(", ")
    } else {
        let supported = &auth_ctx.supported_models;
        all_models
            .into_iter()
            .filter(|m| supported.contains(m))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut info = auth_ctx.api_key.clone();
    info.supported_models = models_str;

    success(json!({
        "stats": stats,
        "info": info
    }))
}

pub async fn login_api_key(Extension(_auth_ctx): Extension<Arc<AuthContext>>) -> Response {
    success_msg("login successfully")
}
