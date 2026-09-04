use anyhow::Result;
use sqlx::Row;

use crate::{
    model::{StatsAPIKey, StatsDaily, StatsHourly, StatsMetrics, StatsTotal, SystemStats},
    op::AppState,
};

pub async fn refresh_stats_cache(state: &AppState) -> Result<()> {
    let now_date = chrono::Local::now().format("%Y%m%d").to_string();

    // 1. 全量总计统计 (Total)
    let total = if let Some(row) = sqlx::query("SELECT * FROM stats_totals WHERE id = 1")
        .fetch_optional(&state.db)
        .await?
    {
        StatsTotal {
            id:      1,
            metrics: StatsMetrics::from_sql_row(&row),
        }
    } else {
        StatsTotal {
            id:      1,
            metrics: StatsMetrics::default(),
        }
    };

    // 2. 今日按天统计 (Daily)
    let daily = if let Some(row) = sqlx::query("SELECT * FROM stats_dailies WHERE date = ?")
        .bind(&now_date)
        .fetch_optional(&state.db)
        .await?
    {
        StatsDaily {
            date:    now_date.clone(),
            metrics: StatsMetrics::from_sql_row(&row),
        }
    } else {
        StatsDaily {
            date:    now_date.clone(),
            metrics: StatsMetrics::default(),
        }
    };

    // 3. 今日每小时统计 (Hourly)
    let hourly_rows = sqlx::query("SELECT * FROM stats_hourlies WHERE date = ?")
        .bind(&now_date)
        .fetch_all(&state.db)
        .await?;

    let mut hourlies = std::array::from_fn(|i| StatsHourly {
        hour:    i as i64,
        date:    now_date.clone(),
        metrics: StatsMetrics::default(),
    });

    for row in hourly_rows {
        let hour: i64 = row.get("hour");
        if (0..24).contains(&hour) {
            hourlies[hour as usize] = StatsHourly {
                hour,
                date: now_date.clone(),
                metrics: StatsMetrics::from_sql_row(&row),
            };
        }
    }
    *state.stats.write() = SystemStats {
        total,
        daily,
        hourly: hourlies,
    };

    // 4. 按 API Key 统计
    state.stats_api_keys.clear();
    let apikey_rows = sqlx::query("SELECT * FROM stats_api_keys")
        .fetch_all(&state.db)
        .await?;
    for row in apikey_rows {
        let api_key_id: i64 = row.get("api_key_id");
        let metrics = StatsMetrics::from_sql_row(&row);
        state
            .stats_api_keys
            .insert(api_key_id, StatsAPIKey { api_key_id, metrics });
    }

    Ok(())
}

fn today_date() -> String {
    chrono::Local::now().format("%Y%m%d").to_string()
}

/// 若内存统计仍停留在前一天，先结转：重置今日与小时统计，并将前一天数据异步落盘。
/// record_metrics（有新流量时）与 flush_stats_to_db（定时）都会调用，保证跨天翻转不依赖流量。
fn rollover_daily_if_needed(state: &AppState, now_date: &str) {
    let (prev_daily, prev_hourlies) = {
        let mut stats = state.stats.write();
        if stats.daily.date == now_date {
            return;
        }
        let prev_daily = stats.daily.clone();
        let prev_hourlies: Vec<StatsHourly> = stats
            .hourly
            .iter()
            .filter(|h| {
                h.date != now_date
                    && (h.metrics.input_token > 0
                        || h.metrics.output_token > 0
                        || h.metrics.request_success > 0
                        || h.metrics.request_failed > 0)
            })
            .cloned()
            .collect();

        stats.daily.date = now_date.to_string();
        stats.daily.metrics = StatsMetrics::default();
        for (i, h) in stats.hourly.iter_mut().enumerate() {
            *h = StatsHourly {
                hour:    i as i64,
                date:    now_date.to_string(),
                metrics: StatsMetrics::default(),
            };
        }
        (prev_daily, prev_hourlies)
    };

    // 写锁外异步落盘，不阻塞统计写入
    let db = state.db.clone();
    tokio::spawn(async move {
        let mut tx = match db.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!("开启每日统计结转事务失败: {}", e);
                return;
            }
        };

        let res = sqlx::query(
            r#"
            INSERT INTO stats_dailies (date, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(date) DO UPDATE SET
                input_token = excluded.input_token,
                output_token = excluded.output_token,
                input_cost = excluded.input_cost,
                output_cost = excluded.output_cost,
                wait_time = excluded.wait_time,
                request_success = excluded.request_success,
                request_failed = excluded.request_failed
            "#,
        )
        .bind(&prev_daily.date)
        .bind(prev_daily.metrics.input_token)
        .bind(prev_daily.metrics.output_token)
        .bind(prev_daily.metrics.input_cost)
        .bind(prev_daily.metrics.output_cost)
        .bind(prev_daily.metrics.wait_time)
        .bind(prev_daily.metrics.request_success)
        .bind(prev_daily.metrics.request_failed)
        .execute(&mut *tx)
        .await;

        if let Err(e) = res {
            tracing::error!("写入结转的每日统计失败: {}", e);
            return;
        }

        for h in prev_hourlies {
            let _ = sqlx::query(
                r#"
                INSERT INTO stats_hourlies (hour, date, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(hour, date) DO UPDATE SET
                    input_token = excluded.input_token,
                    output_token = excluded.output_token,
                    input_cost = excluded.input_cost,
                    output_cost = excluded.output_cost,
                    wait_time = excluded.wait_time,
                    request_success = excluded.request_success,
                    request_failed = excluded.request_failed
                "#,
            )
            .bind(h.hour)
            .bind(&h.date)
            .bind(h.metrics.input_token)
            .bind(h.metrics.output_token)
            .bind(h.metrics.input_cost)
            .bind(h.metrics.output_cost)
            .bind(h.metrics.wait_time)
            .bind(h.metrics.request_success)
            .bind(h.metrics.request_failed)
            .execute(&mut *tx)
            .await;
        }

        if let Err(e) = tx.commit().await {
            tracing::error!("提交每日统计结转事务失败: {}", e);
        }
    });
}
pub fn record_metrics(state: &AppState, api_key_id: i64, channel_id: i64, channel_model_id: i64, delta: &StatsMetrics) {
    let now_date = today_date();
    let current_hour = chrono::Local::now()
        .format("%H")
        .to_string()
        .parse::<usize>()
        .unwrap_or(0);

    // 0. 跨天先结转（独立写锁；落盘在锁外异步执行）
    rollover_daily_if_needed(state, &now_date);

    // 1. 原子更新总计、今日与当前小时统计
    {
        let mut stats = state.stats.write();
        stats.total.metrics.add(delta);
        stats.daily.metrics.add(delta);

        if current_hour < 24 {
            if stats.hourly[current_hour].date != now_date {
                stats.hourly[current_hour].date = now_date.clone();
                stats.hourly[current_hour].metrics = StatsMetrics::default();
            }
            stats.hourly[current_hour].metrics.add(delta);
        }
    }
    // 2. 更新 API Key 内存统计
    if api_key_id > 0 {
        state
            .stats_api_keys
            .entry(api_key_id)
            .or_insert_with(|| StatsAPIKey {
                api_key_id,
                metrics: StatsMetrics::default(),
            })
            .metrics
            .add(delta);
    }

    // 3. 更新渠道及渠道模型内存指标
    if let Some(mut ch) = state.channels.get_mut(&channel_id) {
        ch.metrics.add(delta);
    }
    if let Some(mut cm) = state.channel_models.get_mut(&channel_model_id) {
        cm.metrics.add(delta);
    }
}

pub fn get_stats_today(state: &AppState) -> StatsDaily {
    state.stats.read().daily.clone()
}

pub fn get_stats_hourly(state: &AppState) -> [StatsHourly; 24] {
    state.stats.read().hourly.clone()
}

pub fn get_stats_total(state: &AppState) -> StatsTotal {
    state.stats.read().total.clone()
}

pub async fn get_stats_daily(state: &AppState) -> Result<Vec<StatsDaily>> {
    let rows = sqlx::query("SELECT * FROM stats_dailies ORDER BY date ASC")
        .fetch_all(&state.db)
        .await?;

    let mut list = Vec::new();
    for row in rows {
        list.push(StatsDaily {
            date:    row.get("date"),
            metrics: StatsMetrics::from_sql_row(&row),
        });
    }

    let today = get_stats_today(state);
    if let Some(pos) = list.iter().position(|d| d.date == today.date) {
        list[pos] = today;
    } else {
        list.push(today);
    }

    Ok(list)
}

pub fn get_stats_api_key(state: &AppState, api_key_id: i64) -> StatsAPIKey {
    state
        .stats_api_keys
        .get(&api_key_id)
        .map(|entry| entry.value().clone())
        .unwrap_or(StatsAPIKey {
            api_key_id,
            metrics: StatsMetrics::default(),
        })
}

pub fn list_stats_api_keys(state: &AppState) -> Vec<StatsAPIKey> {
    state
        .stats_api_keys
        .iter()
        .map(|entry| entry.value().clone())
        .collect()
}

pub async fn flush_stats_to_db(state: &AppState) -> Result<()> {
    // 落盘前先处理跨天结转：零流量时 /stats/today 也能及时翻转
    rollover_daily_if_needed(state, &today_date());
    let mut tx = state.db.begin().await?;

    // 1. 落盘总计统计
    let (total, daily, hourlies) = {
        let s = state.stats.read();
        (s.total.clone(), s.daily.clone(), s.hourly.clone())
    };
    sqlx::query(
        r#"
        INSERT INTO stats_totals (id, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
        VALUES (1, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            input_token = excluded.input_token,
            output_token = excluded.output_token,
            input_cost = excluded.input_cost,
            output_cost = excluded.output_cost,
            wait_time = excluded.wait_time,
            request_success = excluded.request_success,
            request_failed = excluded.request_failed
        "#,
    )
    .bind(total.metrics.input_token)
    .bind(total.metrics.output_token)
    .bind(total.metrics.input_cost)
    .bind(total.metrics.output_cost)
    .bind(total.metrics.wait_time)
    .bind(total.metrics.request_success)
    .bind(total.metrics.request_failed)
    .execute(&mut *tx)
    .await?;

    // 2. 落盘今日统计
    sqlx::query(
        r#"
        INSERT INTO stats_dailies (date, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(date) DO UPDATE SET
            input_token = excluded.input_token,
            output_token = excluded.output_token,
            input_cost = excluded.input_cost,
            output_cost = excluded.output_cost,
            wait_time = excluded.wait_time,
            request_success = excluded.request_success,
            request_failed = excluded.request_failed
        "#,
    )
    .bind(&daily.date)
    .bind(daily.metrics.input_token)
    .bind(daily.metrics.output_token)
    .bind(daily.metrics.input_cost)
    .bind(daily.metrics.output_cost)
    .bind(daily.metrics.wait_time)
    .bind(daily.metrics.request_success)
    .bind(daily.metrics.request_failed)
    .execute(&mut *tx)
    .await?;

    // 3. 落盘每小时统计
    for h in &hourlies {
        sqlx::query(
            r#"
            INSERT INTO stats_hourlies (hour, date, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(hour, date) DO UPDATE SET
                input_token = excluded.input_token,
                output_token = excluded.output_token,
                input_cost = excluded.input_cost,
                output_cost = excluded.output_cost,
                wait_time = excluded.wait_time,
                request_success = excluded.request_success,
                request_failed = excluded.request_failed
            "#,
        )
        .bind(h.hour)
        .bind(&h.date)
        .bind(h.metrics.input_token)
        .bind(h.metrics.output_token)
        .bind(h.metrics.input_cost)
        .bind(h.metrics.output_cost)
        .bind(h.metrics.wait_time)
        .bind(h.metrics.request_success)
        .bind(h.metrics.request_failed)
        .execute(&mut *tx)
        .await?;
    }

    // 4. 落盘各 API Key 统计
    for entry in state.stats_api_keys.iter() {
        let k = entry.value();
        sqlx::query(
            r#"
            INSERT INTO stats_api_keys (api_key_id, input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(api_key_id) DO UPDATE SET
                input_token = excluded.input_token,
                output_token = excluded.output_token,
                input_cost = excluded.input_cost,
                output_cost = excluded.output_cost,
                wait_time = excluded.wait_time,
                request_success = excluded.request_success,
                request_failed = excluded.request_failed
            "#,
        )
        .bind(k.api_key_id)
        .bind(k.metrics.input_token)
        .bind(k.metrics.output_token)
        .bind(k.metrics.input_cost)
        .bind(k.metrics.output_cost)
        .bind(k.metrics.wait_time)
        .bind(k.metrics.request_success)
        .bind(k.metrics.request_failed)
        .execute(&mut *tx)
        .await?;
    }

    // 5. 落盘渠道与渠道模型指标
    for entry in state.channels.iter() {
        let ch = entry.value();
        sqlx::query(
            r#"
            UPDATE channels SET
                input_token = ?, output_token = ?, input_cost = ?, output_cost = ?,
                wait_time = ?, request_success = ?, request_failed = ?
            WHERE id = ?
            "#,
        )
        .bind(ch.metrics.input_token)
        .bind(ch.metrics.output_token)
        .bind(ch.metrics.input_cost)
        .bind(ch.metrics.output_cost)
        .bind(ch.metrics.wait_time)
        .bind(ch.metrics.request_success)
        .bind(ch.metrics.request_failed)
        .bind(ch.id)
        .execute(&mut *tx)
        .await?;
    }

    for entry in state.channel_models.iter() {
        let cm = entry.value();
        sqlx::query(
            r#"
            UPDATE channel_models SET
                input_token = ?, output_token = ?, input_cost = ?, output_cost = ?,
                wait_time = ?, request_success = ?, request_failed = ?
            WHERE id = ?
            "#,
        )
        .bind(cm.metrics.input_token)
        .bind(cm.metrics.output_token)
        .bind(cm.metrics.input_cost)
        .bind(cm.metrics.output_cost)
        .bind(cm.metrics.wait_time)
        .bind(cm.metrics.request_success)
        .bind(cm.metrics.request_failed)
        .bind(cm.id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::db::init_db;

    async fn setup_test_state() -> (AppState, PathBuf) {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("octopus_stats_test_{}.db", rnd));
        let pool = init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();
        (state, db_path)
    }

    #[tokio::test]
    async fn test_cross_day_stats_rollover_persists_previous_day() {
        let (state, db_path) = setup_test_state().await;

        // 设置初始前一日指标数据
        let yesterday = "20250101".to_string();
        {
            let mut stats = state.stats.write();
            stats.daily.date = yesterday.clone();
            stats.daily.metrics.input_token = 500;
            stats.daily.metrics.output_token = 200;
            stats.daily.metrics.request_success = 5;
            stats.hourly[23].date = yesterday.clone();
            stats.hourly[23].metrics.input_token = 500;
            stats.hourly[23].metrics.output_token = 200;
            stats.hourly[23].metrics.request_success = 5;
        }

        // 触发今日一条新的用量记录
        let delta = StatsMetrics {
            input_token: 10,
            output_token: 5,
            request_success: 1,
            ..Default::default()
        };
        record_metrics(&state, 0, 0, 0, &delta);

        // 短暂等待后台异步落盘任务执行
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // 验证前一日统计数据已成功写入 stats_dailies 表中
        let row = sqlx::query("SELECT input_token, output_token, request_success FROM stats_dailies WHERE date = ?")
            .bind(&yesterday)
            .fetch_optional(&state.db)
            .await
            .unwrap();

        assert!(row.is_some(), "Yesterday's daily stats should be persisted on rollover");
        let r = row.unwrap();
        assert_eq!(r.get::<i64, _>("input_token"), 500);
        assert_eq!(r.get::<i64, _>("output_token"), 200);
        assert_eq!(r.get::<i64, _>("request_success"), 5);

        // 验证今日内存指标已重置并累加上最新增量
        let today_stats = state.stats.read().daily.clone();
        assert_ne!(today_stats.date, yesterday);
        assert_eq!(today_stats.metrics.input_token, 10);
        assert_eq!(today_stats.metrics.output_token, 5);
        assert_eq!(today_stats.metrics.request_success, 1);

        let _ = std::fs::remove_file(db_path);
    }
}
