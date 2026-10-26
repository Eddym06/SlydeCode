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
    },
];

pub fn find(id: &str) -> Option<&'static ProviderDef> {
    #[cfg(test)]
    if let Some(mock) = tests_support::MOCK.get().filter(|mock| mock.id == id) {
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

    pub fn env_key(self) -> Option<String> {
        self.key_env.iter().find_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    }
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
}

/// A scripted provider for end-to-end tests of the tool loop: tests point it at a
/// local mock server instead of a vendor.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use std::sync::OnceLock;

    pub static MOCK: OnceLock<ProviderDef> = OnceLock::new();

    pub fn register_mock(base: &str) {
        let base: &'static str = Box::leak(base.to_string().into_boxed_str());
        let _ = MOCK.set(ProviderDef {
            id: "mock",
            name: "Mock",
            company: "Mock",
            icon: "mock",
            key_url: "",
            key_env: &[],
            wire: Wire::OpenAi,
            chat_base: base,
            models_url: base,
            model_shape: ModelShape::OpenAi,
        });
    }
}
