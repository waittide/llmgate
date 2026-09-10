//! 按客户端协议构造错误响应体。
//!
//! 上游（或网关自身）只提供状态码与可读消息，错误信封由本模块按客户端格式生成：
//! 把上游协议的错误体原样透传给另一种协议的客户端时，SDK 解析不出错误结构，
//! 用户只能看到“未知错误”，因此统一改写为客户端自身的错误信封。

use axum::http::StatusCode;
use bytes::Bytes;
use serde_json::{Value, json};

use super::protocol::ApiFormat;

/// 无可读上游消息时的兜底文案。
const FALLBACK_MESSAGE: &str = "上游请求在所有尝试后仍失败";

/// 错误体中抽取出的可读信息。
#[derive(Debug, PartialEq)]
pub struct ErrorInfo {
    pub message: String,
    /// 上游原始错误码（OpenAI `code`、Anthropic `type`、Gemini `code` 等）
    pub code:    Option<Value>,
}

/// 面向客户端的错误响应：字节体 + 写日志用的可读消息。
#[derive(Debug, PartialEq)]
pub struct ClientError {
    pub body:    Bytes,
    pub message: String,
}

/// 消息候选路径：OpenAI/Anthropic/Gemini 用 `error.message`，部分上游用顶层字段。
const MESSAGE_POINTERS: [&str; 6] = [
    "/error/message",
    "/error/msg",
    "/error/detail",
    "/message",
    "/msg",
    "/detail",
];

/// 错误码候选路径：Gemini 的 `error.code` 是数字，OpenAI/Anthropic 用字符串。
const CODE_POINTERS: [&str; 5] = ["/error/code", "/error/status", "/code", "/status", "/error/type"];

/// 从错误响应体抽取可读消息与错误码；非 JSON、或没有任何可读消息时返回 None。
pub fn extract(body: &str) -> Option<ErrorInfo> {
    let value: Value = serde_json::from_str(body).ok()?;
    let message = MESSAGE_POINTERS
        .iter()
        .find_map(|pointer| value.pointer(pointer))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())?
        .to_string();
    let code = CODE_POINTERS
        .iter()
        .find_map(|pointer| value.pointer(pointer))
        .filter(|code| code.is_string() || code.is_number())
        .cloned();
    Some(ErrorInfo { message, code })
}

/// 构造面向客户端的错误响应。
pub fn build_client_error(format: ApiFormat, status: StatusCode, upstream_body: &str) -> ClientError {
    let info = extract(upstream_body);
    let message = info
        .as_ref()
        .map(|info| info.message.clone())
        .unwrap_or_else(|| {
            if upstream_body.trim().is_empty() {
                FALLBACK_MESSAGE.to_string()
            } else {
                upstream_body.to_string()
            }
        });
    let code = info.and_then(|info| info.code);
    let value = envelope(format, status, &message, code);
    ClientError {
        body: Bytes::from(serde_json::to_vec(&value).unwrap_or_default()),
        message,
    }
}

/// 按客户端协议构造错误信封。
///
/// HTTP 错误响应（`build_client_error`）与流内错误事件（上游 200 但事件体报错）共用，
/// 保证同一个错误在两种路径下对客户端呈现一致。
pub fn envelope(format: ApiFormat, status: StatusCode, message: &str, code: Option<Value>) -> Value {
    match format {
        ApiFormat::OpenaiCompletion | ApiFormat::OpenaiResponse => openai(status, message, code),
        ApiFormat::AnthropicMessage => anthropic(status, message),
        ApiFormat::GeminiGenerate => gemini(status, message),
    }
}

fn openai(status: StatusCode, message: &str, code: Option<Value>) -> Value {
    json!({
        "error": {
            "message": message,
            "type": if status.is_client_error() { "invalid_request_error" } else { "upstream_error" },
            "param": Value::Null,
            "code": code.unwrap_or_else(|| json!(status.as_u16())),
        }
    })
}

fn anthropic(status: StatusCode, message: &str) -> Value {
    json!({
        "type": "error",
        "error": { "type": anthropic_error_type(status), "message": message }
    })
}

fn anthropic_error_type(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        500 => "api_error",
        529 => "overloaded_error",
        code if code >= 500 => "api_error",
        _ => "invalid_request_error",
    }
}

fn gemini(status: StatusCode, message: &str) -> Value {
    json!({
        "error": {
            "code": status.as_u16(),
            "message": message,
            "status": gemini_status(status),
        }
    })
}

fn gemini_status(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        408 | 504 => "DEADLINE_EXCEEDED",
        409 => "ABORTED",
        429 => "RESOURCE_EXHAUSTED",
        499 => "CANCELLED",
        500 => "INTERNAL",
        503 => "UNAVAILABLE",
        code if code >= 500 => "INTERNAL",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENAI_BODY: &str =
        r#"{"error":{"message":"model not found","type":"invalid_request_error","code":"model_not_found"}}"#;
    const ANTHROPIC_BODY: &str = r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-x"}}"#;
    const GEMINI_BODY: &str = r#"{"error":{"code":429,"message":"quota exceeded","status":"RESOURCE_EXHAUSTED"}}"#;

    fn body_of(format: ApiFormat, status: StatusCode, upstream: &str) -> Value {
        let built = build_client_error(format, status, upstream);
        serde_json::from_slice(&built.body).unwrap()
    }

    #[test]
    fn extracts_message_and_code_from_every_envelope() {
        assert_eq!(
            extract(OPENAI_BODY),
            Some(ErrorInfo {
                message: "model not found".to_string(),
                code:    Some(json!("model_not_found")),
            })
        );
        assert_eq!(
            extract(ANTHROPIC_BODY),
            Some(ErrorInfo {
                message: "model: claude-x".to_string(),
                code:    Some(json!("not_found_error")),
            })
        );
        assert_eq!(
            extract(GEMINI_BODY),
            Some(ErrorInfo {
                message: "quota exceeded".to_string(),
                code:    Some(json!(429)),
            })
        );
        assert_eq!(extract("<html>502 Bad Gateway</html>"), None);
        assert_eq!(extract(r#"{"error":{}}"#), None);
    }

    #[test]
    fn openai_client_gets_openai_envelope() {
        let value = body_of(ApiFormat::OpenaiCompletion, StatusCode::BAD_REQUEST, ANTHROPIC_BODY);
        assert_eq!(value.pointer("/error/message").unwrap(), "model: claude-x");
        assert_eq!(value.pointer("/error/type").unwrap(), "invalid_request_error");
        assert_eq!(value.pointer("/error/code").unwrap(), "not_found_error");
        assert!(value.pointer("/error/param").unwrap().is_null());

        // 无上游错误码时回落到 HTTP 状态码，仍是 OpenAI 信封
        let value = body_of(ApiFormat::OpenaiResponse, StatusCode::BAD_GATEWAY, "boom");
        assert_eq!(value.pointer("/error/message").unwrap(), "boom");
        assert_eq!(value.pointer("/error/type").unwrap(), "upstream_error");
        assert_eq!(value.pointer("/error/code").unwrap(), 502);
    }

    #[test]
    fn anthropic_client_gets_anthropic_envelope() {
        let value = body_of(ApiFormat::AnthropicMessage, StatusCode::TOO_MANY_REQUESTS, OPENAI_BODY);
        assert_eq!(value.get("type").unwrap(), "error");
        assert_eq!(value.pointer("/error/type").unwrap(), "rate_limit_error");
        assert_eq!(value.pointer("/error/message").unwrap(), "model not found");
        // Anthropic 信封没有 param/code 字段，不额外夹带
        assert!(value.pointer("/error/code").is_none());
    }

    #[test]
    fn gemini_client_gets_gemini_envelope() {
        let value = body_of(ApiFormat::GeminiGenerate, StatusCode::NOT_FOUND, OPENAI_BODY);
        assert_eq!(value.pointer("/error/code").unwrap(), 404);
        assert_eq!(value.pointer("/error/status").unwrap(), "NOT_FOUND");
        assert_eq!(value.pointer("/error/message").unwrap(), "model not found");
    }

    #[test]
    fn empty_upstream_body_falls_back_to_exhausted_message() {
        let built = build_client_error(ApiFormat::AnthropicMessage, StatusCode::BAD_GATEWAY, "");
        assert_eq!(built.message, FALLBACK_MESSAGE);
        let value: Value = serde_json::from_slice(&built.body).unwrap();
        assert_eq!(value.pointer("/error/message").unwrap(), FALLBACK_MESSAGE);
    }
}
