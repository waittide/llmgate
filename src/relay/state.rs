use std::collections::HashMap;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::model::RequestLog;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens:      i64,
    #[serde(default)]
    pub completion_tokens:  i64,
    #[serde(default)]
    pub total_tokens:       i64,
    /// 命中上游 prompt 缓存的输入 token（prompt_tokens 的子集）
    #[serde(default)]
    pub cache_read_tokens:  i64,
    /// 写入上游 prompt 缓存的输入 token（prompt_tokens 的子集）
    #[serde(default)]
    pub cache_write_tokens: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestOverview {
    pub id:             u64,
    pub status:         String,
    pub started_at:     String,
    pub duration:       i64, // 耗时（毫秒）
    pub model:          String,
    pub usage:          Usage,
    pub cost:           f64,
    pub round:          i64,
    pub target_channel: String,
    pub target_model:   String,
    pub sending:        bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error:          Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rounds:         Vec<crate::model::RoundLog>,
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
            id:             log.id as u64,
            status:         log.status.clone(),
            started_at:     millis_to_rfc3339(log.created_at),
            duration:       log.wait_time,
            model:          log.model.clone(),
            usage:          Usage {
                prompt_tokens:      log.prompt_tokens,
                completion_tokens:  log.completion_tokens,
                total_tokens:       log.total_tokens,
                cache_read_tokens:  log.cache_read_tokens,
                cache_write_tokens: log.cache_write_tokens,
            },
            cost:           log.cost,
            round:          log.round,
            target_channel: log.target_channel.clone(),
            target_model:   log.target_model.clone(),
            sending:        log.status == crate::model::request_status::RUNNING
                || log.status == crate::model::request_status::COMMITTED,
            error:          log.error.clone(),
            rounds:         serde_json::from_str(&log.rounds_detail).unwrap_or_default(),
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
    pub fn pick(&self, channel_id: i64, key_count: usize) -> usize {
        if key_count == 0 {
            return 0;
        }
        let mut entry = self.states.entry(channel_id).or_default();
        let state = entry.value_mut();
        let now = chrono::Utc::now().timestamp_millis();
        for idx in 0..key_count {
            if state.cooldowns.get(&idx).copied().unwrap_or(0) <= now {
                return idx;
            }
        }
        (0..key_count)
            .min_by_key(|idx| state.cooldowns.get(idx).copied().unwrap_or(0))
            .unwrap_or(0)
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

        // 粘性：固定优先级顺序，连续选取同一 key（利于上游 prompt 缓存命中）
        assert_eq!(pool.pick(1, 3), 0);
        assert_eq!(pool.pick(1, 3), 0);

        // 失败的 key 进入冷却，顺延到下一个
        pool.report_failure(1, 0, 60);
        assert_eq!(pool.pick(1, 3), 1);

        // 全部冷却时返回最早解冻的下标探活
        pool.report_failure(1, 1, 60);
        pool.report_failure(1, 2, 60);
        assert_eq!(pool.pick(1, 3), 0);

        // 成功后解除冷却，回到首序 key
        pool.report_success(1, 0);
        assert_eq!(pool.pick(1, 3), 0);
    }
}
