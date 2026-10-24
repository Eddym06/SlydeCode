//! Concurrent-safe presentation state with snapshot history.
//!
//! Every mutation clones the current presentation, applies the change and commits
//! it as a new history entry. Decks are small (a few hundred KB at most), so the
//! clone is cheap and the history model stays trivially correct.

use crate::error::CoreError;
use crate::model::{
    AnimationStep, AspectRatio, Presentation, Slide, SlideId, StateView, TemplateType, Theme,
    now_ms,
};

pub const DEFAULT_MAX_HISTORY: usize = 100;
pub const DEFAULT_MAX_SLIDES: usize = 200;
pub const DEFAULT_MAX_CODE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_history: usize,
    pub max_slides: usize,
    pub max_code_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_history: DEFAULT_MAX_HISTORY,
            max_slides: DEFAULT_MAX_SLIDES,
            max_code_bytes: DEFAULT_MAX_CODE_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub label: String,
    pub presentation: Presentation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UpsertOutcome {
    pub index: usize,
    pub slide_id: SlideId,
    /// `true` when the slide was inserted, `false` when an existing one was updated.
    pub inserted: bool,
}

#[derive(Debug, Clone)]
pub struct PresentationState {
    history: Vec<HistoryEntry>,
    cursor: usize,
    revision: u64,
    limits: Limits,
}

impl PresentationState {
    pub fn new(title: impl Into<String>, theme: Theme, aspect_ratio: AspectRatio) -> Self {
        Self::with_limits(title, theme, aspect_ratio, Limits::default())
    }

    pub fn with_limits(
        title: impl Into<String>,
        theme: Theme,
        aspect_ratio: AspectRatio,
        limits: Limits,
    ) -> Self {
        let presentation = Presentation::new(title, theme, aspect_ratio);
        Self {
            history: vec![HistoryEntry {
                label: "init".into(),
                presentation,
            }],
            cursor: 0,
            revision: 0,
            limits,
        }
    }

    pub fn presentation(&self) -> &Presentation {
        &self.history[self.cursor].presentation
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }

    pub fn history_labels(&self) -> Vec<&str> {
        self.history.iter().map(|e| e.label.as_str()).collect()
    }

    pub fn view(&self) -> StateView {
        StateView {
            presentation: self.presentation().clone(),
            revision: self.revision,
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            history_len: self.history.len(),
        }
    }

    /// Replaces the whole deck, as when a project file is opened.
    ///
    /// History restarts: undoing into the deck that was just replaced would be
    /// confusing, and could silently resurrect work the user believed gone.
    pub fn replace(&mut self, presentation: Presentation) -> Result<(), CoreError> {
        presentation.theme.validate()?;
        self.history = vec![HistoryEntry {
            label: "open".into(),
            presentation,
        }];
        self.cursor = 0;
        self.revision += 1;
        Ok(())
    }

    pub fn undo(&mut self) -> Result<(), CoreError> {
        if !self.can_undo() {
            return Err(CoreError::NothingToUndo);
        }
        self.cursor -= 1;
        self.revision += 1;
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), CoreError> {
        if !self.can_redo() {
            return Err(CoreError::NothingToRedo);
        }
        self.cursor += 1;
        self.revision += 1;
        Ok(())
    }

    pub fn set_title(&mut self, title: impl Into<String>) -> Result<(), CoreError> {
        let title = title.into();
        let mut next = self.presentation().clone();
        next.title = title;
        self.commit("title", next);
        Ok(())
    }

    pub fn set_theme(&mut self, theme: Theme) -> Result<(), CoreError> {
        theme.validate()?;
        let mut next = self.presentation().clone();
        next.theme = theme;
        self.commit("theme", next);
        Ok(())
    }

    pub fn set_aspect_ratio(&mut self, aspect_ratio: AspectRatio) -> Result<(), CoreError> {
        let mut next = self.presentation().clone();
        next.aspect_ratio = aspect_ratio;
        self.commit("aspect_ratio", next);
        Ok(())
    }

    pub fn set_notes(&mut self, slide_id: &str, notes: Option<String>) -> Result<(), CoreError> {
        let mut next = self.presentation().clone();
        let slide = next
            .slides
            .iter_mut()
            .find(|s| s.id == slide_id)
            .ok_or_else(|| CoreError::SlideNotFound(slide_id.to_string()))?;
        slide.notes = notes;
        self.commit("notes", next);
        Ok(())
    }

    /// Insert a new slide or update an existing one.
    ///
    /// `slide_id` is authoritative: if it matches an existing slide, that slide is
    /// updated in place and `index` is ignored (the returned index tells the caller
    /// where it actually lives). If the id is new, the slide is inserted at `index`
    /// (clamped to the end). Reusing an id makes agent retries idempotent.
    pub fn upsert_slide(
        &mut self,
        index: usize,
        slide_id: Option<SlideId>,
        code: String,
        template: TemplateType,
        animation: Vec<AnimationStep>,
    ) -> Result<UpsertOutcome, CoreError> {
        if code.trim().is_empty() {
            return Err(CoreError::EmptySlideCode);
        }
        if code.len() > self.limits.max_code_bytes {
            return Err(CoreError::CodeTooLarge {
                actual: code.len(),
                max: self.limits.max_code_bytes,
            });
        }

        let mut next = self.presentation().clone();

        if let Some(existing_index) = slide_id.as_deref().and_then(|id| next.slide_index(id)) {
            let slide = &mut next.slides[existing_index];
            slide.code = code;
            slide.template = template;
            slide.animation = animation;
            let id = slide.id.clone();
            self.commit("update_slide", next);
            return Ok(UpsertOutcome {
                index: existing_index,
                slide_id: id,
                inserted: false,
            });
        }

        if next.slides.len() >= self.limits.max_slides {
            return Err(CoreError::SlideLimit {
                max: self.limits.max_slides,
            });
        }
        if index > next.slides.len() {
            return Err(CoreError::IndexOutOfRange {
                index,
                len: next.slides.len(),
            });
        }

        let id = slide_id.unwrap_or_else(new_slide_id);
        if next.slide(&id).is_some() {
            return Err(CoreError::DuplicateSlideId(id));
        }

        let mut slide = Slide::new(id.clone(), code);
        slide.template = template;
        slide.animation = animation;
        next.slides.insert(index, slide);
        self.commit("insert_slide", next);

        Ok(UpsertOutcome {
            index,
            slide_id: id,
            inserted: true,
        })
    }

    pub fn remove_slide(&mut self, slide_id: &str) -> Result<usize, CoreError> {
        let mut next = self.presentation().clone();
        let index = next
            .slide_index(slide_id)
            .ok_or_else(|| CoreError::SlideNotFound(slide_id.to_string()))?;
        next.slides.remove(index);
        self.commit("remove_slide", next);
        Ok(index)
    }

    pub fn move_slide(&mut self, from: usize, to: usize) -> Result<usize, CoreError> {
        let mut next = self.presentation().clone();
        let len = next.slides.len();
        if from >= len {
            return Err(CoreError::IndexOutOfRange { index: from, len });
        }
        if to >= len {
            return Err(CoreError::IndexOutOfRange { index: to, len });
        }
        if from == to {
            return Ok(to);
        }
        let slide = next.slides.remove(from);
        next.slides.insert(to, slide);
        self.commit("move_slide", next);
        Ok(to)
    }

    /// Read-only lookup used by the server to answer `trigger_animation`.
    pub fn animation_step(
        &self,
        slide_id: &str,
        step_index: usize,
    ) -> Result<AnimationStep, CoreError> {
        let slide = self
            .presentation()
            .slide(slide_id)
            .ok_or_else(|| CoreError::SlideNotFound(slide_id.to_string()))?;
        slide
            .animation
            .get(step_index)
            .cloned()
            .ok_or_else(|| CoreError::AnimationStepNotFound {
                slide_id: slide_id.to_string(),
                index: step_index,
            })
    }

    fn commit(&mut self, label: &str, next: Presentation) {
        self.revision += 1;
        let mut next = next;
        next.updated_at_ms = now_ms();

        // Any new mutation invalidates the redo branch.
        self.history.truncate(self.cursor + 1);
        self.history.push(HistoryEntry {
            label: label.into(),
            presentation: next,
        });
        self.cursor = self.history.len() - 1;

        if self.history.len() > self.limits.max_history {
            let excess = self.history.len() - self.limits.max_history;
            self.history.drain(0..excess);
            self.cursor = self.cursor.saturating_sub(excess);
        }
    }
}

pub fn new_slide_id() -> SlideId {
    format!("sld_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AnimationKind, Author};

    fn state() -> PresentationState {
        PresentationState::new("Deck", Theme::studio(), AspectRatio::R16x9)
    }

    fn step(target: &str) -> AnimationStep {
        AnimationStep {
            target: target.into(),
            kind: AnimationKind::FadeUp,
            delay_ms: 0,
            duration_ms: 400,
            easing: "ease-out".into(),
        }
    }

    #[test]
    fn starts_empty_with_no_history_to_undo() {
        let s = state();
        assert!(s.presentation().slides.is_empty());
        assert!(!s.can_undo());
        assert!(!s.can_redo());
        assert_eq!(s.revision(), 0);
    }

    #[test]
    fn inserts_at_index_and_keeps_order() {
        let mut s = state();
        s.upsert_slide(0, Some("a".into()), "<h1>A</h1>".into(), TemplateType::Hero, vec![])
            .unwrap();
        s.upsert_slide(0, Some("b".into()), "<h1>B</h1>".into(), TemplateType::Bullet, vec![])
            .unwrap();
        let ids: Vec<_> = s.presentation().slides.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);
        assert_eq!(s.revision(), 2);
    }

    #[test]
    fn appends_when_index_equals_len() {
        let mut s = state();
        s.upsert_slide(0, None, "<h1>A</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        let outcome = s
            .upsert_slide(1, None, "<h1>B</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        assert!(outcome.inserted);
        assert_eq!(outcome.index, 1);
        assert_eq!(s.presentation().slides.len(), 2);
    }

    #[test]
    fn updating_by_id_ignores_index_and_is_idempotent() {
        let mut s = state();
        s.upsert_slide(0, Some("a".into()), "<h1>A</h1>".into(), TemplateType::Hero, vec![])
            .unwrap();
        let outcome = s
            .upsert_slide(7, Some("a".into()), "<h1>A2</h1>".into(), TemplateType::Chart, vec![step(".t")])
            .unwrap();
        assert!(!outcome.inserted);
        assert_eq!(outcome.index, 0);
        assert_eq!(s.presentation().slides.len(), 1);
        let slide = &s.presentation().slides[0];
        assert_eq!(slide.code, "<h1>A2</h1>");
        assert_eq!(slide.template, TemplateType::Chart);
        assert_eq!(slide.animation.len(), 1);
        assert_eq!(slide.generated_by, Author::Ai);
    }

    #[test]
    fn rejects_out_of_range_index() {
        let mut s = state();
        let err = s
            .upsert_slide(5, None, "<h1>X</h1>".into(), TemplateType::Code, vec![])
            .unwrap_err();
        assert_eq!(err, CoreError::IndexOutOfRange { index: 5, len: 0 });
    }

    #[test]
    fn rejects_empty_and_oversized_code() {
        let mut s = state();
        assert_eq!(
            s.upsert_slide(0, None, "   ".into(), TemplateType::Code, vec![])
                .unwrap_err(),
            CoreError::EmptySlideCode
        );

        let mut small = PresentationState::with_limits(
            "Deck",
            Theme::studio(),
            AspectRatio::R16x9,
            Limits {
                max_code_bytes: 8,
                ..Limits::default()
            },
        );
        let err = small
            .upsert_slide(0, None, "0123456789".into(), TemplateType::Code, vec![])
            .unwrap_err();
        assert!(matches!(err, CoreError::CodeTooLarge { actual: 10, max: 8 }));
    }

    #[test]
    fn enforces_slide_limit() {
        let mut s = PresentationState::with_limits(
            "Deck",
            Theme::studio(),
            AspectRatio::R16x9,
            Limits {
                max_slides: 1,
                ..Limits::default()
            },
        );
        s.upsert_slide(0, None, "<h1>A</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        let err = s
            .upsert_slide(1, None, "<h1>B</h1>".into(), TemplateType::Code, vec![])
            .unwrap_err();
        assert_eq!(err, CoreError::SlideLimit { max: 1 });
    }

    #[test]
    fn undo_and_redo_restore_content_and_bump_revision() {
        let mut s = state();
        s.upsert_slide(0, Some("a".into()), "<h1>A</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        let after_insert = s.revision();
        s.set_title("Renamed").unwrap();

        s.undo().unwrap();
        assert_eq!(s.presentation().title, "Deck");
        assert_eq!(s.presentation().slides.len(), 1);
        assert!(s.revision() > after_insert);

        s.undo().unwrap();
        assert!(s.presentation().slides.is_empty());
        assert!(!s.can_undo());

        s.redo().unwrap();
        assert_eq!(s.presentation().slides.len(), 1);
        s.redo().unwrap();
        assert_eq!(s.presentation().title, "Renamed");
        assert!(!s.can_redo());
    }

    #[test]
    fn new_mutation_clears_the_redo_branch() {
        let mut s = state();
        s.upsert_slide(0, Some("a".into()), "<h1>A</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        s.upsert_slide(1, Some("b".into()), "<h1>B</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        s.undo().unwrap();
        assert!(s.can_redo());

        s.set_title("Diverged").unwrap();
        assert!(!s.can_redo());
        assert_eq!(s.presentation().slides.len(), 1);
        assert_eq!(s.presentation().title, "Diverged");
    }

    #[test]
    fn history_is_capped_and_cursor_stays_valid() {
        let mut s = PresentationState::with_limits(
            "Deck",
            Theme::studio(),
            AspectRatio::R16x9,
            Limits {
                max_history: 3,
                ..Limits::default()
            },
        );
        for i in 0..10 {
            s.set_title(format!("t{i}")).unwrap();
        }
        assert_eq!(s.view().history_len, 3);
        assert_eq!(s.presentation().title, "t9");
        s.undo().unwrap();
        assert_eq!(s.presentation().title, "t8");
        s.undo().unwrap();
        assert_eq!(s.presentation().title, "t7");
        assert!(!s.can_undo());
    }

    #[test]
    fn move_reorders_slides() {
        let mut s = state();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            s.upsert_slide(i, Some((*id).into()), format!("<h1>{id}</h1>"), TemplateType::Code, vec![])
                .unwrap();
        }
        s.move_slide(0, 2).unwrap();
        let ids: Vec<_> = s.presentation().slides.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
        assert!(matches!(
            s.move_slide(0, 9),
            Err(CoreError::IndexOutOfRange { .. })
        ));
    }

    #[test]
    fn remove_then_undo_restores_the_slide_at_its_original_index() {
        let mut s = state();
        for (i, id) in ["a", "b"].iter().enumerate() {
            s.upsert_slide(i, Some((*id).into()), format!("<h1>{id}</h1>"), TemplateType::Code, vec![])
                .unwrap();
        }
        assert_eq!(s.remove_slide("a").unwrap(), 0);
        assert_eq!(s.presentation().slides.len(), 1);
        s.undo().unwrap();
        assert_eq!(s.presentation().slides[0].id, "a");
    }

    #[test]
    fn animation_steps_are_readable_by_index() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("a".into()),
            "<h1>A</h1>".into(),
            TemplateType::Hero,
            vec![step(".title"), step(".bars")],
        )
        .unwrap();
        assert_eq!(s.animation_step("a", 1).unwrap().target, ".bars");
        assert!(matches!(
            s.animation_step("a", 2),
            Err(CoreError::AnimationStepNotFound { .. })
        ));
        assert!(matches!(
            s.animation_step("missing", 0),
            Err(CoreError::SlideNotFound(_))
        ));
    }

    #[test]
    fn invalid_theme_is_rejected_before_commit() {
        let mut s = state();
        let mut theme = Theme::studio();
        theme.accent = "javascript:alert(1)".into();
        assert!(s.set_theme(theme).is_err());
        assert!(!s.can_undo());
        assert_eq!(s.revision(), 0);
    }

    #[test]
    fn replace_swaps_the_deck_and_restarts_history() {
        let mut s = state();
        s.upsert_slide(0, None, "<h1>A</h1>".into(), TemplateType::Code, vec![])
            .unwrap();
        assert!(s.can_undo());

        let mut loaded = Presentation::new("Loaded", Theme::paper(), AspectRatio::R4x3);
        loaded.slides.push(Slide::new("sld_1".into(), "<h1>L</h1>".into()));
        s.replace(loaded).unwrap();

        assert_eq!(s.presentation().title, "Loaded");
        assert_eq!(s.presentation().slides.len(), 1);
        assert!(!s.can_undo());
        assert!(!s.can_redo());
        assert_eq!(s.history_labels(), vec!["open"]);
        assert_eq!(s.revision(), 2);
    }

    #[test]
    fn replace_rejects_an_invalid_theme_without_touching_the_deck() {
        let mut s = state();
        let mut incoming = Presentation::new("Bad", Theme::studio(), AspectRatio::R16x9);
        incoming.theme.background = "url(evil)".into();
        assert!(s.replace(incoming).is_err());
        assert_eq!(s.presentation().title, "Deck");
        assert_eq!(s.revision(), 0);
    }
}
