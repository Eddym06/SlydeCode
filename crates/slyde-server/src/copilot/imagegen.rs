//! Image generation through a provider the user already connected.
//!
//! Only the providers whose key also unlocks an image model are offered:
//! OpenAI (`gpt-image-1`), Gemini (`gemini-2.5-flash-image`), xAI
//! (`grok-2-image-1212`) and OpenRouter (its routed image endpoint). Everything
//! is server-side: the browser never sees the key, and the result goes straight
//! into the deck's asset store.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use super::catalog::{self, ImageWire, ProviderDef};
use super::error::CopilotError;
use super::{CopilotState, webfetch};

/// A prompt is a sentence or two, not a document.
const MAX_PROMPT_CHARS: usize = 4_000;
/// Same ceiling as an uploaded picture, so a generated one behaves exactly like
/// a pasted one.
const MAX_IMAGE_BYTES: usize = crate::assets::MAX_IMAGE_BYTES;
/// A balance probe must not delay the text prompt for a minute.
const BALANCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub provider: &'static str,
    pub model: String,
    pub revised_prompt: Option<String>,
    /// Real amount the provider charged, when it reports one (OpenRouter).
    pub cost_usd: Option<f64>,
}

#[derive(Debug, PartialEq)]
struct Reply {
    b64: Option<String>,
    url: Option<String>,
    mime: Option<String>,
    revised_prompt: Option<String>,
    cost_usd: Option<f64>,
}

/// Generates one picture with `provider_id`, or with the first connected
/// provider that offers an image model.
pub async fn generate(
    copilot: &CopilotState,
    provider_id: Option<&str>,
    prompt: &str,
) -> Result<GeneratedImage, CopilotError> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(CopilotError::InvalidRequest(
            "an image prompt is required".into(),
        ));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(CopilotError::InvalidRequest(format!(
            "the image prompt is over {MAX_PROMPT_CHARS} characters"
        )));
    }

    let provider = pick(copilot, provider_id).await?;
    let spec = provider.image.ok_or_else(|| {
        CopilotError::InvalidRequest(format!("{} does not offer image generation", provider.name))
    })?;
    let key = copilot
        .key_for(provider)
        .await
        .ok_or(CopilotError::MissingKey)?;
    let model = copilot.image_model_for(provider).await;
    let model = if model.trim().is_empty() {
        spec.model.to_string()
    } else {
        model
    };
    let url = provider.image_url_for(&key, &model).ok_or_else(|| {
        CopilotError::InvalidRequest(format!("{} does not offer image generation", provider.name))
    })?;

    let body = match spec.wire {
        ImageWire::OpenAi => {
            let mut body = json!({
                "model": model.clone(),
                "prompt": prompt,
                "n": 1,
            });
            // The OpenAI-compatible endpoint has model-specific sizes. xAI's
            // model accepts the prompt as-is and does not need one.
            if provider.id == "openai"
                && let Some(size) = openai_size(&model)
            {
                body["size"] = json!(size);
            }
            body
        }
        ImageWire::OpenRouter => json!({
            "model": model.clone(),
            "prompt": prompt,
        }),
        ImageWire::Gemini => json!({
            "contents": [{ "parts": [{ "text": prompt }] }]
        }),
    };

    let mut request = copilot
        .http()
        .post(&url)
        .header("accept", "application/json")
        .json(&body);
    match spec.wire {
        ImageWire::OpenAi | ImageWire::OpenRouter => {
            request = request.bearer_auth(&key);
            if spec.wire == ImageWire::OpenRouter {
                request = request.header("X-Title", "SlydeCode");
            }
        }
        ImageWire::Gemini => {}
    }
    let response = request
        .send()
        .await
        .map_err(|error| CopilotError::from_network(&error))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(CopilotError::from_status_redacting(status, &body, &key));
    }
    let payload: Value = response.json().await.map_err(|_| {
        CopilotError::Upstream("the provider returned an unexpected image response".into())
    })?;

    let reply = match spec.wire {
        ImageWire::OpenAi => parse_openai(&payload),
        ImageWire::OpenRouter => parse_openrouter(&payload),
        ImageWire::Gemini => parse_gemini(&payload),
    }
    .ok_or_else(|| CopilotError::Upstream("the provider returned no image".into()))?;

    let bytes = match (reply.b64, reply.url) {
        (Some(b64), _) => STANDARD.decode(b64).map_err(|_| {
            CopilotError::Upstream("the provider returned image data that is not base64".into())
        })?,
        (None, Some(remote)) => webfetch::fetch_image(&remote).await.map_err(|error| {
            CopilotError::Upstream(format!(
                "the generated image could not be downloaded: {error}"
            ))
        })?,
        (None, None) => {
            return Err(CopilotError::Upstream(
                "the provider returned no image".into(),
            ));
        }
    };
    if bytes.is_empty() {
        return Err(CopilotError::Upstream(
            "the provider returned an empty image".into(),
        ));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(CopilotError::TooLarge(format!(
            "the generated image is larger than {MAX_IMAGE_BYTES} bytes"
        )));
    }

    Ok(GeneratedImage {
        bytes,
        mime: reply.mime.unwrap_or_else(|| "image/png".to_string()),
        provider: provider.id,
        model,
        revised_prompt: reply.revised_prompt,
        cost_usd: reply.cost_usd,
    })
}

/// OpenAI's image endpoint rejects a size the model does not support. Keep the
/// 3:2 slide aspect where the model allows it, and fall back to its native size.
fn openai_size(model: &str) -> Option<&'static str> {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("dall-e-3") {
        Some("1792x1024")
    } else if lower.starts_with("dall-e-2") {
        Some("1024x1024")
    } else if lower.contains("gpt-image") {
        Some("1536x1024")
    } else {
        None
    }
}

async fn pick(
    copilot: &CopilotState,
    provider_id: Option<&str>,
) -> Result<&'static ProviderDef, CopilotError> {
    if let Some(id) = provider_id {
        let provider = catalog::find(id)
            .ok_or_else(|| CopilotError::InvalidRequest("unknown provider".into()))?;
        return Ok(provider);
    }
    // The provider the user chose in "Model for: image" wins over the first
    // connected one, so a deck can be written and illustrated by different
    // vendors.
    if let Some(preferred) = copilot.preferred_image_provider().await {
        return Ok(preferred);
    }
    for provider in catalog::PROVIDERS.iter().filter(|p| p.image.is_some()) {
        if copilot.key_for(provider).await.is_some() {
            return Ok(provider);
        }
    }
    Err(CopilotError::MissingKey)
}

/// Remaining credit reported by OpenRouter, when the key is allowed to ask.
///
/// OpenRouter documents `/credits` for management keys and `/key` for the
/// per-key limit. Both are tried, and an inference key that can call neither
/// simply leaves the balance unknown instead of failing the turn.
pub async fn balance(copilot: &CopilotState, provider: &ProviderDef, key: &str) -> Option<f64> {
    if provider.id != "openrouter" {
        return None;
    }
    // The capability probe sits in front of the text prompt, so it must never
    // inherit the client's 60-second budget. Both endpoints are asked at once;
    // the first one that reports a balance wins.
    let client = copilot.http();
    let credits = client
        .get("https://openrouter.ai/api/v1/credits")
        .bearer_auth(key)
        .header("X-Title", "SlydeCode")
        .timeout(BALANCE_TIMEOUT)
        .send();
    let key_limit = client
        .get("https://openrouter.ai/api/v1/key")
        .bearer_auth(key)
        .header("X-Title", "SlydeCode")
        .timeout(BALANCE_TIMEOUT)
        .send();
    let (credits, key_limit) = tokio::join!(credits, key_limit);

    if let Ok(response) = credits
        && response.status().is_success()
        && let Ok(payload) = response.json::<Value>().await
        && let Some(balance) = parse_credits(&payload)
    {
        return Some(balance);
    }

    let response = key_limit.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let payload = response.json::<Value>().await.ok()?;
    parse_key_limit(&payload)
}

fn parse_credits(payload: &Value) -> Option<f64> {
    let data = payload.get("data")?;
    let total = data.get("total_credits").and_then(Value::as_f64)?;
    let used = data
        .get("total_usage")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    Some((total - used).max(0.0))
}

fn parse_key_limit(payload: &Value) -> Option<f64> {
    let data = payload.get("data")?;
    let limit = data
        .get("limit")
        .and_then(Value::as_f64)
        .filter(|limit| *limit > 0.0)?;
    let used = data.get("usage").and_then(Value::as_f64).unwrap_or(0.0);
    Some((limit - used).max(0.0))
}

fn parse_openai(payload: &Value) -> Option<Reply> {
    let first = payload.get("data")?.as_array()?.first()?;
    let text = |key: &str| {
        first
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let b64 = text("b64_json");
    let url = text("url");
    if b64.is_none() && url.is_none() {
        return None;
    }
    Some(Reply {
        b64,
        url,
        mime: None,
        revised_prompt: text("revised_prompt"),
        cost_usd: None,
    })
}

/// OpenRouter's routed image endpoint answers with the same `data` array as
/// OpenAI, plus the media type of the picture and the real amount charged.
fn parse_openrouter(payload: &Value) -> Option<Reply> {
    let first = payload.get("data")?.as_array()?.first()?;
    let text = |key: &str| {
        first
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let b64 = text("b64_json");
    let url = text("url");
    if b64.is_none() && url.is_none() {
        return None;
    }
    let mime = text("media_type")
        .or_else(|| text("mime_type"))
        .or_else(|| {
            // Some routed providers only describe the format.
            first
                .get("output_format")
                .and_then(Value::as_str)
                .map(|format| format!("image/{format}"))
        });
    Some(Reply {
        b64,
        url,
        mime,
        revised_prompt: text("revised_prompt"),
        cost_usd: payload
            .pointer("/usage/cost")
            .and_then(Value::as_f64)
            .filter(|cost| *cost >= 0.0),
    })
}

fn parse_gemini(payload: &Value) -> Option<Reply> {
    for candidate in payload.get("candidates")?.as_array()? {
        let Some(parts) = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for part in parts {
            let Some(inline) = part.get("inlineData").or_else(|| part.get("inline_data")) else {
                continue;
            };
            let Some(data) = inline
                .get("data")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            return Some(Reply {
                b64: Some(data.to_string()),
                url: None,
                mime: inline
                    .get("mimeType")
                    .or_else(|| inline.get("mime_type"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                revised_prompt: None,
                cost_usd: None,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use tokio::net::TcpListener;

    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUg==";

    #[test]
    fn openai_wire_responses_keep_base64_urls_and_the_revised_prompt() {
        let inline = parse_openai(&json!({
            "data": [{ "b64_json": "abc", "revised_prompt": "A calmer version" }]
        }))
        .unwrap();
        assert_eq!(inline.b64.as_deref(), Some("abc"));
        assert_eq!(inline.revised_prompt.as_deref(), Some("A calmer version"));

        let remote =
            parse_openai(&json!({ "data": [{ "url": "https://cdn.example/a.png" }] })).unwrap();
        assert_eq!(remote.url.as_deref(), Some("https://cdn.example/a.png"));
        assert!(parse_openai(&json!({ "data": [] })).is_none());
        assert!(parse_openai(&json!({ "error": "nope" })).is_none());
    }

    #[test]
    fn openrouter_responses_carry_media_type_and_the_real_cost() {
        let reply = parse_openrouter(&json!({
            "data": [{
                "b64_json": "abc",
                "media_type": "image/webp",
                "revised_prompt": "Refined"
            }],
            "usage": { "cost": 0.0391 }
        }))
        .unwrap();
        assert_eq!(reply.b64.as_deref(), Some("abc"));
        assert_eq!(reply.mime.as_deref(), Some("image/webp"));
        assert_eq!(reply.cost_usd, Some(0.0391));
        assert_eq!(reply.revised_prompt.as_deref(), Some("Refined"));

        let format_only = parse_openrouter(&json!({
            "data": [{ "b64_json": "abc", "output_format": "png" }]
        }))
        .unwrap();
        assert_eq!(format_only.mime.as_deref(), Some("image/png"));
        assert_eq!(format_only.cost_usd, None, "a missing cost stays unknown");
        assert!(parse_openrouter(&json!({ "data": [] })).is_none());
    }

    #[test]
    fn openrouter_balance_understands_both_credit_shapes() {
        assert_eq!(
            parse_credits(&json!({
                "data": { "total_credits": 10.0, "total_usage": 3.25 }
            })),
            Some(6.75)
        );
        assert_eq!(
            parse_credits(&json!({ "data": { "total_credits": 1.0, "total_usage": 2.0 } })),
            Some(0.0),
            "an overdrawn account has no balance, not a negative number"
        );
        assert_eq!(parse_credits(&json!({ "data": {} })), None);

        assert_eq!(
            parse_key_limit(&json!({ "data": { "limit": 5.0, "usage": 1.5 } })),
            Some(3.5)
        );
        assert_eq!(
            parse_key_limit(&json!({ "data": { "limit": null, "usage": 1.5 } })),
            None,
            "an unlimited key reports no finite balance"
        );
    }

    #[test]
    fn gemini_responses_take_the_first_inline_image_part() {
        let reply = parse_gemini(&json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "Here you go" },
                    { "inlineData": { "mimeType": "image/jpeg", "data": "abc" } }
                ]}
            }]
        }))
        .unwrap();
        assert_eq!(reply.b64.as_deref(), Some("abc"));
        assert_eq!(reply.mime.as_deref(), Some("image/jpeg"));
        assert!(parse_gemini(&json!({ "candidates": [{ "content": { "parts": [] } }] })).is_none());
    }

    #[tokio::test]
    async fn an_empty_prompt_is_rejected_before_a_provider_is_chosen() {
        let state = CopilotState::in_memory();
        let error = generate(&state, None, "   ").await.unwrap_err();
        assert_eq!(error.code(), "invalid_request");
    }

    #[tokio::test]
    async fn a_provider_without_an_image_model_says_so() {
        let state = CopilotState::in_memory();
        let error = generate(&state, Some("anthropic"), "a red apple")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_request");
        assert!(error.public_message().contains("Claude"), "{error:?}");
    }

    /// The OpenAI-wire path is exercised against a local server: the request is
    /// real, the provider is not. This keeps the test independent of a vendor
    /// key while still proving that the response is decoded into bytes.
    #[tokio::test]
    async fn an_openai_wire_provider_returns_the_decoded_picture() {
        let app = Router::new().route(
            "/images/generations",
            post(|| async { Json(json!({ "data": [{ "b64_json": PNG_B64 }] })) }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        catalog::tests_support::register_image_mock_as(
            "mock-image",
            &format!("http://127.0.0.1:{port}"),
        );

        let state = CopilotState::in_memory();
        state
            .config
            .write()
            .await
            .keys
            .insert("mock-image".into(), "mock-key".into());
        let image = generate(&state, Some("mock-image"), "a red apple")
            .await
            .unwrap();
        assert_eq!(image.bytes, STANDARD.decode(PNG_B64).unwrap());
        assert_eq!(image.model, "mock-image");
        assert_eq!(image.provider, "mock-image");
    }
}
