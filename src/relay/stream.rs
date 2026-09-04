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

/// 将单个供应商原生 SSE 事件转换为 OpenAI Chat Completion SSE chunk 字符串。
pub type SseChunkConverter = fn(&str, &str, i64, &str) -> Option<String>;

/// 将供应商原生 SSE 字节流转换为标准 OpenAI Chat Completion SSE chunk 流。
pub struct ProviderToOpenaiStream<S> {
    inner:          S,
    buffer:         Vec<u8>,
    pending_chunks: VecDeque<Bytes>,
    model:          String,
    chatcmpl_id:    String,
    done_emitted:   bool,
    convert_event:  SseChunkConverter,
}

impl<S> ProviderToOpenaiStream<S> {
    pub fn new(inner: S, model: String, convert_event: SseChunkConverter) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            pending_chunks: VecDeque::new(),
            model,
            chatcmpl_id: format!("chatcmpl-{}", nanoid::nanoid!(24)),
            done_emitted: false,
            convert_event,
        }
    }
}

impl<S> Stream for ProviderToOpenaiStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();

        if let Some(chunk) = this.pending_chunks.pop_front() {
            if chunk.as_ref() == b"data: [DONE]\n\n" {
                this.done_emitted = true;
            }
            return Poll::Ready(Some(Ok(chunk)));
        }

        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.buffer.extend_from_slice(&bytes);
                let created = chrono::Utc::now().timestamp();
                while let Some(end) = sse_event_end(&this.buffer) {
                    let event_str = String::from_utf8_lossy(&this.buffer[..end]).to_string();
                    if let Some(chunk) = (this.convert_event)(&event_str, &this.model, created, &this.chatcmpl_id) {
                        this.pending_chunks.push_back(Bytes::from(chunk));
                    }
                    this.buffer.drain(..end);
                }

                if let Some(chunk) = this.pending_chunks.pop_front() {
                    if chunk.as_ref() == b"data: [DONE]\n\n" {
                        this.done_emitted = true;
                    }
                    Poll::Ready(Some(Ok(chunk)))
                } else {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                if !this.done_emitted {
                    this.done_emitted = true;
                    Poll::Ready(Some(Ok(Bytes::from_static(b"data: [DONE]\n\n"))))
                } else {
                    Poll::Ready(None)
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Default)]
struct ToolCallState {
    item_id:      String,
    call_id:      String,
    name:         String,
    arguments:    String,
    output_index: usize,
}

/// 将 OpenAI Chat Completions SSE 流适配转换为 OpenAI Responses SSE 流。
pub struct ChatCompletionsToResponsesStream<S> {
    inner:           S,
    buffer:          Vec<u8>,
    pending:         VecDeque<Bytes>,
    response_id:     String,
    message_id:      String,
    model:           String,
    created_at:      i64,
    sequence:        u64,
    started:         bool,
    message_started: bool,
    completed:       bool,
    text:            String,
    tools:           BTreeMap<usize, ToolCallState>,
    usage:           Option<Usage>,
}

impl<S> ChatCompletionsToResponsesStream<S> {
    pub fn new(inner: S, model: String) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            pending: VecDeque::new(),
            response_id: format!("resp_{}", nanoid::nanoid!(24)),
            message_id: format!("msg_{}", nanoid::nanoid!(24)),
            model,
            created_at: chrono::Utc::now().timestamp(),
            sequence: 0,
            started: false,
            message_started: false,
            completed: false,
            text: String::new(),
            tools: BTreeMap::new(),
            usage: None,
        }
    }

    fn push_event(&mut self, event_type: &str, mut data: Value) {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("type".to_string(), Value::String(event_type.to_string()));
            obj.insert("sequence_number".to_string(), json!(self.sequence));
        }
        self.sequence += 1;
        self.pending.push_back(Bytes::from(format!(
            "event: {event_type}\ndata: {}\n\n",
            serde_json::to_string(&data).unwrap_or_default()
        )));
    }

    fn start_response(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.push_event(
            "response.created",
            json!({
                "response": {
                    "id": self.response_id,
                    "object": "response",
                    "created_at": self.created_at,
                    "completed_at": null,
                    "status": "in_progress",
                    "model": self.model,
                    "output": [],
                    "error": null,
                    "incomplete_details": null,
                    "usage": null
                }
            }),
        );
    }

    fn start_message(&mut self) {
        if self.message_started {
            return;
        }
        self.message_started = true;
        self.push_event(
            "response.output_item.added",
            json!({
                "output_index": 0,
                "item": {
                    "id": self.message_id,
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": []
                }
            }),
        );
        self.push_event(
            "response.content_part.added",
            json!({
                "item_id": self.message_id,
                "output_index": 0,
                "content_index": 0,
                "part": { "type": "output_text", "text": "", "annotations": [], "logprobs": [] }
            }),
        );
    }

    fn handle_data(&mut self, data: &str) {
        if data == "[DONE]" {
            self.complete();
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        // 将提取到的真实 Token 用量记录到 self.usage，供 response.completed 事件发送真实用量
        if let Some(u) = extract_usage_from_json(&value) {
            self.usage = Some(u);
        }
        let Some(choice) = value.pointer("/choices/0") else {
            return;
        };
        let delta = choice.get("delta").unwrap_or(&Value::Null);

        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            self.start_message();
            self.text.push_str(text);
            self.push_event(
                "response.output_text.delta",
                json!({
                    "item_id": self.message_id,
                    "output_index": 0,
                    "content_index": 0,
                    "delta": text,
                    "logprobs": []
                }),
            );
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let is_new = !self.tools.contains_key(&index);
                if is_new {
                    let call_id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{}", nanoid::nanoid!(20)));
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let item_id = format!("fc_{}", nanoid::nanoid!(20));
                    let output_index = usize::from(self.message_started) + index;
                    self.tools.insert(
                        index,
                        ToolCallState {
                            item_id: item_id.clone(),
                            call_id: call_id.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                            output_index,
                        },
                    );
                    self.push_event(
                        "response.output_item.added",
                        json!({
                            "output_index": output_index,
                            "item": {
                                "id": item_id,
                                "type": "function_call",
                                "status": "in_progress",
                                "call_id": call_id,
                                "name": name,
                                "arguments": ""
                            }
                        }),
                    );
                }

                let arguments = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !arguments.is_empty() {
                    let Some(tool) = self.tools.get_mut(&index) else {
                        continue;
                    };
                    tool.arguments.push_str(arguments);
                    let item_id = tool.item_id.clone();
                    let output_index = tool.output_index;
                    self.push_event(
                        "response.function_call_arguments.delta",
                        json!({
                            "item_id": item_id,
                            "output_index": output_index,
                            "delta": arguments
                        }),
                    );
                }
            }
        }

        if choice
            .get("finish_reason")
            .is_some_and(|reason| !reason.is_null())
        {
            self.complete();
        }
    }

    fn complete(&mut self) {
        if self.completed {
            return;
        }
        self.completed = true;
        let mut output = Vec::new();

        if self.message_started {
            self.push_event(
                "response.output_text.done",
                json!({
                    "item_id": self.message_id,
                    "output_index": 0,
                    "content_index": 0,
                    "text": self.text,
                    "logprobs": []
                }),
            );
            self.push_event(
                "response.content_part.done",
                json!({
                    "item_id": self.message_id,
                    "output_index": 0,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": self.text, "annotations": [], "logprobs": [] }
                }),
            );
            let message = json!({
                "id": self.message_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": self.text, "annotations": [], "logprobs": [] }]
            });
            self.push_event(
                "response.output_item.done",
                json!({ "output_index": 0, "item": message }),
            );
            output.push(message);
        }

        let tool_indexes: Vec<usize> = self.tools.keys().copied().collect();
        for index in tool_indexes {
            let Some(tool) = self.tools.remove(&index) else {
                continue;
            };
            self.push_event(
                "response.function_call_arguments.done",
                json!({
                    "item_id": tool.item_id,
                    "output_index": tool.output_index,
                    "arguments": tool.arguments
                }),
            );
            let item = json!({
                "id": tool.item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": tool.call_id,
                "name": tool.name,
                "arguments": tool.arguments
            });
            self.push_event(
                "response.output_item.done",
                json!({ "output_index": tool.output_index, "item": item }),
            );
            output.push(item);
        }

        self.push_event(
            "response.completed",
            json!({
                "response": {
                    "id": self.response_id,
                    "object": "response",
                    "created_at": self.created_at,
                    "completed_at": chrono::Utc::now().timestamp(),
                    "status": "completed",
                    "model": self.model,
                    "output": output,
                    "error": null,
                    "incomplete_details": null,
                    "usage": self
                        .usage
                        .as_ref()
                        .map(|u| {
                            json!({
                                "input_tokens": u.prompt_tokens,
                                "output_tokens": u.completion_tokens,
                                "total_tokens": u.total_tokens
                            })
                        })
                        .unwrap_or_else(|| json!({
                            "input_tokens": 0,
                            "output_tokens": 0,
                            "total_tokens": 0
                        }))
                }
            }),
        );
    }
}

impl<S> Stream for ChatCompletionsToResponsesStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        this.start_response();
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
                this.complete();
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
    async fn converts_chat_completion_sse_to_responses_events() {
        let chunks = vec![
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            )),
        ];
        let output = ChatCompletionsToResponsesStream::new(stream::iter(chunks), "test-model".to_string())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(String::new(), |mut output, chunk| {
                output.push_str(&String::from_utf8_lossy(&chunk));
                output
            });

        assert!(output.contains("event: response.created"));
        assert!(output.contains("event: response.output_text.delta"));
        assert!(output.contains("\"delta\":\"Hello\""));
        assert_eq!(output.matches("event: response.completed").count(), 1);
    }
}
