//! SlydeCode core: domain model, presentation state and history.
//!
//! This crate has no I/O. Transport crates (`slyde-server`, `slyde-mcp`) and the
//! export pipeline depend on it; it never depends on them.

pub mod error;
pub mod model;
pub mod project;
pub mod state;

pub use error::CoreError;
pub use model::{
    AnimationKind, AnimationStep, AspectRatio, Author, Presentation, Slide, SlideId, StateView,
    TemplateType, Theme,
};
pub use project::{FILE_EXTENSION, FORMAT_ID, FORMAT_VERSION, ProjectFile};
pub use state::{
    DEFAULT_MAX_CODE_BYTES, DEFAULT_MAX_HISTORY, DEFAULT_MAX_SLIDES, HistoryEntry, Limits,
    PresentationState, UpsertOutcome, new_slide_id,
};
