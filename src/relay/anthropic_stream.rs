use std::{
    collections::{BTreeMap, VecDeque},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::Stream;
use serde_json::{Value, json};

use super::sse_event_end;

#[derive(Clone, Copy)]
pub enum OpenaiStreamFormat {
    ChatCompletions,
    Responses,
}

#[derive(Default)]
struct ToolState {
    arguments: String,
    block_index: usize,
}

/// Adapts Chat Completions or Responses SSE into Anthropic Messages SSE.
pub struct OpenaiToAnthropicStream<S> {
    inner: S,
    format: OpenaiStreamFormat,
    buffer: Vec<u8>,
    pending: VecDeque<Bytes>,
    model: String,
    message_id: String,
    text_started: bool,
    next_block_index: usize,
    tools: BTreeMap<String, ToolState>,
    completed: bool,
}

impl<S> OpenaiToAnthropicStream<S> {
    pub fn new(inner: S, model: String, format: OpenaiStreamFormat) -> Self {
        let mut stream = Self {
            inner,
            format,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            model,
            message_id: format!("msg_{}", nanoid::nanoid!(24)),
            text_started: false,
            next_block_index: 0,
            tools: BTreeMap::new(),
            completed: false,
        };
        stream.push(
            "message_start",
            json!({
                "message": {
                    "id": stream.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": stream.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                }
            }),
        );
        stream
    }

    fn push(&mut self, event: &str, mut data: Value) {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("type".to_string(), Value::String(event.to_string()));
        }
        self.pending.push_back(Bytes::from(format!(
            "event: {event}\ndata: {}\n\n",
            serde_json::to_string(&data).unwrap_or_default()
        )));
    }

    fn push_text(&mut self, text: &str) {
        if !self.text_started {
            self.text_started = true;
            self.next_block_index = 1;
            self.push(
                "content_block_start",
                json!({ "index": 0, "content_block": { "type": "text", "text": "" } }),
            );
        }
        self.push(
            "content_block_delta",
            json!({ "index": 0, "delta": { "type": "text_delta", "text": text } }),
        );
    }

    fn start_tool(&mut self, key: String, id: String, name: String) {
        if self.tools.contains_key(&key) {
            return;
        }
        let block_index = self.next_block_index;
        self.tools.insert(key.clone(), ToolState {
            arguments: String::new(),
            block_index,
        });
        self.push(
            "content_block_start",
            json!({
                "index": block_index,
                "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} }
            }),
        );
    }

    fn push_tool_arguments(&mut self, key: &str, arguments: &str) {
        let Some(tool) = self.tools.get_mut(key) else {
            return;
        };
        tool.arguments.push_str(arguments);
        let block_index = tool.block_index;
        self.push(
            "content_block_delta",
            json!({
                "index": block_index,
                "delta": { "type": "input_json_delta", "partial_json": arguments }
            }),
        );
    }

    fn handle_chat(&mut self, data: &str) {
        if data == "[DONE]" {
            self.complete(None);
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let Some(choice) = value.pointer("/choices/0") else {
            return;
        };
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            self.push_text(text);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let key = index.to_string();
                if !self.tools.contains_key(&key) {
                    let id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("toolu_{}", nanoid::nanoid!(20)));
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    self.start_tool(key.clone(), id, name);
                }
                if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    self.push_tool_arguments(&key, arguments);
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.complete(Some(reason));
        }
    }

    fn handle_responses(&mut self, event: &str, data: &str) {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        match event {
            "response.output_text.delta" => {
                if let Some(text) = value.get("delta").and_then(Value::as_str) {
                    self.push_text(text);
                }
            }
            "response.output_item.added" => {
                let Some(item) = value.get("item") else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let key = item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("0")
                        .to_string();
                    let id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or(&key)
                        .to_string();
                    let name = item.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    self.start_tool(key, id, name);
                }
            }
            "response.function_call_arguments.delta" => {
                if let (Some(key), Some(arguments)) = (
                    value.get("item_id").and_then(Value::as_str),
                    value.get("delta").and_then(Value::as_str),
                ) {
                    self.push_tool_arguments(key, arguments);
                }
            }
            "response.completed" => self.complete(None),
            _ => {}
        }
    }

    fn handle_event(&mut self, event: &str) {
        let event_type = event
            .lines()
            .find_map(|line| line.trim().strip_prefix("event:").map(str::trim))
            .unwrap_or("");
        let data = event
            .lines()
            .find_map(|line| line.trim().strip_prefix("data:").map(str::trim));
        let Some(data) = data else {
            return;
        };
        match self.format {
            OpenaiStreamFormat::ChatCompletions => self.handle_chat(data),
            OpenaiStreamFormat::Responses => self.handle_responses(event_type, data),
        }
    }

    fn complete(&mut self, finish_reason: Option<&str>) {
        if self.completed {
            return;
        }
        self.completed = true;
        if self.text_started {
            self.push("content_block_stop", json!({ "index": 0 }));
        }
        let tool_indexes: Vec<usize> = self.tools.values().map(|tool| tool.block_index).collect();
        for index in tool_indexes {
            self.push("content_block_stop", json!({ "index": index }));
        }
        let stop_reason = if !self.tools.is_empty() || finish_reason == Some("tool_calls") {
            "tool_use"
        } else if finish_reason == Some("length") {
            "max_tokens"
        } else {
            "end_turn"
        };
        self.push(
            "message_delta",
            json!({ "delta": { "stop_reason": stop_reason, "stop_sequence": null }, "usage": { "output_tokens": 0 } }),
        );
        self.push("message_stop", json!({}));
    }
}

impl<S> Stream for OpenaiToAnthropicStream<S>
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
                    this.handle_event(&event);
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
    async fn converts_chat_completion_text_to_anthropic_events() {
        let chunks = vec![
            Ok(Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n")),
            Ok(Bytes::from_static(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")),
        ];
        let output = OpenaiToAnthropicStream::new(
            stream::iter(chunks),
            "test-model".to_string(),
            OpenaiStreamFormat::ChatCompletions,
        )
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .fold(String::new(), |mut output, chunk| {
            output.push_str(&String::from_utf8_lossy(&chunk));
            output
        });

        assert!(output.contains("event: message_start"));
        assert!(output.contains("event: content_block_delta"));
        assert!(output.contains("\"text\":\"Hello\""));
        assert_eq!(output.matches("event: message_stop").count(), 1);
    }
}
