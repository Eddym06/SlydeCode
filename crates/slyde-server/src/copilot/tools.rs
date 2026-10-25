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

pub fn definitions() -> Vec<ToolDef> {
    vec![
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
                        "theme": { "type": "string", "enum": ["studio", "paper", "night"] },
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
                        "index": { "type": "integer", "minimum": 0 },
                        "slide_id": { "type": "string", "description": "Existing slide id when replacing; omit to insert." },
                        "html_code": { "type": "string", "description": "Inner HTML and an optional <style> block. No document wrapper." },
                        "template": { "type": "string", "enum": ["hero", "bullet", "diagram", "chart", "image_layout", "code"] },
                        "animation": {
                            "type": "array",
                            "maxItems": 3,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "target": { "type": "string", "description": "CSS selector inside the slide." },
                                    "kind": { "type": "string", "enum": ["fade_up", "fade", "scale_in", "slide_in_left", "slide_in_right", "typewriter"] },
                                    "delay_ms": { "type": "integer", "minimum": 0, "maximum": 3000 },
                                    "duration_ms": { "type": "integer", "minimum": 120, "maximum": 2000 },
                                    "easing": { "type": "string" }
                                },
                                "required": ["target", "kind"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["index", "html_code", "template"],
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
            description: "Apply surgical edits (text, styles, inner HTML) to a specific element by ID (e.g. #title-1) without re-rendering or altering the rest of the slide.",
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "element_id": { "type": "string", "description": "Element ID inside the slide document." },
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

pub async fn execute(
    state: &AppState,
    name: &str,
    arguments: Value,
) -> Result<ToolOutcome, String> {
    match name {
        READ_DECK => read_deck(state).await,
        READ_SLIDE => read_slide(state, &arguments).await,
        GET_SELECTED_ELEMENT => get_selected_element(state, &arguments).await,
        CREATE_PRESENTATION | UPSERT_SLIDE | REMOVE_SLIDE | MOVE_SLIDE | TRIGGER_ANIMATION
        | SET_NOTES | MUTATE_ELEMENT => mutate(state, name, arguments).await,
        _ => Err(format!("unknown tool: {name}")),
    }
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
        .map(|upsert| upsert.slide_id.clone());
    let slide_index = outcome.upsert.as_ref().map(|upsert| upsert.index);
    let result = json!({
        "ok": true,
        "tool": name,
        "created": outcome.created,
        "upsert": outcome.upsert,
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
            _ if !in_tag => {
                if !ch.is_whitespace() || !out.ends_with(' ') {
                    out.push(if ch.is_whitespace() { ' ' } else { ch });
                }
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

    #[test]
    fn slide_previews_drop_style_and_script_bodies() {
        let preview = one_line_preview(
            "<style>.hero { color: red; }</style><section><h1>Real copy</h1></section><script>noise()</script>",
        );
        assert_eq!(preview, "Real copy");
    }
}
