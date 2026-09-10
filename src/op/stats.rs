use anyhow::Result;
use chrono::Timelike;
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

/// 跨天结转（调用方须已持有写锁）：内存统计仍停留前一天时重置今日与小时统计，
/// 返回需要异步落盘的前一天数据；日期未变则返回 None。
///
/// 不变量：`hourly[*].date` 恒等于 `daily.date`（两者总在同一处一起重置），
/// 因此日期等于今天即代表小时统计也已就位，调用方无需再逐小时校验。
fn take_rollover(stats: &mut SystemStats, now_date: &str) -> Option<(StatsDaily, Vec<StatsHourly>)> {
    if stats.daily.date == now_date {
        return None;
    }
    let prev_daily = stats.daily.clone();
    let prev_hourlies: Vec<StatsHourly> = stats
        .hourly
        .iter()
        .filter(|h| {
            h.metrics.input_token > 0
                || h.metrics.output_token > 0
                || h.metrics.request_success > 0
                || h.metrics.request_failed > 0
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
    Some((prev_daily, prev_hourlies))
}

/// 将结转出的前一天统计异步落盘；零流量时 /stats/today 也能及时翻转。
fn persist_rollover(state: &AppState, prev_daily: StatsDaily, prev_hourlies: Vec<StatsHourly>) {
    let db = state.db.clone();
    tokio::spawn(async move {
        let mut tx = match db.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!("开启每日统计结转事务失败: {}", e);
                return;
            }
        };

        // 单条写入失败即中止整个结转事务（tx drop 自动回滚），
        // 避免部分写入被提交造成统计静默损坏
        if let Err(e) = upsert_daily_stats(&mut tx, &prev_daily).await {
            tracing::error!("写入结转的每日统计失败: {}", e);
            return;
        }

        for h in prev_hourlies {
            if let Err(e) = upsert_hourly_stats(&mut tx, &h).await {
                tracing::error!(
                    "写入结转的小时统计失败（hour={} date={}），事务已回滚: {}",
                    h.hour,
                    h.date,
                    e
                );
                return;
            }
        }

        if let Err(e) = tx.commit().await {
            tracing::error!("提交每日统计结转事务失败: {}", e);
        }
    });
}

/// 若内存统计仍停留在前一天，先结转：重置今日与小时统计，并将前一天数据异步落盘。
/// record_metrics（有新流量时）与 flush_stats_to_db（定时）都会调用，保证跨天翻转不依赖流量。
fn rollover_daily_if_needed(state: &AppState, now_date: &str) {
    let pending = take_rollover(&mut state.stats.write(), now_date);
    // 写锁外异步落盘，不阻塞统计写入
    if let Some((prev_daily, prev_hourlies)) = pending {
        persist_rollover(state, prev_daily, prev_hourlies);
    }
}
/// 按固定列序绑定指标七元组：`input_token / output_token / input_cost /
/// output_cost / wait_time / request_success / request_failed`。
///
/// 六处统计写入（日、小时、总计、API Key、渠道、渠道模型）共用这一份绑定顺序，
/// 新增指标只需改本宏与各 SQL 的列清单——原先六份逐字重复的绑定链正是漏改的来源。
macro_rules! bind_metrics {
    ($query:expr, $metrics:expr) => {
        $query
            .bind($metrics.input_token)
            .bind($metrics.output_token)
            .bind($metrics.input_cost)
            .bind($metrics.output_cost)
            .bind($metrics.wait_time)
            .bind($metrics.request_success)
            .bind($metrics.request_failed)
    };
}

/// 日统计 upsert：跨天结转与定时落盘共用同一份语句，列集不漂移。
async fn upsert_daily_stats(conn: &mut sqlx::SqliteConnection, daily: &StatsDaily) -> sqlx::Result<()> {
    bind_metrics!(
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
        .bind(&daily.date),
        &daily.metrics
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// 小时统计 upsert（同上）。
async fn upsert_hourly_stats(conn: &mut sqlx::SqliteConnection, hourly: &StatsHourly) -> sqlx::Result<()> {
    bind_metrics!(
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
        .bind(hourly.hour)
        .bind(&hourly.date),
        &hourly.metrics
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// 渠道指标的落盘 SQL（渠道与渠道模型列名、语义完全一致，仅表名不同）。
const UPDATE_CHANNEL_METRICS: &str = "UPDATE channels SET input_token = ?, output_token = ?, input_cost = ?, \
     output_cost = ?, wait_time = ?, request_success = ?, request_failed = ? WHERE id = ?";

/// 渠道模型指标的落盘 SQL。
const UPDATE_CHANNEL_MODEL_METRICS: &str = "UPDATE channel_models SET input_token = ?, output_token = ?, \
     input_cost = ?, output_cost = ?, wait_time = ?, request_success = ?, request_failed = ? WHERE id = ?";

/// 渠道 / 渠道模型指标落盘（语句由调用方以常量给出，不接受运行时拼接）。
async fn upsert_owner_metrics(
    conn: &mut sqlx::SqliteConnection,
    sql: &'static str,
    id: i64,
    m: &StatsMetrics,
) -> sqlx::Result<()> {
    bind_metrics!(sqlx::query(sql), m)
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// 指标落盘的脏标记：只写变动过的渠道 / 渠道模型。
///
/// 落盘周期（默认 10s）固定遍历全部渠道与模型时，绝大多数行的值自上一轮起并未
/// 变化，却每条都产生一次 UPDATE——纯写放大，且都落在 SQLite 唯一的写路径上。
/// 由写入侧标记、落盘侧消费。
#[derive(Default)]
pub struct StatsDirty {
    channels:       parking_lot::Mutex<std::collections::HashSet<i64>>,
    channel_models: parking_lot::Mutex<std::collections::HashSet<i64>>,
}

impl StatsDirty {
    pub fn mark_channel(&self, id: i64) {
        self.channels.lock().insert(id);
    }

    pub fn mark_channel_model(&self, id: i64) {
        self.channel_models.lock().insert(id);
    }

    /// 取走当前待落盘的两个集合。
    ///
    /// 顺序是先取走、后读值：取走后发生的更新会重新入集、留待下一轮（不丢），
    /// 而本轮读到的值必定不早于标记时刻（不旧）。
    fn take(&self) -> (Vec<i64>, Vec<i64>) {
        let channels = std::mem::take(&mut *self.channels.lock())
            .into_iter()
            .collect();
        let channel_models = std::mem::take(&mut *self.channel_models.lock())
            .into_iter()
            .collect();
        (channels, channel_models)
    }
}

pub fn record_metrics(state: &AppState, api_key_id: i64, channel_id: i64, channel_model_id: i64, delta: &StatsMetrics) {
    let now = chrono::Local::now();
    let now_date = now.format("%Y%m%d").to_string();

    // 1. 跨天结转与总量/今日/当前小时累加在同一把写锁内完成：结转本身需要写锁，
    //    再来一次只会让每请求多付一轮锁竞争。
    let pending = {
        let mut stats = state.stats.write();
        let pending = take_rollover(&mut stats, &now_date);
        stats.total.metrics.add(delta);
        stats.daily.metrics.add(delta);
        stats.hourly[now.hour() as usize].metrics.add(delta);
        pending
    };
    if let Some((prev_daily, prev_hourlies)) = pending {
        persist_rollover(state, prev_daily, prev_hourlies);
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

    // 3. 更新渠道及渠道模型内存指标，并标记为待落盘（只有它们变动过）
    let touched_channel = if let Some(mut ch) = state.channels.get_mut(&channel_id) {
        ch.metrics.add(delta);
        true
    } else {
        false
    };
    if touched_channel {
        state.stats_dirty.mark_channel(channel_id);
    }

    let touched_model = if let Some(mut cm) = state.channel_models.get_mut(&channel_model_id) {
        cm.metrics.add(delta);
        true
    } else {
        false
    };
    if touched_model {
        state.stats_dirty.mark_channel_model(channel_model_id);
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
    bind_metrics!(
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
        ),
        &total.metrics
    )
    .execute(&mut *tx)
    .await?;

    // 2. 落盘今日统计
    upsert_daily_stats(&mut tx, &daily).await?;

    // 3. 落盘每小时统计
    for h in &hourlies {
        upsert_hourly_stats(&mut tx, h).await?;
    }

    // 4. 落盘各 API Key 统计
    // 先取快照再写库：DashMap 的迭代守卫不跨 await 持有，避免写库期间占着分片锁
    let api_key_stats: Vec<(i64, StatsMetrics)> = state
        .stats_api_keys
        .iter()
        .map(|entry| (entry.api_key_id, entry.metrics.clone()))
        .collect();
    for (api_key_id, metrics) in api_key_stats {
        bind_metrics!(
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
            .bind(api_key_id),
            &metrics
        )
        .execute(&mut *tx)
        .await?;
    }

    // 5. 落盘渠道与渠道模型指标：只写自上一轮起变动过的行
    flush_owner_metrics(state, &mut tx).await?;

    tx.commit().await?;
    Ok(())
}

/// 落盘自上一轮起变动过的渠道 / 渠道模型指标。
///
/// 失败时把标记还回去：本轮事务已回滚，这些行仍是脏的，下一轮必须重试——
/// 否则一次瞬时写失败会让它们的落盘值一直停在变化之前。
async fn flush_owner_metrics(state: &AppState, conn: &mut sqlx::SqliteConnection) -> Result<()> {
    let (channels, channel_models) = state.stats_dirty.take();

    // 指标先克隆出来再写库：DashMap 的读守卫不跨 await 持有
    let channel_rows: Vec<(i64, StatsMetrics)> = channels
        .iter()
        .filter_map(|id| state.channels.get(id).map(|ch| (ch.id, ch.metrics.clone())))
        .collect();
    let model_rows: Vec<(i64, StatsMetrics)> = channel_models
        .iter()
        .filter_map(|id| {
            state
                .channel_models
                .get(id)
                .map(|cm| (cm.id, cm.metrics.clone()))
        })
        .collect();

    let mut result = Ok(());
    for (id, metrics) in &channel_rows {
        if let Err(e) = upsert_owner_metrics(conn, UPDATE_CHANNEL_METRICS, *id, metrics).await {
            result = Err(e.into());
            break;
        }
    }
    if result.is_ok() {
        for (id, metrics) in &model_rows {
            if let Err(e) = upsert_owner_metrics(conn, UPDATE_CHANNEL_MODEL_METRICS, *id, metrics).await {
                result = Err(e.into());
                break;
            }
        }
    }

    if result.is_err() {
        for id in channels {
            state.stats_dirty.mark_channel(id);
        }
        for id in channel_models {
            state.stats_dirty.mark_channel_model(id);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::setup_test_state;

    /// 落盘必须把内存中的渠道与渠道模型指标写进各自的表：
    /// 两个 UPDATE 语句列集不同，写错会静默丢字段。
    #[tokio::test]
    async fn flush_persists_channel_and_model_metrics() {
        let (state, db_path) = setup_test_state("stats_flush").await;

        let ch_id =
            sqlx::query("INSERT INTO channels (name, type, base_url, key) VALUES ('c', 'openai', 'https://x', 'k')")
                .execute(&state.db)
                .await
                .unwrap()
                .last_insert_rowid();
        let cm_id = sqlx::query("INSERT INTO channel_models (channel_id, name) VALUES (?, 'm')")
            .bind(ch_id)
            .execute(&state.db)
            .await
            .unwrap()
            .last_insert_rowid();
        crate::op::channel::refresh_channel_cache(&state)
            .await
            .unwrap();

        let delta = StatsMetrics {
            input_token:     7,
            output_token:    3,
            input_cost:      0.5,
            output_cost:     1.5,
            wait_time:       12,
            request_success: 1,
            request_failed:  0,
        };
        record_metrics(&state, 0, ch_id, cm_id, &delta);
        flush_stats_to_db(&state).await.unwrap();

        let ch_row: (i64, i64, f64, f64, i64, i64, i64) = sqlx::query_as(
            "SELECT input_token, output_token, input_cost, output_cost, wait_time, request_success, request_failed FROM channels WHERE id = ?",
        )
        .bind(ch_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(ch_row, (7, 3, 0.5, 1.5, 12, 1, 0));

        let cm_row: (i64, i64, f64, i64) = sqlx::query_as(
            "SELECT input_token, output_token, output_cost, request_success FROM channel_models WHERE id = ?",
        )
        .bind(cm_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(cm_row, (7, 3, 1.5, 1));

        // 今日与当前小时统计同样落盘（内存值即数据库值）
        let today = state.stats.read().daily.date.clone();
        let daily: (i64, i64) = sqlx::query_as("SELECT input_token, output_token FROM stats_dailies WHERE date = ?")
            .bind(&today)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(daily, (7, 3));

        let _ = std::fs::remove_file(db_path);
    }

    /// 脏标记被落盘消费后，其后的新增指标必须仍能进入下一轮落盘。
    /// 回归：若消费后不再重新标记（或标记只在建缓存时打），第二次增量会永远
    /// 停在内存里，数据库停在上一轮的值。
    #[tokio::test]
    async fn metrics_recorded_after_a_flush_are_persisted_by_the_next_one() {
        let (state, db_path) = setup_test_state("stats_dirty_cycle").await;

        let ch_id =
            sqlx::query("INSERT INTO channels (name, type, base_url, key) VALUES ('c', 'openai', 'https://x', 'k')")
                .execute(&state.db)
                .await
                .unwrap()
                .last_insert_rowid();
        let cm_id = sqlx::query("INSERT INTO channel_models (channel_id, name) VALUES (?, 'm')")
            .bind(ch_id)
            .execute(&state.db)
            .await
            .unwrap()
            .last_insert_rowid();
        crate::op::channel::refresh_channel_cache(&state)
            .await
            .unwrap();

        let delta = StatsMetrics {
            input_token: 7,
            request_success: 1,
            ..Default::default()
        };
        for _ in 0..2 {
            record_metrics(&state, 0, ch_id, cm_id, &delta);
            flush_stats_to_db(&state).await.unwrap();
        }

        let ch_input: (i64,) = sqlx::query_as("SELECT input_token FROM channels WHERE id = ?")
            .bind(ch_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(ch_input.0, 14, "第二轮增量也必须落盘");
        let cm_input: (i64,) = sqlx::query_as("SELECT input_token FROM channel_models WHERE id = ?")
            .bind(cm_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(cm_input.0, 14);

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_cross_day_stats_rollover_persists_previous_day() {
        let (state, db_path) = setup_test_state("stats").await;

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
