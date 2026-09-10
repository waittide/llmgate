//! 协议转换的中间定义（IR）。
//!
//! 所有格式转换都经由 IR：`原始格式 --parse--> IR --emit--> 目标格式`。
//! 字段卫生由此按能力集中决策——parser 只接收各格式已知字段（未知即剥离），
//! emitter 按目标支持面决定映射或丢弃，替代散落各转换器的逐份清理逻辑。

use serde_json::Value;

pub mod request;
pub mod response;
pub mod streaming;
use crate::{
    model::ChannelProvider,
    relay::{protocol::ApiFormat, state::Usage},
};

/// 渠道提供方 ↔ 协议格式的唯一映射。
pub fn channel_format(provider: &ChannelProvider) -> ApiFormat {
    match provider {
        ChannelProvider::OpenaiCompletion => ApiFormat::OpenaiCompletion,
        ChannelProvider::OpenaiResponses => ApiFormat::OpenaiResponse,
        ChannelProvider::Anthropic => ApiFormat::AnthropicMessage,
        ChannelProvider::Gemini => ApiFormat::GeminiGenerate,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub enum ImageSource {
    /// http(s) 或 `data:` URI；`data:` 的拆分由目标 emitter 负责
    Url(String),
    InlineBase64 {
        media_type: String,
        data:       String,
    },
}

#[derive(Debug, Clone)]
pub enum ContentPart {
    Text {
        text:          String,
        cache_control: Option<Value>,
    },
    Image(ImageSource),
    Thinking {
        text:      String,
        signature: Option<String>,
    },
    ToolCall {
        id:        String,
        name:      String,
        /// 统一存 JSON Value；Chat 目标发序列化字符串
        arguments: Value,
    },
    /// 工具结果；`name` 供 Gemini functionResponse 使用（解析时按此前 ToolCall 回填）
    ToolResult {
        call_id: String,
        name:    Option<String>,
        content: Value,
    },
    /// 无法建模的原始块：显式携带，由 emitter 按目标支持面决定保留或丢弃
    Unknown(Value),
}

#[derive(Debug, Clone, Default)]
pub struct Sampling {
    pub temperature:       Option<Value>,
    pub top_p:             Option<Value>,
    pub top_k:             Option<Value>,
    pub max_tokens:        Option<Value>,
    pub stop:              Vec<String>,
    pub seed:              Option<Value>,
    pub frequency_penalty: Option<Value>,
    pub presence_penalty:  Option<Value>,
    pub reasoning_effort:  Option<String>,
    pub verbosity:         Option<String>,
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub name:        String,
    pub description: Option<String>,
    /// JSON Schema；`Value::Null` 表示源格式未提供
    pub parameters:  Value,
}

#[derive(Debug, Clone)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Tool(String),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role:    Role,
    pub content: Vec<ContentPart>,
}

/// 请求的中间定义：四种格式公共字段的并集，各格式专有字段在解析时剥离。
#[derive(Debug, Clone, Default)]
pub struct RelayRequest {
    pub model:               String,
    /// 顶层系统提示（system / instructions / systemInstruction 归一）
    pub system:              Vec<ContentPart>,
    pub messages:            Vec<Message>,
    pub sampling:            Sampling,
    pub tools:               Vec<Tool>,
    pub tool_choice:         Option<ToolChoice>,
    pub parallel_tool_calls: Option<bool>,
    pub store:               Option<bool>,
    pub service_tier:        Option<Value>,
    pub metadata:            Option<Value>,
    pub safety_identifier:   Option<String>,
    pub prompt_cache_key:    Option<String>,
    pub user:                Option<String>,
    pub stream:              bool,
}

/// 终止原因；未知原因透传（Chat finish_reason 是开放集合）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StopReason {
    #[default]
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Other(String),
}

// ---------- 终止原因与调用 ID 的协议语义 ----------
//
// 解析方向（上游 → IR）与发射方向（IR → 客户端）共用同一份映射：同一语义在
// 两处各写一遍，流式与非流式就会各自漂移，出现“同一个上游响应、两种终止原因”。

/// Chat `finish_reason` → `StopReason`；未知原因透传给客户端。
pub fn stop_reason_from_chat(reason: Option<&str>) -> StopReason {
    match reason {
        Some("length") => StopReason::Length,
        Some("tool_calls") => StopReason::ToolUse,
        Some("content_filter") => StopReason::ContentFilter,
        Some("stop") | None => StopReason::Stop,
        Some(other) => StopReason::Other(other.to_string()),
    }
}

/// `StopReason` → Chat `finish_reason`；存在工具调用时优先 `tool_calls`。
pub fn chat_finish_reason(reason: &StopReason, has_tool_calls: bool) -> String {
    if *reason == StopReason::Stop && has_tool_calls {
        return "tool_calls".to_string();
    }
    match reason {
        StopReason::Stop => "stop".to_string(),
        StopReason::Length => "length".to_string(),
        StopReason::ToolUse => "tool_calls".to_string(),
        StopReason::ContentFilter => "content_filter".to_string(),
        StopReason::Other(other) => other.clone(),
    }
}

/// Anthropic `stop_reason` → `StopReason`；
/// `end_turn` / `stop_sequence` / `refusal` 等统一视为正常结束。
pub fn stop_reason_from_anthropic(reason: Option<&str>) -> StopReason {
    match reason {
        Some("max_tokens") => StopReason::Length,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::Stop,
    }
}

/// `StopReason` → Anthropic `stop_reason`；存在工具调用时（`has_tool_calls`）必须报 `tool_use`。
pub fn anthropic_stop_reason(reason: &StopReason, has_tool_calls: bool) -> &'static str {
    if has_tool_calls {
        return "tool_use";
    }
    match reason {
        StopReason::Length => "max_tokens",
        StopReason::ToolUse => "tool_use",
        _ => "end_turn",
    }
}

/// Gemini `finishReason` → `StopReason`；
/// `STOP` / `MALFORMED_FUNCTION_CALL` / 未知原因统一视为正常结束。
pub fn stop_reason_from_gemini(reason: Option<&str>) -> StopReason {
    match reason {
        Some("MAX_TOKENS") => StopReason::Length,
        // 安全过滤 / 违禁词 / 复述拦截等阻断场景
        Some("SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "RECITATION") => StopReason::ContentFilter,
        _ => StopReason::Stop,
    }
}

/// `StopReason` → Gemini `finishReason`（仅安全过滤可区分，其余归为 STOP）。
pub fn gemini_finish_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::Length => "MAX_TOKENS",
        StopReason::ContentFilter => "SAFETY",
        _ => "STOP",
    }
}

/// Gemini functionCall 的确定性调用 ID：同一调用跨 chunk 重复下发时幂等。
pub fn gemini_call_id(name: &str, arguments_json: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    arguments_json.hash(&mut hasher);
    format!("call_{:016x}", hasher.finish())
}

/// 非流式响应的中间定义。
#[derive(Debug, Clone, Default)]
pub struct RelayResponse {
    pub model:       String,
    /// 产出顺序：Thinking 在前（Chat reasoning_content 语义）
    pub content:     Vec<ContentPart>,
    pub stop_reason: StopReason,
    pub usage:       Usage,
}

/// 解析原始请求载荷为 IR；无法解析（非对象等）返回 None。
pub fn parse_request(format: ApiFormat, val: &Value) -> Option<RelayRequest> {
    match format {
        ApiFormat::OpenaiCompletion => request::parse_chat(val),
        ApiFormat::OpenaiResponse => request::parse_responses(val),
        ApiFormat::AnthropicMessage => request::parse_anthropic(val),
        ApiFormat::GeminiGenerate => request::parse_gemini(val),
    }
}

/// 将 IR 发射为目标格式载荷。
pub fn emit_request(req: &RelayRequest, format: ApiFormat) -> Value {
    match format {
        ApiFormat::OpenaiCompletion => request::emit_chat(req),
        ApiFormat::OpenaiResponse => request::emit_responses(req),
        ApiFormat::AnthropicMessage => request::emit_anthropic(req),
        ApiFormat::GeminiGenerate => request::emit_gemini(req),
    }
}

/// 解析上游非流式响应为 IR。
pub fn parse_response(format: ApiFormat, val: &Value, model_fallback: &str) -> Option<RelayResponse> {
    match format {
        ApiFormat::OpenaiCompletion => response::parse_chat(val, model_fallback),
        ApiFormat::OpenaiResponse => response::parse_responses(val, model_fallback),
        ApiFormat::AnthropicMessage => response::parse_anthropic(val, model_fallback),
        ApiFormat::GeminiGenerate => response::parse_gemini(val, model_fallback),
    }
}

/// 将 IR 响应发射为目标格式载荷。
pub fn emit_response(resp: &RelayResponse, format: ApiFormat) -> Value {
    match format {
        ApiFormat::OpenaiCompletion => response::emit_chat(resp),
        ApiFormat::OpenaiResponse => response::emit_responses(resp),
        ApiFormat::AnthropicMessage => response::emit_anthropic(resp),
        ApiFormat::GeminiGenerate => response::emit_gemini(resp),
    }
}
