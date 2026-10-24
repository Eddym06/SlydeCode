//! The `.slyde` project file: the durable representation of a deck.
//!
//! The file is deliberately plain JSON. A deck is text — HTML, CSS and
//! animation metadata — so there is nothing binary to store. Keeping it
//! readable means a user can open, diff or repair a deck with a text editor,
//! and support can ask for the file without a special tool.
//!
//! The header (`format` + `version`) exists so a future format change can
//! migrate the file instead of rejecting it.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::model::{Presentation, now_ms};
use crate::state::{DEFAULT_MAX_CODE_BYTES, DEFAULT_MAX_SLIDES};

/// Marker written into every file. Guards against opening unrelated JSON.
pub const FORMAT_ID: &str = "slydecore";

/// Current schema version. Additive changes keep the same version; anything
/// that would break older readers bumps it.
pub const FORMAT_VERSION: u32 = 1;

/// Suggested extension for saved decks, without the dot.
pub const FILE_EXTENSION: &str = "slyde";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub format: String,
    pub version: u32,
    /// When this snapshot was written, in epoch milliseconds.
    #[serde(default)]
    pub saved_at_ms: u64,
    pub presentation: Presentation,
}

impl ProjectFile {
    pub fn new(presentation: Presentation) -> Self {
        Self {
            format: FORMAT_ID.to_string(),
            version: FORMAT_VERSION,
            saved_at_ms: now_ms(),
            presentation,
        }
    }

    pub fn to_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(self)
            .map_err(|err| CoreError::InvalidProject(format!("could not serialize deck: {err}")))
    }

    /// Parses and validates a `.slyde` document.
    ///
    /// Validation runs here rather than at the transport layer so every caller
    /// — autosave restore, file upload, tests — enforces the same limits.
    pub fn from_json(raw: &str) -> Result<Self, CoreError> {
        let parsed: Self = serde_json::from_str(raw).map_err(|err| {
            CoreError::InvalidProject(format!("not a valid .slyde document: {err}"))
        })?;
        parsed.validate()?;
        Ok(parsed)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.format != FORMAT_ID {
            return Err(CoreError::InvalidProject(format!(
                "unexpected format {:?}, expected {FORMAT_ID:?}",
                self.format
            )));
        }
        if self.version == 0 || self.version > FORMAT_VERSION {
            return Err(CoreError::InvalidProject(format!(
                "unsupported .slyde version {} (this build reads up to {FORMAT_VERSION})",
                self.version
            )));
        }

        let deck = &self.presentation;
        deck.theme.validate()?;

        if deck.slides.len() > DEFAULT_MAX_SLIDES {
            return Err(CoreError::SlideLimit {
                max: DEFAULT_MAX_SLIDES,
            });
        }

        let mut seen = std::collections::HashSet::with_capacity(deck.slides.len());
        for slide in &deck.slides {
            if slide.id.trim().is_empty() {
                return Err(CoreError::InvalidProject("slide id is empty".into()));
            }
            if !seen.insert(slide.id.as_str()) {
                return Err(CoreError::DuplicateSlideId(slide.id.clone()));
            }
            if slide.code.trim().is_empty() {
                return Err(CoreError::EmptySlideCode);
            }
            if slide.code.len() > DEFAULT_MAX_CODE_BYTES {
                return Err(CoreError::CodeTooLarge {
                    actual: slide.code.len(),
                    max: DEFAULT_MAX_CODE_BYTES,
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AnimationKind, AnimationStep, AspectRatio, Slide, TemplateType, Theme};

    fn deck() -> Presentation {
        let mut deck = Presentation::new("Quarterly review", Theme::studio(), AspectRatio::R16x9);
        let mut slide = Slide::new("sld_cover".into(), "<h1 class=\"title\">Hi</h1>".into());
        slide.template = TemplateType::Hero;
        slide.notes = Some("Say hello".into());
        slide.animation.push(AnimationStep {
            target: ".title".into(),
            kind: AnimationKind::FadeUp,
            delay_ms: 60,
            duration_ms: 500,
            easing: "ease-out".into(),
        });
        deck.slides.push(slide);
        deck
    }

    #[test]
    fn round_trips_a_deck_without_losing_anything() {
        let original = deck();
        let file = ProjectFile::new(original.clone());
        let json = file.to_json().unwrap();
        let parsed = ProjectFile::from_json(&json).unwrap();
        assert_eq!(parsed.presentation, original);
        assert_eq!(parsed.format, FORMAT_ID);
        assert_eq!(parsed.version, FORMAT_VERSION);
        assert!(parsed.saved_at_ms > 0);
    }

    #[test]
    fn rejects_json_that_is_not_a_project() {
        let err = ProjectFile::from_json(r#"{"hello":"world"}"#).unwrap_err();
        assert!(matches!(err, CoreError::InvalidProject(_)));
    }

    #[test]
    fn rejects_a_foreign_format_marker() {
        let mut file = ProjectFile::new(deck());
        file.format = "something-else".into();
        let json = serde_json::to_string(&file).unwrap();
        assert!(matches!(
            ProjectFile::from_json(&json),
            Err(CoreError::InvalidProject(_))
        ));
    }

    #[test]
    fn rejects_a_newer_schema_version() {
        let mut file = ProjectFile::new(deck());
        file.version = FORMAT_VERSION + 1;
        let json = serde_json::to_string(&file).unwrap();
        let err = ProjectFile::from_json(&json).unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn rejects_duplicate_slide_ids() {
        let mut deck = deck();
        let mut twin = deck.slides[0].clone();
        twin.code = "<h1>Other</h1>".into();
        deck.slides.push(twin);
        let json = serde_json::to_string(&ProjectFile::new(deck)).unwrap();
        assert!(matches!(
            ProjectFile::from_json(&json),
            Err(CoreError::DuplicateSlideId(_))
        ));
    }

    #[test]
    fn rejects_empty_or_oversized_slide_code() {
        let mut empty = deck();
        empty.slides[0].code = "   ".into();
        let json = serde_json::to_string(&ProjectFile::new(empty)).unwrap();
        assert_eq!(
            ProjectFile::from_json(&json).unwrap_err(),
            CoreError::EmptySlideCode
        );

        let mut huge = deck();
        huge.slides[0].code = "x".repeat(DEFAULT_MAX_CODE_BYTES + 1);
        let json = serde_json::to_string(&ProjectFile::new(huge)).unwrap();
        assert!(matches!(
            ProjectFile::from_json(&json),
            Err(CoreError::CodeTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_a_theme_with_unsafe_colors() {
        let mut deck = deck();
        deck.theme.accent = "javascript:alert(1)".into();
        let json = serde_json::to_string(&ProjectFile::new(deck)).unwrap();
        assert!(matches!(
            ProjectFile::from_json(&json),
            Err(CoreError::InvalidTheme(_))
        ));
    }
    #[test]
    fn accepts_a_deck_with_no_slides() {
        let deck = Presentation::new("Empty", Theme::paper(), AspectRatio::R4x3);
        let json = ProjectFile::new(deck).to_json().unwrap();
        assert!(ProjectFile::from_json(&json).is_ok());
    }
}
