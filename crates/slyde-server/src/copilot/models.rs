//! Provider model-list normalization.
//!
//! Every vendor describes models differently. The UI needs one small, stable
//! shape: id, label, free flag, context window and optional vision capability.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

use super::catalog::{ModelShape, ProviderDef};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub free: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

pub fn parse_models(provider: &ProviderDef, payload: &Value) -> Vec<ModelInfo> {
    let list = match provider.model_shape {
        ModelShape::Gemini => payload.get("models"),
        _ => payload.get("data"),
    }
    .and_then(Value::as_array)
    .or_else(|| payload.as_array());

    let Some(list) = list else {
        return Vec::new();
    };

    let mut models = list
        .iter()
        .filter_map(|item| match provider.model_shape {
            ModelShape::Gemini => gemini_model(item),
            ModelShape::Anthropic => anthropic_model(item),
            ModelShape::OpenAi => openai_model(item),
        })
        .filter(|model| !is_non_chat(&model.id))
        .collect::<Vec<_>>();

    let mut seen = HashSet::new();
    models.retain(|model| seen.insert(model.id.clone()));
    models.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    models
}

fn openai_model(item: &Value) -> Option<ModelInfo> {
    let id = string_at(item, "id")?;
    let name = string_at(item, "name")
        .or_else(|| string_at(item, "display_name"))
        .unwrap_or_else(|| id.clone());
    let free = is_free_model(item, &id);
    let context_window = number_at(item, "context_window")
        .or_else(|| number_at(item, "context_length"))
        .or_else(|| number_at(item, "max_context_window"))
        .or_else(|| number_at(item, "max_input_tokens"))
        .or_else(|| nested_number(item, "limits", "max_input_tokens"));
    let vision = vision_at(item);
    Some(ModelInfo {
        id,
        name,
        free,
        vision,
        context_window,
    })
}

fn anthropic_model(item: &Value) -> Option<ModelInfo> {
    let id = string_at(item, "id")?;
    let name = string_at(item, "display_name").unwrap_or_else(|| id.clone());
    let vision = item
        .get("capabilities")
        .and_then(|caps| caps.get("image_input"))
        .and_then(|image| image.get("supported"))
        .and_then(Value::as_bool);
    Some(ModelInfo {
        id,
        name,
        free: false,
        vision,
        context_window: number_at(item, "context_window")
            .or_else(|| number_at(item, "max_input_tokens")),
    })
}

fn gemini_model(item: &Value) -> Option<ModelInfo> {
    let raw = string_at(item, "name")?;
    let id = raw.strip_prefix("models/").unwrap_or(&raw).to_string();
    let methods = item
        .get("supportedGenerationMethods")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if !methods.is_empty() && !methods.contains(&"generateContent") {
        return None;
    }
    let name = string_at(item, "displayName").unwrap_or_else(|| id.clone());
    // Every current `generateContent` Gemini model accepts image input; the
    // native model list does not expose a per-model vision capability flag.
    let vision = Some(true);
    Some(ModelInfo {
        id,
        name,
        free: false,
        vision,
        context_window: number_at(item, "inputTokenLimit"),
    })
}

fn is_free_model(item: &Value, id: &str) -> bool {
    if id.ends_with(":free") {
        return true;
    }
    let Some(pricing) = item.get("pricing") else {
        return false;
    };
    let prompt = pricing.get("prompt");
    let completion = pricing.get("completion");
    match (prompt, completion) {
        (Some(prompt), Some(completion)) => is_zero(prompt) && is_zero(completion),
        _ => false,
    }
}

fn is_zero(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64().is_some_and(|n| n == 0.0),
        Value::String(text) => text.trim().parse::<f64>().is_ok_and(|n| n == 0.0),
        _ => false,
    }
}

fn vision_at(item: &Value) -> Option<bool> {
    let modalities = item
        .get("architecture")
        .and_then(|architecture| architecture.get("input_modalities"))
        .or_else(|| item.get("input_modalities"))
        .or_else(|| item.get("modalities"));
    let values = modalities.and_then(Value::as_array)?;
    Some(values.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|text| text.eq_ignore_ascii_case("image"))
    }))
}

fn string_at(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn number_at(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn nested_number(value: &Value, outer: &str, inner: &str) -> Option<u64> {
    value
        .get(outer)
        .and_then(|node| node.get(inner))
        .and_then(Value::as_u64)
}

/// Model lists mix chat models with embeddings, speech and image generators.
/// The copilot only exposes models that can hold a conversation.
fn is_non_chat(id: &str) -> bool {
    const BLOCKED: &[&str] = &[
        "embedding",
        "embed-",
        "whisper",
        "transcribe",
        "transcription",
        "text-to-speech",
        "tts",
        "tts-",
        "dall-e",
        "stable-diffusion",
        "flux-",
        "sdxl",
        "rerank",
        "moderation",
        "guardrail",
        "imagine-image",
        "imagine-video",
        "veo-",
        "lyria",
        "nano-banana",
        "speech",
        "audio",
        "realtime",
        "computer-use",
        "image-generation",
        "gpt-image",
        "-image",
        "sora",
    ];
    let lower = id.to_ascii_lowercase();
    BLOCKED.iter().any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copilot::catalog::find;
    use serde_json::json;

    #[test]
    fn openrouter_zero_price_and_free_suffix_are_free() {
        let payload = json!({
            "data": [
                { "id": "vendor/model:free", "name": "Free model", "pricing": { "prompt": "0", "completion": "0" } },
                { "id": "vendor/paid", "name": "Paid model", "pricing": { "prompt": "0.000001", "completion": "0.000002" } }
            ]
        });
        let models = parse_models(find("openrouter").unwrap(), &payload);
        assert_eq!(models.len(), 2);
        assert!(
            models
                .iter()
                .find(|m| m.id.ends_with(":free"))
                .unwrap()
                .free
        );
        assert!(!models.iter().find(|m| m.id == "vendor/paid").unwrap().free);
    }

    #[test]
    fn gemini_names_are_stripped_and_embeddings_dropped() {
        let payload = json!({
            "models": [
                { "name": "models/gemini-2.5-pro", "displayName": "Gemini 2.5 Pro", "supportedGenerationMethods": ["generateContent"], "inputTokenLimit": 1048576 },
                { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] }
            ]
        });
        let models = parse_models(find("gemini").unwrap(), &payload);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-2.5-pro");
        assert_eq!(models[0].name, "Gemini 2.5 Pro");
        assert_eq!(models[0].context_window, Some(1048576));
    }

    #[test]
    fn duplicate_model_ids_keep_the_first_name() {
        let payload = json!({
            "data": [
                { "id": "gpt-4o", "name": "GPT-4o" },
                { "id": "gpt-4o", "name": "GPT-4o duplicate" },
                { "id": "gpt-4o-mini" }
            ]
        });
        let models = parse_models(find("openai").unwrap(), &payload);
        assert_eq!(models.len(), 2);
        assert_eq!(
            models.iter().find(|m| m.id == "gpt-4o").unwrap().name,
            "GPT-4o"
        );
    }

    #[test]
    fn non_chat_models_are_filtered() {
        let payload = json!({
            "data": [
                { "id": "gpt-4o" },
                { "id": "text-embedding-3-large" },
                { "id": "whisper-large-v3" },
                { "id": "dall-e-3" },
                { "id": "gpt-image-1" },
                { "id": "gpt-4o-mini-tts" },
                { "id": "gpt-4o-realtime-preview" },
                { "id": "sora-2" }
            ]
        });
        let models = parse_models(find("openai").unwrap(), &payload);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4o");
    }
}
