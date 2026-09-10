//! 协议转换的中间定义（IR）。
//!
//! 所有格式转换都经由 IR：`原始格式 --parse--> IR --emit--> 目标格式`。
//! 字段卫生由此按能力集中决策——parser 只接收各格式已知字段（未知即剥离），
//! emitter 按目标支持面决定映射或丢弃，替代散落各转换器的逐份清理逻辑。

use serde_json::{Value, json};

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

/// `StopReason` → Anthropic `stop_reason`。
///
/// 与 `chat_finish_reason` 同一约定：仅在正常结束时才因存在工具调用改报 `tool_use`。
/// 截断（`max_tokens`）与内容过滤必须保留原终止原因——否则客户端会把只生成了一半
/// 参数的 tool_use 当作完整调用执行。
pub fn anthropic_stop_reason(reason: &StopReason, has_tool_calls: bool) -> &'static str {
    if *reason == StopReason::Stop && has_tool_calls {
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

/// `StopReason` → Responses 的 `(status, incomplete_details)`。
///
/// 截断与内容过滤都属于未完成：终态事件名与非流式 `status` 由同一份映射决定，
/// 避免流式报 `response.completed` 却带着 `status: incomplete` 的自相矛盾。
pub fn responses_status(reason: &StopReason) -> (&'static str, Value) {
    match reason {
        StopReason::Length => ("incomplete", json!({ "reason": "max_output_tokens" })),
        StopReason::ContentFilter => ("incomplete", json!({ "reason": "content_filter" })),
        _ => ("completed", Value::Null),
    }
}

/// IR 用量 → 各协议的 usage 对象。
///
/// 单一来源：流式 sink 与非流式 emitter 必须写出同一形状，否则同一次上游结果
/// 在两条路径下对客户端的字段口径会不一致。
/// 缓存明细在各协议里的位置不同，且 Anthropic 的口径与 IR 相反（见 `anthropic_usage`）。
fn total_or_sum(usage: &Usage) -> i64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.prompt_tokens + usage.completion_tokens
    }
}

/// Chat / Responses 共用的 OpenAI 风格缓存明细字段名。
pub fn chat_usage(usage: &Usage) -> Value {
    let mut value = json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": total_or_sum(usage)
    });
    if usage.cache_read_tokens > 0 {
        value["prompt_tokens_details"] = json!({ "cached_tokens": usage.cache_read_tokens });
    }
    value
}

pub fn responses_usage(usage: &Usage) -> Value {
    let mut value = json!({
        "input_tokens": usage.prompt_tokens,
        "output_tokens": usage.completion_tokens,
        "total_tokens": total_or_sum(usage)
    });
    if usage.cache_read_tokens > 0 {
        value["input_tokens_details"] = json!({ "cached_tokens": usage.cache_read_tokens });
    }
    value
}

/// Anthropic 的 usage。
///
/// 口径差异：IR 的 `prompt_tokens` 是含缓存的**总**输入（OpenAI 语义），而 Anthropic 的
/// `input_tokens` **不含**缓存部分。原样透出会让客户端把缓存量重复计入，故此处拆分，
/// 并把两部分单列——客户端自行相加即得总输入，计费侧也按同一口径还原。
pub fn anthropic_usage(usage: &Usage) -> Value {
    let total = usage.prompt_tokens.max(0);
    let cache_read = usage.cache_read_tokens.max(0).min(total);
    let cache_write = usage.cache_write_tokens.max(0).min(total - cache_read);
    let mut value = json!({
        "input_tokens": total - cache_read - cache_write,
        "output_tokens": usage.completion_tokens
    });
    if cache_read > 0 {
        value["cache_read_input_tokens"] = json!(cache_read);
    }
    if cache_write > 0 {
        value["cache_creation_input_tokens"] = json!(cache_write);
    }
    value
}

pub fn gemini_usage(usage: &Usage) -> Value {
    let mut value = json!({
        "promptTokenCount": usage.prompt_tokens,
        "candidatesTokenCount": usage.completion_tokens,
        "totalTokenCount": total_or_sum(usage)
    });
    if usage.cache_read_tokens > 0 {
        value["cachedContentTokenCount"] = json!(usage.cache_read_tokens);
    }
    value
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
