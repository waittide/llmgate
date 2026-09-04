use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;

use crate::{
    model::{Channel, ChannelUpdateRequest},
    op::{AppState, channel},
    server::resp::{error, success, success_msg},
    task::sync::fetch_upstream_models,
};

pub async fn list_channel(State(state): State<AppState>) -> Response {
    success(channel::list_channels(&state))
}

pub async fn create_channel(State(state): State<AppState>, Json(mut payload): Json<Channel>) -> Response {
    match channel::create_channel(&state, &mut payload).await {
        Ok(_) => {
            // 如有需要，自动为这些模型添加价格字典条目
            let model_names: Vec<String> = payload.models.iter().map(|m| m.name.clone()).collect();
            let _ = crate::task::price::ensure_model_prices(&state, &model_names).await;
            success(payload)
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn update_channel(State(state): State<AppState>, Json(payload): Json<ChannelUpdateRequest>) -> Response {
    match channel::update_channel(&state, &payload).await {
        Ok(ch) => {
            let model_names: Vec<String> = ch.models.iter().map(|m| m.name.clone()).collect();
            let _ = crate::task::price::ensure_model_prices(&state, &model_names).await;
            success(ch)
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct EnableChannelReq {
    pub id:      i64,
    pub enabled: bool,
}

pub async fn enable_channel(State(state): State<AppState>, Json(payload): Json<EnableChannelReq>) -> Response {
    match channel::set_channel_enabled(&state, payload.id, payload.enabled).await {
        Ok(_) => success_msg("渠道启用状态已更新"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn delete_channel(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    match channel::delete_channel(&state, id).await {
        Ok(_) => success_msg("渠道已删除"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn fetch_model(State(_state): State<AppState>, Json(payload): Json<Channel>) -> Response {
    match fetch_upstream_models(&payload).await {
        Ok(models) => success(models),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn sync_channel(State(state): State<AppState>) -> Response {
    match crate::task::sync::sync_all_channel_models(&state).await {
        Ok(_) => success_msg("渠道模型同步成功"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn get_last_sync_time(State(_state): State<AppState>) -> Response {
    let last_time = crate::task::sync::get_last_sync_time();
    success(last_time)
}
