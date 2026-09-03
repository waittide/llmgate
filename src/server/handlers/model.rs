use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    model::{AnthropicModel, AnthropicModelList, LLMInfo, OpenAIModel, OpenAIModelList},
    op::{AppState, group, llm},
    server::{
        middleware::AuthContext,
        resp::{error, success, success_msg},
    },
};

pub async fn list_llm(State(state): State<AppState>) -> Response {
    success(llm::list_llm_infos(&state))
}

pub async fn create_llm(State(state): State<AppState>, Json(payload): Json<LLMInfo>) -> Response {
    match llm::batch_create_llm_infos(&state, vec![payload]).await {
        Ok(_) => success_msg("model created"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn update_llm(State(state): State<AppState>, Json(payload): Json<LLMInfo>) -> Response {
    match llm::update_llm_info(&state, &payload).await {
        Ok(_) => success_msg("model updated"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct DeleteLLMReq {
    pub name: String,
}

pub async fn delete_llm(State(state): State<AppState>, Json(payload): Json<DeleteLLMReq>) -> Response {
    match llm::delete_llm_info(&state, &payload.name).await {
        Ok(_) => success_msg("model deleted"),
        Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn update_llm_price(State(state): State<AppState>) -> Response {
    match crate::task::price::update_llm_prices(&state).await {
        Ok(_) => success_msg("prices updated from models.dev"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn rebuild_llm_price(State(state): State<AppState>) -> Response {
    match crate::task::price::rebuild_all_prices(&state).await {
        Ok(_) => success_msg("prices rebuilt"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn get_last_update_time(State(_state): State<AppState>) -> Response {
    let last = crate::task::price::get_last_price_update_time();
    success(last)
}

// ---------------- Public Client Route: GET /v1/models ----------------
pub async fn get_model_list(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<Arc<AuthContext>>,
) -> Response {
    let mut models = group::list_model_names(&state);

    if !auth_ctx.supported_models.is_empty() {
        models.retain(|m| auth_ctx.supported_models.contains(m));
    }

    if auth_ctx.is_anthropic_header {
        let anthropic_models: Vec<AnthropicModel> = models
            .into_iter()
            .map(|m| AnthropicModel {
                id:           m.clone(),
                created_at:   "2024-01-01T00:00:00Z".to_string(),
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
        let open_ai_models: Vec<OpenAIModel> = models
            .into_iter()
            .map(|m| OpenAIModel {
                id:       m,
                object:   "model".to_string(),
                created:  1763395200,
                owned_by: "octopus".to_string(),
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
