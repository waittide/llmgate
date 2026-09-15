//! 流式响应体终结器：传输中实时解析用量，流结束/中断时完成指标统计与日志持久化。

use std::{
    io::ErrorKind,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures::Stream;

use crate::{
    model::{ModelPrice, RequestLog, StatsMetrics, request_status},
    op::{AppState, log, stats},
    relay::{
        protocol::{calculate_cost, calculate_input_cost},
        sse::extract_usage_from_sse_buffer,
        state::{Usage, finish_request},
    },
};

/// 流式响应体内存捕获上限（64KB），用于防止大体量日志撑爆存储。
pub(crate) const RESPONSE_CAPTURE_LIMIT: usize = 64 * 1024;

/// 一次流式转发的最终结局。
///
/// 收尾时必须说清「是谁结束了这条流」：上游故障与客户端主动挂断曾经都写成
/// `stream_error`，导致失败率被客户端行为（如读完 `[DONE]` 即挂断）严重抬高，
/// 真正的上游错误文本也在此时被捕获体覆盖而丢失。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamOutcome {
    /// 上游流读到自然结束
    Completed,
    /// 上游连接中途断开，附带 reqwest/hyper 给出的原因
    UpstreamError(String),
    /// 客户端在流结束前断开（响应体被 Drop）
    ClientClosed,
    /// 管理端主动中断
    Canceled,
    /// 上游长时间无新字节，由停滞看门狗收尾
    Stalled(String),
}

impl StreamOutcome {
    /// 请求日志里的终态
    fn status(&self) -> &'static str {
        match self {
            Self::Completed => request_status::SUCCESS,
            Self::UpstreamError(_) | Self::Stalled(_) => request_status::STREAM_ERROR,
            Self::ClientClosed => request_status::CLIENT_CLOSED,
            Self::Canceled => request_status::INTERRUPTED,
        }
    }

    /// 失败信息：只记录真实原因，不再回落到捕获到的响应体
    fn error(&self) -> Option<String> {
        match self {
            Self::Completed => None,
            Self::UpstreamError(e) | Self::Stalled(e) => Some(e.clone()),
            Self::ClientClosed => Some("客户端在流结束前断开连接".to_string()),
            Self::Canceled => Some("stream interrupted by user".to_string()),
        }
    }

    /// 该轮上游交互的结局码（供轮次明细展示）
    fn round_status(&self) -> u16 {
        match self {
            Self::Completed => 200,
            // 499 沿用 nginx 语义：客户端提前关闭
            Self::ClientClosed | Self::Canceled => 499,
            Self::UpstreamError(_) => 502,
            Self::Stalled(_) => 504,
        }
    }

    /// 计入「成功」的统计量
    fn counted_success(&self) -> i64 {
        i64::from(matches!(self, Self::Completed))
    }

    /// 计入「失败」的统计量：客户端断开与管理端中断都不算上游失败
    fn counted_failed(&self) -> i64 {
        i64::from(matches!(self, Self::UpstreamError(_) | Self::Stalled(_)))
    }
}

/// 包装上游流式响应体，在传输过程中实时解析累加 Token 用量，
/// 并在流结束、上游断开或客户端提前断开时完成指标统计与日志持久化。
pub(crate) struct StreamFinalizer<S> {
    pub(crate) inner:            S,
    pub(crate) state:            AppState,
    pub(crate) api_key_id:       i64,
    pub(crate) channel_id:       i64,
    pub(crate) channel_model_id: i64,
    pub(crate) price:            Option<ModelPrice>,
    pub(crate) prompt_tokens:    i64,
    pub(crate) buffer:           Vec<u8>,
    pub(crate) output_bytes:     usize,
    pub(crate) usage:            Option<Usage>,
    pub(crate) start_time:       Instant,
    pub(crate) done:             bool,
    pub(crate) log_template:     RequestLog,
    pub(crate) previous_rounds:  Vec<crate::model::RoundLog>,
    pub(crate) key_index:        Option<usize>,
    pub(crate) response_capture: Vec<u8>,
    pub(crate) cancel_rx:        Option<tokio::sync::watch::Receiver<bool>>,
    pub(crate) stall_timer:      Option<Pin<Box<tokio::time::Sleep>>>,
    /// 流式停滞超时（可经 settings.stream_stall_timeout_seconds 配置）
    pub(crate) stall_timeout:    Duration,
    /// 收尾结局：由 `finalize` 写入，供观测与测试断言
    pub(crate) outcome:          Option<StreamOutcome>,
}

impl<S> StreamFinalizer<S> {
    fn extract_usage_from_sse(&mut self) {
        extract_usage_from_sse_buffer(&mut self.buffer, &mut self.usage);
    }

    fn finalize(&mut self, outcome: StreamOutcome) {
        if self.done {
            return;
        }
        self.done = true;

        // 先把各项判定取出，再把结局移入字段供观测（避免克隆错误文本）
        let status_str = outcome.status();
        let error_opt = outcome.error();
        let round_status = outcome.round_status();
        let request_success = outcome.counted_success();
        let request_failed = outcome.counted_failed();
        self.outcome = Some(outcome);

        let est_completion = (self.output_bytes as f64 / 3.0).ceil() as i64;
        let (prompt_tokens, completion_tokens) = match &self.usage {
            Some(u) => (
                if u.prompt_tokens > 0 {
                    u.prompt_tokens
                } else {
                    self.prompt_tokens
                },
                if u.completion_tokens > 0 {
                    u.completion_tokens
                } else {
                    est_completion
                },
            ),
            None => (self.prompt_tokens, est_completion),
        };

        let cache_read_tokens = self
            .usage
            .as_ref()
            .map(|u| u.cache_read_tokens)
            .unwrap_or(0);
        let cache_write_tokens = self
            .usage
            .as_ref()
            .map(|u| u.cache_write_tokens)
            .unwrap_or(0);
        let cost = calculate_cost(
            prompt_tokens,
            completion_tokens,
            cache_read_tokens,
            cache_write_tokens,
            self.price.as_ref(),
        );
        let elapsed_ms = self.start_time.elapsed().as_millis() as i64;
        let output_cost =
            (completion_tokens as f64 / 1_000_000.0) * self.price.as_ref().map(|p| p.output).unwrap_or(0.0);
        let input_cost = calculate_input_cost(
            prompt_tokens,
            cache_read_tokens,
            cache_write_tokens,
            self.price.as_ref(),
        );
        let delta = StatsMetrics {
            input_token: prompt_tokens,
            output_token: completion_tokens,
            input_cost,
            output_cost,
            wait_time: elapsed_ms,
            request_success,
            request_failed,
        };
        stats::record_metrics(
            &self.state,
            self.api_key_id,
            self.channel_id,
            self.channel_model_id,
            &delta,
        );
        let response_body_str = String::from_utf8_lossy(&self.response_capture).to_string();

        let mut all_rounds = self.previous_rounds.clone();
        all_rounds.push(crate::model::RoundLog {
            key_index:        self.key_index,
            round:            self.log_template.round,
            channel_name:     self.log_template.target_channel.clone(),
            model_name:       self.log_template.target_model.clone(),
            request_url:      self.log_template.request_url.clone(),
            request_headers:  self.log_template.request_headers.clone(),
            request_body:     self.log_template.request_body.clone(),
            response_url:     self.log_template.response_url.clone(),
            response_headers: self.log_template.response_headers.clone(),
            response_body:    response_body_str.clone(),
            status:           round_status,
            duration_ms:      elapsed_ms,
            error:            error_opt.clone(),
        });

        let mut req_log = self.log_template.clone();
        req_log.status = status_str.to_string();
        req_log.response_body = response_body_str;
        req_log.prompt_tokens = prompt_tokens;
        req_log.completion_tokens = completion_tokens;
        req_log.total_tokens = prompt_tokens + completion_tokens;
        req_log.cache_read_tokens = cache_read_tokens;
        req_log.cache_write_tokens = cache_write_tokens;
        req_log.cost = cost;
        req_log.wait_time = elapsed_ms;
        req_log.error = error_opt;
        req_log.rounds_detail = log::serialize_rounds_detail(&self.state, &all_rounds);

        finish_request(&self.state, req_log);
    }
}

impl<S> Stream for StreamFinalizer<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        let this = self.as_mut().get_mut();
        if let Some(rx) = &this.cancel_rx
            && *rx.borrow()
        {
            this.finalize(StreamOutcome::Canceled);
            return Poll::Ready(None);
        }
        let inner_poll = Pin::new(&mut this.inner).poll_next(cx);
        if inner_poll.is_pending() {
            let mut cancel_triggered = false;
            if let Some(rx) = &mut this.cancel_rx {
                let mut changed_fut = std::pin::pin!(rx.changed());
                if let Poll::Ready(Ok(())) = changed_fut.as_mut().poll(cx) {
                    cancel_triggered = true;
                }
            }
            if cancel_triggered
                && let Some(rx) = &this.cancel_rx
                && *rx.borrow()
            {
                this.finalize(StreamOutcome::Canceled);
                return Poll::Ready(None);
            }
        }
        match inner_poll {
            Poll::Ready(Some(Ok(bytes))) => {
                this.output_bytes += bytes.len();
                this.buffer.extend_from_slice(&bytes);
                this.extract_usage_from_sse();
                if this.response_capture.len() < RESPONSE_CAPTURE_LIMIT {
                    let take = (RESPONSE_CAPTURE_LIMIT - this.response_capture.len()).min(bytes.len());
                    this.response_capture.extend_from_slice(&bytes[..take]);
                }
                this.stall_timer = None;
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                // 上游连接中途断开：保留原始错误文本，并继续把错误抛给下游
                this.finalize(StreamOutcome::UpstreamError(e.to_string()));
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                this.finalize(StreamOutcome::Completed);
                Poll::Ready(None)
            }
            Poll::Pending => {
                // 停滞检测：上游长时间无新字节时自唤醒并强制终结，避免连接永久挂起
                let timer = this
                    .stall_timer
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(this.stall_timeout)));
                if timer.as_mut().poll(cx).is_ready() {
                    let msg = format!("上游流停滞超过 {}s，连接已中止", this.stall_timeout.as_secs());
                    this.finalize(StreamOutcome::Stalled(msg.clone()));
                    let err = std::io::Error::new(ErrorKind::TimedOut, msg);
                    return Poll::Ready(Some(Err(err)));
                }
                Poll::Pending
            }
        }
    }
}

impl<S> Drop for StreamFinalizer<S> {
    fn drop(&mut self) {
        // 响应体在完成前被 Drop：客户端主动断开（也可能是运行时关停）。
        // 单独成态，不再算作上游失败——客户端读到结束标记就挂断是常见行为，
        // 过去它会把失败率抬高一截。
        self.finalize(StreamOutcome::ClientClosed);
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};
    use sqlx::Row;

    use super::*;

    struct Harness {
        state:    AppState,
        db_path:  std::path::PathBuf,
        template: RequestLog,
    }

    /// 临时库 + `AppState` + 一条 `running` 日志行：收尾逻辑按 id UPDATE，行必须先存在。
    async fn harness() -> Harness {
        let db_path = std::env::temp_dir().join(format!("llmgate_finalizer_{}.db", nanoid::nanoid!(12)));
        let pool = crate::db::init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();
        let id = crate::op::log::create_initial_log(&state, "m", "/chat/completions", "{}", "{}", 0)
            .await
            .unwrap();
        let template = RequestLog {
            id,
            model: "m".to_string(),
            ..RequestLog::default()
        };
        Harness {
            state,
            db_path,
            template,
        }
    }

    impl Harness {
        fn finalizer<S>(&self, inner: S) -> StreamFinalizer<S>
        where
            S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
        {
            StreamFinalizer {
                inner,
                state: self.state.clone(),
                api_key_id: 1,
                channel_id: 1,
                channel_model_id: 1,
                price: None,
                prompt_tokens: 10,
                buffer: Vec::new(),
                output_bytes: 0,
                usage: None,
                start_time: Instant::now(),
                done: false,
                log_template: self.template.clone(),
                previous_rounds: Vec::new(),
                key_index: Some(0),
                response_capture: Vec::new(),
                cancel_rx: None,
                stall_timer: None,
                stall_timeout: Duration::from_secs(300),
                outcome: None,
            }
        }

        fn cleanup(self) {
            let _ = std::fs::remove_file(self.db_path);
        }
    }

    /// 落盘是异步的：轮询等待终态，返回 (状态, 原因, 最后一轮的状态码)
    async fn wait_finished(state: &AppState, id: i64) -> (String, Option<String>, i64) {
        for _ in 0..200 {
            let row = sqlx::query("SELECT status, error, rounds_detail FROM request_logs WHERE id = ?")
                .bind(id)
                .fetch_one(&state.db)
                .await
                .unwrap();
            let status: String = row.get("status");
            if status != request_status::RUNNING {
                let error: Option<String> = row.get("error");
                let rounds: String = row.get("rounds_detail");
                let rounds: Vec<serde_json::Value> = serde_json::from_str(&rounds).unwrap_or_default();
                let code = rounds
                    .last()
                    .and_then(|r| r.get("status"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                return (status, error, code);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("请求日志未在预期时间内收尾");
    }

    #[tokio::test]
    async fn immediate_cancel_on_pending_returns_none() {
        let h = harness().await;
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let mut finalizer = h.finalizer(stream::pending::<Result<Bytes, std::io::Error>>());
        finalizer.cancel_rx = Some(cancel_rx);

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = cancel_tx.send(true);
        });

        let next_item = tokio::time::timeout(std::time::Duration::from_secs(1), finalizer.next()).await;
        assert!(next_item.is_ok());
        assert!(next_item.unwrap().is_none());
        assert_eq!(finalizer.outcome, Some(StreamOutcome::Canceled));
        drop(finalizer);

        let (status, _, code) = wait_finished(&h.state, h.template.id).await;
        assert_eq!(status, request_status::INTERRUPTED);
        assert_eq!(code, 499, "中断属于客户端侧结束，轮次状态码用 499");

        h.cleanup();
    }

    /// 上游中途断开：状态是 stream_error，原因保留 reqwest/hyper 的原文（不再被捕获体覆盖），
    /// 轮次状态码是 502，并计入失败。
    #[tokio::test]
    async fn upstream_break_is_recorded_with_reason_and_counted_as_failure() {
        let h = harness().await;
        let inner = stream::iter(vec![
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            )),
            Err(std::io::Error::other("上游连接被重置")),
        ]);
        let mut finalizer = h.finalizer(inner);

        assert!(finalizer.next().await.unwrap().is_ok());
        assert!(
            finalizer.next().await.unwrap().is_err(),
            "错误要继续抛给下游，客户端据此判定截断"
        );
        drop(finalizer);

        let (status, error, code) = wait_finished(&h.state, h.template.id).await;
        assert_eq!(status, request_status::STREAM_ERROR);
        assert_eq!(error.as_deref(), Some("上游连接被重置"));
        assert_eq!(code, 502);
        let metrics = h.state.stats.read().total.metrics.clone();
        assert_eq!(metrics.request_failed, 1);
        assert_eq!(metrics.request_success, 0);

        h.cleanup();
    }

    /// 客户端在读完后自行挂断（响应体被 Drop）：单独成态、不算失败，
    /// 否则「读到结束标记就关连接」的客户端会把失败率抬高一大截。
    #[tokio::test]
    async fn client_disconnect_is_recorded_separately_without_failure_count() {
        let h = harness().await;
        let finalizer = h.finalizer(stream::pending::<Result<Bytes, std::io::Error>>());
        drop(finalizer);

        let (status, error, code) = wait_finished(&h.state, h.template.id).await;
        assert_eq!(status, request_status::CLIENT_CLOSED);
        assert_eq!(error.as_deref(), Some("客户端在流结束前断开连接"));
        assert_eq!(code, 499);
        let metrics = h.state.stats.read().total.metrics.clone();
        assert_eq!(metrics.request_failed, 0);
        assert_eq!(metrics.request_success, 0);

        h.cleanup();
    }

    /// 正常读完：成功、无错误、轮次状态码 200，并且读到结束标记就挂断的客户端
    /// 不会再被记成失败（这里以先读完再 Drop 的方式覆盖 Drop 幂等）。
    #[tokio::test]
    async fn completed_stream_is_recorded_as_success() {
        let h = harness().await;
        let inner = stream::iter(vec![Ok(Bytes::from_static(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n",
        ))]);
        let mut finalizer = h.finalizer(inner);
        while finalizer.next().await.is_some() {}
        drop(finalizer);

        let (status, error, code) = wait_finished(&h.state, h.template.id).await;
        assert_eq!(status, request_status::SUCCESS);
        assert!(error.is_none());
        assert_eq!(code, 200);
        let metrics = h.state.stats.read().total.metrics.clone();
        assert_eq!(metrics.request_success, 1);
        assert_eq!(metrics.request_failed, 0);

        h.cleanup();
    }
}
