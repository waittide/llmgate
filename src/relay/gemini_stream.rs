use std::{
    collections::{BTreeMap, VecDeque},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::Stream;
use serde_json::{Value, json};

use crate::relay::{SseBuffer, protocol::extract_usage_from_json, state::Usage};

#[derive(Default)]
struct ToolState {
    name:      String,
    arguments: String,
}

/// 将 OpenAI Chat Completions SSE 流适配转换为 Gemini generateContent SSE 流。
/// Gemini 无分片 functionCall：参数增量先累积，finish 时一次性补发完整调用。
pub struct OpenaiToGeminiStream<S> {
    inner:     S,
    sse:       SseBuffer,
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
            sse: SseBuffer::default(),
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
            self.complete();
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

        let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
        if finish_reason.is_some() {
            let mapped = finish_reason
                .map(gemini_stream_finish_reason)
                .unwrap_or("STOP");
            let text = delta.get("content").and_then(Value::as_str).unwrap_or("");
            self.emit_final(mapped, text);
            return;
        }

        let usage = self.usage.clone();
        if parts.is_empty() && usage.is_none() {
            return;
        }
        let mut chunk = json!({
            "candidates": [{
                "content": { "role": "model", "parts": parts },
                "finishReason": Value::Null
            }]
        });
        if let Some(usage) = usage {
            chunk["usageMetadata"] = json!({
                "promptTokenCount": usage.prompt_tokens,
                "candidatesTokenCount": usage.completion_tokens,
                "totalTokenCount": usage.total_tokens
            });
        }
        self.push(chunk);
    }

    /// 发射终止 chunk（补发累积的完整 functionCall 与真实用量）；completed 防止重复发射。
    fn emit_final(&mut self, finish_reason: &str, text: &str) {
        if self.completed {
            return;
        }
        self.completed = true;
        let mut parts = Vec::new();
        if !text.is_empty() {
            parts.push(json!({ "text": text }));
        }
        for tool in self.tools.values() {
            let parsed: Value = serde_json::from_str(&tool.arguments).unwrap_or_else(|_| json!({}));
            parts.push(json!({ "functionCall": { "name": tool.name, "args": parsed } }));
        }
        let (prompt_tokens, completion_tokens, total_tokens) = self
            .usage
            .as_ref()
            .map(|u| (u.prompt_tokens, u.completion_tokens, u.total_tokens))
            .unwrap_or((0, 0, 0));
        self.push(json!({
            "candidates": [{
                "content": { "role": "model", "parts": parts },
                "finishReason": finish_reason
            }],
            "usageMetadata": {
                "promptTokenCount": prompt_tokens,
                "candidatesTokenCount": completion_tokens,
                "totalTokenCount": total_tokens
            }
        }));
    }

    fn complete(&mut self) {
        // 上游未发 finish_reason（如直接 [DONE]）时按正常结束补发终止 chunk
        self.emit_final("STOP", "");
    }
}

/// OpenAI finish_reason → Gemini finishReason。
fn gemini_stream_finish_reason(reason: &str) -> &str {
    match reason {
        "length" => "MAX_TOKENS",
        "content_filter" => "SAFETY",
        _ => "STOP",
    }
}

impl<S> Stream for OpenaiToGeminiStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        loop {
            if let Some(chunk) = this.pending.pop_front() {
                return Poll::Ready(Some(Ok(chunk)));
            }
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.sse.feed(&bytes);
                    while let Some(event) = this.sse.next_event() {
                        for data in event
                            .lines()
                            .filter_map(|line| line.trim().strip_prefix("data:").map(str::trim))
                        {
                            this.handle_data(data);
                        }
                    }
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => {
                    this.complete();
                    return match this.pending.pop_front() {
                        Some(chunk) => Poll::Ready(Some(Ok(chunk))),
                        None => Poll::Ready(None),
                    };
                }
                Poll::Pending => return Poll::Pending,
            }
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

        assert!(output.contains("\"functionCall\":{\"args\":{\"x\":1},\"name\":\"calc\"}"));
        assert!(output.contains("\"finishReason\":\"STOP\""));
        assert!(output.contains("\"promptTokenCount\":3"));
    }

    #[tokio::test]
    async fn emits_single_final_chunk_without_duplicates() {
        let chunks = vec![
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"calc\",\"arguments\":\"{\\\"x\\\":1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n",
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
        // 终止块只发射一次，functionCall 不重复
        assert_eq!(output.matches("\"finishReason\":\"STOP\"").count(), 1);
        assert_eq!(output.matches("\"functionCall\"").count(), 1);
    }

    #[tokio::test]
    async fn maps_length_finish_reason_at_done() {
        let chunks = vec![
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            )),
            Ok(Bytes::from_static(b"data: [DONE]\n\n")),
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
        assert!(output.contains("\"finishReason\":\"STOP\""));
    }

    #[tokio::test]
    async fn maps_length_to_max_tokens() {
        let chunks = vec![Ok(Bytes::from_static(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"length\"}]}\n\n",
        ))];
        let output = OpenaiToGeminiStream::new(stream::iter(chunks), "gemini-test".to_string())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(String::new(), |mut output, chunk| {
                output.push_str(&String::from_utf8_lossy(&chunk));
                output
            });
        assert!(output.contains("\"finishReason\":\"MAX_TOKENS\""));
    }
}
