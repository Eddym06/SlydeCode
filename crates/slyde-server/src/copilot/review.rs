//! The review loop: the model sees what it drew.
//!
//! A model that writes slide code works blind: it cannot tell that a headline
//! runs off the canvas, that the body text is 11 px, or that two blocks sit on
//! top of each other. After a round that wrote slides, the slides are opened in a
//! real browser and measured (`slyde_export::audit_slides`), and what was found
//! goes back to the model inside the tool result it is about to read, so it can
//! fix it in the next round. The budget is small and fixed, because every round
//! resends the whole conversation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use slyde_export::{SlideAudit, SlideTarget};

use crate::state::AppState;

/// The name the review shows under in the tool list.
pub const TOOL_NAME: &str = "review_slides";

/// Reviews a single turn may trigger. Two is enough to fix and confirm; more
/// would burn tokens polishing what the user can adjust by hand.
pub const MAX_REVIEWS_PER_TURN: usize = 2;

/// Problems listed per slide, so a bad slide never becomes a wall of text.
const MAX_PER_SLIDE: usize = 6;

/// Hard stop for one review: a browser that hangs must not hang the turn.
const REVIEW_TIMEOUT: Duration = Duration::from_secs(90);

/// What the model is told.
#[derive(Debug, PartialEq, Eq)]
pub struct Review {
    pub text: String,
    /// Findings across all slides; zero means the slides passed.
    pub problems: usize,
}

/// Slides whose code differs from `before`, in deck order.
pub fn changed_slides(
    before: &HashMap<String, String>,
    deck: &slyde_core::Presentation,
) -> Vec<String> {
    deck.slides
        .iter()
        .filter(|slide| before.get(&slide.id) != Some(&slide.code))
        .map(|slide| slide.id.clone())
        .collect()
}

/// Opens the slides in a browser and returns the findings, or `None` when no
/// review can be made (the feature is off, no browser, a failure). A review that
/// cannot run is silent: the turn simply continues as it always did.
pub async fn review_slides(state: &Arc<AppState>, slide_ids: &[String]) -> Option<Review> {
    if slide_ids.is_empty() || !state.copilot.review_enabled() {
        return None;
    }
    let browser = crate::chrome::discover_browser().ok()?;

    let view = state.view().await;
    let deck = &view.presentation;
    let token = crate::sandbox::percent_encode(&state.sandbox_token);
    let mut targets = Vec::new();
    let mut positions = Vec::new();
    for id in slide_ids {
        let Some(index) = deck.slides.iter().position(|slide| &slide.id == id) else {
            continue;
        };
        positions.push((index + 1, id.clone()));
        targets.push(SlideTarget {
            id: id.clone(),
            url: format!(
                "{}/s/{}?static=1&t={token}",
                state.sandbox_origin,
                crate::sandbox::percent_encode(id)
            ),
        });
    }
    if targets.is_empty() {
        return None;
    }

    let audits = match tokio::time::timeout(
        REVIEW_TIMEOUT,
        slyde_export::audit_slides(&browser.path, &targets, deck.aspect_ratio),
    )
    .await
    {
        Ok(Ok(audits)) => audits,
        Ok(Err(error)) => {
            tracing::warn!(%error, "slide review could not run");
            return None;
        }
        Err(_) => {
            tracing::warn!("slide review timed out");
            return None;
        }
    };
    let width_by_height = slyde_export::png_dimensions(deck.aspect_ratio);
    Some(format_review(&positions, &audits, width_by_height))
}

/// Builds the message for the model from the findings.
pub fn format_review(
    positions: &[(usize, String)],
    audits: &[SlideAudit],
    canvas: (u32, u32),
) -> Review {
    let mut problems = 0;
    let mut sections = Vec::new();
    let mut clean = Vec::new();
    for ((number, id), audit) in positions.iter().zip(audits) {
        if audit.issues.is_empty() {
            clean.push(number.to_string());
            continue;
        }
        problems += audit.issues.len();
        let shown = audit.issues.iter().take(MAX_PER_SLIDE);
        let mut lines = vec![format!(
            "Slide {number} (id {id}), {} problem{}:",
            audit.issues.len(),
            if audit.issues.len() == 1 { "" } else { "s" }
        )];
        for issue in shown {
            if issue.element.is_empty() {
                lines.push(format!("- {}: {}", issue.kind, issue.detail));
            } else {
                lines.push(format!(
                    "- {} on {}: {}",
                    issue.kind, issue.element, issue.detail
                ));
            }
        }
        if audit.issues.len() > MAX_PER_SLIDE {
            lines.push(format!("- and {} more", audit.issues.len() - MAX_PER_SLIDE));
        }
        sections.push(lines.join("\n"));
    }

    if problems == 0 {
        return Review {
            text: "Automatic review: no layout problems found in the slides you wrote. \
                   Do not call more tools; give the short summary."
                .to_string(),
            problems: 0,
        };
    }

    let mut text = format!(
        "Automatic review of the slides you just wrote, measured in a real render of the \
         {}x{} canvas. Fix every problem below in ONE call (write_slides for several slides, \
         mutate_element for a single block), change only what is listed, then stop.\n\n{}",
        canvas.0,
        canvas.1,
        sections.join("\n\n")
    );
    if !clean.is_empty() {
        text.push_str(&format!(
            "\n\nNo problems on slide(s) {}.",
            clean.join(", ")
        ));
    }
    Review { text, problems }
}

/// Puts the review inside the last tool result of the round, where the model
/// reads it next. Providers require a tool message per call, so the review rides
/// along instead of becoming a message of its own.
pub fn attach(content: &str, review: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(serde_json::Value::Object(mut fields)) => {
            fields.insert(
                "review".to_string(),
                serde_json::Value::String(review.to_string()),
            );
            serde_json::Value::Object(fields).to_string()
        }
        _ => format!("{content}\n\n{review}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slyde_core::{AspectRatio, Presentation, Slide, Theme};
    use slyde_export::AuditIssue;

    fn issue(kind: &str, element: &str, detail: &str) -> AuditIssue {
        AuditIssue {
            kind: kind.into(),
            element: element.into(),
            detail: detail.into(),
        }
    }

    #[test]
    fn only_slides_whose_code_changed_are_reviewed() {
        let mut deck = Presentation::new("T", Theme::studio(), AspectRatio::R16x9);
        deck.slides
            .push(Slide::new("a".into(), "<p>one</p>".into()));
        deck.slides
            .push(Slide::new("b".into(), "<p>two</p>".into()));
        let before: HashMap<String, String> = [
            ("a".to_string(), "<p>one</p>".to_string()),
            ("b".to_string(), "<p>old</p>".to_string()),
        ]
        .into();
        assert_eq!(changed_slides(&before, &deck), vec!["b"]);
        deck.slides
            .push(Slide::new("c".into(), "<p>new</p>".into()));
        assert_eq!(
            changed_slides(&before, &deck),
            vec!["b", "c"],
            "a new slide counts"
        );
    }

    #[test]
    fn findings_are_listed_by_slide_with_the_element_and_the_measurement() {
        let positions = vec![
            (1, "a".to_string()),
            (2, "b".to_string()),
            (3, "c".to_string()),
        ];
        let audits = vec![
            SlideAudit::unavailable(),
            SlideAudit {
                words: 40,
                issues: vec![
                    issue(
                        "overflow",
                        "h1#title \"Big\"",
                        "extends beyond the canvas right by 80px",
                    ),
                    issue("small", "p \"tiny\"", "type is 10px; use at least 18px"),
                ],
            },
            SlideAudit::unavailable(),
        ];
        let review = format_review(&positions, &audits, (1280, 720));
        assert_eq!(review.problems, 2);
        assert!(review.text.contains("1280x720"));
        assert!(review.text.contains("Slide 2 (id b), 2 problems:"));
        assert!(
            review.text.contains(
                "- overflow on h1#title \"Big\": extends beyond the canvas right by 80px"
            )
        );
        assert!(review.text.contains("No problems on slide(s) 1, 3."));
        assert!(
            review.text.contains("ONE call"),
            "the model is told to fix in one round"
        );
    }

    #[test]
    fn a_clean_deck_tells_the_model_to_stop() {
        let review = format_review(
            &[(1, "a".to_string())],
            &[SlideAudit::unavailable()],
            (1280, 720),
        );
        assert_eq!(review.problems, 0);
        assert!(review.text.contains("no layout problems"));
        assert!(review.text.contains("Do not call more tools"));
    }

    #[test]
    fn a_bad_slide_never_becomes_a_wall_of_text() {
        let many: Vec<AuditIssue> = (0..12)
            .map(|index| issue("small", "p", &format!("type is {index}px")))
            .collect();
        let review = format_review(
            &[(1, "a".to_string())],
            &[SlideAudit {
                words: 10,
                issues: many,
            }],
            (1280, 720),
        );
        assert!(review.text.contains("12 problems"));
        assert!(review.text.contains("and 6 more"));
        assert_eq!(review.text.matches("- small").count(), MAX_PER_SLIDE);
    }

    #[test]
    fn the_review_rides_inside_the_tool_result() {
        let merged = attach(r#"{"ok":true,"slides":2}"#, "fix the title");
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["review"], "fix the title");
        // A result that is not an object still carries it.
        assert!(attach("plain text", "fix it").ends_with("fix it"));
    }
}
