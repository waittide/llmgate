use anyhow::Result;
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
            message: "成功".to_string(),
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

/// 统一 handler 出口：Ok → success(data)，Err → 500 + 错误文本。
pub fn ok_or_500<T: Serialize>(result: Result<T>) -> Response {
    match result {
        Ok(data) => success(data),
        Err(e) => internal_error(e),
    }
}

/// 统一 handler 出口：Ok → success_msg(msg)（忽略载荷），Err → 500 + 错误文本。
pub fn msg_or_500<T>(result: Result<T>, msg: &str) -> Response {
    match result {
        Ok(_) => success_msg(msg),
        Err(e) => internal_error(e),
    }
}

/// 500 + 错误文本（供 Ok 分支需要额外处理的 handler 复用）。
pub fn internal_error(e: impl std::fmt::Display) -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}
