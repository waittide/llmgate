use axum::{extract::State, response::Response};
use serde::Serialize;

use crate::{
    model::StatsDaily,
    op::{AppState, stats},
    server::resp::{ok_or_500, success},
};

#[derive(Serialize)]
pub struct StatsDailyResponse {
    pub max_request_count: i64,
    pub items:             Vec<StatsDaily>,
}

pub async fn get_stats_today(State(state): State<AppState>) -> Response {
    success(stats::get_stats_today(&state))
}

pub async fn get_stats_daily(State(state): State<AppState>) -> Response {
    ok_or_500(stats::get_stats_daily(&state).await.map(|items| {
        let mut max_request_count = 0;
        for item in &items {
            let count = item.metrics.request_success + item.metrics.request_failed;
            if count > max_request_count {
                max_request_count = count;
            }
        }
        StatsDailyResponse {
            max_request_count,
            items,
        }
    }))
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
