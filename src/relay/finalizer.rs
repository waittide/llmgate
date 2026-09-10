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
        state::{RequestOverview, Usage},
    },
};

/// 流式响应体内存捕获上限（64KB），用于防止大体量日志撑爆存储。
pub(crate) const RESPONSE_CAPTURE_LIMIT: usize = 64 * 1024;

/// 流式传输停滞超时：超过该时长没有任何新字节则判定上游挂死，主动断开并记为失败。
/// 包装上游流式响应体，在传输过程中实时解析累加 Token 用量，
/// 并在流结束或客户端提前断开连接时，且仅在此时完成指标统计与日志持久化。
pub(crate) struct StreamFinalizer<S> {
    pub(crate) inner:            S,
    pub(crate) state:            AppState,
    pub(crate) req_id:           u64,
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
    pub(crate) interrupted:      bool,
    pub(crate) stall_timer:      Option<Pin<Box<tokio::time::Sleep>>>,
    /// 流式停滞超时（可经 settings.stream_stall_timeout_seconds 配置）
    pub(crate) stall_timeout:    Duration,
    pub(crate) error_override:   Option<String>,
}

impl<S> StreamFinalizer<S> {
    fn extract_usage_from_sse(&mut self) {
        extract_usage_from_sse_buffer(&mut self.buffer, &mut self.usage);
    }

    fn finalize(&mut self, completed: bool) {
        if self.done {
            return;
        }
        self.done = true;

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
            request_success: if completed { 1 } else { 0 },
            request_failed: if completed { 0 } else { 1 },
        };
        stats::record_metrics(
            &self.state,
            self.api_key_id,
            self.channel_id,
            self.channel_model_id,
            &delta,
        );
        let status_str = if self.interrupted {
            request_status::INTERRUPTED
        } else if completed {
            request_status::SUCCESS
        } else {
            request_status::STREAM_ERROR
        };

        let response_body_str = String::from_utf8_lossy(&self.response_capture).to_string();
        let error_opt = if completed {
            None
        } else if let Some(e) = &self.error_override {
            Some(e.clone())
        } else if self.interrupted {
            Some("stream interrupted by user".to_string())
        } else if !response_body_str.is_empty() {
            Some(response_body_str.clone())
        } else {
            Some("stream interrupted".to_string())
        };

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
            status:           if completed { 200 } else { 500 },
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

        let overview = RequestOverview::from(&req_log);
        self.state.in_flight.broadcast(overview);
        self.state.in_flight.unregister(self.req_id);

        let state = self.state.clone();
        tokio::spawn(async move {
            if let Err(e) = log::update_completed_log(&state, &req_log).await {
                tracing::error!("写入已完成请求日志失败: {}", e);
            }
        });
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
            this.interrupted = true;
            this.finalize(false);
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
                this.interrupted = true;
                this.finalize(false);
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
                this.finalize(false);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                this.finalize(true);
                Poll::Ready(None)
            }
            Poll::Pending => {
                // 停滞检测：上游长时间无新字节时自唤醒并强制终结，避免连接永久挂起
                let timer = this
                    .stall_timer
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(this.stall_timeout)));
                if timer.as_mut().poll(cx).is_ready() {
                    let msg = format!("上游流停滞超过 {}s，连接已中止", this.stall_timeout.as_secs());
                    this.error_override = Some(msg.clone());
                    this.finalize(false);
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
        // 响应体在完成前被 Drop（例如客户端主动断开连接）：记录该次尝试并保存日志
        self.finalize(false);
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};

    use super::*;

    #[tokio::test]
    async fn immediate_cancel_on_pending_returns_none() {
        let rnd = nanoid::nanoid!(12);
        let db_path = std::env::temp_dir().join(format!("llmgate_cancel_test_{}.db", rnd));
        let pool = crate::db::init_db(&db_path).await.unwrap();
        let state = AppState::init(pool).await.unwrap();

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let pending_stream = stream::pending::<Result<Bytes, std::io::Error>>();

        let mut finalizer = StreamFinalizer {
            inner:            pending_stream,
            state:            state.clone(),
            req_id:           1,
            api_key_id:       1,
            channel_id:       1,
            channel_model_id: 1,
            price:            None,
            stall_timeout:    Duration::from_secs(300),
            prompt_tokens:    10,
            buffer:           Vec::new(),
            output_bytes:     0,
            usage:            None,
            start_time:       Instant::now(),
            done:             false,
            log_template:     RequestLog::default(),
            key_index:        Some(0),
            previous_rounds:  Vec::new(),
            response_capture: Vec::new(),
            cancel_rx:        Some(cancel_rx),
            interrupted:      false,
            stall_timer:      None,
            error_override:   None,
        };

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = cancel_tx.send(true);
        });

        let next_item = tokio::time::timeout(std::time::Duration::from_secs(1), finalizer.next()).await;
        assert!(next_item.is_ok());
        assert!(next_item.unwrap().is_none());
        assert!(finalizer.interrupted);

        let _ = std::fs::remove_file(db_path);
    }
}
