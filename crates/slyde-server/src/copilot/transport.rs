//! Streaming transports for the two wire protocols the copilot supports.
//!
//! Seven providers expose an OpenAI-compatible Chat Completions endpoint;
//! Anthropic keeps its own Messages protocol. Both are normalized into a
//! single `Turn`, so the tool loop never branches on the vendor.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use super::catalog::{ProviderDef, Wire};
use super::error::CopilotError;
use super::tools;
use super::wire::{ChatEvent, ImagePart, ToolCall, Turn};

/// One streaming turn: everything the transport needs except the event sink.
pub struct StreamRequest<'a> {
    pub provider: &'static ProviderDef,
    pub key: &'a str,
    pub model: &'a str,
    pub system: &'a str,
    pub turns: &'a [Turn],
    /// Flipped by `POST /api/copilot/cancel`; checked per chunk so cancelling
    /// drops the upstream response instead of waiting for it to finish.
    pub cancel: &'a tokio::sync::watch::Receiver<bool>,
}

pub async fn stream(
    client: &reqwest::Client,
    request: &StreamRequest<'_>,
    tx: &UnboundedSender<ChatEvent>,
) -> Result<Turn, CopilotError> {
    match request.provider.wire {
        Wire::OpenAi => stream_openai(client, request, tx).await,
        Wire::Anthropic => stream_anthropic(client, request, tx).await,
    }
}

async fn post(
    client: &reqwest::Client,
    provider: &ProviderDef,
    key: &str,
    body: &Value,
) -> Result<reqwest::Response, CopilotError> {
    let mut request = client
        .post(provider.chat_url())
        .header("accept", "text/event-stream")
        .json(body)
        .timeout(Duration::from_secs(300));

    if provider.wire == Wire::Anthropic {
        request = request
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01");
    } else {
        request = request.bearer_auth(key);
    }
    if provider.id == "openrouter" {
        request = request.header("X-Title", "SlydeCode");
    }

    let response = request
        .send()
        .await
        .map_err(|error| CopilotError::from_network(&error))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(CopilotError::from_status(status, &body));
    }
    Ok(response)
}

async fn stream_openai(
    client: &reqwest::Client,
    request: &StreamRequest<'_>,
    tx: &UnboundedSender<ChatEvent>,
) -> Result<Turn, CopilotError> {
    let StreamRequest {
        provider,
        key,
        model,
        system,
        turns,
        cancel,
    } = *request;
    let body = json!({
        "model": model,
        "messages": openai_messages(system, turns),
        "tools": tools::openai_tools(),
        "tool_choice": "auto",
        "stream": true
    });
    let response = post(client, provider, key, &body).await?;
    let mut state = OpenAiAccum::default();

    consume_sse(response, tx, cancel, |data| {
        if data == "[DONE]" {
            return Ok(());
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => return Ok(()),
        };
        if let Some(error) = provider_error(&value) {
            return Err(CopilotError::Upstream(error));
        }
        if let Some(text) = state.apply(&value)? {
            send_delta(tx, &text)?;
        }
        if let Some(thinking) = state.take_thinking() {
            send_thinking(tx, &thinking)?;
        }
        Ok(())
    })
    .await?;

    let tool_calls = state.finish()?;
    let text = if state.text.trim().is_empty()
        && !state.reasoning.trim().is_empty()
        && tool_calls.is_empty()
    {
        state.reasoning
    } else {
        state.text
    };
    Ok(Turn::Assistant { text, tool_calls })
}

async fn stream_anthropic(
    client: &reqwest::Client,
    request: &StreamRequest<'_>,
    tx: &UnboundedSender<ChatEvent>,
) -> Result<Turn, CopilotError> {
    let StreamRequest {
        provider,
        key,
        model,
        system,
        turns,
        cancel,
    } = *request;
    let body = json!({
        "model": model,
        "system": system,
        "messages": anthropic_messages(turns),
        "tools": tools::anthropic_tools(),
        "max_tokens": 8192,
        "stream": true
    });
    let response = post(client, provider, key, &body).await?;
    let mut state = AnthropicAccum::default();

    consume_sse(response, tx, cancel, |data| {
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => return Ok(()),
        };
        if let Some(text) = state.apply(&value)? {
            send_delta(tx, &text)?;
        }
        if let Some(thinking) = state.take_thinking() {
            send_thinking(tx, &thinking)?;
        }
        Ok(())
    })
    .await?;

    let tool_calls = state.finish();
    let text = if state.text.trim().is_empty()
        && !state.reasoning.trim().is_empty()
        && tool_calls.is_empty()
    {
        state.reasoning
    } else {
        state.text
    };
    Ok(Turn::Assistant { text, tool_calls })
}

async fn consume_sse(
    response: reqwest::Response,
    tx: &UnboundedSender<ChatEvent>,
    cancel: &tokio::sync::watch::Receiver<bool>,
    mut on_data: impl FnMut(&str) -> Result<(), CopilotError>,
) -> Result<(), CopilotError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        // Dropping the response here is what actually aborts the provider call.
        if tx.is_closed() || *cancel.borrow() {
            return Err(CopilotError::Cancelled);
        }
        let bytes = chunk.map_err(|error| CopilotError::from_network(&error))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(index) = buffer.find('\n') {
            let line = buffer[..index].trim_end_matches('\r').to_string();
            buffer.drain(..=index);
            if let Some(data) = line.strip_prefix("data:") {
                on_data(data.trim())?;
            }
        }
    }
    if let Some(data) = buffer.trim().strip_prefix("data:") {
        on_data(data.trim())?;
    }
    Ok(())
}

fn send_delta(tx: &UnboundedSender<ChatEvent>, text: &str) -> Result<(), CopilotError> {
    if text.is_empty() {
        return Ok(());
    }
    if tx.is_closed() {
        return Err(CopilotError::Cancelled);
    }
    tx.send(ChatEvent::Delta {
        text: text.to_string(),
    })
    .map_err(|_| CopilotError::Cancelled)
}

fn send_thinking(tx: &UnboundedSender<ChatEvent>, text: &str) -> Result<(), CopilotError> {
    if text.is_empty() {
        return Ok(());
    }
    if tx.is_closed() {
        return Err(CopilotError::Cancelled);
    }
    tx.send(ChatEvent::Thinking {
        text: text.to_string(),
    })
    .map_err(|_| CopilotError::Cancelled)
}

fn provider_error(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(text) = error.as_str() {
        return Some(text.to_string());
    }
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn openai_messages(system: &str, turns: &[Turn]) -> Vec<Value> {
    let mut messages = vec![json!({ "role": "system", "content": system })];
    for turn in turns {
        match turn {
            Turn::User { text, images } => {
                if images.is_empty() {
                    messages.push(json!({ "role": "user", "content": text }));
                } else {
                    let mut parts = vec![json!({ "type": "text", "text": text })];
                    for image in images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", image.mime, image.data)
                            }
                        }));
                    }
                    messages.push(json!({ "role": "user", "content": parts }));
                }
            }
            Turn::Assistant { text, tool_calls } => {
                let mut message = json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { Value::String(text.clone()) }
                });
                if !tool_calls.is_empty() {
                    message["tool_calls"] = Value::Array(
                        tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": {
                                        "name": call.name,
                                        "arguments": call.arguments.to_string()
                                    }
                                })
                            })
                            .collect(),
                    );
                }
                messages.push(message);
            }
            Turn::Tool {
                call_id, content, ..
            } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content
                }));
            }
        }
    }
    messages
}

fn anthropic_messages(turns: &[Turn]) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut index = 0;
    while index < turns.len() {
        match &turns[index] {
            Turn::User { text, images } => {
                let mut content = vec![json!({ "type": "text", "text": text })];
                for image in images {
                    content.push(anthropic_image(image));
                }
                messages.push(json!({ "role": "user", "content": content }));
                index += 1;
            }
            Turn::Assistant { text, tool_calls } => {
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for call in tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments
                    }));
                }
                if content.is_empty() {
                    content.push(json!({ "type": "text", "text": "…" }));
                }
                messages.push(json!({ "role": "assistant", "content": content }));
                index += 1;
            }
            Turn::Tool { .. } => {
                // Anthropic requires every tool_result for one assistant turn
                // to arrive in the same user message.
                let mut content = Vec::new();
                while index < turns.len() {
                    let Turn::Tool {
                        call_id,
                        content: body,
                        ..
                    } = &turns[index]
                    else {
                        break;
                    };
                    content.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": body
                    }));
                    index += 1;
                }
                messages.push(json!({ "role": "user", "content": content }));
            }
        }
    }
    messages
}

fn anthropic_image(image: &ImagePart) -> Value {
    json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": image.mime,
            "data": image.data
        }
    })
}

#[derive(Debug, Default)]
struct OpenAiAccum {
    text: String,
    reasoning: String,
    emitted_thinking: String,
    calls: Vec<OpenAiCall>,
}

#[derive(Debug, Default)]
struct OpenAiCall {
    id: String,
    name: String,
    arguments: String,
}

impl OpenAiAccum {
    fn take_thinking(&mut self) -> Option<String> {
        if self.emitted_thinking.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.emitted_thinking))
        }
    }

    fn apply(&mut self, value: &Value) -> Result<Option<String>, CopilotError> {
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            return Ok(None);
        };
        let delta = choice.get("delta").or_else(|| choice.get("message"));
        let mut emitted = String::new();
        if let Some(content) = delta.and_then(|delta| delta.get("content")) {
            let text = content_text(content);
            if !text.is_empty() {
                self.text.push_str(&text);
                emitted.push_str(&text);
            }
        }
        if let Some(reasoning) = delta
            .and_then(|delta| {
                delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
            })
            .and_then(Value::as_str)
        {
            if !reasoning.is_empty() {
                self.reasoning.push_str(reasoning);
                self.emitted_thinking.push_str(reasoning);
            }
        }
        if let Some(calls) = delta
            .and_then(|delta| delta.get("tool_calls"))
            .and_then(Value::as_array)
        {
            for call in calls {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.calls.len() as u64) as usize;
                while self.calls.len() <= index {
                    self.calls.push(OpenAiCall::default());
                }
                let slot = &mut self.calls[index];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    if !id.is_empty() {
                        slot.id = id.to_string();
                    }
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        if !name.is_empty() {
                            slot.name = name.to_string();
                        }
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        slot.arguments.push_str(arguments);
                    }
                }
            }
        }
        Ok(Some(emitted).filter(|text| !text.is_empty()))
    }

    fn finish(&mut self) -> Result<Vec<ToolCall>, CopilotError> {
        let mut calls = Vec::new();
        for (index, call) in std::mem::take(&mut self.calls).into_iter().enumerate() {
            if call.name.is_empty() {
                continue;
            }
            let arguments = parse_arguments(&call.arguments)?;
            calls.push(ToolCall {
                id: if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                },
                name: call.name,
                arguments,
            });
        }
        Ok(calls)
    }
}

#[derive(Debug, Default)]
struct AnthropicAccum {
    text: String,
    reasoning: String,
    emitted_thinking: String,
    partials: HashMap<usize, PartialTool>,
    finished: Vec<ToolCall>,
}

#[derive(Debug, Default)]
struct PartialTool {
    id: String,
    name: String,
    arguments: String,
}

impl AnthropicAccum {
    fn take_thinking(&mut self) -> Option<String> {
        if self.emitted_thinking.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.emitted_thinking))
        }
    }

    fn apply(&mut self, value: &Value) -> Result<Option<String>, CopilotError> {
        match value.get("type").and_then(Value::as_str) {
            Some("error") => {
                let message = provider_error(value)
                    .unwrap_or_else(|| "the provider rejected the request".into());
                Err(CopilotError::Upstream(message))
            }
            Some("content_block_start") => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = value.get("content_block").cloned().unwrap_or(Value::Null);
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    self.partials.insert(
                        index,
                        PartialTool {
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            name: block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            arguments: String::new(),
                        },
                    );
                }
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                    if !text.is_empty() {
                        self.text.push_str(text);
                        return Ok(Some(text.to_string()));
                    }
                }
                Ok(None)
            }
            Some("content_block_delta") => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = value.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                        if !text.is_empty() {
                            self.text.push_str(text);
                            return Ok(Some(text.to_string()));
                        }
                    }
                    Some("thinking_delta") => {
                        let text = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                        if !text.is_empty() {
                            self.reasoning.push_str(text);
                            self.emitted_thinking.push_str(text);
                        }
                    }
                    Some("input_json_delta") => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if let Some(tool) = self.partials.get_mut(&index) {
                            tool.arguments.push_str(partial);
                        }
                    }
                    _ => {}
                }
                Ok(None)
            }
            Some("content_block_stop") => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if let Some(tool) = self.partials.remove(&index) {
                    if !tool.name.is_empty() {
                        self.finished.push(ToolCall {
                            id: if tool.id.is_empty() {
                                format!("tool_{index}")
                            } else {
                                tool.id
                            },
                            name: tool.name,
                            arguments: parse_arguments(&tool.arguments)?,
                        });
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn finish(&mut self) -> Vec<ToolCall> {
        for (index, tool) in self.partials.drain() {
            if !tool.name.is_empty() {
                self.finished.push(ToolCall {
                    id: if tool.id.is_empty() {
                        format!("tool_{index}")
                    } else {
                        tool.id
                    },
                    name: tool.name,
                    arguments: parse_arguments(&tool.arguments).unwrap_or_else(|_| json!({})),
                });
            }
        }
        std::mem::take(&mut self.finished)
    }
}

fn parse_arguments(raw: &str) -> Result<Value, CopilotError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed)
        .map_err(|_| CopilotError::Upstream("the model returned malformed tool arguments".into()))
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_stream_accumulates_text_and_partial_tools() {
        let mut state = OpenAiAccum::default();
        let first = json!({
            "choices": [{ "delta": { "content": "Hello", "tool_calls": [
                { "index": 0, "id": "call_1", "function": { "name": "upsert_slide", "arguments": "{\"index\":" } }
            ] } }]
        });
        let second = json!({
            "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "function": { "arguments": "0}" } }
            ] } }]
        });
        assert_eq!(state.apply(&first).unwrap().as_deref(), Some("Hello"));
        assert!(state.apply(&second).unwrap().is_none());
        let calls = state.finish().unwrap();
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "upsert_slide");
        assert_eq!(calls[0].arguments["index"], 0);
    }

    #[test]
    fn openai_stream_captures_reasoning_content() {
        let mut state = OpenAiAccum::default();
        let first = json!({
            "choices": [{ "delta": { "reasoning_content": "Pensando..." } }]
        });
        let second = json!({
            "choices": [{ "delta": { "content": "Listo" } }]
        });
        assert!(state.apply(&first).unwrap().is_none());
        assert_eq!(state.take_thinking().as_deref(), Some("Pensando..."));
        assert_eq!(state.apply(&second).unwrap().as_deref(), Some("Listo"));
        assert!(state.take_thinking().is_none());
    }

    #[test]
    fn openai_messages_attach_images_as_data_urls() {
        let turns = vec![Turn::User {
            text: "describe".into(),
            images: vec![ImagePart {
                mime: "image/png".into(),
                data: "abc".into(),
            }],
        }];
        let messages = openai_messages("system", &turns);
        let content = &messages[1]["content"];
        assert_eq!(content[1]["type"], "image_url");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn anthropic_messages_use_tool_blocks() {
        let turns = vec![
            Turn::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "tool_1".into(),
                    name: "read_deck".into(),
                    arguments: json!({}),
                }],
            },
            Turn::Tool {
                call_id: "tool_1".into(),
                name: "read_deck".into(),
                content: "{}".into(),
            },
        ];
        let messages = anthropic_messages(&turns);
        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn parallel_anthropic_tool_results_share_one_user_turn() {
        let turns = vec![
            Turn::Assistant {
                text: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "tool_1".into(),
                        name: "read_deck".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "tool_2".into(),
                        name: "read_slide".into(),
                        arguments: json!({ "slide_id": "sld_a" }),
                    },
                ],
            },
            Turn::Tool {
                call_id: "tool_1".into(),
                name: "read_deck".into(),
                content: "{}".into(),
            },
            Turn::Tool {
                call_id: "tool_2".into(),
                name: "read_slide".into(),
                content: "{}".into(),
            },
        ];
        let messages = anthropic_messages(&turns);
        assert_eq!(messages.len(), 2);
        let results = messages[1]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool_use_id"], "tool_1");
        assert_eq!(results[1]["tool_use_id"], "tool_2");
    }

    #[test]
    fn anthropic_stream_builds_tool_calls_from_json_deltas() {
        let mut state = AnthropicAccum::default();
        state
            .apply(&json!({
                "type": "content_block_start",
                "index": 1,
                "content_block": { "type": "tool_use", "id": "tool_1", "name": "set_notes", "input": {} }
            }))
            .unwrap();
        state
            .apply(&json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": { "type": "input_json_delta", "partial_json": "{\"slide_id\":\"sld_a\"}" }
            }))
            .unwrap();
        state
            .apply(&json!({ "type": "content_block_stop", "index": 1 }))
            .unwrap();
        let calls = state.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "set_notes");
        assert_eq!(calls[0].arguments["slide_id"], "sld_a");
    }
}
