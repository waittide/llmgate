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

    info!("=== 正在启动 {} (Rust 版) v{} ===", conf::APP_NAME, conf::VERSION);
    info!("数据目录: {:?}", config.data_dir);

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
    // 优雅停机：信号触发后停止接收新连接，等待在途请求排空；
    // drain_deadline 给排空加 30 秒上限，避免长连接（SSE）阻塞退出。
    tokio::select! {
        res = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()) => {
            if let Err(e) = res {
                tracing::error!("服务运行出错: {}", e);
            }
        }
        _ = drain_deadline() => {
            tracing::warn!("在途请求排空超时（30s），强制中断剩余连接");
        }
    }

    info!("停机前正在将内存统计落盘到数据库...");
    if let Err(e) = op::stats::flush_stats_to_db(&state).await {
        tracing::error!("停机时落盘统计失败: {}", e);
    }

    // 通知后台任务退出并限时等待，防止停机瞬间丢失统计结转
    let _ = shutdown_tx.send(true);
    for handle in task_handles {
        let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
    }
    info!("服务已优雅退出");
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

/// axum 优雅停机触发源。
async fn shutdown_signal() {
    wait_signal().await;
    info!("收到停机信号，正在停止服务...");
}

/// 排空上限：信号到达后最多再等 30 秒。
async fn drain_deadline() {
    wait_signal().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
}
