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
    op::{AppState, log as op_log},
    relay::state::RequestOverview,
    server::resp::{error, success, success_msg},
};

pub async fn stream_overview(State(state): State<AppState>) -> Response {
    let initial_logs = op_log::get_recent_logs(&state, 50)
        .await
        .unwrap_or_default();
    let initial_recent: Vec<RequestOverview> = initial_logs.iter().map(RequestOverview::from).collect();

    let rx = state.in_flight.subscribe();

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
    #[serde(default = "default_page")]
    pub page:      i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    #[serde(default)]
    pub model:     String,
    #[serde(default)]
    pub status:    String,
    #[serde(default)]
    pub keyword:   String,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

#[derive(Serialize)]
pub struct LogListPageDto {
    pub total: i64,
    pub items: Vec<RequestOverview>,
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
            items: items.iter().map(RequestOverview::from).collect(),
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
    match op_log::get_request_body(&state, id as i64).await {
        Ok(Some(body)) => success(body),
        _ => error(StatusCode::NOT_FOUND, "Request body not found"),
    }
}

pub async fn get_response_body(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    match op_log::get_response_body(&state, id as i64).await {
        Ok(Some(body)) => success(body),
        _ => error(StatusCode::NOT_FOUND, "Response body not found"),
    }
}

pub async fn interrupt_round(State(state): State<AppState>, Path((id, _round)): Path<(u64, i64)>) -> Response {
    if state.in_flight.interrupt(id) {
        success_msg("interrupted successfully")
    } else {
        error(StatusCode::NOT_FOUND, "request not found or already finished")
    }
}

pub async fn clear_log(State(state): State<AppState>) -> Response {
    match op_log::clear_all_logs(&state).await {
        Ok(_) => success_msg("logs cleared"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::db::init_db;

    async fn setup_test_state() -> (AppState, PathBuf) {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("octopus_log_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();
        (state, db_path)
    }

    #[tokio::test]
    async fn test_log_body_consistent_by_request_id() {
        let (state, db_path) = setup_test_state().await;

        // 1. Insert historical log with request_id = 999
        sqlx::query(
            r#"
            INSERT INTO request_logs (
                id, status, model, request_body, response_body, created_at
            ) VALUES (999, 'success', 'gpt-4o', 'body for request 999', 'response for request 999', 1700000000000)
            "#,
        )
        .execute(&state.db)
        .await
        .unwrap();

        // 2. Active runtime request created via create_initial_log
        let runtime_req_id = op_log::create_initial_log(
            &state,
            "gpt-4o",
            "/v1/chat/completions",
            "{}",
            "body for active request",
            1700000001000,
        )
        .await
        .unwrap() as u64;

        // 3. Querying request 999 returns its DB body
        let resp_hist = get_request_body(State(state.clone()), Path(999)).await;
        assert_eq!(resp_hist.status(), StatusCode::OK);
        let body_hist = axum::body::to_bytes(resp_hist.into_body(), 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body_hist).contains("body for request 999"));

        // 4. Querying runtime request returns its active DB body
        let resp_rt = get_request_body(State(state.clone()), Path(runtime_req_id)).await;
        assert_eq!(resp_rt.status(), StatusCode::OK);
        let body_rt = axum::body::to_bytes(resp_rt.into_body(), 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body_rt).contains("body for active request"));

        let _ = std::fs::remove_file(db_path);
    }
}
