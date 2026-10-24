//! MCP tool definitions and dispatch.
//!
//! The tool surface is a public contract with external agents (see AGENTS.md
//! §12): names, argument names and response shapes are versioned. Changes must
//! be additive or carry a migration note.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use slyde_server::RuntimeInfo;
use slyde_server::http::{get_json, post_json};

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
                        "enum": ["studio", "paper", "night"],
                        "description": "Visual theme. studio = warm dark, paper = warm light, night = deep indigo."
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
                    "index": { "type": "integer", "minimum": 0, "description": "Insertion position, 0-based. Use the slide count to append." },
                    "slide_id": { "type": "string", "description": "Stable id. Omit to let the server generate one; pass the id returned by a previous call to update that slide." },
                    "html_code": {
                        "type": "string",
                        "description": "Self-contained HTML fragment for the slide body. Inline <style> is allowed. The stage is 1280x720 for 16:9 (720x1280 for 9:16, 1024x768 for 4:3). No network requests: images must be data: or SVG. Theme colors are available as var(--slyde-bg), var(--slyde-fg), var(--slyde-muted), var(--slyde-accent), var(--slyde-font)."
                    },
                    "template": {
                        "type": "string",
                        "enum": ["code", "diagram", "bullet", "hero", "image_layout", "chart"],
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
                                    "enum": ["fade_up", "fade", "scale_in", "slide_in_left", "slide_in_right", "typewriter"]
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
                "required": ["index", "html_code"],
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
            "description": "Export the deck through the same Chrome renderer the presenter uses. PDF produces one page per slide at the deck aspect ratio; PNG produces one image per slide.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "format": { "type": "string", "enum": ["pdf", "png"], "default": "pdf" },
                    "output_path": {
                        "type": "string",
                        "description": "File path for PDF, directory for PNG. Defaults to exports/ inside the working directory."
                    }
                },
                "additionalProperties": false
            }
        }
    ])
}

#[derive(Debug)]
pub struct ToolFailure(pub String);

pub async fn call(
    runtime: &RuntimeInfo,
    name: &str,
    args: &Value,
) -> Result<String, ToolFailure> {
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
                    .unwrap_or(&title),
                response
                    .pointer("/view/presentation/theme/id")
                    .and_then(Value::as_str)
                    .unwrap_or("studio")
            ))
        }

        "upsert_slide" => {
            let index = require_u64(args, "index")?;
            let html_code = require_str(args, "html_code")?;
            let mut command = json!({
                "action": "upsert_slide",
                "index": index,
                "html_code": html_code
            });
            if let Some(slide_id) = optional_str(args, "slide_id") {
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
                if upsert.get("inserted").and_then(Value::as_bool).unwrap_or(false) {
                    "Inserted"
                } else {
                    "Updated"
                },
                upsert.get("slide_id").and_then(Value::as_str).unwrap_or("?"),
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
            let format = optional_str(args, "format").unwrap_or("pdf").to_ascii_lowercase();
            let output = optional_str(args, "output_path").map(PathBuf::from);
            match format.as_str() {
                "pdf" => {
                    let path = output.unwrap_or_else(default_pdf);
                    let written = slyde_export::export_pdf(runtime, &path)
                        .await
                        .map_err(|err| ToolFailure(err.to_string()))?;
                    Ok(format!("Exported PDF to {}", written.display()))
                }
                "png" => {
                    let dir = output.unwrap_or_else(|| PathBuf::from("exports").join("slides"));
                    let files = slyde_export::export_png(runtime, &dir)
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
                    "unsupported format: {other} (use pdf or png)"
                ))),
            }
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

fn default_pdf() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from("exports").join(format!("deck-{stamp}.pdf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_name_description_and_schema() {
        let tools = definitions();
        let list = tools.as_array().unwrap();
        assert_eq!(list.len(), 7);
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
