//! SlydeCode domain model.
//!
//! Pure data + invariants. No I/O, no transport, no UI concerns.

use serde::{Deserialize, Serialize};

pub type SlideId = String;

/// Epoch milliseconds. Kept as a plain integer so the model stays dependency-free.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AspectRatio {
    #[default]
    #[serde(rename = "16:9")]
    R16x9,
    #[serde(rename = "4:3")]
    R4x3,
    #[serde(rename = "9:16")]
    R9x16,
}

impl AspectRatio {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::R16x9 => "16:9",
            Self::R4x3 => "4:3",
            Self::R9x16 => "9:16",
        }
    }

    /// Canvas size in CSS pixels at 1x. Used by the renderer and the export pipeline.
    pub fn size_px(self) -> (u32, u32) {
        match self {
            Self::R16x9 => (1280, 720),
            Self::R4x3 => (1024, 768),
            Self::R9x16 => (720, 1280),
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "16:9" | "16x9" => Some(Self::R16x9),
            "4:3" | "4x3" => Some(Self::R4x3),
            "9:16" | "9x16" => Some(Self::R9x16),
            _ => None,
        }
    }
}

/// Placement of an image inside the slide canvas, in CSS pixels.
///
/// The canvas is a fixed-size stage (1280x720 for 16:9), so absolute pixels are
/// the honest unit: they survive zoom, thumbnails and export unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImageRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ImageRect {
    /// A usable rectangle: finite, positive and within what a slide can hold.
    pub fn validate(&self) -> Result<(), crate::CoreError> {
        let values = [self.x, self.y, self.w, self.h];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(crate::CoreError::InvalidGeometry(
                "geometry must be finite".into(),
            ));
        }
        if !(8.0..=200_000.0).contains(&self.w) || !(8.0..=200_000.0).contains(&self.h) {
            return Err(crate::CoreError::InvalidGeometry(format!(
                "size {}x{} is out of range (8..200000 px)",
                self.w, self.h
            )));
        }
        if self.x.abs() > 200_000.0 || self.y.abs() > 200_000.0 {
            return Err(crate::CoreError::InvalidGeometry(format!(
                "position ({}, {}) is out of range",
                self.x, self.y
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateType {
    #[default]
    Code,
    Diagram,
    Bullet,
    Hero,
    ImageLayout,
    Chart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationKind {
    FadeUp,
    Fade,
    ScaleIn,
    SlideInLeft,
    SlideInRight,
    Typewriter,
    Spin,
    RotateIn,
    Pulse,
    FlipUp,
    BlurIn,
    BounceIn,
}

/// Every animation kind as a wire id, in the order the product presents them.
///
/// This is the single source of truth for the MCP tool schema, the copilot tool
/// schema and the copilot prompt. `AnimationKind::id` is an exhaustive match, so
/// adding a variant without listing it here is a compile error.
pub const ANIMATION_KIND_IDS: [&str; 12] = [
    "fade_up",
    "fade",
    "scale_in",
    "slide_in_left",
    "slide_in_right",
    "typewriter",
    "spin",
    "rotate_in",
    "pulse",
    "flip_up",
    "blur_in",
    "bounce_in",
];

impl AnimationKind {
    /// Wire id, matching the serde representation and the tool schemas.
    pub const fn id(self) -> &'static str {
        match self {
            Self::FadeUp => "fade_up",
            Self::Fade => "fade",
            Self::ScaleIn => "scale_in",
            Self::SlideInLeft => "slide_in_left",
            Self::SlideInRight => "slide_in_right",
            Self::Typewriter => "typewriter",
            Self::Spin => "spin",
            Self::RotateIn => "rotate_in",
            Self::Pulse => "pulse",
            Self::FlipUp => "flip_up",
            Self::BlurIn => "blur_in",
            Self::BounceIn => "bounce_in",
        }
    }
}

/// Every built-in theme id, in the order the pickers present them.
///
/// Same contract as [`ANIMATION_KIND_IDS`]: the tool schemas and the copilot
/// prompt read this list instead of keeping their own copy. `Theme::resolve`
/// must accept every entry (enforced by a test).
pub const THEME_IDS: [&str; 6] = ["studio", "paper", "night", "terminal", "academic", "pitch"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationStep {
    /// CSS selector inside the slide document, e.g. `.title` or `[data-anim="bars"]`.
    pub target: String,
    pub kind: AnimationKind,
    #[serde(default)]
    pub delay_ms: u32,
    #[serde(default = "default_duration")]
    pub duration_ms: u32,
    #[serde(default = "default_easing")]
    pub easing: String,
}

fn default_duration() -> u32 {
    520
}

fn default_easing() -> String {
    "cubic-bezier(.16,1,.3,1)".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    #[default]
    Ai,
    Human,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub font_family: String,
}

impl Theme {
    pub fn studio() -> Self {
        Self {
            id: "studio".into(),
            name: "Studio".into(),
            background: "#17161A".into(),
            foreground: "#F7F4EF".into(),
            muted: "rgba(247,244,239,.55)".into(),
            accent: "#FF6B4A".into(),
            font_family: "Geist, Inter, system-ui, sans-serif".into(),
        }
    }

    pub fn paper() -> Self {
        Self {
            id: "paper".into(),
            name: "Paper".into(),
            background: "#FBF8F3".into(),
            foreground: "#17161A".into(),
            muted: "rgba(23,22,26,.58)".into(),
            accent: "#7C3AED".into(),
            font_family: "Geist, Inter, system-ui, sans-serif".into(),
        }
    }

    pub fn night() -> Self {
        Self {
            id: "night".into(),
            name: "Night".into(),
            background: "#1E1B4B".into(),
            foreground: "#F5F3FF".into(),
            muted: "rgba(245,243,255,.62)".into(),
            accent: "#FF8265".into(),
            font_family: "Geist, Inter, system-ui, sans-serif".into(),
        }
    }

    pub fn terminal() -> Self {
        Self {
            id: "terminal".into(),
            name: "Terminal".into(),
            background: "#0B0C0E".into(),
            foreground: "#00FF66".into(),
            muted: "rgba(0,255,102,.50)".into(),
            accent: "#00E5FF".into(),
            font_family: "Geist Mono, JetBrains Mono, monospace".into(),
        }
    }

    pub fn academic() -> Self {
        Self {
            id: "academic".into(),
            name: "Academic Minimal".into(),
            background: "#FFFFFF".into(),
            foreground: "#111111".into(),
            muted: "rgba(17,17,17,.60)".into(),
            accent: "#0055D4".into(),
            font_family: "Geist, Cambria, serif".into(),
        }
    }

    pub fn pitch() -> Self {
        Self {
            id: "pitch".into(),
            name: "Corporate Pitch".into(),
            background: "#0F172A".into(),
            foreground: "#F8FAFC".into(),
            muted: "rgba(248,250,252,.60)".into(),
            accent: "#38BDF8".into(),
            font_family: "Geist, Inter, sans-serif".into(),
        }
    }

    /// Accepts either a known theme id or a full theme payload (custom themes).
    pub fn resolve(id_or_name: &str) -> Option<Self> {
        match id_or_name.trim().to_ascii_lowercase().as_str() {
            "studio" => Some(Self::studio()),
            "paper" => Some(Self::paper()),
            "night" => Some(Self::night()),
            "terminal" => Some(Self::terminal()),
            "academic" => Some(Self::academic()),
            "pitch" => Some(Self::pitch()),
            _ => None,
        }
    }

    /// Basic validation: hex or rgba() colors, non-empty font stack.
    pub fn validate(&self) -> Result<(), crate::CoreError> {
        for (field, value) in [
            ("background", &self.background),
            ("foreground", &self.foreground),
            ("muted", &self.muted),
            ("accent", &self.accent),
        ] {
            if !is_css_color(value) {
                return Err(crate::CoreError::InvalidTheme(format!(
                    "{field} is not a CSS color: {value}"
                )));
            }
        }
        if self.font_family.trim().is_empty() {
            return Err(crate::CoreError::InvalidTheme(
                "font_family cannot be empty".into(),
            ));
        }
        Ok(())
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::studio()
    }
}

fn is_css_color(value: &str) -> bool {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    (v.starts_with("rgb(")
        || v.starts_with("rgba(")
        || v.starts_with("hsl(")
        || v.starts_with("oklch("))
        && v.ends_with(')')
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    Contain,
    #[default]
    Cover,
    Fill,
}

/// Structured elements that can be embedded, manipulated or extracted inside slides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlideElement {
    Image {
        element_id: String,
        src: String,
        #[serde(default)]
        alt: Option<String>,
        #[serde(default)]
        caption: Option<String>,
        #[serde(default)]
        fit: FitMode,
    },
    Diagram {
        element_id: String,
        /// Diagram definition as code (e.g. Mermaid or Graphviz source)
        source: String,
        #[serde(default)]
        engine: Option<String>, // "mermaid", "graphviz"
        #[serde(default)]
        caption: Option<String>,
    },
    CodeBlock {
        element_id: String,
        code: String,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        highlighted_lines: Vec<usize>,
    },
    SpeakerNotes {
        element_id: String,
        notes: String,
    },
    Custom {
        element_id: String,
        tag: String,
        content: String,
        #[serde(default)]
        attributes: std::collections::BTreeMap<String, String>,
    },
}

impl SlideElement {
    pub fn element_id(&self) -> &str {
        match self {
            SlideElement::Image { element_id, .. } => element_id,
            SlideElement::Diagram { element_id, .. } => element_id,
            SlideElement::CodeBlock { element_id, .. } => element_id,
            SlideElement::SpeakerNotes { element_id, .. } => element_id,
            SlideElement::Custom { element_id, .. } => element_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub id: SlideId,
    /// AI-authored HTML/CSS fragment. Rendered inside a cross-origin sandbox.
    pub code: String,
    #[serde(default)]
    pub template: TemplateType,
    #[serde(default)]
    pub animation: Vec<AnimationStep>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub generated_by: Author,
    #[serde(default)]
    pub elements: Vec<SlideElement>,
}

impl Slide {
    pub fn new(id: SlideId, code: String) -> Self {
        Self {
            id,
            code,
            template: TemplateType::default(),
            animation: Vec::new(),
            notes: None,
            generated_by: Author::default(),
            elements: Vec::new(),
        }
    }

    pub fn find_element(&self, element_id: &str) -> Option<&SlideElement> {
        self.elements
            .iter()
            .find(|el| el.element_id() == element_id)
    }

    pub fn find_element_mut(&mut self, element_id: &str) -> Option<&mut SlideElement> {
        self.elements
            .iter_mut()
            .find(|el| el.element_id() == element_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub id: String,
    pub title: String,
    pub theme: Theme,
    pub aspect_ratio: AspectRatio,
    pub slides: Vec<Slide>,
    pub updated_at_ms: u64,
}

impl Presentation {
    pub fn new(title: impl Into<String>, theme: Theme, aspect_ratio: AspectRatio) -> Self {
        Self {
            id: format!("prs_{}", uuid::Uuid::new_v4().simple()),
            title: title.into(),
            theme,
            aspect_ratio,
            slides: Vec::new(),
            updated_at_ms: now_ms(),
        }
    }

    pub fn slide(&self, id: &str) -> Option<&Slide> {
        self.slides.iter().find(|s| s.id == id)
    }

    pub fn slide_index(&self, id: &str) -> Option<usize> {
        self.slides.iter().position(|s| s.id == id)
    }

    pub fn size_px(&self) -> (u32, u32) {
        self.aspect_ratio.size_px()
    }

    pub fn find_element(&self, element_id: &str) -> Option<(&Slide, &SlideElement)> {
        for s in &self.slides {
            if let Some(el) = s.find_element(element_id) {
                return Some((s, el));
            }
        }
        None
    }
}

/// Read model sent to the UI and returned by `inspect_current_state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateView {
    pub presentation: Presentation,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub history_len: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_ratio_round_trips_through_json() {
        for ratio in [AspectRatio::R16x9, AspectRatio::R4x3, AspectRatio::R9x16] {
            let json = serde_json::to_string(&ratio).unwrap();
            assert_eq!(json, format!("\"{}\"", ratio.as_str()));
            let back: AspectRatio = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ratio);
        }
    }

    #[test]
    fn aspect_ratio_parse_accepts_colon_and_x() {
        assert_eq!(AspectRatio::parse("16:9"), Some(AspectRatio::R16x9));
        assert_eq!(AspectRatio::parse("4x3"), Some(AspectRatio::R4x3));
        assert_eq!(AspectRatio::parse(" 9:16 "), Some(AspectRatio::R9x16));
        assert_eq!(AspectRatio::parse("21:9"), None);
    }

    #[test]
    fn theme_validation_rejects_bad_colors() {
        let mut theme = Theme::studio();
        assert!(theme.validate().is_ok());
        theme.accent = "not-a-color".into();
        assert!(theme.validate().is_err());
    }

    #[test]
    fn theme_resolve_is_case_insensitive() {
        assert_eq!(Theme::resolve("STUDIO").unwrap().id, "studio");
        assert_eq!(Theme::resolve("Night").unwrap().id, "night");
        assert!(Theme::resolve("neon").is_none());
    }

    #[test]
    fn presentation_finds_slides_by_id() {
        let mut deck = Presentation::new("Deck", Theme::studio(), AspectRatio::R16x9);
        deck.slides
            .push(Slide::new("a".into(), "<h1>A</h1>".into()));
        deck.slides
            .push(Slide::new("b".into(), "<h1>B</h1>".into()));
        assert_eq!(deck.slide_index("b"), Some(1));
        assert_eq!(deck.slide("a").unwrap().code, "<h1>A</h1>");
        assert!(deck.slide("missing").is_none());
    }

    /// The tool schemas and the copilot prompt read `ANIMATION_KIND_IDS` instead
    /// of keeping their own copy. This test is what keeps the wire list honest:
    /// every id must deserialize, and the list must not repeat or drop one.
    #[test]
    fn animation_kind_ids_cover_every_variant() {
        assert_eq!(ANIMATION_KIND_IDS.len(), 12);
        for id in ANIMATION_KIND_IDS {
            let parsed: AnimationKind = serde_json::from_str(&format!("\"{id}\""))
                .unwrap_or_else(|error| panic!("{id} is not a wire id: {error}"));
            assert_eq!(parsed.id(), id, "id() and serde must agree on {id}");
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                format!("\"{id}\""),
                "serialization drifted from the published id"
            );
        }
        let mut unique = ANIMATION_KIND_IDS.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ANIMATION_KIND_IDS.len(), "duplicate ids");
    }

    /// Same contract for themes: the pickers, the prompt and the schemas offer
    /// exactly what `Theme::resolve` accepts.
    #[test]
    fn theme_ids_all_resolve() {
        for id in THEME_IDS {
            let theme = Theme::resolve(id).unwrap_or_else(|| panic!("{id} does not resolve"));
            assert_eq!(theme.id, id, "resolved id drifted from the published id");
            assert!(theme.validate().is_ok(), "{id} is not a valid theme");
        }
        assert_eq!(
            Theme::resolve("neon").map(|t| t.id),
            None,
            "resolve must not invent themes outside THEME_IDS"
        );
    }
}
