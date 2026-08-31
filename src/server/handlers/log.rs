use crate::{
    op::AppState,
    server::resp::{error, success, success_msg},
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures::StreamExt;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream_overview(State(state): State<AppState>) -> Response {
    let rx = state.request_state_manager.subscribe();
    let initial_recent = state.request_state_manager.get_recent_overviews();

    let initial_stream = futures::stream::iter(initial_recent.into_iter().map(|overview| {
        let json_str = serde_json::to_string(&overview).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event("overview").data(json_str))
    }));

    let broadcast_stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(overview) => {
                let json_str = serde_json::to_string(&overview).unwrap_or_default();
                Some(Ok::<Event, Infallible>(
                    Event::default().event("overview").data(json_str),
                ))
            }
            Err(_) => None,
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

pub async fn get_request_body(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    match state.request_state_manager.get_request_body(id) {
        Some(body) => success(body),
        None => error(StatusCode::NOT_FOUND, "Request body not found"),
    }
}

pub async fn get_response_body(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    match state.request_state_manager.get_response_body(id) {
        Some(body) => success(body),
        None => error(StatusCode::NOT_FOUND, "Response body not found"),
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
    success_msg("logs cleared")
}
