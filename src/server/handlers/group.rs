use crate::{
    model::{Group, GroupActiveItemUpdateRequest, GroupUpdateRequest},
    op::{group, AppState},
    server::resp::{error, success, success_msg},
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::StreamExt;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

pub async fn list_groups(State(state): State<AppState>) -> Response {
    success(group::list_groups(&state))
}

pub async fn create_group(
    State(state): State<AppState>,
    Json(mut payload): Json<Group>,
) -> Response {
    match group::create_group(&state, &mut payload).await {
        Ok(_) => success(payload),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn update_group(
    State(state): State<AppState>,
    Json(payload): Json<GroupUpdateRequest>,
) -> Response {
    match group::update_group(&state, &payload).await {
        Ok(grp) => success(grp),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn update_group_active_item(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<GroupActiveItemUpdateRequest>,
) -> Response {
    match group::set_group_active_item(&state, id, &payload).await {
        Ok(grp) => success(grp),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn delete_group(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match group::delete_group(&state, id).await {
        Ok(_) => success_msg("group deleted"),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn stream_group_runtime(State(state): State<AppState>) -> Response {
    let rx = state.route_manager.subscribe();
    let initial_snapshots = state.route_manager.get_snapshot();

    let initial_stream = futures::stream::iter(initial_snapshots.into_iter().map(|state| {
        let json_str = serde_json::to_string(&state).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event("runtime").data(json_str))
    }));

    let broadcast_stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(route_state) => {
                let json_str = serde_json::to_string(&route_state).unwrap_or_default();
                Some(Ok::<Event, Infallible>(
                    Event::default().event("runtime").data(json_str),
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
