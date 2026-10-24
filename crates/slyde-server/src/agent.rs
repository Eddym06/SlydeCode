//! Agent-facing command surface.
//!
//! The MCP server is a thin proxy over these commands: it never owns a second
//! copy of the deck. Everything funnels into the same `PresentationState` the
//! UI edits, so the storyboard updates live while an agent writes.

use serde::Deserialize;
use slyde_core::{
    AnimationStep, AspectRatio, CoreError, PresentationState, Theme, UpsertOutcome,
};

use crate::state::{AppState, ServerEvent};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentCommand {
    CreatePresentation {
        title: String,
        #[serde(default)]
        theme: Option<String>,
        #[serde(default)]
        aspect_ratio: Option<String>,
    },
    UpsertSlide {
        index: usize,
        #[serde(default)]
        slide_id: Option<String>,
        html_code: String,
        #[serde(default)]
        template: Option<String>,
        #[serde(default)]
        animation: Option<Vec<AnimationStep>>,
    },
    RemoveSlide {
        slide_id: String,
    },
    MoveSlide {
        from: usize,
        to: usize,
    },
    TriggerAnimation {
        slide_id: String,
        step_index: usize,
    },
    SetNotes {
        slide_id: String,
        notes: Option<String>,
    },
}

#[derive(Debug, Default)]
pub struct AgentOutcome {
    pub upsert: Option<UpsertOutcome>,
    pub created: bool,
    pub animation: Option<ServerEvent>,
}

pub async fn handle(state: &AppState, command: AgentCommand) -> Result<AgentOutcome, CoreError> {
    let mut core = state.core.write().await;
    match command {
        AgentCommand::CreatePresentation {
            title,
            theme,
            aspect_ratio,
        } => {
            let theme = match theme.as_deref() {
                Some(id) => Theme::resolve(id)
                    .ok_or_else(|| CoreError::InvalidTheme(format!("unknown theme: {id}")))?,
                None => Theme::studio(),
            };
            theme.validate()?;
            let ratio = match aspect_ratio.as_deref() {
                Some(raw) => AspectRatio::parse(raw)
                    .ok_or_else(|| CoreError::InvalidAspectRatio(raw.to_string()))?,
                None => AspectRatio::R16x9,
            };
            *core = PresentationState::new(title, theme, ratio);
            Ok(AgentOutcome {
                created: true,
                ..AgentOutcome::default()
            })
        }
        AgentCommand::UpsertSlide {
            index,
            slide_id,
            html_code,
            template,
            animation,
        } => {
            let template = template
                .as_deref()
                .map(crate::starter::template_type)
                .unwrap_or_default();
            let outcome = core.upsert_slide(index, slide_id, html_code, template, animation.unwrap_or_default())?;
            Ok(AgentOutcome {
                upsert: Some(outcome),
                ..AgentOutcome::default()
            })
        }
        AgentCommand::RemoveSlide { slide_id } => {
            core.remove_slide(&slide_id)?;
            Ok(AgentOutcome::default())
        }
        AgentCommand::MoveSlide { from, to } => {
            core.move_slide(from, to)?;
            Ok(AgentOutcome::default())
        }
        AgentCommand::TriggerAnimation {
            slide_id,
            step_index,
        } => {
            let step = core.animation_step(&slide_id, step_index)?;
            Ok(AgentOutcome {
                animation: Some(ServerEvent::Animation {
                    slide_id,
                    step_index,
                    step,
                }),
                ..AgentOutcome::default()
            })
        }
        AgentCommand::SetNotes { slide_id, notes } => {
            core.set_notes(&slide_id, notes)?;
            Ok(AgentOutcome::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn state() -> std::sync::Arc<AppState> {
        AppState::new(
            PresentationState::new("Deck", Theme::studio(), AspectRatio::R16x9),
            "tok".into(),
            PathBuf::from("ui"),
            vec![],
            "http://127.0.0.1:1".into(),
        )
    }

    #[tokio::test]
    async fn upsert_returns_the_slide_identity_for_follow_up_calls() {
        let state = state();
        let outcome = handle(
            &state,
            AgentCommand::UpsertSlide {
                index: 0,
                slide_id: None,
                html_code: "<h1>Hi</h1>".into(),
                template: Some("hero".into()),
                animation: None,
            },
        )
        .await
        .unwrap();

        let upsert = outcome.upsert.unwrap();
        assert!(upsert.inserted);
        assert_eq!(upsert.index, 0);
        assert!(upsert.slide_id.starts_with("sld_"));
    }

    #[tokio::test]
    async fn create_presentation_replaces_the_deck() {
        let state = state();
        handle(
            &state,
            AgentCommand::UpsertSlide {
                index: 0,
                slide_id: None,
                html_code: "<h1>Old</h1>".into(),
                template: None,
                animation: None,
            },
        )
        .await
        .unwrap();

        handle(
            &state,
            AgentCommand::CreatePresentation {
                title: "Fresh".into(),
                theme: Some("paper".into()),
                aspect_ratio: Some("4:3".into()),
            },
        )
        .await
        .unwrap();

        let core = state.core.read().await;
        assert_eq!(core.presentation().title, "Fresh");
        assert_eq!(core.presentation().theme.id, "paper");
        assert_eq!(core.presentation().aspect_ratio, AspectRatio::R4x3);
        assert!(core.presentation().slides.is_empty());
    }

    #[tokio::test]
    async fn unknown_theme_is_rejected_without_wiping_the_deck() {
        let state = state();
        let err = handle(
            &state,
            AgentCommand::CreatePresentation {
                title: "X".into(),
                theme: Some("neon".into()),
                aspect_ratio: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("neon"));
        assert_eq!(state.core.read().await.presentation().title, "Deck");
    }
}
