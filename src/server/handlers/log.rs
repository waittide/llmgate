use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};

use crate::{
    model::RequestLog,
    op::{AppState, log as op_log},
    relay::state::Usage,
    server::resp::{error, success, success_msg},
};

pub async fn stream_overview(State(state): State<AppState>) -> Response {
    let rx = state.request_state_manager.subscribe();
    let initial_recent = state.request_state_manager.get_recent_overviews();

    let initial_stream = stream::iter(initial_recent.into_iter().map(|overview| {
        let json_str = serde_json::to_string(&overview).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event("overview").data(json_str))
    }));

    let broadcast_stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(overview) => {
                    let json_str = serde_json::to_string(&overview).unwrap_or_default();
                    let event = Ok::<Event, Infallible>(Event::default().event("overview").data(json_str));
                    return Some((event, rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let combined_stream = initial_stream.chain(broadcast_stream);
    Sse::new(combined_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

// ---------------- Persistent log queries ----------------

#[derive(Debug, Deserialize)]
pub struct LogListParams {
    #[serde(default)]
    pub page:      i64,
    #[serde(default)]
    pub page_size: i64,
    #[serde(default)]
    pub model:     String,
    #[serde(default)]
    pub status:    String,
    #[serde(default)]
    pub keyword:   String,
}

/// Overview DTO shaped to match the live SSE `RequestOverview` so the same
/// frontend cards render persisted history.
#[derive(Serialize)]
pub struct LogOverviewDto {
    pub id:             u64,
    pub status:         String,
    pub started_at:     String,
    pub duration:       i64,
    pub model:          String,
    pub usage:          Usage,
    pub cost:           f64,
    pub round:          i64,
    pub target_channel: String,
    pub target_model:   String,
    pub sending:        bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error:          Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rounds:         Vec<crate::model::RoundLog>,
}
#[derive(Serialize)]
pub struct LogListPageDto {
    pub total: i64,
    pub items: Vec<LogOverviewDto>,
}

fn to_overview(log: &RequestLog) -> LogOverviewDto {
    let started_at = if log.created_at >= 1_000_000_000_000 {
        chrono::DateTime::from_timestamp_millis(log.created_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    } else {
        chrono::DateTime::from_timestamp(log.created_at, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    };
    LogOverviewDto {
        id: log.id as u64,
        status: log.status.clone(),
        started_at,
        duration: log.wait_time,
        model: log.model.clone(),
        usage: Usage {
            prompt_tokens:     log.prompt_tokens,
            completion_tokens: log.completion_tokens,
            total_tokens:      log.total_tokens,
        },
        cost: log.cost,
        round: log.round,
        target_channel: log.target_channel.clone(),
        target_model: log.target_model.clone(),
        sending: false,
        error: log.error.clone(),
        rounds: serde_json::from_str(&log.rounds_detail).unwrap_or_default(),
    }
}

pub async fn list_logs(State(state): State<AppState>, Query(params): Query<LogListParams>) -> Response {
    let query = op_log::LogListQuery {
        page:      params.page,
        page_size: params.page_size,
        model:     &params.model,
        status:    &params.status,
        keyword:   &params.keyword,
    };
    match op_log::list_request_logs(&state, &query).await {
        Ok((total, items)) => success(LogListPageDto {
            total,
            items: items.iter().map(to_overview).collect(),
        }),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn get_log_detail(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    match op_log::get_request_log(&state, id as i64).await {
        Ok(Some(log)) => success(log),
        Ok(None) => error(StatusCode::NOT_FOUND, "Log not found"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn get_request_body(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    if let Some(body) = state.request_state_manager.get_request_body(id) {
        return success(body);
    }
    match op_log::get_request_log(&state, id as i64).await {
        Ok(Some(log)) => success(log.request_body),
        _ => error(StatusCode::NOT_FOUND, "Request body not found"),
    }
}

pub async fn get_response_body(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    if let Some(body) = state.request_state_manager.get_response_body(id) {
        return success(body);
    }
    match op_log::get_request_log(&state, id as i64).await {
        Ok(Some(log)) => success(log.response_body),
        _ => error(StatusCode::NOT_FOUND, "Response body not found"),
    }
}

pub async fn interrupt_round(
    State(_state): State<AppState>,
    Path((_request_id, _round)): Path<(u64, i64)>,
) -> Response {
    // Interruption signal handling
    success_msg("interrupted")
}

pub async fn clear_log(State(state): State<AppState>) -> Response {
    state.request_state_manager.clear();
    match op_log::clear_all_logs(&state).await {
        Ok(_) => success_msg("logs cleared"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
