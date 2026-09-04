use std::convert::Infallible;

use axum::{
    Json,
    extract::{Path, State},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{StreamExt, stream};

use crate::{
    model::{Group, GroupActiveItemUpdateRequest, GroupUpdateRequest},
    op::{AppState, group},
    server::resp::{internal_error, msg_or_500, ok_or_500, success},
};
pub async fn list_groups(State(state): State<AppState>) -> Response {
    success(group::list_groups(&state))
}

pub async fn create_group(State(state): State<AppState>, Json(mut payload): Json<Group>) -> Response {
    match group::create_group(&state, &mut payload).await {
        Ok(_) => success(payload),
        Err(e) => internal_error(e),
    }
}

pub async fn update_group(State(state): State<AppState>, Json(payload): Json<GroupUpdateRequest>) -> Response {
    ok_or_500(group::update_group(&state, &payload).await)
}

pub async fn update_group_active_item(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<GroupActiveItemUpdateRequest>,
) -> Response {
    ok_or_500(group::set_group_active_item(&state, id, &payload).await)
}

pub async fn delete_group(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    msg_or_500(group::delete_group(&state, id).await, "分组已删除")
}

pub async fn stream_group_runtime(State(state): State<AppState>) -> Response {
    let rx = state.route_manager.subscribe();
    let initial_snapshots = state.route_manager.get_snapshot();

    let initial_stream = stream::iter(initial_snapshots.into_iter().map(|state| {
        let json_str = serde_json::to_string(&state).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event("runtime").data(json_str))
    }));

    let broadcast_stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(route_state) => {
                    let json_str = serde_json::to_string(&route_state).unwrap_or_default();
                    let event = Ok::<Event, Infallible>(Event::default().event("runtime").data(json_str));
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
