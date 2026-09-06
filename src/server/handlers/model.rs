use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    model::{AnthropicModel, AnthropicModelList, ModelInfo, OpenAIModel, OpenAIModelList},
    op::{AppState, group, model},
    server::{
        middleware::AuthContext,
        resp::{error, msg_or_500, success, success_msg},
    },
};

pub async fn list_model(State(state): State<AppState>) -> Response {
    success(model::list_model_infos(&state))
}

pub async fn create_model(State(state): State<AppState>, Json(payload): Json<ModelInfo>) -> Response {
    msg_or_500(
        model::batch_create_model_infos(&state, vec![payload]).await,
        "模型已创建",
    )
}

pub async fn update_model(State(state): State<AppState>, Json(payload): Json<ModelInfo>) -> Response {
    msg_or_500(model::update_model_info(&state, &payload).await, "模型已更新")
}

#[derive(Deserialize)]
pub struct DeleteModelReq {
    pub name: String,
}

pub async fn delete_model(State(state): State<AppState>, Json(payload): Json<DeleteModelReq>) -> Response {
    match model::delete_model_info(&state, &payload.name).await {
        Ok(_) => success_msg("模型已删除"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn update_model_price(State(state): State<AppState>) -> Response {
    msg_or_500(
        crate::task::price::update_model_prices(&state).await,
        "已从 models.dev 更新价格",
    )
}

pub async fn rebuild_model_price(State(state): State<AppState>) -> Response {
    msg_or_500(crate::task::price::rebuild_all_prices(&state).await, "价格已重建")
}

pub async fn get_last_update_time(State(_state): State<AppState>) -> Response {
    let last = crate::task::price::get_last_price_update_time();
    success(last)
}

// ---------------- 客户端公开接口: GET /v1/models ----------------
pub async fn get_model_list(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
) -> Response {
    let mut models = group::list_model_names(&state);

    if !auth_ctx.supported_models.is_empty() {
        models.retain(|m| auth_ctx.supported_models.contains(m));
    }

    if auth_ctx.is_anthropic_header {
        let now_str = chrono::Utc::now().to_rfc3339();
        let anthropic_models: Vec<AnthropicModel> = models
            .into_iter()
            .map(|m| AnthropicModel {
                id:           m.clone(),
                created_at:   now_str.clone(),
                display_name: m,
                model_type:   "model".to_string(),
            })
            .collect();

        let first_id = anthropic_models.first().map(|m| m.id.clone());
        let last_id = anthropic_models.last().map(|m| m.id.clone());

        let res = AnthropicModelList {
            data: anthropic_models,
            first_id,
            has_more: false,
            last_id,
        };
        Json(res).into_response()
    } else {
        let now_sec = chrono::Utc::now().timestamp();
        let open_ai_models: Vec<OpenAIModel> = models
            .into_iter()
            .map(|m| OpenAIModel {
                id:       m,
                object:   "model".to_string(),
                created:  now_sec,
                owned_by: "llmgate".to_string(),
            })
            .collect();

        let res = OpenAIModelList {
            object:  "list".to_string(),
            data:    open_ai_models,
            success: true,
        };
        Json(res).into_response()
    }
}
