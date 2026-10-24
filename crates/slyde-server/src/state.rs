use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderMap, header};
use slyde_core::{AnimationStep, CoreError, Presentation, PresentationState, StateView};
use tokio::sync::{RwLock, broadcast, watch};

pub const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Everything the transport layer needs. Shared by the app router and the
/// sandbox router so both read from the same presentation.
#[derive(Debug)]
pub struct AppState {
    pub core: RwLock<PresentationState>,
    pub events: broadcast::Sender<ServerEvent>,
    pub token: String,
    pub allowed_origins: Vec<String>,
    pub ui_dir: PathBuf,
    /// Origin of the sandbox listener, e.g. `http://127.0.0.1:4322`.
    pub sandbox_origin: String,
    /// When present, every committed deck is also handed to the autosave task.
    /// Tests and ephemeral servers leave it `None` and never touch the disk.
    deck_tx: Option<watch::Sender<Presentation>>,
}

impl AppState {
    pub fn new(
        core: PresentationState,
        token: String,
        ui_dir: PathBuf,
        allowed_origins: Vec<String>,
        sandbox_origin: String,
    ) -> Arc<Self> {
        Self::with_deck_sink(core, token, ui_dir, allowed_origins, sandbox_origin, None)
    }

    pub fn with_deck_sink(
        core: PresentationState,
        token: String,
        ui_dir: PathBuf,
        allowed_origins: Vec<String>,
        sandbox_origin: String,
        deck_tx: Option<watch::Sender<Presentation>>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Arc::new(Self {
            core: RwLock::new(core),
            events,
            token,
            allowed_origins,
            ui_dir,
            sandbox_origin,
            deck_tx,
        })
    }

    pub async fn view(&self) -> StateView {
        self.core.read().await.view()
    }

    pub fn broadcast(&self, event: ServerEvent) {
        // No subscribers is a normal condition (UI not open yet).
        let _ = self.events.send(event);
    }

    /// Publishes the new read model to the UI and, when persistence is enabled,
    /// to the autosave task. Called after every successful mutation, including
    /// undo/redo, so the saved file always matches what the user sees.
    pub async fn publish_state(&self) {
        let view = self.view().await;
        if let Some(tx) = &self.deck_tx {
            let _ = tx.send(view.presentation.clone());
        }
        self.broadcast(ServerEvent::StateChanged { view });
    }
}

/// Phase of an agent command as observed by the studio.
///
/// The server only emits `Writing`, `Adjusting`, `Animating`, `Done` and
/// `Error`. `Thinking` is reserved for a future explicit generation channel
/// and is understood by the UI so the protocol stays additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Thinking,
    Writing,
    Adjusting,
    Animating,
    Done,
    Error,
}

/// The kind of mutation an agent requested. Stable and structured: the UI
/// localizes it, so agents never inject display strings across the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAction {
    CreatePresentation,
    UpsertSlide,
    RemoveSlide,
    MoveSlide,
    SetNotes,
    TriggerAnimation,
}

/// Messages pushed to every connected UI over WebSocket.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Full read model. Decks are small; sending the whole view keeps the UI
    /// dumb and avoids cache-coherency bugs.
    StateChanged { view: StateView },
    /// Play one animation step inside the sandboxed slide.
    Animation {
        slide_id: String,
        step_index: usize,
        step: AnimationStep,
    },
    /// What an external agent is doing right now. Inferred from `/api/agent`
    /// calls; the UI owns the timing so a fast mutation is still legible.
    AgentActivity {
        phase: AgentPhase,
        action: AgentAction,
        #[serde(skip_serializing_if = "Option::is_none")]
        slide_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        slide_index: Option<usize>,
    },
    /// Progress for an export started from the UI. The HTTP response still
    /// carries the finished file; this only powers the progress label.
    ExportProgress {
        format: String,
        completed: usize,
        total: usize,
    },
}

/// Commands the UI can send. Deliberately narrow: everything that changes the
/// deck goes through the same state manager the MCP tools use.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiCommand {
    SetTitle {
        title: String,
    },
    SetTheme {
        theme_id: String,
    },
    SetAspectRatio {
        ratio: String,
    },
    SetNotes {
        slide_id: String,
        notes: Option<String>,
    },
    AddSlide {
        template: String,
    },
    MoveSlide {
        from: usize,
        to: usize,
    },
    RemoveSlide {
        slide_id: String,
    },
    Undo,
    Redo,
    TriggerAnimation {
        slide_id: String,
        step_index: usize,
    },
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AuthQuery {
    #[serde(default)]
    pub t: Option<String>,
}

/// Constant-time comparison so token checks do not leak length/prefix timing.
pub fn token_matches(candidate: &str, expected: &str) -> bool {
    let a = candidate.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

pub fn is_authorized(state: &AppState, headers: &HeaderMap, query: &AuthQuery) -> bool {
    let provided = query
        .t
        .as_deref()
        .filter(|t| !t.is_empty())
        .or_else(|| bearer_token(headers))
        .unwrap_or("");
    token_matches(provided, &state.token)
}

/// Browsers always send `Origin` on cross-origin and WebSocket requests. When
/// present it must be one of our own origins; when absent (CLI, tests) the
/// token alone is enough.
pub fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    state.allowed_origins.iter().any(|o| o == origin)
}

pub type CommandResult = Result<(), CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_is_exact() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("ab", "abc"));
        assert!(!token_matches("", "abc"));
    }

    #[test]
    fn bearer_header_is_parsed() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer tok_123".parse().unwrap());
        assert_eq!(bearer_token(&headers), Some("tok_123"));
    }

    #[test]
    fn agent_activity_serializes_structured_fields_only() {
        let event = ServerEvent::AgentActivity {
            phase: AgentPhase::Writing,
            action: AgentAction::UpsertSlide,
            slide_id: Some("sld_a".into()),
            slide_index: Some(1),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "agent_activity");
        assert_eq!(json["phase"], "writing");
        assert_eq!(json["action"], "upsert_slide");
        assert_eq!(json["slide_id"], "sld_a");
        assert_eq!(json["slide_index"], 1);

        let done = ServerEvent::AgentActivity {
            phase: AgentPhase::Done,
            action: AgentAction::CreatePresentation,
            slide_id: None,
            slide_index: None,
        };
        let json = serde_json::to_value(&done).unwrap();
        assert_eq!(json["type"], "agent_activity");
        assert!(json.get("slide_id").is_none());
        assert!(json.get("slide_index").is_none());
    }

    #[test]
    fn export_progress_uses_the_shared_event_envelope() {
        let event = ServerEvent::ExportProgress {
            format: "png".into(),
            completed: 1,
            total: 2,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "export_progress");
        assert_eq!(json["format"], "png");
        assert_eq!(json["completed"], 1);
        assert_eq!(json["total"], 2);
    }

    #[test]
    fn origin_check_allows_missing_origin_and_rejects_unknown() {
        let state = AppState::new(
            PresentationState::new(
                "D",
                slyde_core::Theme::studio(),
                slyde_core::AspectRatio::R16x9,
            ),
            "tok".into(),
            PathBuf::from("."),
            vec!["http://127.0.0.1:4321".into()],
            "http://127.0.0.1:4322".into(),
        );

        let empty = HeaderMap::new();
        assert!(origin_allowed(&state, &empty));

        let mut bad = HeaderMap::new();
        bad.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!origin_allowed(&state, &bad));

        let mut good = HeaderMap::new();
        good.insert(header::ORIGIN, "http://127.0.0.1:4321".parse().unwrap());
        assert!(origin_allowed(&state, &good));
    }
}
