pub mod price;
pub mod sync;

use std::time::Duration;

use tracing::{error, info, warn};

use crate::op::{AppState, stats::flush_stats_to_db};

pub fn start_background_tasks(state: AppState) {
    // 1. 定时落盘统计（读取 stats_save_interval，单位分钟，默认 10 分钟）
    let state_stats = state.clone();
    tokio::spawn(async move {
        loop {
            let minutes = state_stats
                .settings
                .get("stats_save_interval")
                .and_then(|e| e.value().parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(10);
            tokio::time::sleep(Duration::from_secs(minutes.max(1) * 60)).await;

            if let Err(e) = flush_stats_to_db(&state_stats).await {
                error!("定时统计落盘失败: {}", e);
            }
        }
    });

    // 2. 渠道自动同步与价格更新（启动 5 秒后首次执行，之后按间隔执行）
    let state_sync = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;

        // 启动时先执行一次同步
        if let Err(e) = sync::sync_all_channel_models(&state_sync).await {
            warn!("启动同步渠道模型失败: {}", e);
        }
        if let Err(e) = price::update_model_prices(&state_sync).await {
            warn!("启动更新模型价格失败: {}", e);
        }

        loop {
            let sync_hours = state_sync
                .settings
                .get("sync_model_interval")
                .and_then(|e| e.value().parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(24);
            let price_hours = state_sync
                .settings
                .get("model_info_update_interval")
                .and_then(|e| e.value().parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(24);
            let wait_hours = sync_hours.min(price_hours).max(1);

            tokio::time::sleep(Duration::from_secs(wait_hours * 3600)).await;

            if let Err(e) = sync::sync_all_channel_models(&state_sync).await {
                warn!("定时同步渠道模型失败: {}", e);
            }
            if let Err(e) = price::update_model_prices(&state_sync).await {
                warn!("定时更新模型价格失败: {}", e);
            }
        }
    });

    // 3. 定期清理过期请求日志（启动时先执行一次，之后每小时一次）
    let state_purge = state.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::op::log::purge_expired_logs(&state_purge).await {
            warn!("启动清理过期日志失败: {}", e);
        }

        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.tick().await; // 消耗立即触发的首次 tick

        loop {
            interval.tick().await;
            if let Err(e) = crate::op::log::purge_expired_logs(&state_purge).await {
                error!("过期请求日志清理失败: {}", e);
            }
        }
    });

    info!("后台任务已初始化");
}
