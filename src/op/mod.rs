pub mod apikey;
pub mod backup;
pub mod channel;
pub mod group;
pub mod log;
pub mod model;
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
        APIKey, Channel, ChannelModel, Group, ModelPrice, StatsAPIKey, StatsDaily, StatsHourly, StatsMetrics,
        StatsTotal, SystemStats,
    },
    relay::{
        route::RouteManager,
        state::{InFlightRegistry, KeyPool},
    },
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
/// 缓存同步约定：op 层所有变异（INSERT/UPDATE/DELETE）完成后必须调用对应的
/// `refresh_*_cache` 从数据库全量重载——数据库是唯一事实源，内存缓存只是它的
/// 只读投影。禁止在变异函数里散落逐键缓存修补（model_prices 除外：单键
/// upsert 与 DB 语句一一对应，且操作原子）。
#[derive(Clone)]
pub struct AppState {
    pub db:             DbPool,
    // 单键缓存（按 id / 键直接读写）：DashMap 足够。
    pub channels:       Arc<DashMap<i64, Channel>>,
    pub channel_models: Arc<DashMap<i64, ChannelModel>>,
    // 双索引缓存（需同时按 id 与 name/key 反查，且两表必须一致）：
    // 用 RwLock 包整块做原子快照，而不是两个 DashMap 各自为政。
    pub groups:         Arc<RwLock<GroupCache>>,
    pub api_keys:       Arc<RwLock<ApiKeyCache>>,
    pub model_prices:   Arc<DashMap<String, ModelPrice>>,
    pub settings:       Arc<DashMap<String, String>>,
    pub stats:          Arc<RwLock<SystemStats>>,
    pub stats_api_keys: Arc<DashMap<i64, StatsAPIKey>>,
    pub route_manager:  Arc<RouteManager>,
    pub key_pool:       Arc<KeyPool>,
    pub in_flight:      Arc<InFlightRegistry>,
    /// 渠道 / 渠道模型指标的脏标记：落盘只写变动过的行（见 op::stats::StatsDirty）
    pub stats_dirty:    Arc<stats::StatsDirty>,
    /// JWT 签名密钥：首次启动生成高熵随机值并持久化到 settings 表（见
    /// op::user::ensure_jwt_secret），跨重启稳定，与账号密码解耦。
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
            model_prices:   Arc::new(DashMap::new()),
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
            stats_dirty:    Arc::new(stats::StatsDirty::default()),
            key_pool:       Arc::new(KeyPool::new()),
            jwt_secret:     Arc::new(RwLock::new("llmgate-jwt-secret-default".to_string())),
        };

        state.refresh_all_caches().await?;
        user::ensure_jwt_secret(&state).await?;
        stats::refresh_stats_cache(&state).await?;
        info!("所有内存缓存初始化完成");
        Ok(state)
    }

    pub async fn refresh_all_caches(&self) -> Result<()> {
        setting::refresh_setting_cache(self).await?;
        apikey::refresh_apikey_cache(self).await?;
        channel::refresh_channel_cache(self).await?;
        group::refresh_group_cache(self).await?;
        model::refresh_model_cache(self).await?;
        Ok(())
    }
}
