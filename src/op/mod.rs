pub mod apikey;
pub mod backup;
pub mod channel;
pub mod group;
pub mod llm;
pub mod log;
pub mod setting;
pub mod stats;
pub mod user;

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::info;

use crate::{
    db::DbPool,
    model::{
        APIKey, Channel, ChannelModel, Group, LLMPrice, RequestLog, StatsAPIKey, StatsDaily, StatsHourly, StatsMetrics,
        StatsTotal, SystemStats,
    },
    relay::{route::RouteManager, state::RequestStateManager},
};

pub struct LogShutdown {
    pub shutdown_tx: tokio::sync::oneshot::Sender<()>,
    pub join_handle: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
pub struct GroupCache {
    pub by_id:   HashMap<i64, Group>,
    pub by_name: HashMap<String, i64>,
}

#[derive(Default)]
pub struct ApiKeyCache {
    pub by_id:  HashMap<i64, APIKey>,
    pub by_key: HashMap<String, i64>,
}

#[derive(Clone)]
pub struct AppState {
    // NOTE: All caches below are Arc-wrapped so `AppState::clone()` shares the
    // underlying storage across handlers, background tasks and router instances.
    // A bare `DashMap`/`RwLock` would be deep-cloned on every clone and cause
    // request paths to observe stale/inconsistent configuration.
    //
    // Shared across all AppState clones:
    pub db:                    DbPool,
    pub channels:              Arc<DashMap<i64, Channel>>,
    pub channel_models:        Arc<DashMap<i64, ChannelModel>>,
    pub groups:                Arc<RwLock<GroupCache>>,
    pub api_keys:              Arc<RwLock<ApiKeyCache>>,
    pub llm_prices:            Arc<DashMap<String, LLMPrice>>,
    pub settings:              Arc<DashMap<String, String>>,
    pub stats:                 Arc<RwLock<SystemStats>>,
    pub stats_api_keys:        Arc<DashMap<i64, StatsAPIKey>>,
    pub route_manager:         Arc<RouteManager>,
    pub request_state_manager: Arc<RequestStateManager>,
    pub jwt_secret:            Arc<RwLock<String>>,
    // NOTE: `log_sender` is a cheap clone (shared channel), not a deep copy.
    pub log_sender:            mpsc::Sender<RequestLog>,
    pub log_shutdown:          Arc<parking_lot::Mutex<Option<LogShutdown>>>,
}

impl AppState {
    pub async fn init(db: DbPool) -> Result<Self> {
        let now_date = chrono::Local::now().format("%Y%m%d").to_string();
        let default_hourlies = std::array::from_fn(|i| StatsHourly {
            hour:    i as i64,
            date:    now_date.clone(),
            metrics: StatsMetrics::default(),
        });

        let (tx, rx) = mpsc::channel(2048);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let join_handle = crate::op::log::start_log_worker(db.clone(), rx, shutdown_rx);
        let log_shutdown = Arc::new(parking_lot::Mutex::new(Some(LogShutdown {
            shutdown_tx,
            join_handle,
        })));

        let max_req_id: (i64,) = sqlx::query_as("SELECT COALESCE(MAX(request_id), 0) FROM request_logs")
            .fetch_one(&db)
            .await
            .unwrap_or((0,));
        let initial_req_id = (max_req_id.0 as u64) + 1;

        let state = Self {
            db: db.clone(),
            channels: Arc::new(DashMap::new()),
            channel_models: Arc::new(DashMap::new()),
            groups: Arc::new(RwLock::new(GroupCache::default())),
            api_keys: Arc::new(RwLock::new(ApiKeyCache::default())),
            llm_prices: Arc::new(DashMap::new()),
            settings: Arc::new(DashMap::new()),
            stats: Arc::new(RwLock::new(SystemStats {
                total:  StatsTotal {
                    id:      1,
                    metrics: StatsMetrics::default(),
                },
                daily:  StatsDaily {
                    date:    now_date.clone(),
                    metrics: StatsMetrics::default(),
                },
                hourly: default_hourlies,
            })),
            stats_api_keys: Arc::new(DashMap::new()),
            route_manager: Arc::new(RouteManager::new()),
            request_state_manager: Arc::new(RequestStateManager::new(initial_req_id)),
            jwt_secret: Arc::new(RwLock::new("octopus-jwt-secret-default".to_string())),
            log_sender: tx,
            log_shutdown,
        };

        state.refresh_all_caches().await?;
        info!("All in-memory caches initialized");
        Ok(state)
    }

    pub async fn refresh_all_caches(&self) -> Result<()> {
        user::refresh_user_cache(self).await?;
        setting::refresh_setting_cache(self).await?;
        apikey::refresh_apikey_cache(self).await?;
        channel::refresh_channel_cache(self).await?;
        group::refresh_group_cache(self).await?;
        llm::refresh_llm_cache(self).await?;
        stats::refresh_stats_cache(self).await?;
        Ok(())
    }

    pub async fn flush_and_stop_log_worker(&self) {
        let shutdown = self.log_shutdown.lock().take();
        if let Some(s) = shutdown {
            let _ = s.shutdown_tx.send(());
            let _ = s.join_handle.await;
        }
    }
}
