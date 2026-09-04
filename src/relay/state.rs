use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::model::RequestLog;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens:     i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens:      i64,
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
                prompt_tokens:     log.prompt_tokens,
                completion_tokens: log.completion_tokens,
                total_tokens:      log.total_tokens,
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
}
