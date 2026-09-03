pub mod apikey;
pub mod backup;
pub mod channel;
pub mod group;
pub mod llm;
pub mod log;
pub mod setting;
pub mod stats;
pub mod user;

use std::sync::Arc;

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

#[derive(Clone)]
pub struct AppState {
    pub db:                    DbPool,
    pub channels:              DashMap<i64, Channel>,
    pub channel_models:        DashMap<i64, ChannelModel>,
    pub groups:                DashMap<i64, Group>,
    pub group_name_index:      DashMap<String, i64>,
    pub api_keys:              DashMap<i64, APIKey>,
    pub api_key_str_index:     DashMap<String, i64>,
    pub llm_prices:            DashMap<String, LLMPrice>,
    pub settings:              DashMap<String, String>,
    pub stats:                 Arc<RwLock<SystemStats>>,
    pub stats_api_keys:        DashMap<i64, StatsAPIKey>,
    pub route_manager:         Arc<RouteManager>,
    pub request_state_manager: Arc<RequestStateManager>,
    pub jwt_secret:            Arc<RwLock<String>>,
    pub log_sender:            mpsc::Sender<RequestLog>,
}

impl AppState {
    pub async fn init(db: DbPool) -> Result<Self> {
        let now_date = chrono::Local::now().format("%Y%m%d").to_string();
        let default_hourlies = std::array::from_fn(|i| StatsHourly {
            hour:    i as i64,
            date:    now_date.clone(),
            metrics: StatsMetrics::default(),
        });

        let state = Self {
            db:                    db.clone(),
            channels:              DashMap::new(),
            channel_models:        DashMap::new(),
            groups:                DashMap::new(),
            group_name_index:      DashMap::new(),
            api_keys:              DashMap::new(),
            api_key_str_index:     DashMap::new(),
            llm_prices:            DashMap::new(),
            settings:              DashMap::new(),
            stats:                 Arc::new(RwLock::new(SystemStats {
                total: StatsTotal {
                    id:      1,
                    metrics: StatsMetrics::default(),
                },
                daily: StatsDaily {
                    date:    now_date.clone(),
                    metrics: StatsMetrics::default(),
                },
                hourly: default_hourlies,
            })),
            stats_api_keys:        DashMap::new(),
            route_manager:         Arc::new(RouteManager::new()),
            request_state_manager: Arc::new(RequestStateManager::new()),
            jwt_secret:            Arc::new(RwLock::new("octopus-jwt-secret-default".to_string())),
            log_sender:            {
                let (tx, rx) = mpsc::channel(2048);
                crate::op::log::start_log_worker(db.clone(), rx);
                tx
            },
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
}
