//! Provider catalog.
//!
//! Everything that is stable about a provider lives here: its public id, the
//! protocol it speaks, where its endpoints live and which environment variable
//! can supply a key. No secrets, no UI copy.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Wire {
    /// `POST {base}/chat/completions`, OpenAI JSON schema.
    OpenAi,
    /// `POST {base}/messages`, Anthropic Messages schema.
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelShape {
    /// `{ "data": [{ "id": ... }] }` with OpenRouter-style metadata.
    OpenAi,
    /// Anthropic `{ "data": [{ "id": ..., "display_name": ... }] }`.
    Anthropic,
    /// Gemini `{ "models": [{ "name": "models/...", ... }] }`.
    Gemini,
}

/// How a provider accepts an image-generation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageWire {
    /// `POST {url}` with `{model, prompt}`, answering `{data:[{b64_json|url}]}`.
    OpenAi,
    /// OpenRouter's image endpoint: `POST {url}` with `{model, prompt}`,
    /// answering `{data:[{b64_json, media_type}], usage:{cost}}`.
    OpenRouter,
    /// Gemini `generateContent` with `{contents:[{parts:[{text}]}]}`, answering
    /// `candidates[0].content.parts[*].inlineData`.
    Gemini,
}

/// The image model a provider offers, when it offers one.
#[derive(Debug, Clone, Copy)]
pub struct ImageSpec {
    pub wire: ImageWire,
    pub url: &'static str,
    pub model: &'static str,
    /// Conservative list price for one picture. Vendors that report the real
    /// cost (OpenRouter) replace it as soon as a job completes.
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderDef {
    pub id: &'static str,
    pub name: &'static str,
    pub company: &'static str,
    pub icon: &'static str,
    pub key_url: &'static str,
    /// First variable wins. `*_API_KEY` is the convention every vendor uses.
    pub key_env: &'static [&'static str],
    pub wire: Wire,
    pub chat_base: &'static str,
    pub models_url: &'static str,
    pub model_shape: ModelShape,
    /// None when the provider has no image model behind the same key.
    pub image: Option<ImageSpec>,
}

pub const PROVIDERS: &[ProviderDef] = &[
    ProviderDef {
        id: "openai",
        name: "ChatGPT",
        company: "OpenAI",
        icon: "openai",
        key_url: "https://platform.openai.com/api-keys",
        key_env: &["OPENAI_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://api.openai.com/v1",
        models_url: "https://api.openai.com/v1/models",
        model_shape: ModelShape::OpenAi,
        image: Some(ImageSpec {
            wire: ImageWire::OpenAi,
            url: "https://api.openai.com/v1/images/generations",
            model: "gpt-image-1",
            cost_usd: 0.04,
        }),
    },
    ProviderDef {
        id: "anthropic",
        name: "Claude",
        company: "Anthropic",
        icon: "claude",
        key_url: "https://console.anthropic.com/settings/keys",
        key_env: &["ANTHROPIC_API_KEY"],
        wire: Wire::Anthropic,
        chat_base: "https://api.anthropic.com/v1",
        models_url: "https://api.anthropic.com/v1/models?limit=1000",
        model_shape: ModelShape::Anthropic,
        image: None,
    },
    ProviderDef {
        id: "gemini",
        name: "Gemini",
        company: "Google",
        icon: "gemini",
        key_url: "https://aistudio.google.com/apikey",
        key_env: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://generativelanguage.googleapis.com/v1beta/openai",
        models_url: "https://generativelanguage.googleapis.com/v1beta/models",
        model_shape: ModelShape::Gemini,
        image: Some(ImageSpec {
            wire: ImageWire::Gemini,
            url: "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-image:generateContent",
            model: "gemini-2.5-flash-image",
            cost_usd: 0.039,
        }),
    },
    ProviderDef {
        id: "xai",
        name: "Grok",
        company: "xAI",
        icon: "xai",
        key_url: "https://console.x.ai",
        key_env: &["XAI_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://api.x.ai/v1",
        models_url: "https://api.x.ai/v1/models",
        model_shape: ModelShape::OpenAi,
        image: Some(ImageSpec {
            wire: ImageWire::OpenAi,
            url: "https://api.x.ai/v1/images/generations",
            model: "grok-2-image-1212",
            cost_usd: 0.07,
        }),
    },
    ProviderDef {
        id: "groq",
        name: "Groq",
        company: "Groq",
        icon: "groq",
        key_url: "https://console.groq.com/keys",
        key_env: &["GROQ_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://api.groq.com/openai/v1",
        models_url: "https://api.groq.com/openai/v1/models",
        model_shape: ModelShape::OpenAi,
        image: None,
    },
    ProviderDef {
        id: "openrouter",
        name: "OpenRouter",
        company: "OpenRouter",
        icon: "openrouter",
        key_url: "https://openrouter.ai/keys",
        key_env: &["OPENROUTER_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://openrouter.ai/api/v1",
        models_url: "https://openrouter.ai/api/v1/models",
        model_shape: ModelShape::OpenAi,
        image: Some(ImageSpec {
            wire: ImageWire::OpenRouter,
            url: "https://openrouter.ai/api/v1/images",
            model: "google/gemini-2.5-flash-image",
            cost_usd: 0.04,
        }),
    },
    ProviderDef {
        id: "deepseek",
        name: "DeepSeek",
        company: "DeepSeek",
        icon: "deepseek",
        key_url: "https://platform.deepseek.com/api_keys",
        key_env: &["DEEPSEEK_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://api.deepseek.com",
        models_url: "https://api.deepseek.com/models",
        model_shape: ModelShape::OpenAi,
        image: None,
    },
    ProviderDef {
        id: "nvidia",
        name: "NVIDIA",
        company: "NVIDIA NIM",
        icon: "nvidia",
        key_url: "https://build.nvidia.com",
        key_env: &["NVIDIA_API_KEY"],
        wire: Wire::OpenAi,
        chat_base: "https://integrate.api.nvidia.com/v1",
        models_url: "https://integrate.api.nvidia.com/v1/models",
        model_shape: ModelShape::OpenAi,
        image: None,
    },
];

pub fn find(id: &str) -> Option<&'static ProviderDef> {
    #[cfg(test)]
    if let Some(mock) = tests_support::lookup(id) {
        return Some(mock);
    }
    PROVIDERS.iter().find(|provider| provider.id == id)
}

impl ProviderDef {
    pub fn chat_url(self) -> String {
        match self.wire {
            Wire::OpenAi => format!("{}/chat/completions", self.chat_base),
            Wire::Anthropic => format!("{}/messages", self.chat_base),
        }
    }

    /// Gemini's native model list uses `?key=`, not an Authorization header.
    pub fn models_url(self, key: &str) -> String {
        match self.model_shape {
            ModelShape::Gemini => format!("{}?key={}", self.models_url, encode_query(key)),
            _ => self.models_url.to_string(),
        }
    }

    /// OpenRouter keeps its full picture catalog on a separate endpoint; the
    /// normal `/models` list only advertises the handful that also answer text.
    pub fn image_models_url(self, key: &str) -> String {
        if self.id == "openrouter" {
            "https://openrouter.ai/api/v1/images/models".to_string()
        } else {
            self.models_url(key)
        }
    }

    /// Per-endpoint pricing for one OpenRouter image model. Direct vendors do
    /// not expose this metadata, so they return `None` and use the catalog
    /// estimate instead.
    pub fn image_endpoints_url(self, model: &str) -> Option<String> {
        if self.id != "openrouter" || !safe_routed_model(model) {
            return None;
        }
        Some(format!(
            "https://openrouter.ai/api/v1/images/models/{model}/endpoints"
        ))
    }

    /// Image endpoint for the provider, with the key folded in where the vendor
    /// expects it in the query string instead of a bearer header.
    pub fn image_url(self, key: &str) -> Option<String> {
        self.image.map(|spec| match spec.wire {
            ImageWire::Gemini => format!("{}?key={}", spec.url, encode_query(key)),
            ImageWire::OpenAi | ImageWire::OpenRouter => spec.url.to_string(),
        })
    }

    /// Image endpoint for a selected model. Gemini puts the model in the path;
    /// the routed and OpenAI-compatible vendors take it in the JSON body.
    pub fn image_url_for(self, key: &str, model: &str) -> Option<String> {
        let spec = self.image?;
        match spec.wire {
            ImageWire::Gemini => {
                if !safe_model_segment(model) {
                    return None;
                }
                let suffix = format!("{}:generateContent", spec.model);
                let stem = spec.url.strip_suffix(&suffix).unwrap_or(spec.url);
                Some(format!(
                    "{stem}{model}:generateContent?key={}",
                    encode_query(key)
                ))
            }
            ImageWire::OpenAi | ImageWire::OpenRouter => Some(spec.url.to_string()),
        }
    }

    pub fn env_key(self) -> Option<String> {
        self.key_env.iter().find_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    }
}

/// Gemini model ids are path segments. Reject anything that could escape the
/// `/models/` segment; the catalog default and every vendor id match this set.
fn safe_model_segment(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && model
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// Routed OpenRouter ids contain a provider prefix and a slash. They are still
/// path segments on the pricing endpoint, so traversal sequences are refused.
fn safe_routed_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && !model.contains("..")
        && !model.starts_with('/')
        && model
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | ':'))
}

fn encode_query(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_has_a_unique_id_and_endpoint() {
        let mut ids = std::collections::BTreeSet::new();
        for provider in PROVIDERS {
            assert!(ids.insert(provider.id));
            assert!(provider.chat_url().starts_with("https://"));
            assert!(provider.models_url.starts_with("https://"));
            assert!(!provider.name.is_empty());
            assert!(!provider.key_url.is_empty());
        }
    }

    #[test]
    fn gemini_models_url_escapes_the_key() {
        let gemini = find("gemini").unwrap();
        let url = gemini.models_url("a b+c");
        assert!(url.contains("key=a%20b%2Bc"));
        assert!(!url.contains("a b+c"));
    }

    #[test]
    fn every_image_model_has_a_secure_endpoint_and_only_known_providers_offer_one() {
        for provider in PROVIDERS {
            if let Some(image) = provider.image {
                assert!(
                    image.url.starts_with("https://"),
                    "{} must not send a key over plain http",
                    provider.id
                );
                assert!(!image.model.is_empty());
            }
        }
        let offer = |id: &str| PROVIDERS.iter().any(|p| p.id == id && p.image.is_some());
        assert!(offer("openai"));
        assert!(offer("gemini"));
        assert!(offer("xai"));
        assert!(offer("openrouter"));
        assert!(!offer("anthropic"));
        assert!(!offer("deepseek"));

        let openrouter = find("openrouter").unwrap().image.unwrap();
        assert_eq!(openrouter.wire, ImageWire::OpenRouter);
        assert!(openrouter.cost_usd > 0.0);

        let gemini = find("gemini").unwrap();
        assert!(gemini.image_url("a b+c").unwrap().contains("key=a%20b%2Bc"));
        assert!(
            gemini
                .image_url_for("a b+c", "gemini-2.5-flash-image-preview")
                .unwrap()
                .contains("/models/gemini-2.5-flash-image-preview:generateContent?key=a%20b%2Bc")
        );
        assert!(
            gemini.image_url_for("key", "../another-model").is_none(),
            "a model id may not escape the path segment"
        );
        let openrouter = find("openrouter").unwrap();
        assert_eq!(
            openrouter.image_models_url("key"),
            "https://openrouter.ai/api/v1/images/models"
        );
        assert_eq!(
            openrouter
                .image_endpoints_url("sourceful/riverflow-v2.6-pro")
                .as_deref(),
            Some(
                "https://openrouter.ai/api/v1/images/models/sourceful/riverflow-v2.6-pro/endpoints"
            )
        );
        assert!(openrouter.image_endpoints_url("../escape").is_none());
        assert!(
            openrouter
                .image_endpoints_url("openai/gpt-image-1")
                .is_some()
        );
        assert!(
            find("openai")
                .unwrap()
                .image_endpoints_url("gpt-image-1")
                .is_none()
        );
    }
}

/// A scripted provider for end-to-end tests of the tool loop: tests point it at a
/// local mock server instead of a vendor.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use std::sync::Mutex;

    static MOCKS: Mutex<Vec<&'static ProviderDef>> = Mutex::new(Vec::new());

    pub fn lookup(id: &str) -> Option<&'static ProviderDef> {
        let mocks = MOCKS.lock().ok()?;
        mocks.iter().copied().find(|mock| mock.id == id)
    }

    /// Registers the scripted provider `mock`.
    pub fn register_mock(base: &str) {
        register_mock_as("mock", base);
    }

    /// Registers a scripted provider under its own id, so tests that script
    /// different behaviors can run side by side.
    pub fn register_mock_as(id: &str, base: &str) {
        let base: &'static str = Box::leak(base.to_string().into_boxed_str());
        let id: &'static str = Box::leak(id.to_string().into_boxed_str());
        let def: &'static ProviderDef = Box::leak(Box::new(ProviderDef {
            id,
            name: "Mock",
            company: "Mock",
            icon: "mock",
            key_url: "",
            key_env: &[],
            wire: Wire::OpenAi,
            chat_base: base,
            models_url: base,
            model_shape: ModelShape::OpenAi,
            image: None,
        }));
        if let Ok(mut mocks) = MOCKS.lock() {
            mocks.retain(|mock| mock.id != id);
            mocks.push(def);
        }
    }

    /// Registers a scripted provider that also answers image generation at
    /// `{base}/images/generations`, so the image path is tested without a vendor.
    #[cfg(test)]
    pub fn register_image_mock_as(id: &str, base: &str) {
        register_mock_as(id, base);
        let image_url: &'static str =
            Box::leak(format!("{base}/images/generations").into_boxed_str());
        let mut mocks = match MOCKS.lock() {
            Ok(mocks) => mocks,
            Err(error) => error.into_inner(),
        };
        if let Some(def) = mocks.iter_mut().find(|mock| mock.id == id) {
            *def = Box::leak(Box::new(ProviderDef {
                image: Some(ImageSpec {
                    wire: ImageWire::OpenAi,
                    url: image_url,
                    model: "mock-image",
                    cost_usd: 0.01,
                }),
                ..**def
            }));
        }
    }
}
