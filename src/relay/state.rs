use std::collections::{HashMap, HashSet};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::{
    model::RequestLog,
    op::{AppState, log},
};

/// 请求终态的唯一出口：广播终态概览、注销中断注册、异步落盘日志。
///
/// 各终结路径（成功、失败、中断、future 被 drop）只负责算出自己的终态记录，
/// 效应一律从这里发出——同一套操作散落多处时，顺序与落盘策略都会各自漂移。
pub(crate) fn finish_request(state: &AppState, entry: RequestLog) {
    state.in_flight.broadcast(RequestOverview::from(&entry));
    state.in_flight.unregister(entry.id as u64);
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = log::update_completed_log(&state, &entry).await {
            tracing::error!("写入已完成请求日志失败: {}", e);
        }
    });
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub prompt_tokens:      i64,
    pub completion_tokens:  i64,
    pub total_tokens:       i64,
    /// 命中上游 prompt 缓存的输入 token（prompt_tokens 的子集）
    pub cache_read_tokens:  i64,
    /// 写入上游 prompt 缓存的输入 token（prompt_tokens 的子集）
    pub cache_write_tokens: i64,
}

impl Usage {
    /// 分阶段用量合并：非零值优先。
    ///
    /// 上游常把一个请求的用量拆在多个事件里（Anthropic 的 message_start 给
    /// input_tokens、message_delta 给 output_tokens），字段级取非零值再自洽化。
    pub fn merge(&mut self, incoming: Usage) {
        if self.prompt_tokens == 0 {
            self.prompt_tokens = incoming.prompt_tokens;
        }
        if self.completion_tokens == 0 {
            self.completion_tokens = incoming.completion_tokens;
        }
        if self.total_tokens == 0 {
            self.total_tokens = incoming.total_tokens;
        }
        if self.cache_read_tokens == 0 {
            self.cache_read_tokens = incoming.cache_read_tokens;
        }
        if self.cache_write_tokens == 0 {
            self.cache_write_tokens = incoming.cache_write_tokens;
        }
        self.normalize_total();
    }

    /// 分阶段用量合并后的自洽化。
    ///
    /// total 小于分项之和，说明它是上一阶段事件的陈旧值（Anthropic 的 input 与 output
    /// 分属 message_start 与 message_delta，先到的 total 只含 input），以分项之和为准；
    /// 反之（如 Gemini 把思考 token 计入 totalTokenCount 却不单列）保留上游口径。
    pub fn normalize_total(&mut self) {
        let sum = self.prompt_tokens + self.completion_tokens;
        if self.total_tokens < sum {
            self.total_tokens = sum;
        }
    }
}

/// 把一次事件里读到的用量并入累加槽（首次出现即初始化）。
pub fn merge_usage(slot: &mut Option<Usage>, incoming: Usage) {
    match slot {
        Some(existing) => existing.merge(incoming),
        None => *slot = Some(incoming),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestOverview {
    pub id:              u64,
    pub status:          String,
    pub started_at:      String,
    pub duration:        i64, // 耗时（毫秒）
    pub model:           String,
    pub usage:           Usage,
    pub cost:            f64,
    pub round:           i64,
    pub target_channel:  String,
    pub target_model:    String,
    pub sending:         bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error:           Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rounds:          Vec<crate::model::RoundLog>,
    /// 本轮发往上游的 URL 与 Header：只在运行中的轮次广播，供面板实时对照
    /// 客户端原始请求头与真正上行的那一份；轮次结束后由 rounds 明细接管。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub request_url:     String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub request_headers: String,
}

/// 毫秒时间戳转 RFC3339 字符串（非法值返回空串）。
pub fn millis_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

impl From<&RequestLog> for RequestOverview {
    fn from(log: &RequestLog) -> Self {
        RequestOverview {
            id:              log.id as u64,
            status:          log.status.clone(),
            started_at:      millis_to_rfc3339(log.created_at),
            duration:        log.wait_time,
            model:           log.model.clone(),
            usage:           Usage {
                prompt_tokens:      log.prompt_tokens,
                completion_tokens:  log.completion_tokens,
                total_tokens:       log.total_tokens,
                cache_read_tokens:  log.cache_read_tokens,
                cache_write_tokens: log.cache_write_tokens,
            },
            cost:            log.cost,
            round:           log.round,
            target_channel:  log.target_channel.clone(),
            target_model:    log.target_model.clone(),
            sending:         log.status == crate::model::request_status::RUNNING
                || log.status == crate::model::request_status::COMMITTED,
            error:           log.error.clone(),
            rounds:          serde_json::from_str(&log.rounds_detail).unwrap_or_default(),
            request_url:     String::new(),
            request_headers: String::new(),
        }
    }
}

/// 活跃请求轻量注册表，仅用于运行中请求的中断信号发送与 SSE 状态广播。
/// 不在内存中保留请求体、响应体或历史日志。
pub struct InFlightRegistry {
    cancels: DashMap<u64, watch::Sender<bool>>,
    tx:      broadcast::Sender<RequestOverview>,
}

impl Default for InFlightRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InFlightRegistry {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            cancels: DashMap::new(),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RequestOverview> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, overview: RequestOverview) {
        let _ = self.tx.send(overview);
    }

    pub fn register(&self, id: u64) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.cancels.insert(id, tx);
        rx
    }

    pub fn unregister(&self, id: u64) {
        self.cancels.remove(&id);
    }

    pub fn get_cancel_receiver(&self, id: u64) -> Option<watch::Receiver<bool>> {
        self.cancels.get(&id).map(|entry| entry.subscribe())
    }

    pub fn interrupt(&self, id: u64) -> bool {
        if let Some(entry) = self.cancels.get(&id) {
            let _ = entry.send(true);
            true
        } else {
            false
        }
    }
}
/// 渠道多 key 的运行态：请求级轮询游标 + 各 key 的冷却截止（毫秒）。
/// 冷却仅存内存，重启即清空。
pub struct KeyPool {
    states: DashMap<i64, KeyState>,
}
#[derive(Default)]
struct KeyState {
    cooldowns: HashMap<usize, i64>,
}

impl Default for KeyPool {
    fn default() -> Self {
        Self { states: DashMap::new() }
    }
}

impl KeyPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// 粘性优先级选取：固定顺序使用第一个未冷却的 key，多轮对话稳定命中同一
    /// key 以最大化上游 prompt 缓存命中；全部冷却时返回最早解冻的下标（探活）。
    ///
    /// 「本轮是否试过」必须由 `tried` 决定，不能靠冷却窗口推断：冷却秒数为 0 时
    /// `report_failure` 写入的冷却立即到期，只看冷却会让每次选取都命中 0 号 key，
    /// 同一渠道的后续 key 永远轮不到。全部 key 都已试过时返回 None，调用方换渠道。
    pub fn pick(&self, channel_id: i64, key_count: usize, tried: &HashSet<usize>) -> Option<usize> {
        if key_count == 0 {
            return None;
        }
        let mut entry = self.states.entry(channel_id).or_default();
        let state = entry.value_mut();
        let now = chrono::Utc::now().timestamp_millis();
        for idx in 0..key_count {
            if !tried.contains(&idx) && state.cooldowns.get(&idx).copied().unwrap_or(0) <= now {
                return Some(idx);
            }
        }
        (0..key_count)
            .filter(|idx| !tried.contains(idx))
            .min_by_key(|idx| state.cooldowns.get(idx).copied().unwrap_or(0))
    }

    /// 标记 key 失败进入冷却。
    pub fn report_failure(&self, channel_id: i64, key_idx: usize, cooldown_seconds: i64) {
        let mut entry = self.states.entry(channel_id).or_default();
        let now = chrono::Utc::now().timestamp_millis();
        entry
            .value_mut()
            .cooldowns
            .insert(key_idx, now + cooldown_seconds * 1000);
    }

    /// key 请求成功后解除冷却。
    pub fn report_success(&self, channel_id: i64, key_idx: usize) {
        if let Some(mut entry) = self.states.get_mut(&channel_id) {
            entry.value_mut().cooldowns.remove(&key_idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_flight_registry_interrupt() {
        let registry = InFlightRegistry::new();
        let rx = registry.register(42);
        assert!(!*rx.borrow());
        assert!(registry.interrupt(42));
        assert!(*rx.borrow());

        registry.unregister(42);
        assert!(!registry.interrupt(42));
    }

    #[test]
    fn test_key_pool_sticky_and_cooldown() {
        let pool = KeyPool::new();
        let none = HashSet::new();

        // 粘性：固定优先级顺序，连续选取同一 key（利于上游 prompt 缓存命中）
        assert_eq!(pool.pick(1, 3, &none), Some(0));
        assert_eq!(pool.pick(1, 3, &none), Some(0));

        // 失败的 key 进入冷却，顺延到下一个
        pool.report_failure(1, 0, 60);
        assert_eq!(pool.pick(1, 3, &none), Some(1));

        // 全部冷却时返回最早解冻的下标探活
        pool.report_failure(1, 1, 60);
        pool.report_failure(1, 2, 60);
        assert_eq!(pool.pick(1, 3, &none), Some(0));

        // 成功后解除冷却，回到首序 key
        pool.report_success(1, 0);
        assert_eq!(pool.pick(1, 3, &none), Some(0));

        // 本轮已试过的 key 必须跳过，即使它已不在冷却（冷却 0 秒即立即到期）
        let tried: HashSet<usize> = [0].into_iter().collect();
        assert_eq!(pool.pick(1, 3, &tried), Some(1));

        // 全部试过 → None：调用方据此切换渠道，不得重复使用同一 key
        let all: HashSet<usize> = [0, 1, 2].into_iter().collect();
        assert_eq!(pool.pick(1, 3, &all), None);
    }
}
