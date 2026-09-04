pub mod price;
pub mod sync;

use std::time::Duration;

use tracing::{error, info};

use crate::op::{AppState, stats::flush_stats_to_db};

pub fn start_background_tasks(state: AppState) {
    // 1. Periodic Stats Flush (reads stats_save_interval in minutes, default 10m)
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
                error!("Periodic stats flush failed: {}", e);
            }
        }
    });

    // 2. Periodic Channel Auto-Sync & Price Update (startup sync after 5s, then interval)
    let state_sync = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Perform first sync once on startup
        let _ = sync::sync_all_channel_models(&state_sync).await;
        let _ = price::update_llm_prices(&state_sync).await;

        loop {
            let sync_hours = state_sync
                .settings
                .get("sync_llm_interval")
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

            let _ = sync::sync_all_channel_models(&state_sync).await;
            let _ = price::update_llm_prices(&state_sync).await;
        }
    });

    // 3. Periodic request-log purge (run once at startup, then every hour)
    let state_purge = state.clone();
    tokio::spawn(async move {
        let _ = crate::op::log::purge_expired_logs(&state_purge).await;

        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.tick().await; // Consume immediate first tick

        loop {
            interval.tick().await;
            if let Err(e) = crate::op::log::purge_expired_logs(&state_purge).await {
                error!("Request-log purge failed: {}", e);
            }
        }
    });

    info!("Background workers initialized");
}
