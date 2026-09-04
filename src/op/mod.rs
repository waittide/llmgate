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
use tracing::info;

use crate::{
    db::DbPool,
    model::{
        APIKey, Channel, ChannelModel, Group, LLMPrice, StatsAPIKey, StatsDaily, StatsHourly, StatsMetrics, StatsTotal,
        SystemStats,
    },
    relay::{route::RouteManager, state::InFlightRegistry},
};

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
    pub db:             DbPool,
    pub channels:       Arc<DashMap<i64, Channel>>,
    pub channel_models: Arc<DashMap<i64, ChannelModel>>,
    pub groups:         Arc<RwLock<GroupCache>>,
    pub api_keys:       Arc<RwLock<ApiKeyCache>>,
    pub llm_prices:     Arc<DashMap<String, LLMPrice>>,
    pub settings:       Arc<DashMap<String, String>>,
    pub stats:          Arc<RwLock<SystemStats>>,
    pub stats_api_keys: Arc<DashMap<i64, StatsAPIKey>>,
    pub route_manager:  Arc<RouteManager>,
    pub in_flight:      Arc<InFlightRegistry>,
    pub jwt_secret:     Arc<RwLock<String>>,
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
            db:             db.clone(),
            channels:       Arc::new(DashMap::new()),
            channel_models: Arc::new(DashMap::new()),
            groups:         Arc::new(RwLock::new(GroupCache::default())),
            api_keys:       Arc::new(RwLock::new(ApiKeyCache::default())),
            llm_prices:     Arc::new(DashMap::new()),
            settings:       Arc::new(DashMap::new()),
            stats:          Arc::new(RwLock::new(SystemStats {
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
            route_manager:  Arc::new(RouteManager::new()),
            in_flight:      Arc::new(InFlightRegistry::new()),
            jwt_secret:     Arc::new(RwLock::new("octopus-jwt-secret-default".to_string())),
        };

        state.refresh_all_caches().await?;
        stats::refresh_stats_cache(&state).await?;
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
        Ok(())
    }
}
