use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("slide not found: {0}")]
    SlideNotFound(String),

    #[error("slide id already exists: {0}")]
    DuplicateSlideId(String),

    #[error("index {index} is out of range for {len} slides")]
    IndexOutOfRange { index: usize, len: usize },

    #[error("presentation already has the maximum of {max} slides")]
    SlideLimit { max: usize },

    #[error("slide code is empty")]
    EmptySlideCode,

    #[error("slide code is {actual} bytes, over the {max} byte limit")]
    CodeTooLarge { actual: usize, max: usize },

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("nothing to redo")]
    NothingToRedo,

    #[error("invalid theme: {0}")]
    InvalidTheme(String),

    #[error("invalid aspect ratio: {0}")]
    InvalidAspectRatio(String),

    #[error("animation step {index} does not exist on slide {slide_id}")]
    AnimationStepNotFound { slide_id: String, index: usize },

    #[error("element {element_id} does not exist in slide or presentation")]
    ElementNotFound { element_id: String },

    #[error("element {element_id} has no inner content to replace (void or self-closing tag)")]
    ElementHasNoContent { element_id: String },

    #[error("invalid animation: {0}")]
    InvalidAnimation(String),

    #[error("invalid project: {0}")]
    InvalidProject(String),
}
