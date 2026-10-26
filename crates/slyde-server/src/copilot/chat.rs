//! The copilot tool loop.
//!
//! A request becomes a short conversation with the provider. Text deltas are
//! forwarded immediately; tool calls are executed against the live deck and
//! fed back to the model. The browser never receives provider JSON.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::state::{AgentAction, AgentPhase, AppState, ServerEvent};

use super::attachments::{self, AttachmentContent, AttachmentInput};
use super::catalog;
use super::error::CopilotError;
use super::prompt;
use super::tools;
use super::transport;
use super::wire::{ChatEvent, ImagePart, TokenUsage, Turn};

const MAX_MESSAGES: usize = 40;
const MAX_TEXT_CHARS: usize = 80_000;
const MAX_TOTAL_ATTACHMENTS: usize = 8;
/// Rounds the model may spend calling tools. A deck of a dozen slides plus a
/// couple of reads fits comfortably; the loop also stops on its own when the
/// model stops making progress.
const MAX_TOOL_ROUNDS: usize = 40;

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessageInput {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<AttachmentInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatMessageInput>,
    #[serde(default)]
    pub selected_slide_id: Option<String>,
    #[serde(default)]
    pub selected_element_id: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    /// Opaque id for this turn. When present, `POST /api/copilot/cancel` with the
    /// same id stops the provider call and the tool rounds it would still run.
    #[serde(default)]
    pub request_id: Option<String>,
}

pub async fn run(state: Arc<AppState>, request: ChatRequest, tx: UnboundedSender<ChatEvent>) {
    let provider_id = request.provider.clone();
    let model = request.model.clone();
    let request_id = request.request_id.clone();
    let _ = tx.send(ChatEvent::Meta {
        provider: provider_id.clone(),
        model: model.clone(),
    });

    let (idle_tx, idle_rx) = tokio::sync::watch::channel(false);
    let cancel = match &request_id {
        Some(id) => state.copilot.begin_turn(id),
        None => idle_rx,
    };
    drop(idle_tx);

    let before = state.view().await;
    state.copilot.begin_snapshot(before.presentation.clone());
    let request_messages = request.messages.clone();
    let result = run_inner(&state, request, &tx, cancel).await;
    let after = state.view().await;
    let reversible = state
        .copilot
        .finish_snapshot(before.revision, after.revision);
    if reversible {
        let _ = tx.send(ChatEvent::Reversible);
        // Keep the deck as it was before the copilot touched it: a checkpoint
        // that outlives the session, unlike the single in-memory undo.
        let label = last_user_text(&request_messages);
        let _ = state.versions.save(
            &state.deck_key(),
            &format!("Before copilot: {label}"),
            true,
            &before.presentation,
        );
    }
    if let Some(id) = &request_id {
        state.copilot.end_turn(id);
    }
    match result {
        Ok(()) => {
            let _ = tx.send(ChatEvent::Done);
            state.broadcast(activity(AgentPhase::Done));
        }
        Err(CopilotError::Cancelled) => {
            state.broadcast(activity(AgentPhase::Done));
        }
        Err(error) => {
            let _ = tx.send(ChatEvent::Error {
                code: error.code().to_string(),
                message: error.public_message(),
            });
            state.broadcast(activity(AgentPhase::Error));
        }
    }
}

async fn run_inner(
    state: &Arc<AppState>,
    request: ChatRequest,
    tx: &UnboundedSender<ChatEvent>,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<(), CopilotError> {
    let provider = catalog::find(&request.provider)
        .ok_or_else(|| CopilotError::InvalidRequest("unknown provider".into()))?;
    if request.model.trim().is_empty() {
        return Err(CopilotError::InvalidRequest("choose a model first".into()));
    }
    let key = state
        .copilot
        .key_for(provider)
        .await
        .ok_or(CopilotError::MissingKey)?;
    let mut turns = build_turns(&request.messages)?;
    if turns.is_empty() {
        return Err(CopilotError::InvalidRequest("write a message first".into()));
    }
    if !matches!(turns.last(), Some(Turn::User { .. })) {
        return Err(CopilotError::InvalidRequest(
            "the last message must come from the user".into(),
        ));
    }

    state.copilot.remember(provider.id, &request.model).await;
    state.broadcast(activity(AgentPhase::Thinking));

    let language = request.language.as_deref().unwrap_or("en");

    let mut input_chars: u64 = 0;
    let mut reported = TokenUsage::default();
    let mut all_reported = true;
    let mut last_signature: Option<String> = None;
    let mut repeats: u32 = 0;

    for round in 0..MAX_TOOL_ROUNDS {
        if tx.is_closed() || crate::copilot::CopilotState::was_cancelled(&cancel) {
            return Err(CopilotError::Cancelled);
        }
        // Tools mutate the deck, so the prompt is rebuilt from the live state on
        // every round. A snapshot taken before the loop told the model nothing
        // had changed, so it re-created slides that already existed and burned
        // through the step budget.
        let view = state.view().await;
        let deck = &view.presentation;
        let selected = request
            .selected_slide_id
            .as_deref()
            .filter(|id| deck.slides.iter().any(|slide| &slide.id == id));
        let system = prompt::system_prompt(
            deck,
            selected,
            request.selected_element_id.as_deref(),
            language,
        );
        compact_history(&mut turns);
        input_chars += (system.len() + turns.iter().map(Turn::chars).sum::<usize>()) as u64;
        // Slides the model finishes while it is still writing the call are
        // applied right away, so a long generation fills the storyboard slide by
        // slide instead of showing nothing until the very end.
        let (partial_tx, mut partial_rx) =
            tokio::sync::mpsc::unbounded_channel::<transport::PartialSlides>();
        let applier_state = state.clone();
        let applier = tokio::spawn(async move {
            let mut applied = tools::Applied::default();
            let mut tool = String::new();
            while let Some(batch) = partial_rx.recv().await {
                tool = batch.tool.clone();
                tools::apply_partial(
                    &applier_state,
                    &batch.tool,
                    batch.header.as_ref(),
                    &batch.slides,
                    &mut applied,
                )
                .await;
            }
            (tool, applied)
        });
        let streamed = transport::stream(
            state.copilot.http(),
            &transport::StreamRequest {
                provider,
                key: &key,
                model: request.model.trim(),
                system: &system,
                turns: &turns,
                cancel: &cancel,
                partial: Some(&partial_tx),
            },
            tx,
        )
        .await;
        drop(partial_tx);
        let (applied_tool, applied) = applier.await.unwrap_or_default();
        let (assistant, round_usage) = streamed?;
        let mut applied =
            Some(applied).filter(|applied| applied.created || !applied.slides.is_empty());

        if round_usage.is_reported() {
            reported.input += round_usage.input;
            reported.output += round_usage.output;
        } else {
            all_reported = false;
        }

        let (text, calls) = match assistant {
            Turn::Assistant { text, tool_calls } => (text, tool_calls),
            _ => return Err(CopilotError::Upstream("unexpected provider turn".into())),
        };

        if calls.is_empty() {
            if text.trim().is_empty() {
                return Err(CopilotError::Upstream(
                    "the model returned an empty response".into(),
                ));
            }
            let _ = tx.send(if all_reported {
                ChatEvent::Usage {
                    calls: round as u32 + 1,
                    input_tokens: reported.input,
                    output_tokens: reported.output,
                    estimated: false,
                }
            } else {
                ChatEvent::Usage {
                    calls: round as u32 + 1,
                    // ~4 characters per token is close enough for a budget hint.
                    input_tokens: input_chars / 4,
                    output_tokens: 0,
                    estimated: true,
                }
            });
            return Ok(());
        }

        // A model that repeats the exact same call is stuck, not working. Stop
        // before spending the rest of the budget on it.
        let signature = calls
            .iter()
            .map(|call| format!("{}|{}", call.name, call.arguments))
            .collect::<Vec<_>>()
            .join(";");
        if last_signature.as_deref() == Some(signature.as_str()) {
            repeats += 1;
        } else {
            repeats = 0;
        }
        last_signature = Some(signature);
        if repeats >= 2 {
            return Err(CopilotError::ToolLimit(
                "the copilot repeated the same step three times and stopped; the changes it already made are saved"
                    .into(),
            ));
        }

        turns.push(Turn::Assistant {
            text: text.clone(),
            tool_calls: calls.clone(),
        });

        for call in calls {
            if tx.is_closed() {
                return Err(CopilotError::Cancelled);
            }
            let _ = tx.send(ChatEvent::Tool {
                name: call.name.clone(),
                status: "running".into(),
            });
            // Only the call that was applied while streaming skips its slides.
            let prefix = if call.name == applied_tool {
                applied.take()
            } else {
                None
            };
            let outcome =
                tools::execute_with(state, &call.name, call.arguments.clone(), prefix.as_ref())
                    .await;
            let (content, status) = match outcome {
                Ok(outcome) => (
                    serde_json::to_string(&outcome.result)
                        .unwrap_or_else(|_| "{\"ok\":true}".to_string()),
                    "done",
                ),
                Err(error) => (json!({ "ok": false, "error": error }).to_string(), "error"),
            };
            let _ = tx.send(ChatEvent::Tool {
                name: call.name.clone(),
                status: status.into(),
            });
            turns.push(Turn::Tool {
                call_id: call.id,
                name: call.name,
                content,
            });
        }
        // The model is between tool execution and its next visible token.
        state.broadcast(activity(AgentPhase::Thinking));
    }

    Err(CopilotError::ToolLimit(format!(
        "the copilot reached its limit of {MAX_TOOL_ROUNDS} steps and stopped; the changes it already made are saved"
    )))
}

/// The user's latest message, shortened for a version name.
fn last_user_text(messages: &[ChatMessageInput]) -> String {
    let text = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.text.trim())
        .unwrap_or("");
    let short: String = text.chars().take(48).collect();
    if text.chars().count() > 48 {
        format!("{short}…")
    } else {
        short
    }
}

/// Strings longer than this inside an already-executed tool call are replaced
/// by a stub when the history is resent.
const STUB_ABOVE_CHARS: usize = 400;

/// Every round resends the whole conversation, and the bulk of it is the slide
/// source the model wrote in earlier rounds. That code is already in the deck,
/// and the system prompt is rebuilt from the live deck each round, so resending
/// it only costs input tokens. Executed tool calls keep their shape (providers
/// require valid JSON arguments) but lose their large string payloads.
fn compact_history(turns: &mut [Turn]) {
    fn stub(value: &mut Value) {
        match value {
            Value::String(text) if text.chars().count() > STUB_ABOVE_CHARS => {
                *value = Value::String(format!(
                    "[{} characters, already applied to the deck]",
                    text.chars().count()
                ));
            }
            Value::Array(items) => items.iter_mut().for_each(stub),
            Value::Object(map) => map.values_mut().for_each(stub),
            _ => {}
        }
    }
    for turn in turns {
        if let Turn::Assistant { tool_calls, .. } = turn {
            for call in tool_calls {
                stub(&mut call.arguments);
            }
        }
    }
}

fn activity(phase: AgentPhase) -> ServerEvent {
    ServerEvent::AgentActivity {
        phase,
        action: AgentAction::Chat,
        slide_id: None,
        slide_index: None,
    }
}

fn build_turns(messages: &[ChatMessageInput]) -> Result<Vec<Turn>, CopilotError> {
    if messages.len() > MAX_MESSAGES {
        return Err(CopilotError::InvalidRequest(format!(
            "a conversation can contain at most {MAX_MESSAGES} messages"
        )));
    }
    let attachment_count: usize = messages.iter().map(|m| m.attachments.len()).sum();
    if attachment_count > MAX_TOTAL_ATTACHMENTS {
        return Err(CopilotError::InvalidRequest(format!(
            "a request can contain at most {MAX_TOTAL_ATTACHMENTS} attachments"
        )));
    }

    let mut turns = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "user" => {
                let mut text = message.text.trim().to_string();
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(CopilotError::InvalidRequest("a message is too long".into()));
                }
                let attachments = attachments::decode_all(&message.attachments)?;
                let mut images = Vec::new();
                let mut documents = Vec::new();
                for attachment in attachments {
                    match attachment {
                        AttachmentContent::Image { mime, data, .. } => {
                            images.push(ImagePart { mime, data });
                        }
                        AttachmentContent::Document { name, text: body } => {
                            documents.push((name, body));
                        }
                    }
                }
                if !documents.is_empty() {
                    text.push_str("\n\nAttached documents:\n");
                    for (name, body) in documents {
                        text.push_str(&format!("\n--- {name} ---\n{body}\n"));
                    }
                }
                if text.is_empty() && images.is_empty() {
                    continue;
                }
                if text.is_empty() {
                    text.push_str("Use the attached image as context.");
                }
                turns.push(Turn::User { text, images });
            }
            "assistant" => {
                let text = message.text.trim();
                if !text.is_empty() {
                    turns.push(Turn::Assistant {
                        text: text.to_string(),
                        tool_calls: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(turns)
}

/// Public shape used by the route tests and, later, by conversation export.
pub fn tool_result_for_test(name: &str, result: Value) -> Value {
    json!({ "tool": name, "result": result })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt as _;

    fn message(role: &str, text: &str) -> ChatMessageInput {
        ChatMessageInput {
            role: role.into(),
            text: text.into(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn history_is_normalized_and_empty_assistant_turns_are_dropped() {
        let turns = build_turns(&[
            message("assistant", ""),
            message("user", "Make a deck"),
            message("assistant", "Working on it"),
            message("user", "Add a slide"),
        ])
        .unwrap();
        assert_eq!(turns.len(), 3);
        assert!(matches!(turns[0], Turn::User { .. }));
        assert!(matches!(turns[1], Turn::Assistant { .. }));
    }

    #[test]
    fn executed_tool_calls_lose_their_bulky_payloads_but_keep_their_shape() {
        let big = "x".repeat(2_000);
        let mut turns = vec![Turn::Assistant {
            text: String::new(),
            tool_calls: vec![super::super::wire::ToolCall {
                id: "c1".into(),
                name: "write_slides".into(),
                arguments: json!({ "slides": [{ "html_code": big, "template": "hero" }] }),
            }],
        }];
        compact_history(&mut turns);
        let Turn::Assistant { tool_calls, .. } = &turns[0] else {
            panic!("assistant turn expected");
        };
        let slide = &tool_calls[0].arguments["slides"][0];
        assert_eq!(slide["template"], "hero");
        let html = slide["html_code"].as_str().unwrap();
        assert!(html.contains("2000 characters") && html.len() < 100);
    }

    /// Drives the whole loop against a scripted OpenAI-compatible server: a
    /// "make a deck" request must finish in two provider calls (one batch tool
    /// call, one summary), and the second request must not resend slide source.
    #[tokio::test]
    async fn a_new_deck_takes_two_provider_calls_and_the_second_is_compact() {
        use axum::{Router, extract::State, response::IntoResponse, routing::post};
        use std::sync::Mutex;

        type Seen = std::sync::Arc<Mutex<Vec<Value>>>;
        async fn completions(
            State(seen): State<Seen>,
            axum::Json(body): axum::Json<Value>,
        ) -> impl IntoResponse {
            let round = {
                let mut seen = seen.lock().unwrap();
                seen.push(body);
                seen.len()
            };
            // Round one streams the deck in pieces with pauses in between, like a
            // slow reasoning model; round two is the closing sentence.
            let mut chunks: Vec<String> = Vec::new();
            if round == 1 {
                let slides: Vec<Value> = (1..=6)
                    .map(|n| json!({
                        "html_code": format!("<section><h1>Slide {n}</h1><p>{}</p></section>", "body ".repeat(300)),
                        "template": "bullet",
                        "notes": format!("note {n}")
                    }))
                    .collect();
                // Written by hand: serde_json sorts object keys, and a real model writes
                // the deck fields before the slides.
                let args = format!(
                    r#"{{"title":"Mock deck","theme":"paper","aspect_ratio":"16:9","slides":{}}}"#,
                    serde_json::to_string(&slides).unwrap()
                );
                let size = args.len().div_ceil(4);
                let mut start = 0;
                for piece in 0..4 {
                    let mut end = (start + size).min(args.len());
                    while !args.is_char_boundary(end) {
                        end += 1;
                    }
                    let mut call =
                        json!({ "index": 0, "function": { "arguments": &args[start..end] } });
                    if piece == 0 {
                        call["id"] = json!("call_1");
                        call["type"] = json!("function");
                        call["function"]["name"] = json!("build_deck");
                    }
                    chunks.push(format!(
                        "data: {}\n\n",
                        json!({ "choices": [{ "delta": { "tool_calls": [call] } }] })
                    ));
                    start = end;
                }
            } else {
                chunks.push(format!(
                    "data: {}\n\n",
                    json!({ "choices": [{ "delta": { "content": "Built a six-slide deck." } }] })
                ));
            }
            chunks.push(format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({ "choices": [], "usage": { "prompt_tokens": 1000, "completion_tokens": 200 } })
            ));
            let stream = futures_util::stream::iter(chunks).then(|chunk| async move {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(chunk))
            });
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(stream))
                .unwrap()
        }

        let seen: Seen = Default::default();
        let app = Router::new()
            .route("/chat/completions", post(completions))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        catalog::tests_support::register_mock(&base);

        let state = AppState::new(
            slyde_core::PresentationState::new(
                "Deck",
                slyde_core::Theme::studio(),
                slyde_core::AspectRatio::R16x9,
            ),
            "tok".into(),
            std::path::PathBuf::from("ui"),
            vec![],
            "http://127.0.0.1:1".into(),
        );
        state
            .copilot
            .config
            .write()
            .await
            .keys
            .insert("mock".into(), "k".into());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let turn = tokio::spawn(run(
            state.clone(),
            ChatRequest {
                provider: "mock".into(),
                model: "m".into(),
                messages: vec![message("user", "Make a deck about launches")],
                selected_slide_id: None,
                selected_element_id: None,
                language: Some("en".into()),
                request_id: None,
            },
            tx,
        ));
        // Watch the deck while the model is still streaming: slides must show up
        // before the call ends, not all at once at the end.
        let mut seen_counts = std::collections::BTreeSet::new();
        while !turn.is_finished() {
            seen_counts.insert(state.view().await.presentation.slides.len());
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        turn.await.unwrap();
        assert!(
            seen_counts.iter().any(|count| (1..6).contains(count)),
            "slides must appear while the call is still streaming: {seen_counts:?}"
        );

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        assert!(
            events.iter().any(|event| matches!(event, ChatEvent::Done)),
            "the turn must finish cleanly: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::Usage {
                    calls: 2,
                    input_tokens: 2000,
                    output_tokens: 400,
                    estimated: false
                }
            )),
            "usage must sum what the provider reported over both calls: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ChatEvent::Progress { items: 6, .. })),
            "a six-slide tool call must report writing progress"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ChatEvent::Reversible)),
            "a turn that changed the deck must be revertible"
        );
        let revision = state.view().await.revision;
        let before = state.copilot.take_snapshot(revision).expect("snapshot");
        assert!(
            before.slides.is_empty(),
            "the snapshot is the deck before the turn"
        );
        assert!(
            state.copilot.take_snapshot(revision).is_err(),
            "a snapshot can only be used once"
        );
        let view = state.view().await;
        assert_eq!(view.presentation.slides.len(), 6);
        assert_eq!(view.presentation.title, "Mock deck");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "one batch call plus one summary");
        // The system message legitimately carries short previews of the live
        // deck; only the replayed conversation must be free of slide source.
        let second = seen[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] != "system")
            .map(Value::to_string)
            .collect::<String>();
        assert!(
            !second.contains("body body body"),
            "executed slide source must not be resent"
        );
        assert!(second.contains("already applied to the deck"));
    }

    #[test]
    fn too_many_attachments_are_rejected_before_decoding() {
        let mut many = message("user", "look");
        many.attachments = (0..9)
            .map(|index| AttachmentInput {
                name: format!("{index}.txt"),
                mime: "text/plain".into(),
                data: "eA==".into(),
            })
            .collect();
        let error = build_turns(&[many]).unwrap_err();
        assert_eq!(error.code(), "invalid_request");
    }
}
