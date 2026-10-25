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
use super::wire::{ChatEvent, ImagePart, Turn};

const MAX_MESSAGES: usize = 40;
const MAX_TEXT_CHARS: usize = 80_000;
const MAX_TOTAL_ATTACHMENTS: usize = 8;
/// Rounds the model may spend calling tools. A deck of a dozen slides plus a
/// couple of reads fits comfortably; the loop also stops on its own when the
/// model stops making progress.
const MAX_TOOL_ROUNDS: usize = 20;

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
}

pub async fn run(state: Arc<AppState>, request: ChatRequest, tx: UnboundedSender<ChatEvent>) {
    let provider_id = request.provider.clone();
    let model = request.model.clone();
    let _ = tx.send(ChatEvent::Meta {
        provider: provider_id.clone(),
        model: model.clone(),
    });

    let result = run_inner(&state, request, &tx).await;
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
    state: &AppState,
    request: ChatRequest,
    tx: &UnboundedSender<ChatEvent>,
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

    let mut last_signature: Option<String> = None;
    let mut repeats: u32 = 0;

    for _round in 0..MAX_TOOL_ROUNDS {
        if tx.is_closed() {
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
        let system = prompt::system_prompt(deck, selected, request.selected_element_id.as_deref(), language);
        let assistant = transport::stream(
            state.copilot.http(),
            provider,
            &key,
            request.model.trim(),
            &system,
            &turns,
            tx,
        )
        .await?;

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
            let outcome = tools::execute(state, &call.name, call.arguments.clone()).await;
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
