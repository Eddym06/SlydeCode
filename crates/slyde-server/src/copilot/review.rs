//! The review loop: the model sees what it drew.
//!
//! A model that writes slide code works blind: it cannot tell that a headline
//! runs off the canvas, that the body text is 11 px, or that two blocks sit on
//! top of each other. After a round that wrote slides, the slides are opened in a
//! real browser and measured (`slyde_export::audit_slides`), and what was found
//! goes back to the model inside the tool result it is about to read, so it can
//! fix it in the next round. Only hard failures are returned as work; cosmetic
//! audit warnings never trigger a rewrite. One review per turn, because every
//! round resends the whole conversation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use slyde_export::{SlideAudit, SlideTarget};

use crate::state::AppState;

/// The name the review shows under in the tool list.
pub const TOOL_NAME: &str = "review_slides";

/// One review per turn. A confirmation pass mostly invites the model to rewrite
/// slides that already work; the user can ask for another pass if needed.
pub const MAX_REVIEWS_PER_TURN: usize = 1;

/// Problems listed per slide, so a bad slide never becomes a wall of text.
const MAX_PER_SLIDE: usize = 6;

/// Only these findings are sent back to the model as work to fix. The audit
/// still measures cosmetic preferences (`small`, `margin`, `dense`), but those
/// are intentional in many designs and must not trigger a whole-deck rewrite.
const HARD_ISSUES: &[&str] = &[
    "broken", "empty", "overflow", "clipped", "overlap", "contrast",
];

fn is_hard_issue(kind: &str) -> bool {
    HARD_ISSUES.contains(&kind)
}

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
pub async fn review_slides(
    state: &Arc<AppState>,
    slide_ids: &[String],
    pending_images: &[String],
) -> Option<Review> {
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
    Some(format_review(
        &positions,
        &audits,
        width_by_height,
        pending_images,
    ))
}

/// Builds the message for the model from the findings.
pub fn format_review(
    positions: &[(usize, String)],
    audits: &[SlideAudit],
    canvas: (u32, u32),
    pending_images: &[String],
) -> Review {
    let pending_images: HashSet<&str> = pending_images.iter().map(String::as_str).collect();
    let mut problems = 0;
    let mut sections = Vec::new();
    let mut clean = Vec::new();
    for ((number, id), audit) in positions.iter().zip(audits) {
        let hard: Vec<&slyde_export::AuditIssue> = audit
            .issues
            .iter()
            .filter(|issue| is_hard_issue(&issue.kind))
            // An image-only slide is empty until its background job lands.
            .filter(|issue| !(issue.kind == "empty" && pending_images.contains(id.as_str())))
            .collect();
        if hard.is_empty() {
            clean.push(number.to_string());
            continue;
        }
        problems += hard.len();
        let mut lines = vec![format!(
            "Slide {number} (id {id}), {} hard problem{}:",
            hard.len(),
            if hard.len() == 1 { "" } else { "s" }
        )];
        for issue in hard.iter().take(MAX_PER_SLIDE) {
            if issue.element.is_empty() {
                lines.push(format!("- {}: {}", issue.kind, issue.detail));
            } else {
                lines.push(format!(
                    "- {} on {}: {}",
                    issue.kind, issue.element, issue.detail
                ));
            }
        }
        if hard.len() > MAX_PER_SLIDE {
            lines.push(format!("- and {} more", hard.len() - MAX_PER_SLIDE));
        }
        sections.push(lines.join("\n"));
    }

    if problems == 0 {
        return Review {
            text: "Automatic review: no hard layout problems found in the slides you wrote. \
                   Do not call more tools; give the short summary."
                .to_string(),
            problems: 0,
        };
    }

    let mut text = format!(
        "Automatic review of the slides you just wrote, measured in a real render of the \
         {}x{} canvas. Fix every hard problem below in ONE call (write_slides for several \
         slides, mutate_element for a single block). Change only what is listed, do not rewrite \
         a slide that already passes, and ignore cosmetic preferences such as small labels, \
         tight margins or dense data. Then stop.\n\n{}",
        canvas.0,
        canvas.1,
        sections.join("\n\n")
    );
    if !clean.is_empty() {
        text.push_str(&format!(
            "\n\nNo hard problems on slide(s) {}.",
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
    fn the_review_policy_is_one_hard_pass() {
        assert_eq!(MAX_REVIEWS_PER_TURN, 1);
        for kind in [
            "broken", "empty", "overflow", "clipped", "overlap", "contrast",
        ] {
            assert!(is_hard_issue(kind), "{kind} is a hard failure");
        }
        for kind in ["small", "margin", "dense"] {
            assert!(!is_hard_issue(kind), "{kind} is cosmetic");
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
                    issue(
                        "clipped",
                        "p \"body\"",
                        "text is cut off vertically (90px of content in 60px)",
                    ),
                ],
            },
            SlideAudit::unavailable(),
        ];
        let review = format_review(&positions, &audits, (1280, 720), &[]);
        assert_eq!(review.problems, 2);
        assert!(review.text.contains("1280x720"));
        assert!(review.text.contains("Slide 2 (id b), 2 hard problems:"));
        assert!(
            review.text.contains(
                "- overflow on h1#title \"Big\": extends beyond the canvas right by 80px"
            )
        );
        assert!(review.text.contains("No hard problems on slide(s) 1, 3."));
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
            &[],
        );
        assert_eq!(review.problems, 0);
        assert!(review.text.contains("no hard layout problems"));
        assert!(review.text.contains("Do not call more tools"));
    }

    #[test]
    fn cosmetic_findings_do_not_trigger_a_rewrite() {
        let review = format_review(
            &[(1, "a".to_string())],
            &[SlideAudit {
                words: 140,
                issues: vec![
                    issue("small", "span.eyebrow", "type is 11px; use at least 18px"),
                    issue(
                        "margin",
                        "h1",
                        "only 18px from the canvas edge; keep at least 40px",
                    ),
                    issue("dense", "", "140 words on one slide"),
                ],
            }],
            (1280, 720),
            &[],
        );
        assert_eq!(review.problems, 0);
        assert!(review.text.contains("no hard layout problems"));
        assert!(!review.text.contains("span.eyebrow"));
        assert!(!review.text.contains("only 18px"));
        assert!(!review.text.contains("140 words"));
    }

    #[test]
    fn a_slide_waiting_for_its_image_is_not_reported_as_empty() {
        let positions = [(1, "a".to_string())];
        let audits = [SlideAudit {
            words: 0,
            issues: vec![issue("empty", "", "the slide shows nothing")],
        }];
        let pending = ["a".to_string()];
        let review = format_review(&positions, &audits, (1280, 720), &pending);
        assert_eq!(review.problems, 0);
        assert!(review.text.contains("no hard layout problems"));

        let review = format_review(&positions, &audits, (1280, 720), &[]);
        assert_eq!(review.problems, 1, "without a pending image it is empty");
    }

    #[test]
    fn a_bad_slide_never_becomes_a_wall_of_text() {
        let many: Vec<AuditIssue> = (0..12)
            .map(|index| {
                issue(
                    "overflow",
                    "p",
                    &format!("runs off the canvas by {index}px"),
                )
            })
            .collect();
        let review = format_review(
            &[(1, "a".to_string())],
            &[SlideAudit {
                words: 10,
                issues: many,
            }],
            (1280, 720),
            &[],
        );
        assert!(review.text.contains("12 hard problems"));
        assert!(review.text.contains("and 6 more"));
        assert_eq!(review.text.matches("- overflow").count(), MAX_PER_SLIDE);
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
