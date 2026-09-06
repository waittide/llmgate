pub mod price;
pub mod sync;

use std::{future::Future, time::Duration};

use tokio::{sync::watch, task::JoinHandle};
use tracing::{error, info};

use crate::op::{AppState, stats::flush_stats_to_db};

/// 等待 `period` 或停机信号；返回 true 表示应退出监督循环。
async fn wait_or_shutdown(shutdown: &mut watch::Receiver<bool>, period: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(period) => false,
        changed = shutdown.changed() => match changed {
            Ok(_) => *shutdown.borrow(),
            Err(_) => true, // 发送端已关闭，视作停机
        },
    }
}

/// 在独立任务中执行工作 future：panic 被捕获并记日志，不会击穿监督循环，
/// 下一周期自动重试。
async fn run_catch(name: &'static str, fut: impl Future<Output = anyhow::Result<()>> + Send + 'static) {
    match tokio::spawn(fut).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => error!("{name}执行失败: {e}"),
        Err(join) => error!("{name}任务异常退出（已捕获，下一周期自动重试）: {join}"),
    }
}

/// 启动全部后台周期任务。返回句柄供停机流程限时等待；
/// 所有任务通过 `shutdown` 信号退出，单次执行的 panic 不会终止任务本身。
pub fn start_background_tasks(state: AppState, shutdown: watch::Receiver<bool>) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();

    // 1. 定时落盘统计（读取 stats_save_interval，单位分钟，默认 10 分钟）
    handles.push(tokio::spawn({
        let state = state.clone();
        let mut shutdown = shutdown.clone();
        async move {
            loop {
                let minutes = state
                    .settings
                    .get("stats_save_interval")
                    .and_then(|e| e.value().parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .unwrap_or(10);
                if wait_or_shutdown(&mut shutdown, Duration::from_secs(minutes.max(1) * 60)).await {
                    break;
                }
                run_catch("定时统计落盘", {
                    let s = state.clone();
                    async move { flush_stats_to_db(&s).await }
                })
                .await;
            }
        }
    }));

    // 2. 渠道自动同步与价格更新（启动 5 秒后首次执行，之后按间隔执行）
    handles.push(tokio::spawn({
        let state = state.clone();
        let mut shutdown = shutdown.clone();
        async move {
            if wait_or_shutdown(&mut shutdown, Duration::from_secs(5)).await {
                return;
            }
            run_catch("启动同步渠道模型", {
                let s = state.clone();
                async move { sync::sync_all_channel_models(&s).await }
            })
            .await;
            run_catch("启动更新模型价格", {
                let s = state.clone();
                async move { price::update_model_prices(&s).await }
            })
            .await;

            loop {
                let sync_hours = state
                    .settings
                    .get("sync_model_interval")
                    .and_then(|e| e.value().parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .unwrap_or(24);
                let price_hours = state
                    .settings
                    .get("model_info_update_interval")
                    .and_then(|e| e.value().parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .unwrap_or(24);
                let wait_hours = sync_hours.min(price_hours).max(1);
                if wait_or_shutdown(&mut shutdown, Duration::from_secs(wait_hours * 3600)).await {
                    break;
                }
                run_catch("定时同步渠道模型", {
                    let s = state.clone();
                    async move { sync::sync_all_channel_models(&s).await }
                })
                .await;
                run_catch("定时更新模型价格", {
                    let s = state.clone();
                    async move { price::update_model_prices(&s).await }
                })
                .await;
            }
        }
    }));

    // 3. 定期清理过期请求日志（启动时先执行一次，之后每小时一次）
    handles.push(tokio::spawn({
        let state = state.clone();
        let mut shutdown = shutdown.clone();
        async move {
            run_catch("启动清理过期日志", {
                let s = state.clone();
                async move { crate::op::log::purge_expired_logs(&s).await.map(|_| ()) }
            })
            .await;

            loop {
                if wait_or_shutdown(&mut shutdown, Duration::from_secs(3600)).await {
                    break;
                }
                run_catch("过期请求日志清理", {
                    let s = state.clone();
                    async move { crate::op::log::purge_expired_logs(&s).await.map(|_| ()) }
                })
                .await;
            }
        }
    }));

    // 4. 定期补偿刷新全部缓存：修复"DB 写入成功但缓存刷新失败"造成的
    //    内存与数据库分歧窗口（H6）
    handles.push(tokio::spawn({
        let state = state.clone();
        let mut shutdown = shutdown.clone();
        async move {
            loop {
                if wait_or_shutdown(&mut shutdown, Duration::from_secs(300)).await {
                    break;
                }
                run_catch("定时补偿刷新缓存", {
                    let s = state.clone();
                    async move { s.refresh_all_caches().await }
                })
                .await;
            }
        }
    }));

    info!("后台任务已初始化");
    handles
}
