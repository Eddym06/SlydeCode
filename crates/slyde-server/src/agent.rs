//! Agent-facing command surface.
//!
//! The MCP server is a thin proxy over these commands: it never owns a second
//! copy of the deck. Everything funnels into the same `PresentationState` the
//! UI edits, so the storyboard updates live while an agent writes.

use serde::Deserialize;
use slyde_core::{AnimationStep, AspectRatio, CoreError, PresentationState, Theme, UpsertOutcome};

use crate::state::{AgentAction, AgentPhase, AppState, ServerEvent};

/// Longest deck title an agent may set; it ends up in file names and the topbar.
const MAX_TITLE_CHARS: usize = 120;

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
        /// Insertion position. Ignored when `slide_id` matches an existing slide,
        /// so agents that already know the id do not have to invent an index.
        #[serde(default)]
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
    MutateElement {
        element_id: String,
        /// Scopes the edit to one slide. Generated element handles are unique per
        /// slide document, not per deck, so an unscoped ambiguous id is rejected.
        #[serde(default)]
        slide_id: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        style: Option<String>,
        #[serde(default)]
        html: Option<String>,
    },
    SetTheme {
        #[serde(default)]
        theme_id: Option<String>,
        #[serde(default)]
        background: Option<String>,
        #[serde(default)]
        foreground: Option<String>,
        #[serde(default)]
        accent: Option<String>,
        #[serde(default)]
        muted: Option<String>,
        #[serde(default)]
        font_family: Option<String>,
    },
    /// Renames the deck without touching its slides.
    SetTitle {
        title: String,
    },
}

#[derive(Debug, Default)]
pub struct AgentOutcome {
    pub upsert: Option<UpsertOutcome>,
    pub created: bool,
    pub animation: Option<ServerEvent>,
    pub mutated_slide_id: Option<String>,
}

/// Structured descriptor of an agent command, used to announce activity before
/// and after the mutation. It deliberately carries no free-form text: the UI
/// owns the localized labels.
#[derive(Debug, Clone)]
pub struct Activity {
    pub phase: AgentPhase,
    pub action: AgentAction,
    pub slide_id: Option<String>,
    pub slide_index: Option<usize>,
}

impl Activity {
    pub fn event(&self, phase: AgentPhase) -> ServerEvent {
        ServerEvent::AgentActivity {
            phase,
            action: self.action,
            slide_id: self.slide_id.clone(),
            slide_index: self.slide_index,
        }
    }
}

impl AgentCommand {
    pub fn activity(&self) -> Activity {
        match self {
            AgentCommand::CreatePresentation { .. } => Activity {
                phase: AgentPhase::Adjusting,
                action: AgentAction::CreatePresentation,
                slide_id: None,
                slide_index: None,
            },
            AgentCommand::UpsertSlide {
                index, slide_id, ..
            } => Activity {
                phase: AgentPhase::Writing,
                action: AgentAction::UpsertSlide,
                slide_id: slide_id.clone(),
                slide_index: Some(*index),
            },
            AgentCommand::RemoveSlide { slide_id } => Activity {
                phase: AgentPhase::Adjusting,
                action: AgentAction::RemoveSlide,
                slide_id: Some(slide_id.clone()),
                slide_index: None,
            },
            AgentCommand::MoveSlide { .. } => Activity {
                phase: AgentPhase::Adjusting,
                action: AgentAction::MoveSlide,
                slide_id: None,
                slide_index: None,
            },
            AgentCommand::SetNotes { slide_id, .. } => Activity {
                phase: AgentPhase::Writing,
                action: AgentAction::SetNotes,
                slide_id: Some(slide_id.clone()),
                slide_index: None,
            },
            AgentCommand::TriggerAnimation { slide_id, .. } => Activity {
                phase: AgentPhase::Animating,
                action: AgentAction::TriggerAnimation,
                slide_id: Some(slide_id.clone()),
                slide_index: None,
            },
            AgentCommand::MutateElement { slide_id, .. } => Activity {
                phase: AgentPhase::Writing,
                action: AgentAction::MutateElement,
                slide_id: slide_id.clone(),
                slide_index: None,
            },
            AgentCommand::SetTheme { .. } => Activity {
                phase: AgentPhase::Writing,
                action: AgentAction::SetTheme,
                slide_id: None,
                slide_index: None,
            },
            AgentCommand::SetTitle { .. } => Activity {
                phase: AgentPhase::Adjusting,
                action: AgentAction::SetTitle,
                slide_id: None,
                slide_index: None,
            },
        }
    }
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
            let outcome = core.upsert_slide(
                index,
                slide_id,
                html_code,
                template,
                animation.unwrap_or_default(),
            )?;
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
        AgentCommand::MutateElement {
            element_id,
            slide_id,
            text,
            style,
            html,
        } => {
            let slide_id =
                core.mutate_element(&element_id, slide_id.as_deref(), text, style, html)?;
            Ok(AgentOutcome {
                mutated_slide_id: Some(slide_id),
                ..AgentOutcome::default()
            })
        }
        AgentCommand::SetTheme {
            theme_id,
            background,
            foreground,
            accent,
            muted,
            font_family,
        } => {
            let mut theme = match theme_id.as_deref() {
                Some(id) => Theme::resolve(id)
                    .ok_or_else(|| CoreError::InvalidTheme(format!("unknown theme: {id}")))?,
                None => core.presentation().theme.clone(),
            };
            if let Some(bg) = background {
                theme.background = bg;
            }
            if let Some(fg) = foreground {
                theme.foreground = fg;
            }
            if let Some(acc) = accent {
                theme.accent = acc;
            }
            if let Some(m) = muted {
                theme.muted = m;
            }
            if let Some(ff) = font_family {
                theme.font_family = ff;
            }
            theme.validate()?;
            core.set_theme(theme)?;
            Ok(AgentOutcome::default())
        }
        AgentCommand::SetTitle { title } => {
            // The editor may clear the title field while typing, so core stays
            // permissive; an agent has no such excuse and must name the deck.
            let title = title.trim();
            if title.is_empty() {
                return Err(CoreError::InvalidTitle("the title is empty".into()));
            }
            if title.chars().count() > MAX_TITLE_CHARS {
                return Err(CoreError::InvalidTitle(format!(
                    "the title is over {MAX_TITLE_CHARS} characters"
                )));
            }
            core.set_title(title)?;
            Ok(AgentOutcome::default())
        }
    }
}

/// Executes a command and publishes every observable side effect.
///
/// Both the MCP bridge (`/api/agent`) and the copilot use this entry point, so
/// they cannot drift in event order or persistence behavior.
pub async fn run(state: &AppState, command: AgentCommand) -> Result<AgentOutcome, CoreError> {
    let mut activity = command.activity();
    state.broadcast(activity.event(activity.phase));

    match handle(state, command).await {
        Ok(outcome) => {
            if let Some(upsert) = &outcome.upsert {
                activity.slide_id = Some(upsert.slide_id.clone());
                activity.slide_index = Some(upsert.index);
            }
            if let Some(event) = outcome.animation.clone() {
                state.broadcast(event);
            }
            state.publish_state().await;
            state.broadcast(activity.event(AgentPhase::Done));
            Ok(outcome)
        }
        Err(error) => {
            state.broadcast(activity.event(AgentPhase::Error));
            Err(error)
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

    #[tokio::test]
    async fn set_title_renames_the_deck_and_keeps_its_slides() {
        let state = state();
        handle(
            &state,
            AgentCommand::UpsertSlide {
                index: 0,
                slide_id: Some("sld_a".into()),
                html_code: "<h1>Hi</h1>".into(),
                template: None,
                animation: None,
            },
        )
        .await
        .unwrap();
        handle(
            &state,
            AgentCommand::SetTitle {
                title: "  Energía solar  ".into(),
            },
        )
        .await
        .unwrap();
        let core = state.core.read().await;
        assert_eq!(core.presentation().title, "Energía solar");
        assert_eq!(core.presentation().slides.len(), 1);
    }

    #[tokio::test]
    async fn set_title_refuses_empty_and_oversized_titles() {
        let state = state();
        for bad in ["   ".to_string(), "x".repeat(MAX_TITLE_CHARS + 1)] {
            let err = handle(&state, AgentCommand::SetTitle { title: bad })
                .await
                .unwrap_err();
            assert!(matches!(err, CoreError::InvalidTitle(_)), "{err}");
        }
        assert_eq!(state.core.read().await.presentation().title, "Deck");
        let ok = "x".repeat(MAX_TITLE_CHARS);
        handle(&state, AgentCommand::SetTitle { title: ok.clone() })
            .await
            .unwrap();
        assert_eq!(state.core.read().await.presentation().title, ok);
    }

    #[test]
    fn commands_map_to_structured_activity() {
        let cases = [
            (
                AgentCommand::CreatePresentation {
                    title: "Deck".into(),
                    theme: None,
                    aspect_ratio: None,
                },
                AgentPhase::Adjusting,
                AgentAction::CreatePresentation,
                None,
                None,
            ),
            (
                AgentCommand::UpsertSlide {
                    index: 2,
                    slide_id: Some("sld_a".into()),
                    html_code: "<h1>Hi</h1>".into(),
                    template: None,
                    animation: None,
                },
                AgentPhase::Writing,
                AgentAction::UpsertSlide,
                Some("sld_a"),
                Some(2),
            ),
            (
                AgentCommand::RemoveSlide {
                    slide_id: "sld_a".into(),
                },
                AgentPhase::Adjusting,
                AgentAction::RemoveSlide,
                Some("sld_a"),
                None,
            ),
            (
                AgentCommand::MoveSlide { from: 0, to: 1 },
                AgentPhase::Adjusting,
                AgentAction::MoveSlide,
                None,
                None,
            ),
            (
                AgentCommand::SetNotes {
                    slide_id: "sld_a".into(),
                    notes: Some("Hi".into()),
                },
                AgentPhase::Writing,
                AgentAction::SetNotes,
                Some("sld_a"),
                None,
            ),
            (
                AgentCommand::TriggerAnimation {
                    slide_id: "sld_a".into(),
                    step_index: 0,
                },
                AgentPhase::Animating,
                AgentAction::TriggerAnimation,
                Some("sld_a"),
                None,
            ),
        ];

        for (command, phase, action, slide_id, slide_index) in cases {
            let activity = command.activity();
            assert_eq!(activity.phase, phase);
            assert_eq!(activity.action, action);
            assert_eq!(activity.slide_id.as_deref(), slide_id);
            assert_eq!(activity.slide_index, slide_index);
        }
    }
}
