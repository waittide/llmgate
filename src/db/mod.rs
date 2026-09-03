use std::{fs, path::Path, str::FromStr, time::Duration};

use anyhow::Result;
use sqlx::{
    Pool, Sqlite,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use tracing::info;

pub type DbPool = Pool<Sqlite>;

pub async fn init_db(db_path: &Path) -> Result<DbPool> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let connection_string = format!("sqlite://{}", db_path.to_string_lossy());
    let options = SqliteConnectOptions::from_str(&connection_string)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(10))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(32)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?;

    migrate(&pool).await?;
    info!("Database initialized successfully at {:?}", db_path);
    Ok(pool)
}

pub async fn migrate(pool: &DbPool) -> Result<()> {
    // 1. users table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Seed default admin user (admin / admin) if users table is empty
    let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    if user_count.0 == 0 {
        let default_password = crate::utils::store_password("admin");
        sqlx::query("INSERT INTO users (username, password) VALUES (?, ?)")
            .bind("admin")
            .bind(default_password)
            .execute(pool)
            .await?;
        info!("Default admin user created (admin / admin)");
    }

    // 2. settings table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Seed default settings
    let default_settings = [
        ("stats_save_interval", "10"),
        ("cors_allow_origins", ""),
        ("model_info_update_interval", "24"),
        ("sync_llm_interval", "24"),
        ("log_retention_days", "7"),
    ];
    for (k, v) in default_settings {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)")
            .bind(k)
            .bind(v)
            .execute(pool)
            .await?;
    }

    // 3. api_keys table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS api_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            api_key TEXT NOT NULL UNIQUE,
            enabled INTEGER NOT NULL DEFAULT 1,
            expire_at INTEGER NOT NULL DEFAULT 0,
            max_cost REAL NOT NULL DEFAULT 0,
            supported_models TEXT NOT NULL DEFAULT ''
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 4. channels table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS channels (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            type TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            base_url TEXT NOT NULL,
            key TEXT NOT NULL,
            auto_sync INTEGER NOT NULL DEFAULT 0,
            custom_header TEXT NOT NULL DEFAULT '[]',
            param_override TEXT,
            input_token INTEGER NOT NULL DEFAULT 0,
            output_token INTEGER NOT NULL DEFAULT 0,
            input_cost REAL NOT NULL DEFAULT 0,
            output_cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            request_success INTEGER NOT NULL DEFAULT 0,
            request_failed INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 5. channel_models table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS channel_models (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            channel_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'auto',
            input_token INTEGER NOT NULL DEFAULT 0,
            output_token INTEGER NOT NULL DEFAULT 0,
            input_cost REAL NOT NULL DEFAULT 0,
            output_cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            request_success INTEGER NOT NULL DEFAULT 0,
            request_failed INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            UNIQUE (channel_id, name)
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 6. groups table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            mode TEXT NOT NULL DEFAULT 'manual',
            active_item_id INTEGER NOT NULL DEFAULT 0,
            relay_config TEXT NOT NULL DEFAULT '{}'
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 7. group_items table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS group_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id INTEGER NOT NULL,
            channel_model_id INTEGER NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE,
            FOREIGN KEY (channel_model_id) REFERENCES channel_models(id) ON DELETE CASCADE,
            UNIQUE (group_id, channel_model_id)
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 8. llm_infos table (pricing)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS llm_infos (
            name TEXT PRIMARY KEY,
            input REAL NOT NULL DEFAULT 0,
            output REAL NOT NULL DEFAULT 0,
            cache_read REAL NOT NULL DEFAULT 0,
            cache_write REAL NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 9. stats tables
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS stats_totals (
            id INTEGER PRIMARY KEY,
            input_token INTEGER NOT NULL DEFAULT 0,
            output_token INTEGER NOT NULL DEFAULT 0,
            input_cost REAL NOT NULL DEFAULT 0,
            output_cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            request_success INTEGER NOT NULL DEFAULT 0,
            request_failed INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS stats_dailies (
            date TEXT PRIMARY KEY,
            input_token INTEGER NOT NULL DEFAULT 0,
            output_token INTEGER NOT NULL DEFAULT 0,
            input_cost REAL NOT NULL DEFAULT 0,
            output_cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            request_success INTEGER NOT NULL DEFAULT 0,
            request_failed INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS stats_hourlies (
            hour INTEGER NOT NULL,
            date TEXT NOT NULL,
            input_token INTEGER NOT NULL DEFAULT 0,
            output_token INTEGER NOT NULL DEFAULT 0,
            input_cost REAL NOT NULL DEFAULT 0,
            output_cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            request_success INTEGER NOT NULL DEFAULT 0,
            request_failed INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (hour, date)
        );
        CREATE TABLE IF NOT EXISTS stats_api_keys (
            api_key_id INTEGER PRIMARY KEY,
            input_token INTEGER NOT NULL DEFAULT 0,
            output_token INTEGER NOT NULL DEFAULT 0,
            input_cost REAL NOT NULL DEFAULT 0,
            output_cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            request_success INTEGER NOT NULL DEFAULT 0,
            request_failed INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(pool)
    .await?;

    // 10. request_logs table (persistent request/response logging)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS request_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            target_channel TEXT NOT NULL DEFAULT '',
            target_model TEXT NOT NULL DEFAULT '',
            request_url TEXT NOT NULL DEFAULT '',
            request_headers TEXT NOT NULL DEFAULT '',
            request_body TEXT NOT NULL DEFAULT '',
            response_url TEXT NOT NULL DEFAULT '',
            response_headers TEXT NOT NULL DEFAULT '',
            response_body TEXT NOT NULL DEFAULT '',
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            cost REAL NOT NULL DEFAULT 0,
            wait_time INTEGER NOT NULL DEFAULT 0,
            round INTEGER NOT NULL DEFAULT 1,
            error TEXT,
            rounds_detail TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_request_logs_created_at ON request_logs (created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_logs_request_id ON request_logs (request_id);
        CREATE INDEX IF NOT EXISTS idx_request_logs_model_created ON request_logs (model, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_request_logs_status_created ON request_logs (status, created_at DESC);
        "#,
    )
    .execute(pool)
    .await?;

    // Ensure rounds_detail column exists for existing databases
    let _ = sqlx::query("ALTER TABLE request_logs ADD COLUMN rounds_detail TEXT NOT NULL DEFAULT '[]'")
        .execute(pool)
        .await;

    Ok(())
}
