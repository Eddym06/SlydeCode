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
pub mod chrome;
pub mod error;
pub mod http;
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
    pub state: PresentationState,
    /// `0` picks an ephemeral port, which is what production uses.
    pub port: u16,
    pub sandbox_port: u16,
    pub token: String,
    /// When set, every committed deck is written here (atomic, debounced).
    pub deck_path: Option<PathBuf>,
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
            deck_path: None,
            assets_dir: None,
        }
    }
}

pub struct RunningServer {
    pub port: u16,
    pub sandbox_port: u16,
    pub token: String,
    pub state: Arc<AppState>,
    shutdown: watch::Sender<bool>,
}

impl RunningServer {
    pub fn app_url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }

    pub fn present_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/present?t={}",
            self.port, self.token
        )
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

    let (deck_tx, autosave) = match &config.deck_path {
        Some(path) => {
            let (tx, rx) = tokio::sync::watch::channel(config.state.presentation().clone());
            (Some(tx), Some((rx, path.clone())))
        }
        None => (None, None),
    };

    let state = AppState::with_deck_sink(
        config.state,
        config.token.clone(),
        config.ui_dir,
        origins,
        format!("http://127.0.0.1:{sandbox_port}"),
        deck_tx,
    );

    if let Some((rx, path)) = autosave {
        tokio::spawn(storage::run_autosave(rx, path));
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
        shutdown,
    })
}

async fn bind(port: u16) -> Result<TcpListener, ServerError> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|source| ServerError::Bind {
            addr: format!("127.0.0.1:{port}"),
            source,
        })
}

/// 128 bits of randomness, hex-encoded. The token is what stops any other local
/// page from reading or mutating the deck.
pub fn new_token() -> String {
    let a = uuid_like();
    let b = uuid_like();
    format!("{a}{b}")
}

fn uuid_like() -> String {
    // Avoids pulling `rand` in just for this: two v4 UUIDs give 244 bits of
    // entropy, more than enough for a local session token.
    let mut bytes = [0u8; 16];
    getrandom_fallback(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn getrandom_fallback(out: &mut [u8; 16]) {
    use std::hash::{BuildHasher, Hasher};
    // `RandomState` is seeded from the OS entropy source.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let a = hasher.finish();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(a);
    hasher.write_u32(std::process::id());
    let b = hasher.finish();
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&b.to_le_bytes());
}

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
        assert!(matches!(start(config).await, Err(ServerError::UiMissing(_))));
    }
}
