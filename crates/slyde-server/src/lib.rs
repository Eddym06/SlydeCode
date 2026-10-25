//! SlydeCode local server.
//!
//! Responsibilities:
//! - serve the editor UI and the presenter page over `127.0.0.1`
//! - expose the state read model and the UI command endpoint
//! - stream changes over WebSocket
//! - serve slide documents from a **separate origin** (sandbox listener)
//! - discover and launch the user's Chrome
//!
//! It never renders HTML itself: Chrome is the only rendering surface.

pub mod agent;
pub mod asset_api;
pub mod assets;
pub mod chrome;
pub mod copilot;
pub mod deck_api;
pub mod error;
pub mod export_api;
pub mod http;
pub mod library;
pub mod rendezvous;
pub mod routes;
pub mod sandbox;
pub mod starter;
pub mod state;
pub mod storage;
pub mod ws;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::services::ServeDir;

pub use chrome::{Browser, BrowserKind, OpenMode, discover_browser};
pub use error::ServerError;
pub use library::{DeckSummary, Library};
pub use rendezvous::{
    RuntimeInfo, app_data_dir, clear_runtime, now_ms, probe_health, read_runtime, runtime_path,
    write_runtime,
};
pub use state::{AppState, ServerEvent, UiCommand};

use slyde_core::PresentationState;

#[derive(Debug)]
pub struct ServerConfig {
    /// Directory that contains `index.html`, `present.html` and `static/`.
    pub ui_dir: PathBuf,
    /// Initial deck. Ignored when `library_dir` points at a library with decks:
    /// the active deck on disk wins, because that is what the user last worked on.
    pub state: PresentationState,
    /// `0` picks an ephemeral port, which is what production uses.
    pub port: u16,
    pub sandbox_port: u16,
    pub token: String,
    /// When set, decks live in this directory (one `.slyde` file each) and the
    /// active one is autosaved there. `None` keeps the server in memory.
    pub library_dir: Option<PathBuf>,
    /// Asset root (fonts, brand, icons), served at `/fonts` on both origins.
    /// `None` is valid: the UI and slides then use the system fallback stack.
    pub assets_dir: Option<PathBuf>,
}

impl ServerConfig {
    pub fn new(ui_dir: impl Into<PathBuf>, state: PresentationState) -> Self {
        Self {
            ui_dir: ui_dir.into(),
            state,
            port: 0,
            sandbox_port: 0,
            token: new_token(),
            library_dir: None,
            assets_dir: None,
        }
    }
}

pub struct RunningServer {
    pub port: u16,
    pub sandbox_port: u16,
    pub token: String,
    pub state: Arc<AppState>,
    /// Present when the server owns a deck library on disk.
    pub library: Option<Arc<Library>>,
    shutdown: watch::Sender<bool>,
}

impl RunningServer {
    pub fn app_url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }

    pub fn present_url(&self) -> String {
        format!("http://127.0.0.1:{}/present?t={}", self.port, self.token)
    }

    pub fn runtime_info(&self) -> RuntimeInfo {
        RuntimeInfo {
            port: self.port,
            sandbox_port: self.sandbox_port,
            token: self.token.clone(),
            pid: std::process::id(),
            started_at_ms: now_ms(),
        }
    }

    pub async fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

pub async fn start(config: ServerConfig) -> Result<RunningServer, ServerError> {
    if !config.ui_dir.is_dir() {
        return Err(ServerError::UiMissing(config.ui_dir.display().to_string()));
    }

    let app_listener = bind(config.port).await?;
    let sandbox_listener = bind(config.sandbox_port).await?;

    let port = app_listener.local_addr()?.port();
    let sandbox_port = sandbox_listener.local_addr()?.port();

    let origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];

    let (library, initial_state) = match config.library_dir.as_ref() {
        Some(dir) => {
            let library = Arc::new(Library::new(dir.clone()));
            // A library written before this feature kept a single
            // `current.slyde`; import it once so no work disappears.
            if let Some(id) = library.migrate_legacy().await? {
                if library.active().is_none() {
                    library.set_active(&id)?;
                }
                tracing::info!(deck = %id, "imported the pre-library deck");
            }
            let (active, deck) = open_healthy_deck(&library).await?;
            library.set_active(&active)?;
            let mut state =
                PresentationState::new(deck.title.clone(), deck.theme.clone(), deck.aspect_ratio);
            state.replace(deck)?;
            (Some(library), state)
        }
        None => (None, config.state),
    };

    let (deck_tx, autosave) = match &library {
        Some(library) => {
            let (tx, rx) = tokio::sync::watch::channel(initial_state.presentation().clone());
            (Some(tx), Some((rx, library.subscribe_path())))
        }
        None => (None, None),
    };

    let state = AppState::with_copilot(
        initial_state,
        config.token.clone(),
        config.ui_dir,
        origins,
        format!("http://127.0.0.1:{sandbox_port}"),
        Arc::new(copilot::CopilotState::persistent()),
        match (&library, deck_tx) {
            (Some(library), Some(deck_tx)) => {
                state::Persistence::with_library(library.clone(), deck_tx)
            }
            _ => state::Persistence::memory(),
        },
    );

    if let Some((deck_rx, path_rx)) = autosave {
        tokio::spawn(storage::run_autosave(deck_rx, path_rx));
    }

    let (shutdown, _) = watch::channel(false);

    let (mut app, mut sandbox) = (
        routes::app_router(state.clone()),
        routes::sandbox_router(state.clone()),
    );
    match config.assets_dir.as_ref().map(|dir| dir.join("fonts")) {
        Some(fonts) if fonts.is_dir() => {
            // Same directory on both origins: the editor paints with Geist, and
            // every slide iframe shares one cached copy from the sandbox origin.
            app = app.nest_service("/fonts", ServeDir::new(&fonts));
            sandbox = sandbox.nest_service("/fonts", ServeDir::new(&fonts));
        }
        Some(fonts) => {
            tracing::warn!(dir = %fonts.display(), "font directory not found; using system fallbacks");
        }
        None => {}
    }

    // Vendored renderers (KaTeX, highlight.js) live on the sandbox origin only:
    // a slide document may load same-origin assets, never the network.
    match config.assets_dir.as_ref().map(|dir| dir.join("vendor")) {
        Some(vendor) if vendor.is_dir() => {
            sandbox = sandbox.nest_service("/vendor", ServeDir::new(&vendor));
        }
        Some(vendor) => {
            tracing::warn!(dir = %vendor.display(), "vendor directory not found; math and code blocks degrade to plain text");
        }
        None => {}
    }

    let mut app_shutdown = shutdown.subscribe();
    tokio::spawn(async move {
        let server = axum::serve(app_listener, app).with_graceful_shutdown(async move {
            let _ = app_shutdown.changed().await;
        });
        if let Err(err) = server.await {
            tracing::error!(error = %err, "app server stopped unexpectedly");
        }
    });

    let mut sandbox_shutdown = shutdown.subscribe();
    tokio::spawn(async move {
        let server = axum::serve(sandbox_listener, sandbox).with_graceful_shutdown(async move {
            let _ = sandbox_shutdown.changed().await;
        });
        if let Err(err) = server.await {
            tracing::error!(error = %err, "sandbox server stopped unexpectedly");
        }
    });

    tracing::info!(port, sandbox_port, "SlydeCode server listening");

    Ok(RunningServer {
        port,
        sandbox_port,
        token: config.token,
        state,
        library,
        shutdown,
    })
}

/// Picks the deck to open at startup.
///
/// A deck that cannot be read must not block the app and must never be
/// overwritten by the autosave: when the active file is unreadable, this falls
/// back to a healthy sibling, or seeds a fresh deck, and leaves the broken file
/// untouched so the user can recover it by hand.
async fn open_healthy_deck(
    library: &Library,
) -> Result<(String, slyde_core::Presentation), ServerError> {
    let active = library.active_or_newest()?;
    if let Some(id) = active.as_deref() {
        match library.load(id) {
            Ok(Some(deck)) => return Ok((id.to_string(), deck)),
            Ok(None) => {}
            Err(error) => {
                tracing::error!(deck = %id, error = %error, "the active deck could not be read; opening another one");
            }
        }
    }

    let siblings = library.list().unwrap_or_default();
    for summary in siblings {
        if Some(summary.id.as_str()) == active.as_deref() {
            continue;
        }
        if let Ok(Some(deck)) = library.load(&summary.id) {
            return Ok((summary.id, deck));
        }
    }

    let deck = library::welcome_deck();
    let id = library.create(&deck.title.clone(), &deck).await?;
    Ok((id, deck))
}

async fn bind(port: u16) -> Result<TcpListener, ServerError> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|source| ServerError::Bind {
            addr: format!("127.0.0.1:{port}"),
            source,
        })
}

/// 256 bits from the OS CSPRNG, hex-encoded.
///
/// The token is what stops another local page or process from reading or
/// mutating the deck, so it has to come from a real entropy source. `getrandom`
/// is the same crate `uuid` already pulls in; deriving bytes from `RandomState`
/// hashes, as an earlier version of this file did, is not a CSPRNG.
pub fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    if let Err(error) = getrandom::fill(&mut bytes) {
        // The OS entropy source is unavailable, which means the whole process is
        // in a broken environment. Do not silently emit a weak session secret:
        // make the degradation loud, and bind the fallback to this process.
        tracing::error!(%error, "OS entropy unavailable; session token is not cryptographic");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_u32(std::process::id());
        let first = hasher.finish().to_le_bytes();
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = first[index % first.len()];
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const TOKEN_BYTES: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;
    use slyde_core::{AspectRatio, Theme};

    #[test]
    fn tokens_are_long_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn missing_ui_dir_is_reported() {
        let state = PresentationState::new("D", Theme::studio(), AspectRatio::R16x9);
        let config = ServerConfig::new("definitely/not/here", state);
        assert!(matches!(
            start(config).await,
            Err(ServerError::UiMissing(_))
        ));
    }
}
