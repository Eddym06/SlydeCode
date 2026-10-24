use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderMap, header};
use slyde_core::{AnimationStep, CoreError, PresentationState, StateView};
use tokio::sync::{RwLock, broadcast};

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
}

impl AppState {
    pub fn new(
        core: PresentationState,
        token: String,
        ui_dir: PathBuf,
        allowed_origins: Vec<String>,
        sandbox_origin: String,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Arc::new(Self {
            core: RwLock::new(core),
            events,
            token,
            allowed_origins,
            ui_dir,
            sandbox_origin,
        })
    }

    pub async fn view(&self) -> StateView {
        self.core.read().await.view()
    }

    pub fn broadcast(&self, event: ServerEvent) {
        // No subscribers is a normal condition (UI not open yet).
        let _ = self.events.send(event);
    }

    pub async fn publish_state(&self) {
        let view = self.view().await;
        self.broadcast(ServerEvent::StateChanged { view });
    }
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
}

/// Commands the UI can send. Deliberately narrow: everything that changes the
/// deck goes through the same state manager the MCP tools use.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiCommand {
    SetTitle { title: String },
    SetTheme { theme_id: String },
    SetAspectRatio { ratio: String },
    SetNotes { slide_id: String, notes: Option<String> },
    AddSlide { template: String },
    MoveSlide { from: usize, to: usize },
    RemoveSlide { slide_id: String },
    Undo,
    Redo,
    TriggerAnimation { slide_id: String, step_index: usize },
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
    fn origin_check_allows_missing_origin_and_rejects_unknown() {
        let state = AppState::new(
            PresentationState::new("D", slyde_core::Theme::studio(), slyde_core::AspectRatio::R16x9),
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
