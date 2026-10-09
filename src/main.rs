mod conf;
mod db;
mod model;
mod op;
mod relay;
mod server;
mod task;

#[cfg(test)]
mod test_support;
mod utils;

use std::{net::SocketAddr, time::Duration};

use anyhow::Result;
use conf::Config;
use op::AppState;
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse_args();

    let env_filter = if config.debug {
        EnvFilter::new("debug,tower_http=debug,axum=debug,llmgate=debug")
    } else {
        EnvFilter::try_from_default_env()
            .map(|filter| filter.add_directive("llmgate=info".parse().unwrap()))
            .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,llmgate=info"))
    };

    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    info!("数据目录: {:?}", config.data_dir);
    config.ensure_data_dir()?;

    // 1. 初始化 SQLite 数据库与表结构
    let db_path = config.db_path();
    let pool = db::init_db(&db_path).await?;

    // 2. 初始化应用状态与内存缓存
    let state = AppState::init(pool).await?;

    // 3. 启动后台定时任务（携带停机信号；句柄用于停机时限时等待）
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let task_handles = task::start_background_tasks(state.clone(), shutdown_rx);

    // 4. 构建 Axum 路由
    let app = server::build_router(state.clone());

    // 5. 启动 HTTP 服务
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    info!("服务已启动，监听 http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // 信号触发即退出 select,serve future 被丢弃 → 所有连接(含 SSE 长连接)立即关闭,
    // 不做排空等待;在途请求的日志/统计由各收尾 Drop 兜底。
    tokio::select! {
        res = axum::serve(listener, app) => {
            if let Err(e) = res {
                tracing::error!("服务运行出错: {}", e);
            }
        }
        _ = shutdown_signal() => {
            info!("收到停机信号，正在关闭所有连接...");
        }
    }

    info!("停机前正在将内存统计落盘到数据库...");
    if let Err(e) = op::stats::flush_stats_to_db(&state).await {
        tracing::error!("停机时落盘统计失败: {}", e);
    }
    // 通知后台任务退出
    let _ = shutdown_tx.send(true);

    // 限时等待后台任务退出(总体 10s 上限,而非每任务 10s 串行)
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        for handle in task_handles {
            let _ = handle.await;
        }
    })
    .await;
    info!("服务已退出");
    Ok(())
}

/// 等待任一停机信号（Ctrl+C / SIGTERM）。
async fn wait_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("安装 Ctrl+C 信号处理器失败");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("安装 terminate 信号处理器失败")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// axum 停机触发源。
async fn shutdown_signal() {
    wait_signal().await;
}
