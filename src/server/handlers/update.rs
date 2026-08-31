use crate::{conf::VERSION, server::resp::success};
use axum::response::Response;
use serde_json::json;

pub async fn check_update() -> Response {
    success(json!({
        "current_version": VERSION,
        "latest_version": VERSION,
        "has_update": false,
        "release_notes": "Up to date",
        "release_url": "https://github.com/bestruirui/octopus"
    }))
}
