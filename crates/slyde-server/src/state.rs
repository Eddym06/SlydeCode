use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderMap, header};
use slyde_core::{AnimationStep, CoreError, Presentation, PresentationState, StateView};
use tokio::sync::{RwLock, broadcast, watch};

use crate::copilot::CopilotState;

pub const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Everything the transport layer needs. Shared by the app router and the
/// sandbox router so both read from the same presentation.
#[derive(Debug)]
pub struct AppState {
    pub core: RwLock<PresentationState>,
    pub events: broadcast::Sender<ServerEvent>,
    pub token: String,
    /// Authorizes slide documents on the sandbox origin only.
    ///
    /// Slide URLs are embedded in `<iframe src>`, so the AI-authored script inside
    /// a slide can read this value out of its own location. It therefore must not
    /// unlock `/api/*`: reading or mutating the deck still requires `token`.
    pub sandbox_token: String,
    pub allowed_origins: Vec<String>,
    pub ui_dir: PathBuf,
    /// Origin of the sandbox listener, e.g. `http://127.0.0.1:4322`.
    pub sandbox_origin: String,
    /// Image store. Always present: an in-memory server points it at a scratch
    /// directory so tests never touch the user's app-data.
    pub assets: Arc<crate::assets::AssetStore>,
    /// When present, the deck endpoints can list or switch decks in the library
    /// and the autosave task writes to the active deck file.
    /// Tests and ephemeral servers leave it `None` and never touch the disk.
    pub library: Option<Arc<crate::library::Library>>,
    /// When present, every committed deck is handed to the autosave task.
    deck_tx: Option<watch::Sender<Presentation>>,
    /// Provider keys, model discovery and chat transports. The deck remains
    /// the only shared mutable domain state; the copilot mutates it through
    /// `agent::run`, never directly.
    pub copilot: Arc<CopilotState>,
    /// Named and automatic checkpoints of the deck.
    pub versions: crate::versions::Versions,
}

/// The on-disk half of the server: which decks exist, where autosave writes and
/// where images live. Grouping them keeps the constructor readable and lets a
/// memory-only server be built in one expression.
#[derive(Clone)]
pub struct Persistence {
    pub library: Option<Arc<crate::library::Library>>,
    pub deck_sink: Option<watch::Sender<Presentation>>,
    pub assets: Arc<crate::assets::AssetStore>,
}

impl Persistence {
    /// Nothing is persisted: tests and ephemeral servers use this.
    pub fn memory() -> Self {
        Self {
            library: None,
            deck_sink: None,
            assets: Arc::new(crate::assets::AssetStore::for_tests()),
        }
    }

    pub fn with_library(
        library: Arc<crate::library::Library>,
        deck_sink: watch::Sender<Presentation>,
    ) -> Self {
        Self {
            library: Some(library),
            deck_sink: Some(deck_sink),
            assets: Arc::new(crate::assets::AssetStore::from_app_data()),
        }
    }
}

impl AppState {
    pub fn new(
        core: PresentationState,
        token: String,
        ui_dir: PathBuf,
        allowed_origins: Vec<String>,
        sandbox_origin: String,
    ) -> Arc<Self> {
        Self::with_copilot(
            core,
            token,
            ui_dir,
            allowed_origins,
            sandbox_origin,
            Arc::new(CopilotState::in_memory()),
            Persistence::memory(),
        )
    }

    pub fn with_deck_sink(
        core: PresentationState,
        token: String,
        ui_dir: PathBuf,
        allowed_origins: Vec<String>,
        sandbox_origin: String,
        deck_tx: Option<watch::Sender<Presentation>>,
    ) -> Arc<Self> {
        Self::with_copilot(
            core,
            token,
            ui_dir,
            allowed_origins,
            sandbox_origin,
            Arc::new(CopilotState::in_memory()),
            Persistence {
                deck_sink: deck_tx,
                ..Persistence::memory()
            },
        )
    }

    /// Production constructor. `copilot` is persistent here; tests keep using
    /// the in-memory constructor above and never read the user's key file.
    #[allow(clippy::too_many_arguments)]
    pub fn with_copilot(
        core: PresentationState,
        token: String,
        ui_dir: PathBuf,
        allowed_origins: Vec<String>,
        sandbox_origin: String,
        copilot: Arc<CopilotState>,
        persistence: Persistence,
    ) -> Arc<Self> {
        let Persistence {
            library,
            deck_sink: deck_tx,
            assets,
        } = persistence;
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let versions = match library
            .as_ref()
            .and_then(|lib| lib.root().parent().map(|p| p.join("versions")))
        {
            Some(dir) => crate::versions::Versions::persistent(dir),
            None => crate::versions::Versions::in_memory(),
        };
        Arc::new(Self {
            versions,
            core: RwLock::new(core),
            events,
            token,
            sandbox_token: crate::new_token(),
            allowed_origins,
            ui_dir,
            sandbox_origin,
            library,
            deck_tx,
            copilot,
            assets,
        })
    }

    /// The key versions are stored under: the active library deck, or a fixed
    /// name for a server without a library.
    pub fn deck_key(&self) -> String {
        self.library
            .as_ref()
            .and_then(|library| library.active())
            .unwrap_or_else(|| "session".to_string())
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
    MutateElement,
    /// The in-app copilot is generating or choosing a tool. This is the only
    /// action that can legitimately sit in `Thinking` phase.
    Chat,
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
    NewDeck {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        template: Option<String>,
    },
    AddSlide {
        template: String,
        #[serde(default)]
        index: Option<usize>,
    },
    DuplicateSlide {
        slide_id: String,
    },
    MoveSlide {
        from: usize,
        to: usize,
    },
    RemoveSlide {
        slide_id: String,
    },
    /// Removes several slides (a multi-selection) as one undoable step.
    RemoveSlides {
        slide_ids: Vec<String>,
    },
    /// Replaces a slide's source from the code editor.
    SetSlideCode {
        slide_id: String,
        code: String,
    },
    Undo,
    Redo,
    TriggerAnimation {
        slide_id: String,
        step_index: usize,
    },
    MutateElement {
        element_id: String,
        /// Scopes the edit to one slide; required when the handle is not unique
        /// across the deck (generated `slyde-N` handles are per slide document).
        #[serde(default)]
        slide_id: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        style: Option<String>,
        #[serde(default)]
        html: Option<String>,
    },
    SetAnimation {
        slide_id: String,
        animation: Vec<slyde_core::AnimationStep>,
    },
    /// Appends a stored image to a slide.
    InsertImage {
        slide_id: String,
        src: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// Inserts a ready-made block (shape, icon, chart, table) from the library.
    InsertBlock {
        slide_id: String,
        html: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// Moves/resizes several elements of one slide at once (align, distribute).
    SetGeometries {
        slide_id: String,
        items: Vec<GeometryItem>,
    },
    /// Replaces the source of several slides in one step (apply a layout).
    SetSlidesCode {
        items: Vec<SlideCodeItem>,
    },
    /// Sets the transition into one, several or (without ids) every slide.
    SetTransition {
        #[serde(default)]
        slide_ids: Option<Vec<String>>,
        kind: String,
        #[serde(default)]
        duration_ms: Option<u32>,
    },
    /// Moves/resizes an element; the canvas sends the final geometry.
    SetGeometry {
        slide_id: String,
        element_id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GeometryItem {
    pub element_id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SlideCodeItem {
    pub slide_id: String,
    pub code: String,
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

/// Slide documents accept the sandbox token (what the editor embeds) or the app
/// token (used by the export pipeline and the CLI, which already hold it).
///
/// A slide document contains user content, so it cannot be public: any local
/// process could otherwise read the deck through `/s/{id}`.
pub fn is_slide_authorized(state: &AppState, query: &AuthQuery) -> bool {
    let Some(provided) = query.t.as_deref().filter(|t| !t.is_empty()) else {
        return false;
    };
    token_matches(provided, &state.sandbox_token) || token_matches(provided, &state.token)
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
