mod conf;
mod db;
mod model;
mod op;
mod relay;
mod server;
mod task;
mod utils;

use anyhow::Result;
use conf::Config;
use op::AppState;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse_args();

    let env_filter = if config.debug {
        EnvFilter::new("debug,tower_http=debug,axum=debug")
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"))
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting {} (Rust Edition) v{}", conf::APP_NAME, conf::VERSION);

    // 1. Initialize SQLite Database & Migrations
    let db_path = config.db_path();
    let pool = db::init_db(&db_path).await?;

    // 2. Initialize Application State & In-memory Caches
    let state = AppState::init(pool).await?;

    // 3. Start Background Tasks
    task::start_background_tasks(state.clone());

    // 4. Build Axum Router
    let app = server::build_router(state);

    // 5. Start Server
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Server stopped gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
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
