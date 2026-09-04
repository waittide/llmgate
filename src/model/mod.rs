use serde::{Deserialize, Serialize};

// ---------------- 用户 (User) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct User {
    pub id:       i64,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserLogin {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub expire:   i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserChangePassword {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserChangeUsername {
    pub new_username: String,
}

// ---------------- 统计指标 (Stats) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct StatsMetrics {
    pub input_token:     i64,
    pub output_token:    i64,
    pub input_cost:      f64,
    pub output_cost:     f64,
    pub wait_time:       i64,
    pub request_success: i64,
    pub request_failed:  i64,
}

impl StatsMetrics {
    pub fn add(&mut self, delta: &StatsMetrics) {
        self.input_token += delta.input_token;
        self.output_token += delta.output_token;
        self.input_cost += delta.input_cost;
        self.output_cost += delta.output_cost;
        self.wait_time += delta.wait_time;
        self.request_success += delta.request_success;
        self.request_failed += delta.request_failed;
    }

    pub fn from_sql_row(row: &sqlx::sqlite::SqliteRow) -> Self {
        use sqlx::Row;
        Self {
            input_token:     row.get("input_token"),
            output_token:    row.get("output_token"),
            input_cost:      row.get("input_cost"),
            output_cost:     row.get("output_cost"),
            wait_time:       row.get("wait_time"),
            request_success: row.get("request_success"),
            request_failed:  row.get("request_failed"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct StatsTotal {
    pub id:      i64,
    #[serde(flatten, default)]
    pub metrics: StatsMetrics,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct StatsHourly {
    pub hour:    i64,
    pub date:    String,
    #[serde(flatten, default)]
    pub metrics: StatsMetrics,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct StatsDaily {
    pub date:    String,
    #[serde(flatten, default)]
    pub metrics: StatsMetrics,
}

#[derive(Debug, Clone)]
pub struct SystemStats {
    pub total:  StatsTotal,
    pub daily:  StatsDaily,
    pub hourly: [StatsHourly; 24],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct StatsAPIKey {
    pub api_key_id: i64,
    #[serde(flatten, default)]
    pub metrics:    StatsMetrics,
}

// ---------------- 渠道与渠道模型 (Channel & ChannelModel) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProvider {
    #[default]
    OpenaiCompletion,
    OpenaiResponses,
    Anthropic,
    Gemini,
}

impl ChannelProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenaiCompletion => "openai",
            Self::OpenaiResponses => "openai_responses",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "openai_responses" => Self::OpenaiResponses,
            "anthropic" => Self::Anthropic,
            "gemini" => Self::Gemini,
            _ => Self::OpenaiCompletion,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelModelSource {
    #[default]
    Auto,
    Manual,
}

impl ChannelModelSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "manual" => Self::Manual,
            _ => Self::Auto,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomHeader {
    pub header_key:   String,
    pub header_value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelModel {
    pub id:         i64,
    pub channel_id: i64,
    pub name:       String,
    pub source:     ChannelModelSource,
    #[serde(flatten, default)]
    pub metrics:    StatsMetrics,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub id:             i64,
    pub name:           String,
    #[serde(rename = "type")]
    pub channel_type:   ChannelProvider,
    #[serde(default = "default_true")]
    pub enabled:        bool,
    pub base_url:       String,
    pub key:            String,
    #[serde(default)]
    pub models:         Vec<ChannelModel>,
    #[serde(default)]
    pub auto_sync:      bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_header:  Vec<CustomHeader>,
    pub param_override: Option<String>,
    #[serde(flatten, default)]
    pub metrics:        StatsMetrics,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelUpdateRequest {
    pub id:             i64,
    pub name:           Option<String>,
    #[serde(rename = "type")]
    pub channel_type:   Option<ChannelProvider>,
    pub enabled:        Option<bool>,
    pub base_url:       Option<String>,
    pub key:            Option<String>,
    pub models:         Option<Vec<ChannelModel>>,
    pub auto_sync:      Option<bool>,
    pub custom_header:  Option<Vec<CustomHeader>>,
    /// 缺省(None)保持现值；Some(空串/空白)清除该覆盖；Some(非空)覆盖原值。
    pub param_override: Option<String>,
}

// ---------------- 分组 (Group) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupMode {
    #[default]
    Manual,
    Failover,
}

impl GroupMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Failover => "failover",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "failover" => Self::Failover,
            _ => Self::Manual,
        }
    }
}
/// 分组内每个渠道项（成员）的重试与超时配置。
/// serde 名保留 `member_` 前缀，兼容数据库已存的 relay_config JSON 与前端字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupRelayConfig {
    #[serde(rename = "member_max_attempts")]
    pub max_attempts:                       i64,
    #[serde(rename = "member_retry_interval_seconds")]
    pub retry_interval_seconds:             i64,
    #[serde(rename = "member_non_stream_response_timeout_seconds")]
    pub non_stream_timeout_seconds:         i64,
    #[serde(rename = "member_stream_first_event_timeout_seconds")]
    pub stream_first_event_timeout_seconds: i64,
    #[serde(rename = "member_cooldown_seconds")]
    pub cooldown_seconds:                   i64,
}

impl Default for GroupRelayConfig {
    fn default() -> Self {
        Self {
            max_attempts:                       2,
            retry_interval_seconds:             1,
            non_stream_timeout_seconds:         60,
            stream_first_event_timeout_seconds: 30,
            cooldown_seconds:                   300,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupItem {
    pub id:               i64,
    pub group_id:         i64,
    pub channel_model_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_model:    Option<ChannelModel>,
    pub priority:         i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Group {
    pub id:             i64,
    pub name:           String,
    pub mode:           GroupMode,
    pub active_item_id: i64,
    pub relay_config:   GroupRelayConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items:          Vec<GroupItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupItemAddRequest {
    pub channel_model_id: i64,
    #[serde(default)]
    pub priority:         i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupItemUpdateRequest {
    pub id:       i64,
    pub priority: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupUpdateRequest {
    pub id:              i64,
    pub name:            Option<String>,
    pub mode:            Option<GroupMode>,
    pub relay_config:    Option<GroupRelayConfig>,
    #[serde(default)]
    pub items_to_add:    Vec<GroupItemAddRequest>,
    #[serde(default)]
    pub items_to_update: Vec<GroupItemUpdateRequest>,
    #[serde(default)]
    pub items_to_delete: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupActiveItemUpdateRequest {
    pub item_id: Option<i64>,
}

// ---------------- API 密钥 (APIKey) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct APIKey {
    pub id:               i64,
    pub name:             String,
    pub api_key:          String,
    #[serde(default = "default_true")]
    pub enabled:          bool,
    pub expire_at:        i64,
    pub max_cost:         f64,
    pub supported_models: String,
}

// ---------------- 系统设置 (Setting) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct Setting {
    pub key:   String,
    pub value: String,
}

// ---------------- 请求日志 (Request Log) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoundLog {
    pub round:            i64,
    pub channel_name:     String,
    pub model_name:       String,
    pub request_url:      String,
    pub request_headers:  String,
    pub request_body:     String,
    pub response_url:     String,
    pub response_headers: String,
    pub response_body:    String,
    pub status:           u16,
    pub duration_ms:      i64,
    pub error:            Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(default)]
pub struct RequestLog {
    pub id:                i64,
    pub status:            String,
    pub model:             String,
    pub target_channel:    String,
    pub target_model:      String,
    pub request_url:       String,
    pub request_headers:   String,
    pub request_body:      String,
    pub response_url:      String,
    pub response_headers:  String,
    pub response_body:     String,
    pub prompt_tokens:     i64,
    pub completion_tokens: i64,
    pub total_tokens:      i64,
    pub cost:              f64,
    pub wait_time:         i64,
    pub round:             i64,
    pub error:             Option<String>,
    pub rounds_detail:     String,
    pub created_at:        i64,
}

/// 请求状态取值：写入数据库行并通过 SSE 广播给前端，与前端约定耦合，集中定义禁止散落字面量。
pub mod request_status {
    pub const RUNNING: &str = "running";
    pub const COMMITTED: &str = "committed";
    pub const SUCCESS: &str = "success";
    pub const FAILED: &str = "failed";
    pub const INTERRUPTED: &str = "interrupted";
    pub const STREAM_ERROR: &str = "stream_error";
}

// ---------------- 模型价格与模型列表 (LLM Pricing & Models) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LLMPrice {
    pub input:       f64,
    pub output:      f64,
    pub cache_read:  f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LLMInfo {
    pub name:  String,
    #[serde(flatten, default)]
    pub price: LLMPrice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIModel {
    pub id:       String,
    pub object:   String,
    pub created:  i64,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIModelList {
    pub object:  String,
    pub data:    Vec<OpenAIModel>,
    #[serde(default)]
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModel {
    pub id:           String,
    pub created_at:   String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub model_type:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModelList {
    pub data:     Vec<AnthropicModel>,
    pub first_id: Option<String>,
    pub has_more: bool,
    pub last_id:  Option<String>,
}

// ---------------- 数据备份与导入 (Backup) ----------------
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupData {
    pub version:     Option<i64>,
    pub exported_at: Option<String>,

    pub settings:       Vec<Setting>,
    pub api_keys:       Vec<APIKey>,
    pub channels:       Vec<Channel>,
    pub channel_models: Vec<ChannelModel>,
    pub groups:         Vec<Group>,
    pub group_items:    Vec<GroupItem>,
    pub llm_infos:      Vec<LLMInfo>,

    pub stats_total:   Option<Vec<StatsTotal>>,
    pub stats_daily:   Vec<StatsDaily>,
    pub stats_hourly:  Vec<StatsHourly>,
    pub stats_api_key: Vec<StatsAPIKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DBImportResult {
    pub rows_affected: std::collections::HashMap<String, i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_metrics_flattened_in_json() {
        let ch = Channel {
            id: 1,
            name: "test-channel".to_string(),
            metrics: StatsMetrics {
                input_token:     100,
                output_token:    200,
                input_cost:      0.05,
                output_cost:     0.10,
                wait_time:       150,
                request_success: 10,
                request_failed:  2,
            },
            ..Default::default()
        };

        let json_val = serde_json::to_value(&ch).expect("序列化渠道失败");
        // 指标字段必须展平在顶层 (flatten)
        assert_eq!(json_val.get("input_token").unwrap(), 100);
        assert_eq!(json_val.get("output_token").unwrap(), 200);
        assert_eq!(json_val.get("input_cost").unwrap(), 0.05);
        assert_eq!(json_val.get("output_cost").unwrap(), 0.10);
        assert_eq!(json_val.get("request_success").unwrap(), 10);
        assert_eq!(json_val.get("request_failed").unwrap(), 2);
        assert_eq!(json_val.get("wait_time").unwrap(), 150);
        assert!(json_val.get("metrics").is_none());
    }
}
