use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
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
}

pub struct RequestStateManager {
    counter:    AtomicU64,
    requests:   DashMap<u64, Arc<RwLock<RequestStateInternal>>>,
    recent_ids: Arc<RwLock<Vec<u64>>>,
    tx:         broadcast::Sender<RequestOverview>,
}

impl RequestStateManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            counter: AtomicU64::new(1),
            requests: DashMap::new(),
            recent_ids: Arc::new(RwLock::new(Vec::new())),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RequestOverview> {
        self.tx.subscribe()
    }

    pub fn get_recent_overviews(&self) -> Vec<RequestOverview> {
        let ids = self.recent_ids.read().clone();
        ids.into_iter()
            .filter_map(|id| self.requests.get(&id).map(|r| r.read().overview.clone()))
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

        let internal = Arc::new(RwLock::new(RequestStateInternal {
            overview:        overview.clone(),
            request_body:    body,
            response_body:   String::new(),
            started_instant: std::time::Instant::now(),
        }));

        self.requests.insert(id, internal);
        {
            let mut recent = self.recent_ids.write();
            recent.insert(0, id);
            if recent.len() > 100 {
                let dropped_id = recent.pop().unwrap();
                self.requests.remove(&dropped_id);
            }
        }

        let _ = self.tx.send(overview);
        id
    }

    pub fn update_round(&self, id: u64, round: i64, target_channel: String, target_model: String, sending: bool) {
        if let Some(req) = self.requests.get(&id) {
            let mut lock = req.write();
            lock.overview.round = round;
            lock.overview.target_channel = target_channel;
            lock.overview.target_model = target_model;
            lock.overview.sending = sending;
            lock.overview.error = None;
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    pub fn update_round_error(&self, id: u64, round: i64, target_channel: String, target_model: String, error: String) {
        if let Some(req) = self.requests.get(&id) {
            let mut lock = req.write();
            lock.overview.round = round;
            lock.overview.target_channel = target_channel;
            lock.overview.target_model = target_model;
            lock.overview.sending = false;
            lock.overview.error = Some(error);
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    pub fn mark_committed(&self, id: u64) {
        if let Some(req) = self.requests.get(&id) {
            let mut lock = req.write();
            lock.overview.status = "committed".to_string();
            lock.overview.sending = true;
            let _ = self.tx.send(lock.overview.clone());
        }
    }

    pub fn record_round_attempt(&self, id: u64, round_log: crate::model::RoundLog) {
        if let Some(req) = self.requests.get(&id) {
            let mut lock = req.write();
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
        if let Some(req) = self.requests.get(&id) {
            let mut lock = req.write();
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
        }
    }

    pub fn get_request_body(&self, id: u64) -> Option<String> {
        self.requests
            .get(&id)
            .map(|r| r.read().request_body.clone())
    }

    pub fn get_response_body(&self, id: u64) -> Option<String> {
        self.requests
            .get(&id)
            .map(|r| r.read().response_body.clone())
    }

    pub fn clear(&self) {
        let mut recent = self.recent_ids.write();
        let running_ids: Vec<u64> = recent
            .iter()
            .filter(|&&id| {
                if let Some(r) = self.requests.get(&id) {
                    r.read().overview.status == "running"
                } else {
                    false
                }
            })
            .copied()
            .collect();

        self.requests.retain(|id, _| running_ids.contains(id));
        *recent = running_ids;
    }
}
