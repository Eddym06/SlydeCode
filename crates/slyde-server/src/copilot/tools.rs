//! The deck tools exposed to the copilot.
//!
//! They deliberately mirror the MCP command surface: one source of truth, one
//! validation path, one set of activity events. The copilot cannot bypass the
//! state manager.

use serde_json::{Value, json};
use slyde_core::Presentation;

use crate::agent::{AgentCommand, AgentOutcome};
use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
}

pub const READ_DECK: &str = "read_deck";
pub const READ_SLIDE: &str = "read_slide";
pub const CREATE_PRESENTATION: &str = "create_presentation";
pub const UPSERT_SLIDE: &str = "upsert_slide";
pub const REMOVE_SLIDE: &str = "remove_slide";
pub const MOVE_SLIDE: &str = "move_slide";
pub const TRIGGER_ANIMATION: &str = "trigger_animation";
pub const SET_NOTES: &str = "set_notes";
pub const MUTATE_ELEMENT: &str = "mutate_element";
pub const GET_SELECTED_ELEMENT: &str = "get_selected_element";
pub const WRITE_SLIDES: &str = "write_slides";
pub const BUILD_DECK: &str = "build_deck";
pub const SET_THEME: &str = "set_theme";
pub const SET_TITLE: &str = "set_title";
/// Most slides one batch call may carry; keeps a single tool call reviewable
/// and bounded even when a model gets carried away.
const MAX_BATCH_SLIDES: usize = 16;

/// Item schema shared by `write_slides` and `build_deck`.
fn slide_item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "slide_id": { "type": "string", "description": "Existing slide id to replace; omit to append a new slide." },
            "html_code": { "type": "string", "description": "Inner HTML and an optional <style> block. No document wrapper." },
            "template": { "type": "string", "enum": ["hero", "bullet", "diagram", "chart", "image_layout", "code", "custom"], "description": "Optional: inferred from the markup when omitted." },
            "notes": { "type": "string", "description": "Speaker notes, written as spoken language." },
            "animation": {
                "type": "array",
                "maxItems": 3,
                "items": {
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "description": "CSS selector inside the slide." },
                        "kind": { "type": "string", "enum": slyde_core::ANIMATION_KIND_IDS },
                        "delay_ms": { "type": "integer", "minimum": 0, "maximum": 3000 },
                        "duration_ms": { "type": "integer", "minimum": 120, "maximum": 2000 },
                        "easing": { "type": "string" }
                    },
                    "required": ["target", "kind"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["html_code"],
        "additionalProperties": false
    })
}

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: BUILD_DECK,
            description: "Create a brand-new deck AND write all of its slides in ONE call. This is the fastest way to answer 'make me a presentation about X': one call instead of one per slide. Each slide may carry its own speaker notes and animation steps. Replaces the current deck, so use it only when the user asks for a new presentation.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Deck title, sentence case." },
                        "theme": { "type": "string", "enum": slyde_core::THEME_IDS },
                        "aspect_ratio": { "type": "string", "enum": ["16:9", "4:3", "9:16"] },
                        "slides": { "type": "array", "minItems": 1, "maxItems": MAX_BATCH_SLIDES, "items": slide_item_schema() }
                    },
                    "required": ["title", "theme", "aspect_ratio", "slides"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: WRITE_SLIDES,
            description: "Write several slides to the EXISTING deck in one call: slides with a slide_id are replaced, the rest are appended in order. Prefer this over repeated upsert_slide calls whenever you change or add more than one slide.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "slides": { "type": "array", "minItems": 1, "maxItems": MAX_BATCH_SLIDES, "items": slide_item_schema() }
                    },
                    "required": ["slides"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: READ_DECK,
            description: "Read the current deck: title, theme, aspect ratio and every slide id, index, template and notes. Use it before editing an existing deck when the slide ids matter.",
            schema: || json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: READ_SLIDE,
            description: "Read the full HTML/CSS and animation steps for one slide.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": { "slide_id": { "type": "string" } },
                    "required": ["slide_id"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: CREATE_PRESENTATION,
            description: "Start a new deck or fully reset the current one. Only use it when the user explicitly asks for a new presentation; for edits, keep the existing deck and use upsert_slide.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Deck title, sentence case." },
                        "theme": { "type": "string", "enum": slyde_core::THEME_IDS },
                        "aspect_ratio": { "type": "string", "enum": ["16:9", "4:3", "9:16"] }
                    },
                    "required": ["title", "theme", "aspect_ratio"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: UPSERT_SLIDE,
            description: "Create a slide at a zero-based index, or replace one by passing its slide_id. Returns the durable slide id. This is the normal way to write a slide.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer", "minimum": 0, "description": "Insertion position. Required only when inserting; ignored when slide_id matches an existing slide." },
                        "slide_id": { "type": "string", "description": "Existing slide id when replacing; omit to insert." },
                        "html_code": { "type": "string", "description": "Inner HTML and an optional <style> block. No document wrapper." },
                        "template": { "type": "string", "enum": ["hero", "bullet", "diagram", "chart", "image_layout", "code", "custom"], "description": "Optional: inferred from the markup when omitted." },
                        "animation": {
                            "type": "array",
                            "maxItems": 3,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "target": { "type": "string", "description": "CSS selector inside the slide." },
                                    "kind": { "type": "string", "enum": slyde_core::ANIMATION_KIND_IDS },
                                    "delay_ms": { "type": "integer", "minimum": 0, "maximum": 3000 },
                                    "duration_ms": { "type": "integer", "minimum": 120, "maximum": 2000 },
                                    "easing": { "type": "string" }
                                },
                                "required": ["target", "kind"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["html_code"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: REMOVE_SLIDE,
            description: "Delete one slide by id.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": { "slide_id": { "type": "string" } },
                    "required": ["slide_id"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: MOVE_SLIDE,
            description: "Move a slide from one zero-based index to another.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "from": { "type": "integer", "minimum": 0 },
                        "to": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["from", "to"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: TRIGGER_ANIMATION,
            description: "Play one animation step on a slide. Use it to demonstrate motion after writing a slide, not to change the deck.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "slide_id": { "type": "string" },
                        "step_index": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["slide_id", "step_index"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: SET_NOTES,
            description: "Write speaker notes for a slide. Notes are shown in the presenter and are not visible to the audience.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "slide_id": { "type": "string" },
                        "notes": { "type": ["string", "null"] }
                    },
                    "required": ["slide_id", "notes"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: MUTATE_ELEMENT,
            description: "Apply surgical edits (text, styles, inner HTML) to a specific element by ID (e.g. #title-1) without re-rendering or altering the rest of the slide. Always pass slide_id: generated handles repeat in every slide, so an unscoped id that matches several slides is rejected.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "element_id": { "type": "string", "description": "Element ID inside the slide document." },
                        "slide_id": { "type": "string", "description": "Slide that owns the element. Required when the id is not unique across the deck." },
                        "text": { "type": "string", "description": "Replacement plain text content." },
                        "style": { "type": "string", "description": "CSS style directives to add/update." },
                        "html": { "type": "string", "description": "Replacement inner HTML snippet." }
                    },
                    "required": ["element_id"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: SET_THEME,
            description: "Change the presentation theme palette in real time. Can switch to a base theme preset (studio, paper, night, terminal, academic, pitch) and optionally customize background, foreground, accent, muted colors, or font family.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "theme_id": { "type": "string", "enum": slyde_core::THEME_IDS, "description": "Base theme identifier" },
                        "background": { "type": "string", "description": "Hex or CSS color for background" },
                        "foreground": { "type": "string", "description": "Hex or CSS color for primary text" },
                        "accent": { "type": "string", "description": "Hex or CSS color for accent highlight" },
                        "muted": { "type": "string", "description": "Hex or CSS color for secondary text" },
                        "font_family": { "type": "string", "description": "CSS font-family string" }
                    },
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: SET_TITLE,
            description: "Rename the current deck without touching its slides. Use it to give an existing deck a meaningful title; create_presentation would reset the slides.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Deck title, sentence case, at most 120 characters." }
                    },
                    "required": ["title"],
                    "additionalProperties": false
                })
            },
        },
        ToolDef {
            name: GET_SELECTED_ELEMENT,
            description: "Read properties, tag name and text of the element the user has currently focused or selected in the canvas.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "element_id": { "type": "string", "description": "Element ID." }
                    },
                    "required": ["element_id"],
                    "additionalProperties": false
                })
            },
        },
    ]
}

pub fn openai_tools() -> Value {
    Value::Array(
        definitions()
            .into_iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": (tool.schema)()
                    }
                })
            })
            .collect(),
    )
}

pub fn anthropic_tools() -> Value {
    Value::Array(
        definitions()
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": (tool.schema)()
                })
            })
            .collect(),
    )
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub result: Value,
    pub slide_id: Option<String>,
    pub slide_index: Option<usize>,
}

/// Slides of a batch call that were already applied while it was streaming.
#[derive(Debug, Default, Clone)]
pub struct Applied {
    /// The deck was created from the call's header.
    pub created: bool,
    /// `(slide_id, index)` of every slide written so far, in call order.
    pub slides: Vec<(String, usize)>,
}

/// Applies the slides a streaming batch call has finished so far. Returns what
/// was written so the call can skip it when it ends.
pub async fn apply_partial(
    state: &AppState,
    tool: &str,
    header: Option<&Value>,
    slides: &[Value],
    applied: &mut Applied,
) {
    if tool == BUILD_DECK && !applied.created {
        // Without a title the call cannot be applied early: it runs at the end.
        let Some(header) = header.filter(|header| header.get("title").is_some()) else {
            return;
        };
        let create = json!({
            "action": CREATE_PRESENTATION,
            "title": header.get("title"),
            "theme": header.get("theme"),
            "aspect_ratio": header.get("aspect_ratio"),
        });
        let Ok(command) = serde_json::from_value::<AgentCommand>(create) else {
            return;
        };
        if crate::agent::run(state, command).await.is_err() {
            return;
        }
        applied.created = true;
    }
    if tool == BUILD_DECK && !applied.created {
        return;
    }
    for item in slides {
        match write_one(state, item).await {
            Ok(written) => applied.slides.push(written),
            // A slide that cannot be written is left for the final call to
            // report; stop so the order of the rest is not disturbed.
            Err(_) => return,
        }
    }
}

pub async fn execute(
    state: &AppState,
    name: &str,
    arguments: Value,
) -> Result<ToolOutcome, String> {
    execute_with(state, name, arguments, None).await
}

/// Like [`execute`], but a batch call skips what `applied` already wrote.
pub async fn execute_with(
    state: &AppState,
    name: &str,
    arguments: Value,
    applied: Option<&Applied>,
) -> Result<ToolOutcome, String> {
    match name {
        READ_DECK => read_deck(state).await,
        READ_SLIDE => read_slide(state, &arguments).await,
        GET_SELECTED_ELEMENT => get_selected_element(state, &arguments).await,
        BUILD_DECK => build_deck(state, arguments, applied).await,
        WRITE_SLIDES => write_slides(state, &arguments, applied).await,
        CREATE_PRESENTATION | UPSERT_SLIDE | REMOVE_SLIDE | MOVE_SLIDE | TRIGGER_ANIMATION
        | SET_NOTES | MUTATE_ELEMENT | SET_THEME | SET_TITLE => {
            mutate(state, name, arguments).await
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// Creates the deck, then writes every slide through the same command path the
/// single-slide tools use, so events, validation and persistence are identical.
async fn build_deck(
    state: &AppState,
    arguments: Value,
    applied: Option<&Applied>,
) -> Result<ToolOutcome, String> {
    let title = arguments
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| "title is required".to_string())?
        .to_string();
    let slides = arguments.get("slides").cloned().unwrap_or(Value::Null);
    // Validate the batch shape before wiping the current deck.
    if !slides.as_array().is_some_and(|list| !list.is_empty()) {
        return Err("slides must be a non-empty array".to_string());
    }
    // The deck may already exist: its header and first slides were applied while
    // the model was still writing.
    if !applied.is_some_and(|applied| applied.created) {
        let create = json!({
            "action": CREATE_PRESENTATION,
            "title": title,
            "theme": arguments.get("theme"),
            "aspect_ratio": arguments.get("aspect_ratio"),
        });
        let command: AgentCommand = serde_json::from_value(create)
            .map_err(|error| format!("invalid arguments: {error}"))?;
        crate::agent::run(state, command)
            .await
            .map_err(|error| error.to_string())?;
    }
    let mut outcome = write_slides(state, &json!({ "slides": slides }), applied).await?;
    if let Some(object) = outcome.result.as_object_mut() {
        object.insert("created".into(), Value::Bool(true));
    }
    Ok(outcome)
}

async fn write_slides(
    state: &AppState,
    arguments: &Value,
    applied: Option<&Applied>,
) -> Result<ToolOutcome, String> {
    let items = arguments
        .get("slides")
        .and_then(Value::as_array)
        .filter(|list| !list.is_empty())
        .ok_or_else(|| "slides must be a non-empty array".to_string())?;
    if items.len() > MAX_BATCH_SLIDES {
        return Err(format!(
            "at most {MAX_BATCH_SLIDES} slides per call; split the rest into another write_slides call"
        ));
    }
    let mut written = Vec::with_capacity(items.len());
    let mut last = (None, None);
    let already = applied
        .map(|applied| applied.slides.as_slice())
        .unwrap_or(&[]);
    for (position, item) in items.iter().enumerate() {
        // Slides applied during streaming are not written twice.
        let attempt = match already.get(position) {
            Some((slide_id, index)) => Ok((slide_id.clone(), *index)),
            None => write_one(state, item).await,
        };
        match attempt {
            Ok((slide_id, index)) => {
                written.push(json!({ "ok": true, "slide_id": slide_id, "index": index }));
                last = (Some(slide_id), Some(index));
            }
            // A bad slide must not discard the good ones: report it and carry on,
            // so the model fixes exactly that item instead of redoing the batch.
            Err(error) => {
                written.push(json!({ "ok": false, "position": position, "error": error }))
            }
        }
    }
    let failed = written.iter().filter(|entry| entry["ok"] == false).count();
    if failed == written.len() {
        let first = written
            .first()
            .and_then(|entry| entry["error"].as_str())
            .unwrap_or("no slide could be written");
        return Err(format!("no slide was written: {first}"));
    }
    Ok(ToolOutcome {
        result: json!({ "ok": failed == 0, "tool": WRITE_SLIDES, "slides": written }),
        slide_id: last.0,
        slide_index: last.1,
    })
}

/// One batch item. A missing or out-of-range position appends instead of
/// failing: the model often guesses an index, and a rejected guess costs a whole
/// extra round trip.
pub(crate) async fn write_one(state: &AppState, item: &Value) -> Result<(String, usize), String> {
    let object = item
        .as_object()
        .ok_or_else(|| "each slide must be an object".to_string())?;
    let append_at = state.view().await.presentation.slides.len();
    let mut command = serde_json::Map::new();
    command.insert("action".into(), Value::String(UPSERT_SLIDE.into()));
    command.insert("index".into(), json!(append_at));
    for key in ["slide_id", "html_code", "template", "animation"] {
        if let Some(value) = object.get(key) {
            command.insert(key.into(), value.clone());
        }
    }
    let command: AgentCommand = serde_json::from_value(Value::Object(command))
        .map_err(|error| format!("invalid slide: {error}"))?;
    let outcome = crate::agent::run(state, command)
        .await
        .map_err(|error| error.to_string())?;
    let upsert = outcome
        .upsert
        .ok_or_else(|| "the slide was not stored".to_string())?;
    if let Some(notes) = object.get("notes").and_then(Value::as_str) {
        let command = AgentCommand::SetNotes {
            slide_id: upsert.slide_id.clone(),
            notes: Some(notes.to_string()),
        };
        crate::agent::run(state, command)
            .await
            .map_err(|error| format!("slide written, notes failed: {error}"))?;
    }
    Ok((upsert.slide_id, upsert.index))
}

async fn get_selected_element(state: &AppState, arguments: &Value) -> Result<ToolOutcome, String> {
    let element_id = arguments
        .get("element_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "element_id is required".to_string())?;
    let view = state.view().await;
    let id_needle = format!("id=\"{element_id}\"");
    let data_needle = format!("data-id=\"{element_id}\"");

    for (index, slide) in view.presentation.slides.iter().enumerate() {
        let structured = slide.find_element(element_id);
        let marker = slide
            .code
            .find(&id_needle)
            .or_else(|| slide.code.find(&data_needle));
        if structured.is_none() && marker.is_none() {
            continue;
        }
        // Return the element itself, not the head of the slide: an agent asking
        // about one block needs that block.
        let markup = marker.map(|position| {
            let start = slide.code[..position].rfind('<').unwrap_or(position);
            slide.code[start..].chars().take(240).collect::<String>()
        });
        return Ok(ToolOutcome {
            result: json!({
                "ok": true,
                "element_id": element_id,
                "slide_id": slide.id,
                "slide_index": index,
                "found": true,
                "element": structured,
                "markup": markup
            }),
            slide_id: Some(slide.id.clone()),
            slide_index: Some(index),
        });
    }

    Ok(ToolOutcome {
        result: json!({
            "ok": true,
            "element_id": element_id,
            "found": false
        }),
        slide_id: None,
        slide_index: None,
    })
}

async fn read_deck(state: &AppState) -> Result<ToolOutcome, String> {
    let view = state.view().await;
    Ok(ToolOutcome {
        result: json!({
            "ok": true,
            "title": view.presentation.title,
            "theme": view.presentation.theme.id,
            "aspect_ratio": view.presentation.aspect_ratio.as_str(),
            "revision": view.revision,
            "slides": view.presentation.slides.iter().enumerate().map(|(index, slide)| json!({
                "index": index,
                "slide_id": slide.id,
                "template": slide.template,
                "animation_steps": slide.animation.len(),
                "notes": slide.notes,
            })).collect::<Vec<_>>()
        }),
        slide_id: None,
        slide_index: None,
    })
}

async fn read_slide(state: &AppState, arguments: &Value) -> Result<ToolOutcome, String> {
    let slide_id = arguments
        .get("slide_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "slide_id is required".to_string())?;
    let view = state.view().await;
    let (index, slide) = view
        .presentation
        .slides
        .iter()
        .enumerate()
        .find(|(_, slide)| slide.id == slide_id)
        .ok_or_else(|| format!("slide not found: {slide_id}"))?;
    Ok(ToolOutcome {
        result: json!({
            "ok": true,
            "index": index,
            "slide": slide,
        }),
        slide_id: Some(slide.id.clone()),
        slide_index: Some(index),
    })
}

async fn mutate(state: &AppState, name: &str, arguments: Value) -> Result<ToolOutcome, String> {
    let mut object = arguments
        .as_object()
        .cloned()
        .ok_or_else(|| "tool arguments must be an object".to_string())?;
    object.insert("action".into(), Value::String(name.into()));
    if name == UPSERT_SLIDE {
        // A guessed position past the end appends instead of burning a round on
        // an out-of-range error.
        let end = state.view().await.presentation.slides.len();
        let wanted = object
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or(end as u64);
        object.insert("index".into(), json!(wanted.min(end as u64)));
    }
    let command: AgentCommand = serde_json::from_value(Value::Object(object))
        .map_err(|error| format!("invalid arguments: {error}"))?;
    let outcome = crate::agent::run(state, command)
        .await
        .map_err(|error| error.to_string())?;
    Ok(outcome_to_tool(name, outcome))
}

fn outcome_to_tool(name: &str, outcome: AgentOutcome) -> ToolOutcome {
    let slide_id = outcome
        .upsert
        .as_ref()
        .map(|upsert| upsert.slide_id.clone())
        .or_else(|| outcome.mutated_slide_id.clone());
    let slide_index = outcome.upsert.as_ref().map(|upsert| upsert.index);
    let result = json!({
        "ok": true,
        "tool": name,
        "created": outcome.created,
        "upsert": outcome.upsert,
        "slide_id": outcome.mutated_slide_id,
        "animation": outcome.animation.is_some(),
    });
    ToolOutcome {
        result,
        slide_id,
        slide_index,
    }
}

/// Compact, prompt-sized deck snapshot used by the system message and by the
/// local read tools.
pub fn deck_summary(deck: &Presentation, selected_id: Option<&str>) -> String {
    let mut out = format!(
        "Deck title: {}\nTheme: {}\nAspect ratio: {}\nSlides: {}\n",
        deck.title,
        deck.theme.id,
        deck.aspect_ratio.as_str(),
        deck.slides.len()
    );
    for (index, slide) in deck.slides.iter().enumerate() {
        let marker = if Some(slide.id.as_str()) == selected_id {
            "  <-- selected"
        } else {
            ""
        };
        let notes = slide.notes.as_deref().map(str::trim).unwrap_or("");
        out.push_str(&format!(
            "\n[{}] id={} template={:?} animations={} notes={}{}\n",
            index,
            slide.id,
            slide.template,
            slide.animation.len(),
            if notes.is_empty() { "none" } else { "yes" },
            marker
        ));
        if let Some(slide) = deck.slides.get(index) {
            if Some(slide.id.as_str()) == selected_id {
                out.push_str("selected slide source:\n");
                let code = truncate(&slide.code, 16_000);
                out.push_str(&code);
                out.push('\n');
            } else {
                out.push_str("preview: ");
                out.push_str(&truncate(&one_line_preview(&slide.code), 180));
                out.push('\n');
            }
        }
    }
    out
}

fn one_line_preview(code: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in strip_embedded_blocks(code).chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if out.ends_with('<') {
                    out.pop();
                }
            }
            _ if !in_tag && (!ch.is_whitespace() || !out.ends_with(' ')) => {
                out.push(if ch.is_whitespace() { ' ' } else { ch });
            }
            _ => {}
        }
        if out.len() > 240 {
            break;
        }
    }
    out.trim().to_string()
}

/// CSS and scripts are the bulk of a slide and read as noise in a one-line
/// preview. Remove their bodies before flattening tags, keeping byte offsets
/// stable by lowercasing ASCII only.
fn strip_embedded_blocks(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut out = String::new();
    let mut cursor = 0;
    while cursor < raw.len() {
        let rest = &lower[cursor..];
        let next = ["<style", "<script"]
            .iter()
            .filter_map(|tag| rest.find(tag).map(|offset| (offset, *tag)))
            .min_by_key(|(offset, _)| *offset);
        let Some((offset, tag)) = next else {
            out.push_str(&raw[cursor..]);
            break;
        };
        let start = cursor + offset;
        out.push_str(&raw[cursor..start]);
        let close = format!("</{}>", &tag[1..]);
        let end = lower[start..]
            .find(&close)
            .map(|found| start + found + close.len())
            .unwrap_or(raw.len());
        cursor = end;
    }
    out
}

fn truncate(raw: &str, max: usize) -> String {
    if raw.chars().count() <= max {
        return raw.to_string();
    }
    let mut text: String = raw.chars().take(max).collect();
    text.push_str("\n[truncated]");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_closed_object_schema() {
        for tool in definitions() {
            let schema = (tool.schema)();
            assert_eq!(schema["type"], "object");
            assert!(schema.get("properties").is_some());
            assert_eq!(schema["additionalProperties"], false);
        }
    }

    /// Same guard as the MCP surface: these schemas are what the model is allowed
    /// to send, so they must carry every kind and theme the core accepts.
    #[test]
    fn schemas_expose_the_full_core_contract() {
        let upsert = definitions()
            .into_iter()
            .find(|tool| tool.name == UPSERT_SLIDE)
            .unwrap();
        let schema = (upsert.schema)();
        let kinds = schema["properties"]["animation"]["items"]["properties"]["kind"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(
            kinds.len(),
            slyde_core::ANIMATION_KIND_IDS.len(),
            "the model cannot use kinds the schema hides"
        );

        let create = definitions()
            .into_iter()
            .find(|tool| tool.name == CREATE_PRESENTATION)
            .unwrap();
        let schema = (create.schema)();
        let themes = schema["properties"]["theme"]["enum"].as_array().unwrap();
        assert_eq!(themes.len(), slyde_core::THEME_IDS.len());

        let set_theme = definitions()
            .into_iter()
            .find(|tool| tool.name == SET_THEME)
            .unwrap();
        let set_theme_schema = (set_theme.schema)();
        assert!(set_theme_schema["properties"]["theme_id"]["enum"].is_array());

        let set_title = definitions()
            .into_iter()
            .find(|tool| tool.name == SET_TITLE)
            .unwrap();
        let set_title_schema = (set_title.schema)();
        assert_eq!(set_title_schema["required"][0], "title");

        let mutate = definitions()
            .into_iter()
            .find(|tool| tool.name == MUTATE_ELEMENT)
            .unwrap();
        let schema = (mutate.schema)();
        assert_eq!(schema["properties"]["slide_id"]["type"], "string");
    }

    #[test]
    fn deck_summary_marks_the_selected_slide_and_keeps_other_previews_short() {
        let mut deck = Presentation::new(
            "Q3",
            slyde_core::Theme::studio(),
            slyde_core::AspectRatio::R16x9,
        );
        deck.slides.push(slyde_core::Slide::new(
            "sld_a".into(),
            "<h1>Hello</h1>".into(),
        ));
        deck.slides.push(slyde_core::Slide::new(
            "sld_b".into(),
            "<h1>Second</h1>".into(),
        ));
        let summary = deck_summary(&deck, Some("sld_b"));
        assert!(summary.contains("id=sld_b"));
        assert!(summary.contains("<h1>Second</h1>"));
        assert!(summary.contains("<-- selected"));
        assert!(!summary.contains("selected slide source:\n<h1>Hello"));
    }

    fn test_state() -> std::sync::Arc<AppState> {
        AppState::new(
            slyde_core::PresentationState::new(
                "Deck",
                slyde_core::Theme::studio(),
                slyde_core::AspectRatio::R16x9,
            ),
            "tok".into(),
            std::path::PathBuf::from("ui"),
            vec![],
            "http://127.0.0.1:1".into(),
        )
    }

    #[tokio::test]
    async fn build_deck_writes_every_slide_with_notes_in_one_call() {
        let state = test_state();
        let outcome = execute(
            &state,
            BUILD_DECK,
            json!({
                "title": "Launch", "theme": "paper", "aspect_ratio": "4:3",
                "slides": [
                    { "html_code": "<h1>One</h1>", "template": "hero", "notes": "Say hi" },
                    { "html_code": "<h1>Two</h1>", "template": "bullet" }
                ]
            }),
        )
        .await
        .unwrap();
        assert_eq!(outcome.result["slides"].as_array().unwrap().len(), 2);
        let view = state.view().await;
        assert_eq!(view.presentation.title, "Launch");
        assert_eq!(view.presentation.slides.len(), 2);
        assert_eq!(view.presentation.slides[0].notes.as_deref(), Some("Say hi"));
    }

    #[tokio::test]
    async fn build_deck_with_no_slides_keeps_the_current_deck() {
        let state = test_state();
        let error = execute(
            &state,
            BUILD_DECK,
            json!({ "title": "X", "theme": "paper", "aspect_ratio": "16:9", "slides": [] }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("non-empty"));
        assert_eq!(state.view().await.presentation.title, "Deck");
    }

    #[tokio::test]
    async fn write_slides_reports_a_bad_item_without_losing_the_good_ones() {
        let state = test_state();
        let outcome = execute(
            &state,
            WRITE_SLIDES,
            json!({ "slides": [
                { "html_code": "<h1>Good</h1>", "template": "hero" },
                { "html_code": "   ", "template": "hero" }
            ]}),
        )
        .await
        .unwrap();
        assert_eq!(outcome.result["ok"], false);
        assert_eq!(outcome.result["slides"][0]["ok"], true);
        assert_eq!(outcome.result["slides"][1]["ok"], false);
        assert_eq!(state.view().await.presentation.slides.len(), 1);
    }

    #[tokio::test]
    async fn upsert_with_a_guessed_index_past_the_end_appends() {
        let state = test_state();
        let outcome = execute(
            &state,
            UPSERT_SLIDE,
            json!({ "index": 9, "html_code": "<h1>Hi</h1>", "template": "hero" }),
        )
        .await
        .unwrap();
        assert_eq!(outcome.slide_index, Some(0));
    }

    #[test]
    fn slide_previews_drop_style_and_script_bodies() {
        let preview = one_line_preview(
            "<style>.hero { color: red; }</style><section><h1>Real copy</h1></section><script>noise()</script>",
        );
        assert_eq!(preview, "Real copy");
    }
}
