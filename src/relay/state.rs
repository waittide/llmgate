use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens:     i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens:      i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestOverview {
    pub id:             u64,
    pub status:         String,
    pub started_at:     String,
    pub duration:       i64, // ms
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

#[derive(Debug)]
pub struct RequestStateInternal {
    pub overview:        RequestOverview,
    pub request_body:    String,
    pub response_body:   String,
    pub started_instant: std::time::Instant,
    pub cancel_tx:       tokio::sync::watch::Sender<bool>,
}

pub struct RequestStateManager {
    counter:    AtomicU64,
    requests:   DashMap<u64, RequestStateInternal>,
    recent_ids: Arc<RwLock<VecDeque<u64>>>,
    tx:         broadcast::Sender<RequestOverview>,
}

impl RequestStateManager {
    pub fn new(initial_id: u64) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            counter: AtomicU64::new(initial_id.max(1)),
            requests: DashMap::new(),
            recent_ids: Arc::new(RwLock::new(VecDeque::new())),
            tx,
        }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RequestOverview> {
        self.tx.subscribe()
    }

    pub fn get_recent_overviews(&self) -> Vec<RequestOverview> {
        let ids: Vec<u64> = self.recent_ids.read().iter().copied().collect();
        ids.into_iter()
            .filter_map(|id| self.requests.get(&id).map(|r| r.overview.clone()))
            .collect()
    }

    pub fn new_request(&self, model: String, body: String) -> u64 {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let now = chrono::Utc::now();
        let overview = RequestOverview {
            id,
            status: "running".to_string(),
            started_at: now.to_rfc3339(),
            duration: 0,
            model,
            usage: Usage::default(),
            cost: 0.0,
            round: 1,
            target_channel: String::new(),
            target_model: String::new(),
            sending: false,
            error: None,
            rounds: Vec::new(),
        };

        let (cancel_tx, _) = tokio::sync::watch::channel(false);

        let internal = RequestStateInternal {
            overview: overview.clone(),
            request_body: body,
            response_body: String::new(),
            started_instant: std::time::Instant::now(),
            cancel_tx,
        };

        self.requests.insert(id, internal);
        {
            let mut recent = self.recent_ids.write();
            recent.push_front(id);
            if recent.len() > 100 {
                let dropped_id = recent.pop_back().unwrap();
                // Never evict an in-flight stream (running or committed): removing it
                // would orphan its in-flight state updates and lose final metrics/logs.
                let is_in_flight = self
                    .requests
                    .get(&dropped_id)
                    .map(|r| {
                        let s = &r.overview.status;
                        s == "running" || s == "committed"
                    })
                    .unwrap_or(false);
                if !is_in_flight {
                    self.requests.remove(&dropped_id);
                }
            }
        }

        let _ = self.tx.send(overview);
        id
    }

    pub fn update_round(&self, id: u64, round: i64, target_channel: String, target_model: String, sending: bool) {
        if let Some(mut lock) = self.requests.get_mut(&id) {
            lock.overview.round = round;
            lock.overview.target_channel = target_channel;
            lock.overview.target_model = target_model;
            lock.overview.sending = sending;
            lock.overview.error = None;
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    pub fn update_round_error(&self, id: u64, round: i64, target_channel: String, target_model: String, error: String) {
        if let Some(mut lock) = self.requests.get_mut(&id) {
            lock.overview.round = round;
            lock.overview.target_channel = target_channel;
            lock.overview.target_model = target_model;
            lock.overview.sending = false;
            lock.overview.error = Some(error);
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    pub fn mark_committed(&self, id: u64) {
        if let Some(mut lock) = self.requests.get_mut(&id) {
            lock.overview.status = "committed".to_string();
            lock.overview.sending = true;
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    pub fn record_round_attempt(&self, id: u64, round_log: crate::model::RoundLog) {
        if let Some(mut lock) = self.requests.get_mut(&id) {
            if let Some(pos) = lock
                .overview
                .rounds
                .iter()
                .position(|r| r.round == round_log.round)
            {
                lock.overview.rounds[pos] = round_log;
            } else {
                lock.overview.rounds.push(round_log);
            }
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        &self,
        id: u64,
        status: &str,
        usage: Usage,
        cost: f64,
        response_body: String,
        error: Option<String>,
        rounds: Vec<crate::model::RoundLog>,
    ) {
        let finished = self.requests.get_mut(&id).is_some_and(|mut lock| {
            lock.overview.duration = lock.started_instant.elapsed().as_millis() as i64;
            lock.overview.status = status.to_string();
            lock.overview.usage = usage;
            lock.overview.cost = cost;
            lock.overview.sending = false;
            lock.overview.error = error;
            lock.response_body = response_body;
            if !rounds.is_empty() {
                lock.overview.rounds = rounds;
            }
            let _ = self.tx.send(lock.overview.clone());
            true
        });

        // A long-running stream that was evicted from the recency window while
        // still in flight is kept in `requests`; once it finishes, its log is
        // already persisted by the caller, so drop it from memory now.
        if finished && !self.recent_ids.read().contains(&id) {
            self.requests.remove(&id);
        }
    }

    pub fn get_request_body(&self, id: u64) -> Option<String> {
        self.requests.get(&id).map(|r| r.request_body.clone())
    }

    pub fn interrupt(&self, id: u64) -> bool {
        if let Some(entry) = self.requests.get(&id) {
            let _ = entry.cancel_tx.send(true);
            true
        } else {
            false
        }
    }

    pub fn get_cancel_receiver(&self, id: u64) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.requests.get(&id).map(|r| r.cancel_tx.subscribe())
    }
    pub fn get_response_body(&self, id: u64) -> Option<String> {
        self.requests.get(&id).map(|r| r.response_body.clone())
    }

    pub fn clear(&self) {
        // Retain every still-running stream even if it already fell out of the
        // recency window (e.g. >100 newer requests arrived while it was in
        // flight). Only finished requests are dropped from memory here.
        let in_flight_ids: Vec<u64> = self
            .requests
            .iter()
            .filter(|entry| {
                let s = &entry.value().overview.status;
                s == "running" || s == "committed"
            })
            .map(|entry| *entry.key())
            .collect();

        self.requests.retain(|id, _| in_flight_ids.contains(id));
        *self.recent_ids.write() = in_flight_ids.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_request_state_manager_interrupt() {
        let manager = RequestStateManager::new(1);
        let req_id = manager.new_request("test-model".to_string(), "{}".to_string());
        let rx = manager
            .get_cancel_receiver(req_id)
            .expect("receiver exists");

        assert!(!*rx.borrow());
        assert!(manager.interrupt(req_id));
        assert!(*rx.borrow());

        // Non-existent id returns false
        assert!(!manager.interrupt(99999));
    }
}
