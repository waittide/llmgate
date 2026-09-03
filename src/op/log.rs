use anyhow::Result;
use sqlx::QueryBuilder;

use crate::{model::RequestLog, op::AppState};

pub const LOG_RETENTION_DAYS_KEY: &str = "log_retention_days";
const DEFAULT_RETENTION_DAYS: i64 = 7;

/// Returns the configured log retention days (fallback: 7).
pub fn get_log_retention_days(state: &AppState) -> i64 {
    state
        .settings
        .get(LOG_RETENTION_DAYS_KEY)
        .and_then(|e| e.value().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

/// Worker that receives request logs from a bounded channel and batches writes to SQLite.
pub fn start_log_worker(
    db: crate::db::DbPool,
    mut rx: tokio::sync::mpsc::Receiver<RequestLog>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(64);
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));

        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => {
                    break;
                }
                Some(log) = rx.recv() => {
                    batch.push(log);
                    if batch.len() >= 64 {
                        flush_log_batch(&db, &mut batch).await;
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        flush_log_batch(&db, &mut batch).await;
                    }
                }
                else => break,
            }
        }

        // Drain any remaining logs queued in rx before shutdown completes
        while let Ok(log) = rx.try_recv() {
            batch.push(log);
            if batch.len() >= 64 {
                flush_log_batch(&db, &mut batch).await;
            }
        }

        if !batch.is_empty() {
            flush_log_batch(&db, &mut batch).await;
        }
    })
}

async fn flush_log_batch(db: &crate::db::DbPool, batch: &mut Vec<RequestLog>) {
    // Wrap the whole batch in one explicit transaction: a single commit/fsync
    // per flush instead of one per row, which removes the per-row synchronous
    // I/O stall from the log worker path.
    let mut tx = match db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("failed to begin log batch transaction: {:?}", e);
            batch.clear();
            return;
        }
    };

    for entry in batch.drain(..) {
        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO request_logs (
                request_id, status, model, target_channel, target_model,
                request_url, request_headers, request_body,
                response_url, response_headers, response_body,
                prompt_tokens, completion_tokens, total_tokens,
                cost, wait_time, round, error, rounds_detail, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry.request_id)
        .bind(&entry.status)
        .bind(&entry.model)
        .bind(&entry.target_channel)
        .bind(&entry.target_model)
        .bind(&entry.request_url)
        .bind(&entry.request_headers)
        .bind(&entry.request_body)
        .bind(&entry.response_url)
        .bind(&entry.response_headers)
        .bind(&entry.response_body)
        .bind(entry.prompt_tokens)
        .bind(entry.completion_tokens)
        .bind(entry.total_tokens)
        .bind(entry.cost)
        .bind(entry.wait_time)
        .bind(entry.round)
        .bind(&entry.error)
        .bind(&entry.rounds_detail)
        .bind(entry.created_at)
        .execute(&mut *tx)
        .await
        {
            // One malformed row aborts the transaction; roll back the rest.
            tracing::error!("failed to insert request log in batch: {:?}", e);
            return;
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("failed to commit log batch transaction: {:?}", e);
    }
}

/// Persists a completed request/response log via bounded channel queue.
pub fn record_request_log(state: AppState, entry: RequestLog) {
    if let Err(e) = state.log_sender.try_send(entry) {
        tracing::warn!("Request log channel full or closed, dropping entry: {}", e);
    }
}
/// Deletes request logs older than the configured retention period.
/// Returns the number of rows removed.
pub async fn purge_expired_logs(state: &AppState) -> Result<i64> {
    let days = get_log_retention_days(state);
    let cutoff_sec = chrono::Utc::now().timestamp() - days * 86400;
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - days * 86400 * 1000;
    let res =
        sqlx::query("DELETE FROM request_logs WHERE (created_at < ? AND created_at < 1000000000000) OR created_at < ?")
            .bind(cutoff_sec)
            .bind(cutoff_ms)
            .execute(&state.db)
            .await?;
    Ok(res.rows_affected() as i64)
}

/// Deletes all request logs from the database.
pub async fn clear_all_logs(state: &AppState) -> Result<i64> {
    let res = sqlx::query("DELETE FROM request_logs")
        .execute(&state.db)
        .await?;
    Ok(res.rows_affected() as i64)
}
/// Fetches a single request log by its database id (or fallback to latest request_id).
pub async fn get_request_log(state: &AppState, id: i64) -> Result<Option<RequestLog>> {
    let row = sqlx::query_as::<_, RequestLog>("SELECT * FROM request_logs WHERE id = ? LIMIT 1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;

    if row.is_some() {
        return Ok(row);
    }

    let row_fallback =
        sqlx::query_as::<_, RequestLog>("SELECT * FROM request_logs WHERE request_id = ? ORDER BY id DESC LIMIT 1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    Ok(row_fallback)
}
pub struct LogListQuery<'a> {
    pub page:      i64,
    pub page_size: i64,
    pub model:     &'a str,
    pub status:    &'a str,
    pub keyword:   &'a str,
}

/// Fetches a paginated, optionally-filtered list of request logs.
/// Returns (total_count, page_items) ordered newest-first.
pub async fn list_request_logs(state: &AppState, query: &LogListQuery<'_>) -> Result<(i64, Vec<RequestLog>)> {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let offset = (page - 1) * page_size;

    let mut where_qb = QueryBuilder::<sqlx::Sqlite>::new("SELECT COUNT(*) FROM request_logs WHERE 1=1");
    let mut list_qb = QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT id, request_id, status, model, target_channel, target_model, '' as request_url, '' as request_headers, '' as request_body, '' as response_url, '' as response_headers, '' as response_body, prompt_tokens, completion_tokens, total_tokens, cost, wait_time, round, error, rounds_detail, created_at FROM request_logs WHERE 1=1",
    );
    if !query.model.trim().is_empty() {
        let model = query.model.trim();
        where_qb.push(" AND model = ").push_bind(model);
        list_qb.push(" AND model = ").push_bind(model);
    }
    if !query.status.trim().is_empty() {
        let status = query.status.trim();
        where_qb.push(" AND status = ").push_bind(status);
        list_qb.push(" AND status = ").push_bind(status);
    }
    if !query.keyword.trim().is_empty() {
        let kw = format!("%{}%", query.keyword.trim());
        where_qb
            .push(" AND (request_body LIKE ")
            .push_bind(&kw)
            .push(" OR response_body LIKE ")
            .push_bind(&kw)
            .push(" OR model LIKE ")
            .push_bind(&kw)
            .push(")");
        list_qb
            .push(" AND (request_body LIKE ")
            .push_bind(&kw)
            .push(" OR response_body LIKE ")
            .push_bind(&kw)
            .push(" OR model LIKE ")
            .push_bind(&kw)
            .push(")");
    }

    let total: (i64,) = where_qb.build_query_as().fetch_one(&state.db).await?;

    list_qb
        .push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind(page_size)
        .push(" OFFSET ")
        .push_bind(offset);
    let items = list_qb
        .build_query_as::<RequestLog>()
        .fetch_all(&state.db)
        .await?;

    Ok((total.0, items))
}
