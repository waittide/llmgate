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

use std::net::SocketAddr;

use anyhow::Result;
use conf::Config;
use op::AppState;
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse_args();

    let env_filter = if config.debug {
        EnvFilter::new("debug,tower_http=debug,axum=debug,octopus=debug")
    } else {
        EnvFilter::try_from_default_env()
            .map(|filter| filter.add_directive("octopus=info".parse().unwrap()))
            .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,octopus=info"))
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

    // 3. 启动后台定时任务
    task::start_background_tasks(state.clone());

    // 4. 构建 Axum 路由
    let app = server::build_router(state.clone());

    // 5. 启动 HTTP 服务
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    info!("服务已启动，监听 http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::select! {
        res = axum::serve(listener, app) => {
            if let Err(e) = res {
                tracing::error!("服务运行出错: {}", e);
            }
        }
        _ = shutdown_signal() => {
            info!("收到停机信号，正在停止服务...");
        }
    }

    info!("停机前正在将内存统计落盘到数据库...");
    if let Err(e) = op::stats::flush_stats_to_db(&state).await {
        tracing::error!("停机时落盘统计失败: {}", e);
    }
    info!("服务已优雅退出");
    Ok(())
}

async fn shutdown_signal() {
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
