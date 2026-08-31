use crate::{
    model::StatsDaily,
    op::{stats, AppState},
    server::resp::{error, success},
};
use axum::{extract::State, http::StatusCode, response::Response};
use serde::Serialize;

#[derive(Serialize)]
pub struct StatsDailyResponse {
    pub max_request_count: i64,
    pub items: Vec<StatsDaily>,
}

pub async fn get_stats_today(State(state): State<AppState>) -> Response {
    success(stats::get_stats_today(&state))
}

pub async fn get_stats_daily(State(state): State<AppState>) -> Response {
    match stats::get_stats_daily(&state).await {
        Ok(items) => {
            let mut max_request_count = 0;
            for item in &items {
                let count = item.metrics.request_success + item.metrics.request_failed;
                if count > max_request_count {
                    max_request_count = count;
                }
            }
            success(StatsDailyResponse {
                max_request_count,
                items,
            })
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn get_stats_hourly(State(state): State<AppState>) -> Response {
    success(stats::get_stats_hourly(&state).to_vec())
}

pub async fn get_stats_total(State(state): State<AppState>) -> Response {
    success(stats::get_stats_total(&state))
}

pub async fn get_stats_apikey(State(state): State<AppState>) -> Response {
    success(stats::list_stats_api_keys(&state))
}
