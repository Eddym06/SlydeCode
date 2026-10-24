use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("no supported browser found; install Google Chrome or set SLYDE_BROWSER")]
    BrowserNotFound,

    #[error("could not bind {addr}: {source}")]
    Bind {
        addr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("ui directory does not exist: {0}")]
    UiMissing(String),

    #[error("deck file: {0}")]
    Deck(String),

    #[error(transparent)]
    Core(#[from] slyde_core::CoreError),
}
