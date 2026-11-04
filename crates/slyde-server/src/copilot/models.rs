//! Provider model-list normalization.
//!
//! Every vendor describes models differently. The UI needs one small, stable
//! shape: id, label, free flag, context window, optional vision capability and
//! optional list prices. Prices are normalized to USD per million tokens so the
//! sheet can compare models without knowing the vendor's raw units.

use std::collections::{HashMap, HashSet};

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
    /// USD per million input tokens, when the vendor publishes a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_price: Option<f64>,
    /// USD per million output tokens, when the vendor publishes a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_price: Option<f64>,
    /// USD for one generated picture, when the model is an image model and the
    /// vendor publishes a per-image price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_price: Option<f64>,
}

pub fn parse_models(provider: &ProviderDef, payload: &Value) -> Vec<ModelInfo> {
    let Some(list) = model_list(payload) else {
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

    normalize(&mut models);
    models
}

/// The models a provider can draw with.
///
/// OpenRouter routes dozens of image models through one endpoint and publishes
/// them at `/images/models`; its general `/models` list only carries the subset
/// that also answers text. Direct vendors expose only a handful; those are
/// filtered by name and, when present, by the declared output modality. A
/// provider with no discoverable list falls back to its catalog default in
/// `CopilotState`.
pub fn parse_image_models(provider: &ProviderDef, payload: &Value) -> Vec<ModelInfo> {
    let Some(list) = model_list(payload) else {
        return Vec::new();
    };

    let mut models = list
        .iter()
        .filter_map(|item| match provider.model_shape {
            ModelShape::Gemini => gemini_image_model(item),
            ModelShape::Anthropic => None,
            ModelShape::OpenAi => openai_image_model(provider, item),
        })
        .collect::<Vec<_>>();

    normalize(&mut models);
    models
}

/// Copies token prices from the general `/models` payload into a picture list.
/// OpenRouter's dedicated image catalog omits pricing, while its general list
/// carries it for the models that also answer text.
pub fn merge_prices(models: &mut [ModelInfo], payload: &Value) {
    let Some(list) = model_list(payload) else {
        return;
    };
    let mut prices = HashMap::new();
    for item in list {
        let Some(id) = string_at(item, "id") else {
            continue;
        };
        let (input, output, _) = prices_at(item);
        if input.is_some() || output.is_some() {
            prices.insert(id, (input, output));
        }
    }
    for model in models {
        let Some((input, output)) = prices.get(&model.id) else {
            continue;
        };
        model.input_price = model.input_price.or(*input);
        model.output_price = model.output_price.or(*output);
    }
}

/// The highest base charge for one generated picture from an OpenRouter image
/// endpoint payload. Taking the maximum across endpoints keeps the budget
/// conservative when several providers serve the same route. Token-priced image
/// models are estimated at 1K image tokens; the real charge replaces it after
/// the first completed job.
pub fn parse_image_price(payload: &Value) -> Option<f64> {
    let endpoints = payload.get("endpoints")?.as_array()?;
    let mut base: Option<f64> = None;
    let mut variant: Option<f64> = None;
    for endpoint in endpoints {
        let Some(pricing) = endpoint.get("pricing").and_then(Value::as_array) else {
            continue;
        };
        for price in pricing {
            if price.get("billable").and_then(Value::as_str) != Some("output_image") {
                continue;
            }
            let Some(cost) = price.get("cost_usd").and_then(as_f64) else {
                continue;
            };
            let estimate = match price.get("unit").and_then(Value::as_str) {
                Some("image") => cost,
                Some("token") => cost * 1_024.0,
                _ => continue,
            };
            if !estimate.is_finite() || estimate <= 0.0 {
                continue;
            }
            if price.get("variant").is_some() {
                variant = Some(variant.map_or(estimate, |current| current.max(estimate)));
            } else {
                base = Some(base.map_or(estimate, |current| current.max(estimate)));
            }
        }
    }
    base.or(variant)
}

fn model_list(payload: &Value) -> Option<&Vec<Value>> {
    payload
        .get("models")
        .or_else(|| payload.get("data"))
        .and_then(Value::as_array)
        .or_else(|| payload.as_array())
}

fn normalize(models: &mut Vec<ModelInfo>) {
    let mut seen = HashSet::new();
    models.retain(|model| seen.insert(model.id.clone()));
    models.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn openai_model(item: &Value) -> Option<ModelInfo> {
    let id = string_at(item, "id")?;
    let name = string_at(item, "name")
        .or_else(|| string_at(item, "display_name"))
        .unwrap_or_else(|| id.clone());
    // OpenRouter lists the request fields each model honors. A model without
    // `tools` cannot drive the copilot, and choosing it only produces a rejection.
    if let Some(supported) = item.get("supported_parameters").and_then(Value::as_array)
        && !supported
            .iter()
            .any(|param| param.as_str() == Some("tools"))
    {
        return None;
    }
    let free = is_free_model(item, &id);
    let context_window = number_at(item, "context_window")
        .or_else(|| number_at(item, "context_length"))
        .or_else(|| number_at(item, "max_context_window"))
        .or_else(|| number_at(item, "max_input_tokens"))
        .or_else(|| nested_number(item, "limits", "max_input_tokens"));
    let vision = vision_at(item);
    let (input_price, output_price, image_price) = prices_at(item);
    Some(ModelInfo {
        id,
        name,
        free,
        vision,
        context_window,
        input_price,
        output_price,
        image_price,
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
    let (input_price, output_price, image_price) = prices_at(item);
    Some(ModelInfo {
        id,
        name,
        free: false,
        vision,
        context_window: number_at(item, "context_window")
            .or_else(|| number_at(item, "max_input_tokens")),
        input_price,
        output_price,
        image_price,
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
    let (input_price, output_price, image_price) = prices_at(item);
    Some(ModelInfo {
        id,
        name,
        free: false,
        vision,
        context_window: number_at(item, "inputTokenLimit"),
        input_price,
        output_price,
        image_price,
    })
}

fn openai_image_model(provider: &ProviderDef, item: &Value) -> Option<ModelInfo> {
    let id = string_at(item, "id")?;
    if !is_image_model(provider, item, &id) {
        return None;
    }
    let name = string_at(item, "name")
        .or_else(|| string_at(item, "display_name"))
        .unwrap_or_else(|| id.clone());
    let (input_price, output_price, image_price) = prices_at(item);
    let free = is_free_model(item, &id);
    Some(ModelInfo {
        id,
        name,
        free,
        vision: vision_at(item),
        context_window: number_at(item, "context_window")
            .or_else(|| number_at(item, "context_length")),
        input_price,
        output_price,
        image_price,
    })
}

fn gemini_image_model(item: &Value) -> Option<ModelInfo> {
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
    if !is_image_model_id(&id) {
        return None;
    }
    let name = string_at(item, "displayName").unwrap_or_else(|| id.clone());
    let (input_price, output_price, image_price) = prices_at(item);
    Some(ModelInfo {
        id,
        name,
        free: false,
        vision: Some(true),
        context_window: number_at(item, "inputTokenLimit"),
        input_price,
        output_price,
        image_price,
    })
}

fn is_image_model(provider: &ProviderDef, item: &Value, id: &str) -> bool {
    if provider.id == "openrouter" {
        if output_modalities(item)
            .is_some_and(|modalities| modalities.iter().any(|modality| modality == "image"))
        {
            return true;
        }
        // Older payloads omit `architecture`; the id and a per-image price are
        // still enough to tell a routed image model apart.
        if item
            .get("pricing")
            .and_then(|pricing| pricing.get("image"))
            .is_some_and(|price| !is_zero(price))
        {
            return true;
        }
    }
    is_image_model_id(id)
}

fn output_modalities(item: &Value) -> Option<Vec<String>> {
    item.get("architecture")
        .and_then(|architecture| architecture.get("output_modalities"))
        .or_else(|| item.get("output_modalities"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.to_ascii_lowercase())
                .collect()
        })
}

fn is_image_model_id(id: &str) -> bool {
    const IMAGE_MARKERS: &[&str] = &[
        "image",
        "imagen",
        "dall-e",
        "gpt-image",
        "flux",
        "stable-diffusion",
        "sdxl",
        "seedream",
        "recraft",
        "ideogram",
        "nano-banana",
        "photon",
        "kolors",
        "qwen-image",
        "wanx",
        "grok-2-image",
    ];
    let lower = id.to_ascii_lowercase();
    IMAGE_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Reads OpenRouter's per-token prices and normalizes them to USD per million.
/// Other vendors do not expose pricing in their model-list payloads, so those
/// fields stay `None` and the sheet shows an honest dash. `pricing.image` is
/// deliberately ignored here: it is an image *input* token rate, not the price
/// of one generated picture.
fn prices_at(item: &Value) -> (Option<f64>, Option<f64>, Option<f64>) {
    let Some(pricing) = item.get("pricing") else {
        return (None, None, None);
    };
    let per_million = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| pricing.get(*key))
            .and_then(as_f64)
            .filter(|value| *value >= 0.0)
            .map(|value| value * 1_000_000.0)
    };
    (
        per_million(&["prompt", "input"]),
        per_million(&["completion", "output"]),
        None,
    )
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
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
    let image = pricing.get("image");
    match (prompt, completion) {
        (Some(prompt), Some(completion)) => {
            is_zero(prompt) && is_zero(completion) && image.is_none_or(is_zero)
        }
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
        // Groq and NVIDIA list safety, speech and retrieval models next to chat ones.
        "guard",
        "playai",
        "orpheus",
        "compound",
        "retriever",
        "nvclip",
        "reward",
        "safety",
        "parse",
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

    #[test]
    fn openrouter_prices_are_normalized_per_million_tokens() {
        let payload = json!({
            "data": [{
                "id": "openai/gpt-4o",
                "name": "GPT-4o",
                "pricing": { "prompt": "0.0000025", "completion": "0.00001" }
            }]
        });
        let models = parse_models(find("openrouter").unwrap(), &payload);
        assert_eq!(models[0].input_price, Some(2.5));
        assert_eq!(models[0].output_price, Some(10.0));
        assert_eq!(models[0].image_price, None);
    }

    #[test]
    fn openrouter_image_models_keep_every_image_capable_route() {
        let payload = json!({
            "data": [
                {
                    "id": "openai/gpt-4o",
                    "name": "GPT-4o",
                    "architecture": { "output_modalities": ["text"] },
                    "pricing": { "prompt": "0.0000025", "completion": "0.00001" }
                },
                {
                    "id": "google/gemini-2.5-flash-image",
                    "name": "Gemini 2.5 Flash Image",
                    "architecture": { "output_modalities": ["text", "image"] },
                    "pricing": { "prompt": "0.0000003", "completion": "0.0000025", "image": "0.039" }
                },
                {
                    "id": "black-forest-labs/flux-1.1-pro",
                    "name": "FLUX 1.1 Pro",
                    "architecture": { "output_modalities": ["image"] },
                    "pricing": { "prompt": "0", "completion": "0", "image": "0.04" }
                }
            ]
        });
        let models = parse_image_models(find("openrouter").unwrap(), &payload);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "black-forest-labs/flux-1.1-pro");
        assert_eq!(
            models[0].image_price, None,
            "pricing.image is an input token rate, not a per-picture price"
        );
        assert!(!models[0].free, "a paid picture is not a free model");
        let gemini = models
            .iter()
            .find(|model| model.id == "google/gemini-2.5-flash-image")
            .unwrap();
        assert_eq!(gemini.input_price, Some(0.3));
        assert_eq!(gemini.output_price, Some(2.5));
        assert_eq!(gemini.image_price, None);
    }

    #[test]
    fn image_lists_include_direct_vendor_image_models_and_skip_chat_models() {
        let payload = json!({
            "data": [
                { "id": "gpt-4o" },
                { "id": "gpt-image-1" },
                { "id": "dall-e-3" },
                { "id": "text-embedding-3-large" }
            ]
        });
        let models = parse_image_models(find("openai").unwrap(), &payload);
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["dall-e-3", "gpt-image-1"]
        );
    }

    #[test]
    fn openrouter_endpoint_prices_take_the_highest_base_and_estimate_tokens() {
        let payload = json!({
            "endpoints": [
                { "pricing": [
                    { "billable": "output_image", "unit": "image", "cost_usd": 0.13 },
                    { "billable": "input_image", "unit": "image", "cost_usd": 0.01 }
                ]},
                { "pricing": [
                    { "billable": "output_image", "unit": "image", "cost_usd": 0.15 },
                    { "billable": "output_image", "unit": "token", "cost_usd": 0.00006 }
                ]},
                { "pricing": [
                    { "billable": "output_image", "unit": "image", "cost_usd": 0.17, "variant": "4k" }
                ]}
            ]
        });
        assert_eq!(
            parse_image_price(&payload),
            Some(0.15),
            "a variant surcharge must not replace the base price"
        );

        let token_priced = json!({
            "endpoints": [{ "pricing": [
                { "billable": "output_image", "unit": "token", "cost_usd": 0.00006 }
            ]}]
        });
        let estimate = parse_image_price(&token_priced).unwrap();
        assert!((estimate - 0.06144).abs() < 1e-12);

        let variant_only = json!({
            "endpoints": [{ "pricing": [
                { "billable": "output_image", "unit": "image", "cost_usd": 0.17, "variant": "4k" }
            ]}]
        });
        assert_eq!(parse_image_price(&variant_only), Some(0.17));
    }

    #[test]
    fn openrouter_prices_merge_into_the_image_catalog_by_id() {
        let mut models = vec![
            ModelInfo {
                id: "google/gemini-2.5-flash-image".into(),
                name: "Gemini 2.5 Flash Image".into(),
                free: false,
                vision: None,
                context_window: None,
                input_price: None,
                output_price: None,
                image_price: None,
            },
            ModelInfo {
                id: "black-forest-labs/flux-1.1-pro".into(),
                name: "FLUX 1.1 Pro".into(),
                free: false,
                vision: None,
                context_window: None,
                input_price: None,
                output_price: None,
                image_price: None,
            },
            ModelInfo {
                id: "unknown/model".into(),
                name: "Unknown".into(),
                free: false,
                vision: None,
                context_window: None,
                input_price: None,
                output_price: None,
                image_price: None,
            },
        ];
        let payload = json!({
            "data": [
                {
                    "id": "google/gemini-2.5-flash-image",
                    "pricing": { "prompt": "0.0000003", "completion": "0.0000025" }
                },
                {
                    "id": "black-forest-labs/flux-1.1-pro",
                    "pricing": { "prompt": "0", "completion": "0", "image": "0.04" }
                }
            ]
        });
        merge_prices(&mut models, &payload);

        assert_eq!(models[0].input_price, Some(0.3));
        assert_eq!(models[0].output_price, Some(2.5));
        assert_eq!(models[1].input_price, Some(0.0));
        assert_eq!(models[1].output_price, Some(0.0));
        assert_eq!(
            models[1].image_price, None,
            "pricing.image is not a per-picture price"
        );
        assert_eq!(models[2].input_price, None);
        assert_eq!(models[2].output_price, None);
    }
}
