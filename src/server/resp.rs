use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code:    u16,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data:    Option<T>,
}

pub fn success<T: Serialize>(data: T) -> Response {
    (
        StatusCode::OK,
        Json(ApiResponse {
            code:    200,
            message: "success".to_string(),
            data:    Some(data),
        }),
    )
        .into_response()
}

pub fn success_msg(msg: &str) -> Response {
    (
        StatusCode::OK,
        Json(ApiResponse::<()> {
            code:    200,
            message: msg.to_string(),
            data:    None,
        }),
    )
        .into_response()
}

pub fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ApiResponse::<()> {
            code:    status.as_u16(),
            message: message.to_string(),
            data:    None,
        }),
    )
        .into_response()
}
