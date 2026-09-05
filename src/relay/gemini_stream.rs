use std::{
    collections::{BTreeMap, VecDeque},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::Stream;
use serde_json::{Value, json};

use super::sse_event_end;
use crate::relay::{protocol::extract_usage_from_json, state::Usage};

#[derive(Default)]
struct ToolState {
    name:      String,
    arguments: String,
}

/// 将 OpenAI Chat Completions SSE 流适配转换为 Gemini generateContent SSE 流。
/// Gemini 无分片 functionCall：参数增量先累积，finish 时一次性补发完整调用。
pub struct OpenaiToGeminiStream<S> {
    inner:     S,
    buffer:    Vec<u8>,
    pending:   VecDeque<Bytes>,
    model:     String,
    tools:     BTreeMap<u64, ToolState>,
    completed: bool,
    usage:     Option<Usage>,
}

impl<S> OpenaiToGeminiStream<S> {
    pub fn new(inner: S, model: String) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            model,
            tools: BTreeMap::new(),
            completed: false,
            usage: None,
        }
    }

    fn push(&mut self, mut data: Value) {
        data["modelVersion"] = Value::String(self.model.clone());
        self.pending.push_back(Bytes::from(format!(
            "data: {}\n\n",
            serde_json::to_string(&data).unwrap_or_default()
        )));
    }

    fn handle_data(&mut self, data: &str) {
        if data == "[DONE]" {
            self.complete(None);
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(u) = extract_usage_from_json(&value) {
            self.usage = Some(u);
        }
        let Some(choice) = value.pointer("/choices/0") else {
            return;
        };
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        let mut parts = Vec::new();
        if let Some(text) = delta.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            parts.push(json!({ "text": text }));
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let entry = self.tools.entry(index).or_default();
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str)
                    && !name.is_empty()
                {
                    entry.name = name.to_string();
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    entry.arguments.push_str(args);
                }
            }
        }

        let finished = choice
            .get("finish_reason")
            .is_some_and(|reason| !reason.is_null());
        if finished {
            for tool in self.tools.values() {
                let parsed: Value = serde_json::from_str(&tool.arguments).unwrap_or_else(|_| json!({}));
                parts.push(json!({ "functionCall": { "name": tool.name, "args": parsed } }));
            }
        }

        let finish_reason = if finished {
            Value::String(
                match choice.get("finish_reason").and_then(Value::as_str) {
                    Some("length") => "MAX_TOKENS",
                    Some("content_filter") => "SAFETY",
                    _ => "STOP",
                }
                .to_string(),
            )
        } else {
            Value::Null
        };

        let usage = self.usage.clone();
        if parts.is_empty() && !finished && usage.is_none() {
            return;
        }
        let mut chunk = json!({
            "candidates": [{
                "content": { "role": "model", "parts": parts },
                "finishReason": finish_reason
            }]
        });
        if let Some(usage) = usage {
            chunk["usageMetadata"] = json!({
                "promptTokenCount": usage.prompt_tokens,
                "candidatesTokenCount": usage.completion_tokens,
                "totalTokenCount": usage.total_tokens
            });
        }
    }

    fn complete(&mut self, finish_reason: Option<&str>) {
        if self.completed {
            return;
        }
        self.completed = true;
        // 若上游未发 finish_reason（如直接 [DONE]），补发累积的工具调用与终止事件
        let mut parts = Vec::new();
        for tool in self.tools.values() {
            let parsed: Value = serde_json::from_str(&tool.arguments).unwrap_or_else(|_| json!({}));
            parts.push(json!({ "functionCall": { "name": tool.name, "args": parsed } }));
        }
        let _ = finish_reason;
        let (prompt_tokens, completion_tokens, total_tokens) = self
            .usage
            .as_ref()
            .map(|u| (u.prompt_tokens, u.completion_tokens, u.total_tokens))
            .unwrap_or((0, 0, 0));
        self.push(json!({
            "candidates": [{
                "content": { "role": "model", "parts": parts },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": prompt_tokens,
                "candidatesTokenCount": completion_tokens,
                "totalTokenCount": total_tokens
            }
        }));
    }
}

impl<S> Stream for OpenaiToGeminiStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if let Some(chunk) = this.pending.pop_front() {
            return Poll::Ready(Some(Ok(chunk)));
        }
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.buffer.extend_from_slice(&bytes);
                while let Some(end) = sse_event_end(&this.buffer) {
                    let event = String::from_utf8_lossy(&this.buffer[..end]).to_string();
                    for data in event
                        .lines()
                        .filter_map(|line| line.trim().strip_prefix("data:").map(str::trim))
                    {
                        this.handle_data(data);
                    }
                    this.buffer.drain(..end);
                }
                if let Some(chunk) = this.pending.pop_front() {
                    Poll::Ready(Some(Ok(chunk)))
                } else {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
            Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(error))),
            Poll::Ready(None) => {
                this.complete(None);
                this.pending
                    .pop_front()
                    .map(|chunk| Poll::Ready(Some(Ok(chunk))))
                    .unwrap_or(Poll::Ready(None))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, stream};

    use super::*;

    #[tokio::test]
    async fn converts_chat_text_and_tools_to_gemini_events() {
        let chunks = vec![
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"calc\",\"arguments\":\"{\\\"x\\\":\"}}]}}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4,\"total_tokens\":7}}\n\ndata: [DONE]\n\n",
            )),
        ];
        let output = OpenaiToGeminiStream::new(stream::iter(chunks), "gemini-test".to_string())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(String::new(), |mut output, chunk| {
                output.push_str(&String::from_utf8_lossy(&chunk));
                output
            });

        eprintln!("OUT1: {}", output.replace('\n', "|"));
        assert!(output.contains("\"functionCall\":{\"args\":{\"x\":1},\"name\":\"calc\"}"));
        assert!(output.contains("\"finishReason\":\"STOP\""));
        assert!(output.contains("\"promptTokenCount\":3"));
    }
}
