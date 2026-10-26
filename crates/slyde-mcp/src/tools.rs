//! MCP tool definitions and dispatch.
//!
//! The tool surface is a public contract with external agents (see AGENTS.md
//! §12): names, argument names and response shapes are versioned. Changes must
//! be additive or carry a migration note.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use slyde_server::RuntimeInfo;
use slyde_server::http::{get_json, post_bytes, post_json};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub fn definitions() -> Value {
    json!([
        {
            "name": "create_presentation",
            "description": "Create a new, empty presentation and replace whatever is currently in the studio. Use this once at the start of a deck; use upsert_slide to add slides.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Deck title, shown in the studio header." },
                    "theme": {
                        "type": "string",
                        "enum": slyde_core::THEME_IDS,
                        "description": "Visual theme id. studio = warm dark, paper = warm light, night = deep indigo, terminal = dark mono, academic = light serif, pitch = deep blue."
                    },
                    "aspect_ratio": {
                        "type": "string",
                        "enum": ["16:9", "4:3", "9:16"],
                        "description": "Canvas aspect ratio. Defaults to 16:9."
                    }
                },
                "required": ["title"],
                "additionalProperties": false
            }
        },
        {
            "name": "upsert_slide",
            "description": "Insert a new slide or update an existing one. slide_id is authoritative: if it matches an existing slide, that slide is updated in place and index is ignored (the response tells you the real index). Reusing the same slide_id makes retries idempotent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "index": { "type": "integer", "minimum": 0, "description": "Insertion position, 0-based. Use the slide count to append. Required only when inserting; ignored when slide_id matches an existing slide." },
                    "slide_id": { "type": "string", "description": "Stable id. Omit to let the server generate one; pass the id returned by a previous call to update that slide." },
                    "html_code": {
                        "type": "string",
                        "description": "Self-contained HTML fragment for the slide body. Inline <style> is allowed. The stage is 1280x720 for 16:9 (720x1280 for 9:16, 1024x768 for 4:3). No network requests: images must be data: or SVG. Theme colors are available as var(--slyde-bg), var(--slyde-fg), var(--slyde-muted), var(--slyde-accent), var(--slyde-font)."
                    },
                    "template": {
                        "type": "string",
                        "enum": ["code", "diagram", "bullet", "hero", "image_layout", "chart", "custom"],
                        "description": "Semantic label for the storyboard. Does not change rendering."
                    },
                    "animation": {
                        "type": "array",
                        "description": "Ordered reveal steps. Targets are CSS selectors inside the slide document.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "target": { "type": "string", "description": "CSS selector, e.g. .title or [data-anim=bar]" },
                                "kind": {
                                    "type": "string",
                                    "enum": slyde_core::ANIMATION_KIND_IDS
                                },
                                "delay_ms": { "type": "integer", "minimum": 0, "default": 0 },
                                "duration_ms": { "type": "integer", "minimum": 0, "default": 520 },
                                "easing": { "type": "string", "default": "cubic-bezier(.16,1,.3,1)" }
                            },
                            "required": ["target", "kind"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["html_code"],
                "additionalProperties": false
            }
        },
        {
            "name": "remove_slide",
            "description": "Remove a slide by id.",
            "inputSchema": {
                "type": "object",
                "properties": { "slide_id": { "type": "string" } },
                "required": ["slide_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "move_slide",
            "description": "Reorder slides by moving one index to another.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from_index": { "type": "integer", "minimum": 0 },
                    "to_index": { "type": "integer", "minimum": 0 }
                },
                "required": ["from_index", "to_index"],
                "additionalProperties": false
            }
        },
        {
            "name": "trigger_animation",
            "description": "Play one animation step in the live studio and in the presenter. Use it to demo motion after writing a slide.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slide_id": { "type": "string" },
                    "step_index": { "type": "integer", "minimum": 0 }
                },
                "required": ["slide_id", "step_index"],
                "additionalProperties": false
            }
        },
        {
            "name": "inspect_current_state",
            "description": "Read the full presentation state: deck metadata, every slide with its code and animation, plus undo/redo availability.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "export_presentation",
            "description": "Export the deck through the same Chrome renderer the presenter uses. PDF produces one page per slide at the deck aspect ratio; PNG produces one image per slide; PPTX produces a PowerPoint file with one full-bleed picture per slide and the speaker notes intact.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "format": { "type": "string", "enum": ["pdf", "png", "pptx"], "default": "pdf" },
                    "output_path": {
                        "type": "string",
                        "description": "File path for PDF, directory for PNG. Defaults to exports/ inside the working directory."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "get_selected_element",
            "description": "Inspect one element by id: either a structured slide element (Image, Diagram, CodeBlock, SpeakerNotes, Custom) or a node marked with id=/data-id= inside the slide HTML. Returns the slide it lives in plus its markup or data.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "element_id": { "type": "string", "description": "Identifier of the element (e.g. title-1, or a structured element id)." }
                },
                "required": ["element_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "mutate_element",
            "description": "Apply surgical style, layout, inner HTML or text changes to a specific component element without reloading the entire slide. Pass slide_id whenever the deck has more than one slide.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "element_id": { "type": "string", "description": "The element ID to mutate." },
                    "slide_id": { "type": "string", "description": "Slide that owns the element. Required when the id is not unique across the deck: generated handles (slyde-1, slyde-2, …) repeat in every slide, and an unscoped ambiguous id is rejected." },
                    "text": { "type": "string", "description": "New plain text content for the element." },
                    "style": { "type": "string", "description": "CSS style directives to merge into the element style attribute." },
                    "html": { "type": "string", "description": "New inner HTML markup for the element." }
                },
                "required": ["element_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "search_asset",
            "description": "Return inline SVG markup for a keyword (cloud, database, server, arrow, code, …) from the built-in curated icon set. Offline by design: local asset files and stock photo catalogs are not wired yet, and asset_type=image reports found=false.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search keyword (e.g. 'arrow', 'cloud', 'database')." },
                    "asset_type": { "type": "string", "enum": ["icon", "image"], "default": "icon" }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        },
        {
            "name": "generate_image",
            "description": "Reserve an image slot with a placeholder inline SVG that echoes the prompt and style. No image model is wired yet: the response is marked placeholder=true and nothing is written to assets/images/.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Visual prompt describing the desired graphic or image." },
                    "style": { "type": "string", "enum": ["minimal_vector", "photographic", "diagrammatic", "editorial"], "default": "minimal_vector" }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }
        }
    ])
}

#[derive(Debug)]
pub struct ToolFailure(pub String);

pub async fn call(runtime: &RuntimeInfo, name: &str, args: &Value) -> Result<String, ToolFailure> {
    match name {
        "create_presentation" => {
            let title = require_str(args, "title")?;
            let mut command = json!({ "action": "create_presentation", "title": title });
            if let Some(theme) = optional_str(args, "theme") {
                command["theme"] = json!(theme);
            }
            if let Some(ratio) = optional_str(args, "aspect_ratio") {
                command["aspect_ratio"] = json!(ratio);
            }
            let response = post(runtime, &command).await?;
            let slides = response
                .pointer("/view/presentation/slides")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            Ok(format!(
                "Created presentation \"{}\" with theme {} and {slides} slides.",
                response
                    .pointer("/view/presentation/title")
                    .and_then(Value::as_str)
                    .unwrap_or(title),
                response
                    .pointer("/view/presentation/theme/id")
                    .and_then(Value::as_str)
                    .unwrap_or("studio")
            ))
        }

        "upsert_slide" => {
            let slide_id = optional_str(args, "slide_id").map(str::to_string);
            // `index` only matters when inserting. Replacing by id ignores it, so
            // an agent that already knows the id never has to invent one.
            let index = match require_u64(args, "index") {
                Ok(index) => index,
                Err(_) if slide_id.is_some() => 0,
                Err(err) => return Err(err),
            };
            let html_code = require_str(args, "html_code")?;
            let mut command = json!({
                "action": "upsert_slide",
                "index": index,
                "html_code": html_code
            });
            if let Some(slide_id) = slide_id {
                command["slide_id"] = json!(slide_id);
            }
            if let Some(template) = optional_str(args, "template") {
                command["template"] = json!(template);
            }
            if let Some(animation) = args.get("animation") {
                if !animation.is_null() {
                    command["animation"] = animation.clone();
                }
            }

            let response = post(runtime, &command).await?;
            let upsert = response.get("upsert").cloned().unwrap_or(Value::Null);
            Ok(format!(
                "{} slide {} at index {}.\n{}",
                if upsert
                    .get("inserted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    "Inserted"
                } else {
                    "Updated"
                },
                upsert
                    .get("slide_id")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                upsert.get("index").and_then(Value::as_u64).unwrap_or(index),
                serde_json::to_string_pretty(&upsert).unwrap_or_default()
            ))
        }

        "remove_slide" => {
            let slide_id = require_str(args, "slide_id")?;
            post(
                runtime,
                &json!({ "action": "remove_slide", "slide_id": slide_id }),
            )
            .await?;
            Ok(format!("Removed slide {slide_id}."))
        }

        "move_slide" => {
            let from = require_u64(args, "from_index")?;
            let to = require_u64(args, "to_index")?;
            post(
                runtime,
                &json!({ "action": "move_slide", "from": from, "to": to }),
            )
            .await?;
            Ok(format!("Moved slide from index {from} to {to}."))
        }

        "trigger_animation" => {
            let slide_id = require_str(args, "slide_id")?;
            let step_index = require_u64(args, "step_index")?;
            post(
                runtime,
                &json!({
                    "action": "trigger_animation",
                    "slide_id": slide_id,
                    "step_index": step_index
                }),
            )
            .await?;
            Ok(format!(
                "Playing animation step {step_index} on slide {slide_id}."
            ))
        }

        "inspect_current_state" => {
            let url = format!(
                "http://127.0.0.1:{}/api/state?t={}",
                runtime.port, runtime.token
            );
            let response = get_json(&url, REQUEST_TIMEOUT)
                .await
                .map_err(|err| ToolFailure(format!("could not read state: {err}")))?;
            let view = response.get("view").cloned().unwrap_or(Value::Null);
            serde_json::to_string_pretty(&view)
                .map_err(|err| ToolFailure(format!("could not serialize state: {err}")))
        }

        "export_presentation" => {
            let format = optional_str(args, "format")
                .unwrap_or("pdf")
                .to_ascii_lowercase();
            let output = optional_str(args, "output_path").map(PathBuf::from);

            let state = get_json(
                &format!(
                    "http://127.0.0.1:{}/api/state?t={}",
                    runtime.port, runtime.token
                ),
                REQUEST_TIMEOUT,
            )
            .await
            .map_err(|err| ToolFailure(format!("could not read the deck: {err}")))?;
            let deck = state
                .pointer("/view/presentation")
                .ok_or_else(|| ToolFailure("the studio returned no deck".into()))?;
            let title = deck.get("title").and_then(Value::as_str).unwrap_or("deck");
            let aspect_ratio = deck
                .get("aspect_ratio")
                .and_then(Value::as_str)
                .and_then(slyde_core::AspectRatio::parse)
                .unwrap_or_default();
            // `/s/{id}` requires a token; the state response publishes the one
            // meant for slide documents, with the session token as a fallback.
            let slide_token = state
                .get("sandbox_token")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| runtime.token.clone());
            let slides: Vec<slyde_export::SlideTarget> = deck
                .get("slides")
                .and_then(Value::as_array)
                .map(|slides| {
                    slides
                        .iter()
                        .filter_map(|slide| slide.get("id").and_then(Value::as_str))
                        .map(|id| slyde_export::SlideTarget {
                            id: id.to_string(),
                            url: format!(
                                "http://127.0.0.1:{}/s/{}?static=1&t={}",
                                runtime.sandbox_port, id, slide_token
                            ),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if slides.is_empty() {
                return Err(ToolFailure("the deck has no slides to export".into()));
            }

            let browser = slyde_server::discover_browser().map_err(|_| {
                ToolFailure(
                    "no supported browser found; install Google Chrome or set SLYDE_BROWSER".into(),
                )
            })?;
            let stem = slyde_server::storage::file_stem_for(title);

            match format.as_str() {
                "pdf" => {
                    let path = output
                        .unwrap_or_else(|| PathBuf::from("exports").join(format!("{stem}.pdf")));
                    let url = format!(
                        "http://127.0.0.1:{}/print?t={}",
                        runtime.sandbox_port, runtime.token
                    );
                    let written = slyde_export::export_pdf(&browser.path, &url, &path, None)
                        .await
                        .map_err(|err| ToolFailure(err.to_string()))?;
                    Ok(format!("Exported PDF to {}", written.display()))
                }
                "pptx" => {
                    // The server owns the pipeline (render + OOXML packaging), so
                    // the MCP surface gets the same file the UI produces.
                    let path = output
                        .unwrap_or_else(|| PathBuf::from("exports").join(format!("{stem}.pptx")));
                    let url = format!(
                        "http://127.0.0.1:{}/api/export?format=pptx&t={}",
                        runtime.port, runtime.token
                    );
                    let bytes = post_bytes(&url, REQUEST_TIMEOUT * 6)
                        .await
                        .map_err(|err| ToolFailure(format!("could not export: {err}")))?;
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|err| ToolFailure(err.to_string()))?;
                    }
                    std::fs::write(&path, bytes).map_err(|err| {
                        ToolFailure(format!("could not write {}: {err}", path.display()))
                    })?;
                    Ok(format!("Exported PPTX to {}", path.display()))
                }
                "png" => {
                    let dir = output.unwrap_or_else(|| PathBuf::from("exports").join(&stem));
                    let files =
                        slyde_export::export_png(&browser.path, &slides, aspect_ratio, &dir, None)
                            .await
                            .map_err(|err| ToolFailure(err.to_string()))?;
                    Ok(format!(
                        "Exported {} PNG files to {}\n{}",
                        files.len(),
                        dir.display(),
                        files
                            .iter()
                            .map(|f| f.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n")
                    ))
                }
                other => Err(ToolFailure(format!(
                    "unsupported format: {other} (use pdf, png or pptx)"
                ))),
            }
        }

        "get_selected_element" => {
            let element_id = require_str(args, "element_id")?;
            let url = format!(
                "http://127.0.0.1:{}/api/state?t={}",
                runtime.port, runtime.token
            );
            let response = get_json(&url, REQUEST_TIMEOUT)
                .await
                .map_err(|err| ToolFailure(format!("could not read state: {err}")))?;
            let slides = response
                .pointer("/view/presentation/slides")
                .and_then(Value::as_array);

            if let Some(slides) = slides {
                let id_needle = format!("id=\"{element_id}\"");
                let data_needle = format!("data-id=\"{element_id}\"");
                for s in slides {
                    let structured = s
                        .get("elements")
                        .and_then(Value::as_array)
                        .and_then(|list| {
                            list.iter().find(|element| {
                                element.get("element_id").and_then(Value::as_str)
                                    == Some(element_id)
                            })
                        });
                    let code = s.get("code").and_then(Value::as_str).unwrap_or("");
                    let marker = code.find(&id_needle).or_else(|| code.find(&data_needle));
                    if structured.is_none() && marker.is_none() {
                        continue;
                    }
                    let markup = marker.map(|position| {
                        let start = code[..position].rfind('<').unwrap_or(position);
                        code[start..].chars().take(240).collect::<String>()
                    });
                    return Ok(json!({
                        "element_id": element_id,
                        "slide_id": s.get("id"),
                        "found": true,
                        "element": structured,
                        "markup": markup
                    })
                    .to_string());
                }
            }
            Ok(json!({
                "element_id": element_id,
                "found": false
            })
            .to_string())
        }

        "mutate_element" => {
            let element_id = require_str(args, "element_id")?;
            let mut command = json!({
                "action": "mutate_element",
                "element_id": element_id
            });
            if let Some(slide_id) = optional_str(args, "slide_id") {
                command["slide_id"] = json!(slide_id);
            }
            if let Some(t) = optional_str(args, "text") {
                command["text"] = json!(t);
            }
            if let Some(s) = optional_str(args, "style") {
                command["style"] = json!(s);
            }
            if let Some(h) = optional_str(args, "html") {
                command["html"] = json!(h);
            }
            let res = post(runtime, &command).await?;
            Ok(match res.get("mutated_slide_id").and_then(Value::as_str) {
                Some(slide) => format!("Mutated element {element_id} on slide {slide}."),
                None => format!("Mutated element {element_id}."),
            })
        }

        "search_asset" => {
            let query = require_str(args, "query")?.to_lowercase();
            let asset_type = optional_str(args, "asset_type").unwrap_or("icon");
            if asset_type == "image" {
                return Ok(json!({
                    "query": query,
                    "asset_type": asset_type,
                    "source": "builtin",
                    "found": false,
                    "note": "image asset search is not wired yet; only the built-in SVG icon set is available"
                })
                .to_string());
            }
            // Curated SVG icons for common technical & design domains. This set is
            // fixed and offline; there is no catalog or network lookup behind it.
            let icon_result = match query.as_str() {
                q if q.contains("cloud") => {
                    "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><path d=\"M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z\"/></svg>"
                }
                q if q.contains("database") || q.contains("db") => {
                    "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><ellipse cx=\"12\" cy=\"5\" rx=\"9\" ry=\"3\"/><path d=\"M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5\"/><path d=\"M3 12c0 1.66 4 3 9 3s9-1.34 9-3\"/></svg>"
                }
                q if q.contains("server") => {
                    "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><rect width=\"20\" height=\"8\" x=\"2\" y=\"2\" rx=\"2\" ry=\"2\"/><rect width=\"20\" height=\"8\" x=\"2\" y=\"14\" rx=\"2\" ry=\"2\"/><line x1=\"6\" x2=\"6.01\" y1=\"6\" y2=\"6\"/><line x1=\"6\" x2=\"6.01\" y1=\"18\" y2=\"18\"/></svg>"
                }
                q if q.contains("arrow") => {
                    "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><path d=\"M5 12h14\"/><path d=\"m12 5 7 7-7 7\"/></svg>"
                }
                q if q.contains("code") || q.contains("terminal") => {
                    "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><polyline points=\"16 18 22 12 16 6\"/><polyline points=\"8 6 2 12 8 18\"/></svg>"
                }
                _ => {
                    "<svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><circle cx=\"12\" cy=\"12\" r=\"10\"/><line x1=\"12\" x2=\"12\" y1=\"8\" y2=\"12\"/><line x1=\"12\" x2=\"12.01\" y1=\"16\" y2=\"16\"/></svg>"
                }
            };
            Ok(json!({
                "query": query,
                "asset_type": asset_type,
                "source": "builtin",
                "found": true,
                "svg": icon_result,
                "usable_markup": format!("<span class=\"slyde-icon\">{}</span>", icon_result)
            })
            .to_string())
        }

        "generate_image" => {
            let prompt = require_str(args, "prompt")?;
            let style = optional_str(args, "style").unwrap_or("minimal_vector");
            // Placeholder only: no image model is wired, so the deck gets an
            // inline, scalable SVG that reserves the slot and echoes the brief.
            let placeholder_svg = format!(
                "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 800 450' width='100%' height='100%'><rect width='100%' height='100%' fill='%23121214'/><rect x='40' y='40' width='720' height='370' rx='12' fill='none' stroke='rgba(255,255,255,0.1)' stroke-dasharray='4 4'/><text x='50%' y='48%' text-anchor='middle' fill='%23F4F3F1' font-family='sans-serif' font-size='20' font-weight='600'>{}</text><text x='50%' y='56%' text-anchor='middle' fill='%23FF6B4A' font-family='sans-serif' font-size='14'>Style: {}</text></svg>",
                prompt.replace('\'', ""),
                style
            );
            Ok(json!({
                "prompt": prompt,
                "style": style,
                "placeholder": true,
                "saved_path": null,
                "note": "no image model is wired yet; this is a placeholder SVG, not a generated image",
                "src": placeholder_svg
            }).to_string())
        }

        other => Err(ToolFailure(format!("unknown tool: {other}"))),
    }
}

async fn post(runtime: &RuntimeInfo, command: &Value) -> Result<Value, ToolFailure> {
    let url = format!(
        "http://127.0.0.1:{}/api/agent?t={}",
        runtime.port, runtime.token
    );
    let response = post_json(&url, command, REQUEST_TIMEOUT)
        .await
        .map_err(|err| ToolFailure(err.to_string()))?;

    if response.get("ok").and_then(Value::as_bool) == Some(false) {
        let message = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("command failed");
        return Err(ToolFailure(message.to_string()));
    }
    Ok(response)
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolFailure> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolFailure(format!("missing required string argument: {key}")))
}

fn optional_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn require_u64(args: &Value, key: &str) -> Result<u64, ToolFailure> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolFailure(format!("missing required integer argument: {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_name_description_and_schema() {
        let tools = definitions();
        let list = tools.as_array().unwrap();
        assert_eq!(list.len(), 11);
        for tool in list {
            assert!(tool["name"].as_str().unwrap().len() > 3);
            assert!(tool["description"].as_str().unwrap().len() > 20);
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn required_argument_errors_name_the_argument() {
        let err = require_str(&json!({}), "html_code").unwrap_err();
        assert!(err.0.contains("html_code"));
    }

    fn tool(name: &str) -> Value {
        definitions()
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is not published"))
    }

    /// The MCP schema is the public contract with external agents. It must accept
    /// exactly what the core validates, never a subset: a model that follows the
    /// schema must not be rejected for using a kind or theme we support.
    #[test]
    fn published_enums_match_the_core_contract() {
        let upsert = tool("upsert_slide");
        let kinds = upsert["inputSchema"]["properties"]["animation"]["items"]["properties"]["kind"]
            ["enum"]
            .as_array()
            .unwrap();
        let expected: Vec<Value> = slyde_core::ANIMATION_KIND_IDS
            .iter()
            .map(|id| json!(id))
            .collect();
        assert_eq!(kinds, &expected);

        let create = tool("create_presentation");
        let themes = create["inputSchema"]["properties"]["theme"]["enum"]
            .as_array()
            .unwrap();
        let expected: Vec<Value> = slyde_core::THEME_IDS.iter().map(|id| json!(id)).collect();
        assert_eq!(themes, &expected);
    }

    /// Replacing a slide by id must not force the caller to invent an index.
    #[test]
    fn upsert_requires_html_only_and_lets_slide_id_replace() {
        let upsert = tool("upsert_slide");
        assert_eq!(upsert["inputSchema"]["required"], json!(["html_code"]));
        assert!(
            upsert["inputSchema"]["properties"]["index"]["description"]
                .as_str()
                .is_some_and(|text| text.contains("ignored"))
        );
    }

    #[test]
    fn mutate_element_documents_the_slide_scope() {
        let mutate = tool("mutate_element");
        assert_eq!(
            mutate["inputSchema"]["properties"]["slide_id"]["type"],
            "string"
        );
        assert!(
            mutate["description"]
                .as_str()
                .is_some_and(|text| text.contains("slide_id")),
            "the description must tell the agent to scope the edit"
        );
    }

    #[tokio::test]
    async fn unknown_tool_is_reported_clearly() {
        let runtime = RuntimeInfo {
            port: 1,
            sandbox_port: 2,
            token: "t".into(),
            pid: 0,
            started_at_ms: 0,
        };
        let err = call(&runtime, "nope", &json!({})).await.unwrap_err();
        assert!(err.0.contains("unknown tool"));
    }
}
