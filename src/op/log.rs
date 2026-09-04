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

/// Creates an initial `running` request log row immediately when a request arrives,
/// returning the database autoincrement row id as the request_id.
pub async fn create_initial_log(
    state: &AppState,
    model: &str,
    request_url: &str,
    request_headers: &str,
    request_body: &str,
    created_at: i64,
) -> Result<i64> {
    let res = sqlx::query(
        r#"
        INSERT INTO request_logs (status, model, request_url, request_headers, request_body, created_at)
        VALUES ('running', ?, ?, ?, ?, ?)
        "#,
    )
    .bind(model)
    .bind(request_url)
    .bind(request_headers)
    .bind(request_body)
    .bind(created_at)
    .execute(&state.db)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Updates the log entry upon request completion/failure/interruption with full details.
pub async fn update_completed_log(state: &AppState, log: &RequestLog) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE request_logs SET
            status = ?,
            target_channel = ?,
            target_model = ?,
            response_url = ?,
            response_headers = ?,
            response_body = ?,
            prompt_tokens = ?,
            completion_tokens = ?,
            total_tokens = ?,
            cost = ?,
            wait_time = ?,
            round = ?,
            error = ?,
            rounds_detail = ?
        WHERE id = ?
        "#,
    )
    .bind(&log.status)
    .bind(&log.target_channel)
    .bind(&log.target_model)
    .bind(&log.response_url)
    .bind(&log.response_headers)
    .bind(&log.response_body)
    .bind(log.prompt_tokens)
    .bind(log.completion_tokens)
    .bind(log.total_tokens)
    .bind(log.cost)
    .bind(log.wait_time)
    .bind(log.round)
    .bind(&log.error)
    .bind(&log.rounds_detail)
    .bind(log.id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Reads the request body directly from SQLite.
pub async fn get_request_body(state: &AppState, id: i64) -> Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>("SELECT request_body FROM request_logs WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row)
}

/// Reads the response body directly from SQLite.
pub async fn get_response_body(state: &AppState, id: i64) -> Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>("SELECT response_body FROM request_logs WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row)
}

/// Returns recent logs ordered newest first for initial live stream feed.
pub async fn get_recent_logs(state: &AppState, limit: i64) -> Result<Vec<RequestLog>> {
    let rows = sqlx::query_as::<_, RequestLog>("SELECT * FROM request_logs ORDER BY id DESC LIMIT ?")
        .bind(limit)
        .fetch_all(&state.db)
        .await?;
    Ok(rows)
}

/// Deletes request logs older than the configured retention period.
/// Returns the number of rows removed.
pub async fn purge_expired_logs(state: &AppState) -> Result<i64> {
    let days = get_log_retention_days(state);
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - days * 86400 * 1000;
    let res = sqlx::query("DELETE FROM request_logs WHERE created_at < ?")
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

/// Fetches a single request log by its id.
pub async fn get_request_log(state: &AppState, id: i64) -> Result<Option<RequestLog>> {
    let row = sqlx::query_as::<_, RequestLog>("SELECT * FROM request_logs WHERE id = ? LIMIT 1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row)
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
        "SELECT id, status, model, target_channel, target_model, '' as request_url, '' as request_headers, '' as request_body, '' as response_url, '' as response_headers, '' as response_body, prompt_tokens, completion_tokens, total_tokens, cost, wait_time, round, error, rounds_detail, created_at FROM request_logs WHERE 1=1",
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
