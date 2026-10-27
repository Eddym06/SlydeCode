//! Layout audit of rendered slides.
//!
//! A model that writes slide code never sees what it drew. After it writes, the
//! slides are opened in a real browser and measured: text that runs off the
//! canvas, is cut off or sits on other text, type too small to read, low
//! contrast, slides that are crowded or empty. The findings go back to the model
//! as facts it can act on. Like the rest of this crate it knows nothing about the
//! studio: it takes URLs and returns what the page reported.

use serde::Deserialize;

/// The script that measures one slide; it returns a JSON string.
pub(crate) const AUDIT_JS: &str = include_str!("audit.js");

/// One measured problem on a slide.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuditIssue {
    /// `broken`, `empty`, `overflow`, `clipped`, `overlap`, `contrast`, `small`,
    /// `dense` or `margin`.
    pub kind: String,
    /// A short description of the element, with its id when it has one.
    #[serde(default)]
    pub element: String,
    /// What was measured, in plain words.
    pub detail: String,
}

/// What the audit found on one slide.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SlideAudit {
    #[serde(default)]
    pub words: u32,
    #[serde(default)]
    pub issues: Vec<AuditIssue>,
}

impl SlideAudit {
    /// A slide the browser could not audit counts as clean: a missing audit must
    /// never become a reason to tell the model its slide is broken.
    pub fn unavailable() -> Self {
        Self {
            words: 0,
            issues: Vec::new(),
        }
    }

    pub(crate) fn parse(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_else(|_| Self::unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_from_the_page_is_parsed() {
        let audit = SlideAudit::parse(
            r#"{"words":42,"issues":[{"kind":"overflow","element":"h1#title \"Big\"","detail":"extends beyond the canvas right by 80px"}]}"#,
        );
        assert_eq!(audit.words, 42);
        assert_eq!(audit.issues.len(), 1);
        assert_eq!(audit.issues[0].kind, "overflow");
    }

    #[test]
    fn an_unreadable_report_is_a_clean_slide_never_a_failure() {
        assert_eq!(SlideAudit::parse("not json"), SlideAudit::unavailable());
        assert_eq!(SlideAudit::parse(""), SlideAudit::unavailable());
        assert!(SlideAudit::parse("{}").issues.is_empty());
    }
}
