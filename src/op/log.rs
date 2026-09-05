use anyhow::Result;
use sqlx::QueryBuilder;

use crate::{model::RequestLog, op::AppState};

pub const LOG_RETENTION_DAYS_KEY: &str = "log_retention_days";
const DEFAULT_RETENTION_DAYS: i64 = 7;

/// 获取配置的日志保留天数（默认 7 天）。
pub fn get_log_retention_days(state: &AppState) -> i64 {
    state
        .settings
        .get(LOG_RETENTION_DAYS_KEY)
        .and_then(|e| e.value().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

/// 在请求到达时立即向数据库插入一条 `running` 状态的初始请求日志，
/// 并将数据库自增的主键 ID 作为本次请求全局唯一的 request_id 返回。
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
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(crate::model::request_status::RUNNING)
    .bind(model)
    .bind(request_url)
    .bind(request_headers)
    .bind(request_body)
    .bind(created_at)
    .execute(&state.db)
    .await?;
    Ok(res.last_insert_rowid())
}

/// 请求完成、失败或中断后，将完整的结果、指标、响应数据更新回数据库日志条目。
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
            cache_read_tokens = ?,
            cache_write_tokens = ?,
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
    .bind(log.cache_read_tokens)
    .bind(log.cache_write_tokens)
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

/// 直接从 SQLite 读取请求体。
pub async fn get_request_body(state: &AppState, id: i64) -> Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>("SELECT request_body FROM request_logs WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row)
}

/// 直接从 SQLite 读取响应体。
pub async fn get_response_body(state: &AppState, id: i64) -> Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>("SELECT response_body FROM request_logs WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row)
}

/// 获取最近的请求日志条目（按 ID 倒序排列），供 SSE 实时概览流建立初始视图。
pub async fn get_recent_logs(state: &AppState, limit: i64) -> Result<Vec<RequestLog>> {
    let rows = sqlx::query_as::<_, RequestLog>("SELECT * FROM request_logs ORDER BY id DESC LIMIT ?")
        .bind(limit)
        .fetch_all(&state.db)
        .await?;
    Ok(rows)
}

/// 删除早于配置保留期限的过期数据，返回清理的请求日志行数。
/// 覆盖 request_logs（按 created_at 毫秒）与统计明细 stats_dailies / stats_hourlies
/// （date 为 %Y%m%d 字符串，字典序即时间序）；stats_totals 与 stats_api_keys 为累计聚合，不清理。
pub async fn purge_expired_logs(state: &AppState) -> Result<i64> {
    let days = get_log_retention_days(state);
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - days * 86400 * 1000;
    let cutoff_date = (chrono::Utc::now() - chrono::Duration::days(days))
        .format("%Y%m%d")
        .to_string();
    let res = sqlx::query("DELETE FROM request_logs WHERE created_at < ?")
        .bind(cutoff_ms)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM stats_dailies WHERE date < ?")
        .bind(&cutoff_date)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM stats_hourlies WHERE date < ?")
        .bind(&cutoff_date)
        .execute(&state.db)
        .await?;
    Ok(res.rows_affected() as i64)
}

/// 清空数据库中的所有请求日志。
pub async fn clear_all_logs(state: &AppState) -> Result<i64> {
    let res = sqlx::query("DELETE FROM request_logs")
        .execute(&state.db)
        .await?;
    Ok(res.rows_affected() as i64)
}

/// 根据 ID 查询单条请求日志。
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

/// 分页并根据过滤条件查询请求日志列表，返回 (总记录数, 分页结果列表)，按最新优先排序。
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
