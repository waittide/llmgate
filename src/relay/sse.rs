//! SSE 缓冲与解析工具：跨网络分片累积字节、按完整事件切分、提取用量。

use crate::relay::{protocol::extract_usage_from_json, state::Usage};

pub(crate) const SSE_BUFFER_LIMIT: usize = 64 * 1024; // 64KB 临时 SSE 累加缓冲区上限

/// 处理可能被流分片边界截断的 SSE 事件结尾判定。
pub fn sse_event_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 2);
        }
        if i + 3 < buf.len() && buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i + 4);
        }
    }
    None
}

/// 解析单个 SSE 事件：返回 `event:` 字段（缺失为 ""）与按标准拼接的 `data:` 载荷。
pub fn parse_sse_event(event_str: &str) -> (&str, String) {
    let mut event_type = "";
    let mut data_parts: Vec<&str> = Vec::new();
    for line in event_str.lines() {
        let trimmed = line.trim();
        if let Some(et) = trimmed.strip_prefix("event:") {
            event_type = et.trim();
        } else if let Some(d) = trimmed.strip_prefix("data:") {
            data_parts.push(d.trim());
        }
    }
    (event_type, data_parts.join("\n"))
}

/// 事件级 SSE 缓冲：跨网络分片累积字节，按完整事件逐个吐出。
#[derive(Default)]
pub struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 取出下一个完整事件（含结束符），无完整事件时返回 None。
    pub(crate) fn next_event(&mut self) -> Option<String> {
        let end = sse_event_end(&self.buf)?;
        let event = String::from_utf8_lossy(&self.buf[..end]).into_owned();
        self.buf.drain(..end);
        Some(event)
    }
}

pub(crate) fn extract_usage_from_sse_buffer(buffer: &mut Vec<u8>, usage: &mut Option<Usage>) {
    while let Some(end) = sse_event_end(buffer) {
        let event_bytes = &buffer[..end];
        // 快速路径：若当前事件不包含 "usage" 关键字，跳过反序列化与行解析以节省性能
        if event_bytes.windows(7).any(|w| w == b"\"usage\"") {
            let text = String::from_utf8_lossy(event_bytes);
            for line in text.lines() {
                if let Some(data) = line.trim_start().strip_prefix("data:") {
                    let trimmed = data.trim();
                    if !trimmed.is_empty()
                        && trimmed != "[DONE]"
                        && let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
                        && let Some(u) = extract_usage_from_json(&val)
                    {
                        // 跨事件合并 Token 计数（例如 Anthropic 的 message_start 提供 input_tokens，
                        // 而 message_delta 提供 output_tokens）。
                        let merged = match usage {
                            Some(existing) => Usage {
                                prompt_tokens:      if existing.prompt_tokens > 0 {
                                    existing.prompt_tokens
                                } else {
                                    u.prompt_tokens
                                },
                                completion_tokens:  if existing.completion_tokens > 0 {
                                    existing.completion_tokens
                                } else {
                                    u.completion_tokens
                                },
                                total_tokens:       if existing.total_tokens > 0 {
                                    existing.total_tokens
                                } else {
                                    u.total_tokens
                                },
                                cache_read_tokens:  if existing.cache_read_tokens > 0 {
                                    existing.cache_read_tokens
                                } else {
                                    u.cache_read_tokens
                                },
                                cache_write_tokens: if existing.cache_write_tokens > 0 {
                                    existing.cache_write_tokens
                                } else {
                                    u.cache_write_tokens
                                },
                            },
                            None => u,
                        };
                        *usage = Some(merged);
                    }
                }
            }
        }
        buffer.drain(..end);
    }
    // 防止无事件边界的异常数据流导致缓冲区无限膨胀
    if buffer.len() > SSE_BUFFER_LIMIT {
        buffer.drain(..buffer.len() - (SSE_BUFFER_LIMIT / 2));
    }
}
