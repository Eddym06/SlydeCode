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
/// How long a continued edit (title, speaker notes) keeps rewriting the same
/// history entry instead of pushing a new one.
pub const DEFAULT_COALESCE_WINDOW_MS: u64 = 1200;
/// A slide reveals in a handful of beats, never in a parade of effects.
pub const MAX_ANIMATION_STEPS: usize = 24;
pub const MAX_ANIMATION_DURATION_MS: u32 = 5000;
pub const MAX_ANIMATION_DELAY_MS: u32 = 10000;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_history: usize,
    pub max_slides: usize,
    pub max_code_bytes: usize,
    /// Window in which consecutive commits with the same label and target merge
    /// into one history entry. Typing in the title or the speaker notes commits
    /// every few hundred milliseconds; without coalescing a single paragraph
    /// buries every real edit in the undo stack. `0` disables coalescing.
    pub coalesce_window_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_history: DEFAULT_MAX_HISTORY,
            max_slides: DEFAULT_MAX_SLIDES,
            max_code_bytes: DEFAULT_MAX_CODE_BYTES,
            coalesce_window_ms: DEFAULT_COALESCE_WINDOW_MS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub label: String,
    pub presentation: Presentation,
    /// Target of the edit (`deck` for the title, `notes:<slide id>` for notes).
    /// Only entries with the same key can merge.
    pub coalesce_key: Option<String>,
    pub committed_at_ms: u64,
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
                coalesce_key: None,
                committed_at_ms: now_ms(),
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
            coalesce_key: None,
            committed_at_ms: now_ms(),
        }];
        self.cursor = 0;
        self.revision += 1;
        Ok(())
    }

    /// Starts a fresh deck with a single slide.
    ///
    /// The previous deck stays one step back in history: "new presentation" is
    /// one click away from a deck the user spent an hour on, and losing that to
    /// a stray confirmation is not worth a tidier history.
    pub fn start_new_deck(
        &mut self,
        title: impl Into<String>,
        code: String,
        template: TemplateType,
        animation: Vec<AnimationStep>,
    ) -> Result<(), CoreError> {
        if code.trim().is_empty() {
            return Err(CoreError::EmptySlideCode);
        }
        if code.len() > self.limits.max_code_bytes {
            return Err(CoreError::CodeTooLarge {
                actual: code.len(),
                max: self.limits.max_code_bytes,
            });
        }
        validate_animation(&animation)?;

        let code = crate::html::ensure_element_ids(&code);
        let current = self.presentation();
        let theme = current.theme.clone();
        let aspect_ratio = current.aspect_ratio;
        let mut presentation = Presentation::new(title, theme, aspect_ratio);
        let mut slide = Slide::new(new_slide_id(), code);
        slide.template = template;
        slide.animation = animation;
        presentation.slides.push(slide);

        self.commit("new_deck", presentation);
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
        self.commit_with("title", Some("deck".into()), next);
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
        self.commit_with("notes", Some(format!("notes:{slide_id}")), next);
        Ok(())
    }

    /// Mutates an element identified by `element_id` directly or updates its occurrence inside HTML code.
    ///
    /// `slide_id` scopes the edit to one slide. Generated handles are only unique
    /// *within* a slide document (`slyde-1` exists in every slide), so an
    /// unscoped id that matches more than one slide is rejected with
    /// [`CoreError::AmbiguousElement`] instead of editing the first match.
    pub fn mutate_element(
        &mut self,
        element_id: &str,
        slide_id: Option<&str>,
        text: Option<String>,
        style: Option<String>,
        html: Option<String>,
    ) -> Result<String, CoreError> {
        let mut next = self.presentation().clone();

        if let Some(slide_id) = slide_id {
            if next.slide(slide_id).is_none() {
                return Err(CoreError::SlideNotFound(slide_id.to_string()));
            }
        }

        // Resolve every candidate before mutating anything: an ambiguous id must
        // fail without leaving a half-edited deck behind.
        let candidates: Vec<usize> = next
            .slides
            .iter()
            .enumerate()
            .filter(|(_, slide)| slide_id.is_none_or(|id| slide.id == id))
            .filter(|(_, slide)| {
                slide.find_element(element_id).is_some()
                    || find_element_span(&slide.code, element_id).is_some()
            })
            .map(|(index, _)| index)
            .collect();

        let index = match candidates.len() {
            0 => {
                return Err(CoreError::ElementNotFound {
                    element_id: element_id.to_string(),
                });
            }
            1 => candidates[0],
            // A scoped lookup can only land in one slide; several matches mean the
            // caller did not scope the edit.
            _ => {
                return Err(CoreError::AmbiguousElement {
                    element_id: element_id.to_string(),
                    slides: candidates.len(),
                });
            }
        };

        let slide = &mut next.slides[index];
        let slide_id = slide.id.clone();

        if let Some(el) = slide.find_element_mut(element_id) {
            if let Some(t) = &text {
                match el {
                    crate::model::SlideElement::Diagram { source, .. } => *source = t.clone(),
                    crate::model::SlideElement::CodeBlock { code, .. } => *code = t.clone(),
                    crate::model::SlideElement::SpeakerNotes { notes, .. } => *notes = t.clone(),
                    crate::model::SlideElement::Custom { content, .. } => *content = t.clone(),
                    crate::model::SlideElement::Image { caption, .. } => *caption = Some(t.clone()),
                }
            }
        }

        // Also mutate the matching node inside the slide HTML. The id can be
        // declared as `id="…"` or `data-id="…"`.
        if let Some(span) = find_element_span(&slide.code, element_id) {
            if span.inner.is_none() && (html.is_some() || text.is_some()) {
                return Err(CoreError::ElementHasNoContent {
                    element_id: element_id.to_string(),
                });
            }
            let source = &slide.code;
            let open_tag = merge_style_attribute(&source[span.open.clone()], style.as_deref());
            let new_inner = match (&html, &text) {
                (Some(markup), _) => Some(markup.clone()),
                (None, Some(plain)) => Some(escape_text(plain)),
                (None, None) => None,
            };
            let mut out = String::with_capacity(source.len() + 32);
            out.push_str(&source[..span.open.start]);
            out.push_str(&open_tag);
            match (&span.inner, new_inner) {
                (Some(inner), Some(inner_html)) => {
                    out.push_str(&source[span.open.end..inner.start]);
                    out.push_str(&inner_html);
                    out.push_str(&source[inner.end..]);
                }
                _ => out.push_str(&source[span.open.end..]),
            }
            slide.code = out;
        }

        self.commit("mutate_element", next);
        Ok(slide_id)
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
        // Every block gets a persistent handle so the canvas and the tools agree
        // on what "this element" means.
        let code = crate::html::ensure_element_ids(&code);
        validate_animation(&animation)?;

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

    /// Replaces every animation step on a slide, as edited in the inspector.
    pub fn set_animation(
        &mut self,
        slide_id: &str,
        steps: Vec<AnimationStep>,
    ) -> Result<(), CoreError> {
        let mut next = self.presentation().clone();
        let slide = next
            .slides
            .iter_mut()
            .find(|slide| slide.id == slide_id)
            .ok_or_else(|| CoreError::SlideNotFound(slide_id.to_string()))?;
        validate_animation(&steps)?;
        slide.animation = steps;
        self.commit("set_animation", next);
        Ok(())
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
        self.commit_with(label, None, next);
    }

    /// Commits a mutation, merging it into the previous entry when it continues
    /// the same edit (`key`) within `limits.coalesce_window_ms`.
    ///
    /// Merging still bumps the revision: the UI has to redraw every time; it just
    /// should not need one undo step per keystroke.
    fn commit_with(&mut self, label: &str, key: Option<String>, next: Presentation) {
        self.revision += 1;
        let mut next = next;
        next.updated_at_ms = now_ms();

        // Any new mutation invalidates the redo branch.
        let at_tip = self.cursor + 1 == self.history.len();
        if at_tip {
            if let (Some(key), Some(top)) = (key.as_deref(), self.history.get(self.cursor)) {
                let same_edit = top.label == label
                    && top.coalesce_key.as_deref() == Some(key)
                    && self.limits.coalesce_window_ms > 0
                    && now_ms().saturating_sub(top.committed_at_ms)
                        <= self.limits.coalesce_window_ms;
                if same_edit {
                    self.history[self.cursor] = HistoryEntry {
                        label: label.into(),
                        presentation: next,
                        coalesce_key: Some(key.to_string()),
                        committed_at_ms: now_ms(),
                    };
                    return;
                }
            }
        }

        self.history.truncate(self.cursor + 1);
        self.history.push(HistoryEntry {
            label: label.into(),
            presentation: next,
            coalesce_key: key,
            committed_at_ms: now_ms(),
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

/// Shared guard for authored and hand-edited timelines.
pub fn validate_animation(steps: &[AnimationStep]) -> Result<(), CoreError> {
    if steps.len() > MAX_ANIMATION_STEPS {
        return Err(CoreError::InvalidAnimation(format!(
            "at most {MAX_ANIMATION_STEPS} steps per slide"
        )));
    }
    for (index, step) in steps.iter().enumerate() {
        if step.target.trim().is_empty() {
            return Err(CoreError::InvalidAnimation(format!(
                "step {} needs a target selector",
                index + 1
            )));
        }
        if step.duration_ms == 0 || step.duration_ms > MAX_ANIMATION_DURATION_MS {
            return Err(CoreError::InvalidAnimation(format!(
                "step {} duration must be between 1 and {MAX_ANIMATION_DURATION_MS} ms",
                index + 1
            )));
        }
        if step.delay_ms > MAX_ANIMATION_DELAY_MS {
            return Err(CoreError::InvalidAnimation(format!(
                "step {} delay cannot exceed {MAX_ANIMATION_DELAY_MS} ms",
                index + 1
            )));
        }
        if step.easing.trim().is_empty() {
            return Err(CoreError::InvalidAnimation(format!(
                "step {} needs an easing curve",
                index + 1
            )));
        }
    }
    Ok(())
}

/// Byte ranges of an element's opening tag and, when it has content, its inner
/// HTML.
struct ElementSpan {
    /// `<tag …>` including both angle brackets.
    open: std::ops::Range<usize>,
    /// Content between the opening and the matching closing tag.
    inner: Option<std::ops::Range<usize>>,
}

/// Locates `<tag id="…">…</tag>` (or `data-id="…"`) in a slide document.
///
/// The scan balances same-name tags, so nested markup is replaced as a whole.
/// Cutting at the first `</` — the naive approach — turns
/// `<h1 id="t">A <em>B</em></h1>` into `<h1 id="t">New</em></h1>`.
fn find_element_span(code: &str, element_id: &str) -> Option<ElementSpan> {
    let needles = [
        format!("id=\"{element_id}\""),
        format!("data-id=\"{element_id}\""),
    ];
    let marker = needles
        .iter()
        .filter_map(|needle| code.find(needle.as_str()))
        .min()?;
    let open_start = code[..marker].rfind('<')?;

    let name: String = code[open_start + 1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect();
    if name.is_empty() {
        // A comment or a doctype, not an element.
        return None;
    }

    let open_end = find_tag_end(code, open_start)?;
    let open = open_start..open_end;
    if is_self_closing(&code[open_start..open_end]) {
        return Some(ElementSpan { open, inner: None });
    }

    // Case-insensitive search for balanced tags. ASCII lowercasing preserves
    // byte offsets, so the ranges stay valid for the original source.
    let lower = code.to_ascii_lowercase();
    let open_needle = format!("<{name}");
    let close_needle = format!("</{name}");
    let mut depth = 1usize;
    let mut cursor = open_end;

    while cursor < code.len() {
        let next_open = lower[cursor..].find(&open_needle).map(|i| cursor + i);
        let next_close = lower[cursor..].find(&close_needle).map(|i| cursor + i);

        let Some(close) = next_close else {
            // Void element (`<img id="…">`) or an unclosed tag: no content.
            return Some(ElementSpan { open, inner: None });
        };

        match next_open {
            Some(open_at) if open_at < close => {
                if !is_tag_name_end(&lower, open_at + open_needle.len()) {
                    cursor = open_at + open_needle.len();
                    continue;
                }
                let tag_end = find_tag_end(code, open_at)?;
                if !is_self_closing(&code[open_at..tag_end]) {
                    depth += 1;
                }
                cursor = tag_end;
            }
            _ => {
                if !is_tag_name_end(&lower, close + close_needle.len()) {
                    cursor = close + close_needle.len();
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(ElementSpan {
                        open,
                        inner: Some(open_end..close),
                    });
                }
                cursor = close + close_needle.len();
            }
        }
    }

    Some(ElementSpan { open, inner: None })
}

fn is_self_closing(tag: &str) -> bool {
    tag.trim_end_matches('>').trim_end().ends_with('/')
}

/// True when the byte after a tag name cannot continue that name.
fn is_tag_name_end(lower: &str, index: usize) -> bool {
    match lower.as_bytes().get(index) {
        None => true,
        Some(byte) => {
            !(byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b':' || *byte == b'_')
        }
    }
}

/// Index just past the `>` closing the tag that starts at `start`, ignoring
/// `>` characters inside quoted attribute values.
fn find_tag_end(code: &str, start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, ch) in code[start..].char_indices() {
        match (quote, ch) {
            (None, '"') | (None, '\'') => quote = Some(ch),
            (Some(open), current) if current == open => quote = None,
            (None, '>') => return Some(start + offset + ch.len_utf8()),
            _ => {}
        }
    }
    None
}

/// Merges `style` into the opening tag, keeping declarations it already has.
fn merge_style_attribute(open_tag: &str, style: Option<&str>) -> String {
    let Some(style) = style else {
        return open_tag.to_string();
    };
    let style = style.trim().trim_end_matches(';').trim();
    if style.is_empty() {
        return open_tag.to_string();
    }

    for quote in ['"', '\''] {
        let needle = format!("style={quote}");
        if let Some(position) = open_tag.find(&needle) {
            let insert_at = position + needle.len();
            let mut merged = String::with_capacity(open_tag.len() + style.len() + 2);
            merged.push_str(&open_tag[..insert_at]);
            merged.push_str(style);
            merged.push_str("; ");
            merged.push_str(&open_tag[insert_at..]);
            return merged;
        }
    }

    match open_tag.rfind('>') {
        Some(position) => {
            let mut merged = String::with_capacity(open_tag.len() + style.len() + 10);
            merged.push_str(open_tag[..position].trim_end());
            merged.push_str(&format!(" style=\"{style}\">"));
            merged
        }
        None => open_tag.to_string(),
    }
}

/// Escapes plain text so `text` never injects markup; use `html` for markup.
fn escape_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
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
    fn start_new_deck_drops_every_slide_and_stays_undoable() {
        let mut s = state();
        s.upsert_slide(0, None, "<h1>A</h1>".into(), TemplateType::Hero, vec![])
            .unwrap();
        s.upsert_slide(1, None, "<h1>B</h1>".into(), TemplateType::Bullet, vec![])
            .unwrap();
        assert_eq!(s.presentation().slides.len(), 2);

        s.start_new_deck(
            "Fresh deck",
            "<h1>Start</h1>".into(),
            TemplateType::Hero,
            vec![step("h1")],
        )
        .unwrap();

        let deck = s.presentation();
        assert_eq!(deck.title, "Fresh deck");
        assert_eq!(deck.slides.len(), 1, "the previous slides are gone");
        assert!(deck.slides[0].code.contains("Start"));
        assert_eq!(deck.slides[0].animation.len(), 1);
        assert!(s.can_undo(), "a stray confirmation must be recoverable");

        s.undo().unwrap();
        assert_eq!(s.presentation().slides.len(), 2);
        assert_eq!(s.presentation().title, "Deck");
    }

    #[test]
    fn start_new_deck_rejects_empty_code() {
        let mut s = state();
        assert!(
            s.start_new_deck("Nope", "   ".into(), TemplateType::Hero, vec![])
                .is_err()
        );
        assert_eq!(
            s.presentation().title,
            "Deck",
            "a rejected reset changes nothing"
        );
    }

    #[test]
    fn set_animation_replaces_reorders_and_validates_steps() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<h1 class=\"title\">A</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();

        s.set_animation("sld1", vec![step(".title"), step(".sub")])
            .unwrap();
        assert_eq!(s.presentation().slides[0].animation.len(), 2);
        assert!(s.can_undo());

        // Reordering is just a new array.
        s.set_animation("sld1", vec![step(".sub"), step(".title")])
            .unwrap();
        assert_eq!(s.presentation().slides[0].animation[0].target, ".sub");

        s.undo().unwrap();
        assert_eq!(s.presentation().slides[0].animation[0].target, ".title");

        let mut broken = step(".title");
        broken.target = "   ".into();
        assert!(matches!(
            s.set_animation("sld1", vec![broken]),
            Err(CoreError::InvalidAnimation(_))
        ));

        let too_long = AnimationStep {
            duration_ms: MAX_ANIMATION_DURATION_MS + 1,
            ..step(".title")
        };
        assert!(matches!(
            s.set_animation("sld1", vec![too_long]),
            Err(CoreError::InvalidAnimation(_))
        ));

        assert!(matches!(
            s.set_animation("missing", vec![]),
            Err(CoreError::SlideNotFound(_))
        ));
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
        s.upsert_slide(
            0,
            Some("a".into()),
            "<h1>A</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        s.upsert_slide(
            0,
            Some("b".into()),
            "<h1>B</h1>".into(),
            TemplateType::Bullet,
            vec![],
        )
        .unwrap();
        let ids: Vec<_> = s
            .presentation()
            .slides
            .iter()
            .map(|x| x.id.as_str())
            .collect();
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
        s.upsert_slide(
            0,
            Some("a".into()),
            "<h1>A</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        let outcome = s
            .upsert_slide(
                7,
                Some("a".into()),
                "<h1>A2</h1>".into(),
                TemplateType::Chart,
                vec![step(".t")],
            )
            .unwrap();
        assert!(!outcome.inserted);
        assert_eq!(outcome.index, 0);
        assert_eq!(s.presentation().slides.len(), 1);
        let slide = &s.presentation().slides[0];
        assert_eq!(slide.code, "<h1 data-id=\"slyde-1\">A2</h1>");
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
        assert!(matches!(
            err,
            CoreError::CodeTooLarge { actual: 10, max: 8 }
        ));
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
        s.upsert_slide(
            0,
            Some("a".into()),
            "<h1>A</h1>".into(),
            TemplateType::Code,
            vec![],
        )
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
        s.upsert_slide(
            0,
            Some("a".into()),
            "<h1>A</h1>".into(),
            TemplateType::Code,
            vec![],
        )
        .unwrap();
        s.upsert_slide(
            1,
            Some("b".into()),
            "<h1>B</h1>".into(),
            TemplateType::Code,
            vec![],
        )
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
                // Every set_title below must push its own entry here.
                coalesce_window_ms: 0,
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
            s.upsert_slide(
                i,
                Some((*id).into()),
                format!("<h1>{id}</h1>"),
                TemplateType::Code,
                vec![],
            )
            .unwrap();
        }
        s.move_slide(0, 2).unwrap();
        let ids: Vec<_> = s
            .presentation()
            .slides
            .iter()
            .map(|x| x.id.as_str())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
        assert!(matches!(
            s.move_slide(0, 9),
            Err(CoreError::IndexOutOfRange { .. })
        ));
    }

    #[test]
    fn mutate_element_updates_html_code_surgically() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<div class=\"card\"><h1 id=\"title-1\">Original Title</h1><p id=\"desc-1\">Some body</p></div>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();

        let updated_slide_id = s
            .mutate_element(
                "title-1",
                None,
                Some("New Concise Title".into()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(updated_slide_id, "sld1");
        assert!(
            s.presentation().slides[0]
                .code
                .contains("New Concise Title")
        );
        assert!(
            s.presentation().slides[0]
                .code
                .contains("<p id=\"desc-1\">Some body</p>")
        );

        s.mutate_element("desc-1", None, None, Some("color: red".into()), None)
            .unwrap();
        assert!(
            s.presentation().slides[0]
                .code
                .contains("style=\"color: red\"")
        );

        let err = s.mutate_element("non-existent", None, Some("test".into()), None, None);
        assert!(matches!(err, Err(CoreError::ElementNotFound { .. })));
    }

    #[test]
    fn mutate_element_replaces_nested_inner_html_without_corrupting_markup() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<div class=\"card\"><h1 id=\"title-1\">Original <em>Title</em></h1><p id=\"desc-1\">Body</p></div>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();

        s.mutate_element("title-1", None, Some("New Title".into()), None, None)
            .unwrap();
        let code = &s.presentation().slides[0].code;
        assert!(
            code.contains("<h1 id=\"title-1\">New Title</h1>"),
            "nested content is replaced as a whole: {code}"
        );
        assert!(
            !code.contains("<em>") && !code.contains("</em>"),
            "no orphan nested tags survive: {code}"
        );
        assert!(
            code.contains("<p id=\"desc-1\">Body</p>"),
            "siblings are untouched: {code}"
        );

        s.mutate_element(
            "desc-1",
            None,
            None,
            None,
            Some("<strong>Bold body</strong>".into()),
        )
        .unwrap();
        let code = &s.presentation().slides[0].code;
        assert!(
            code.contains("<p id=\"desc-1\"><strong>Bold body</strong></p>"),
            "html replacement keeps the element wrapper: {code}"
        );
    }

    #[test]
    fn mutate_element_escapes_plain_text_and_preserves_existing_styles() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<p id=\"t\" style=\"font-size: 20px\">Old</p>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();

        s.mutate_element("t", None, Some("<b>not markup</b>".into()), None, None)
            .unwrap();
        let code = &s.presentation().slides[0].code;
        assert!(
            code.contains("&lt;b&gt;not markup&lt;/b&gt;"),
            "`text` stays plain text: {code}"
        );

        s.mutate_element("t", None, None, Some("color: red".into()), None)
            .unwrap();
        let code = &s.presentation().slides[0].code;
        assert!(
            code.contains("style=\"color: red; font-size: 20px\""),
            "existing declarations are kept: {code}"
        );
    }

    #[test]
    fn mutate_element_rejects_content_changes_on_void_elements() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<div class=\"card\"><img id=\"hero\" src=\"data:image/png;base64,AA\"></div>".into(),
            TemplateType::ImageLayout,
            vec![],
        )
        .unwrap();

        let err = s
            .mutate_element("hero", None, Some("nope".into()), None, None)
            .unwrap_err();
        assert!(
            matches!(err, CoreError::ElementHasNoContent { .. }),
            "a void element has no content: {err:?}"
        );

        s.mutate_element("hero", None, None, Some("border-radius: 8px".into()), None)
            .unwrap();
        assert!(s.presentation().slides[0].code.contains(
            "<img id=\"hero\" src=\"data:image/png;base64,AA\" style=\"border-radius: 8px\">"
        ));
    }

    #[test]
    fn remove_then_undo_restores_the_slide_at_its_original_index() {
        let mut s = state();
        for (i, id) in ["a", "b"].iter().enumerate() {
            s.upsert_slide(
                i,
                Some((*id).into()),
                format!("<h1>{id}</h1>"),
                TemplateType::Code,
                vec![],
            )
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
        loaded
            .slides
            .push(Slide::new("sld_1".into(), "<h1>L</h1>".into()));
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

    /// Every slide gets the same generated handles (`slyde-1`, `slyde-2`, …), so
    /// an unscoped edit must refuse to guess which slide the caller meant.
    #[test]
    fn mutate_element_refuses_to_guess_between_slides() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<h1>First slide</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        s.upsert_slide(
            1,
            Some("sld2".into()),
            "<h1>Second slide</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        assert!(
            s.presentation().slides[0]
                .code
                .contains("data-id=\"slyde-1\"")
        );
        assert!(
            s.presentation().slides[1]
                .code
                .contains("data-id=\"slyde-1\"")
        );

        let err = s
            .mutate_element("slyde-1", None, Some("Wrong".into()), None, None)
            .unwrap_err();
        assert!(
            matches!(err, CoreError::AmbiguousElement { slides: 2, .. }),
            "an ambiguous handle must fail loudly: {err:?}"
        );
        assert!(
            !s.presentation().slides[0].code.contains("Wrong")
                && !s.presentation().slides[1].code.contains("Wrong"),
            "a rejected edit changes nothing"
        );

        let edited = s
            .mutate_element(
                "slyde-1",
                Some("sld2"),
                Some("Second, edited".into()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(edited, "sld2");
        assert!(s.presentation().slides[0].code.contains("First slide"));
        assert!(s.presentation().slides[1].code.contains("Second, edited"));

        assert!(matches!(
            s.mutate_element("slyde-1", Some("missing"), Some("x".into()), None, None),
            Err(CoreError::SlideNotFound(_))
        ));
    }

    #[test]
    fn scoped_element_edit_still_reports_missing_elements() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<h1>Only slide</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();

        assert!(matches!(
            s.mutate_element("nope", Some("sld1"), Some("x".into()), None, None),
            Err(CoreError::ElementNotFound { .. })
        ));
    }

    #[test]
    fn consecutive_typing_merges_into_one_history_entry() {
        let mut s = state();
        s.upsert_slide(
            0,
            Some("sld1".into()),
            "<h1>A</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        let before = s.view().history_len;

        for i in 0..12 {
            s.set_title(format!("Deck {i}")).unwrap();
        }
        assert_eq!(
            s.view().history_len,
            before + 1,
            "a rename burst is one undo step, not twelve"
        );
        assert_eq!(s.presentation().title, "Deck 11");

        s.undo().unwrap();
        assert_eq!(s.presentation().title, "Deck", "one undo reverts the burst");
        assert_eq!(s.presentation().slides.len(), 1);
    }

    #[test]
    fn coalescing_is_scoped_by_slide_and_by_edit_kind() {
        let mut s = state();
        for (i, id) in ["a", "b"].iter().enumerate() {
            s.upsert_slide(
                i,
                Some((*id).into()),
                format!("<h1>{id}</h1>"),
                TemplateType::Code,
                vec![],
            )
            .unwrap();
        }
        let before = s.view().history_len;

        s.set_notes("a", Some("first note".into())).unwrap();
        s.set_notes("a", Some("first note, extended".into()))
            .unwrap();
        assert_eq!(
            s.view().history_len,
            before + 1,
            "notes typed on one slide merge"
        );

        s.set_notes("b", Some("other slide".into())).unwrap();
        assert_eq!(
            s.view().history_len,
            before + 2,
            "another slide is another edit"
        );

        s.set_title("Renamed deck").unwrap();
        assert_eq!(
            s.view().history_len,
            before + 3,
            "a different label never merges"
        );
    }

    #[test]
    fn coalescing_never_merges_across_an_undo() {
        let mut s = state();
        s.set_title("First").unwrap();
        s.undo().unwrap();
        assert_eq!(s.presentation().title, "Deck");

        s.set_title("Second").unwrap();
        assert_eq!(s.presentation().title, "Second");
        assert!(
            !s.can_redo(),
            "a new edit after undo drops the redo branch, it does not merge into it"
        );
        assert_eq!(
            s.history_labels(),
            vec!["init", "title"],
            "the new edit starts its own entry"
        );

        s.undo().unwrap();
        assert_eq!(
            s.presentation().title,
            "Deck",
            "one undo still reaches the pre-edit state"
        );
    }
}
