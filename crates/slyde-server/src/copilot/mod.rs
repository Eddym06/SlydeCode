//! The in-app copilot.
//!
//! The browser owns the visible chat; the server owns secrets, provider
//! transports, attachments and the tool loop. Every deck mutation goes through
//! `agent::run`, the same path used by MCP, so there is still exactly one deck.

pub mod attachments;
pub mod catalog;
pub mod chat;
pub mod config;
pub mod error;
pub mod models;
pub mod prompt;
pub mod routes;
pub mod tools;
pub mod transport;
pub mod wire;

use std::time::Duration;

use serde::Serialize;
use tokio::sync::RwLock;

use catalog::{PROVIDERS, ProviderDef};
use config::{ConfigStore, CopilotConfig};
use error::CopilotError;
use models::ModelInfo;

#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub company: &'static str,
    pub icon: &'static str,
    pub key_url: &'static str,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CopilotConfigView {
    pub providers: Vec<ProviderStatus>,
    pub last_provider: Option<String>,
    pub last_model: Option<String>,
}

/// Live turns: request id → the flag a running turn polls.
type TurnCancels = std::collections::HashMap<String, tokio::sync::watch::Sender<bool>>;

#[derive(Debug)]
pub struct CopilotState {
    store: ConfigStore,
    config: RwLock<CopilotConfig>,
    http: reqwest::Client,
    /// Live turns by request id. The browser sends the id with the turn and can
    /// cancel it with `POST /api/copilot/cancel`, which is the difference between
    /// "stop" aborting a stream locally and the server really stopping the
    /// provider call (and the tool rounds it would run).
    cancels: std::sync::Mutex<TurnCancels>,
}

impl CopilotState {
    pub fn persistent() -> Self {
        let store = ConfigStore::persistent();
        let config = store.load();
        Self::with_store(store, config)
    }

    pub fn in_memory() -> Self {
        Self::with_store(ConfigStore::in_memory(), CopilotConfig::default())
    }

    fn with_store(store: ConfigStore, config: CopilotConfig) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("SlydeCode/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            store,
            config: RwLock::new(config),
            http,
            cancels: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Registers a turn and returns the flag it must poll to notice a cancel.
    pub(crate) fn begin_turn(&self, request_id: &str) -> tokio::sync::watch::Receiver<bool> {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let mut cancels = self
            .cancels
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        cancels.insert(request_id.to_string(), tx);
        rx
    }

    /// Removes a finished turn. Returns false when the turn was already gone.
    pub(crate) fn end_turn(&self, request_id: &str) -> bool {
        let mut cancels = self
            .cancels
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        cancels.remove(request_id).is_some()
    }

    /// Signals a running turn. Returns false when no such turn is in flight.
    pub(crate) fn cancel_turn(&self, request_id: &str) -> bool {
        let cancels = self
            .cancels
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match cancels.get(request_id) {
            Some(tx) => tx.send(true).is_ok(),
            None => false,
        }
    }

    /// True when the browser asked to stop this turn.
    pub(crate) fn was_cancelled(flag: &tokio::sync::watch::Receiver<bool>) -> bool {
        *flag.borrow()
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.store.path()
    }

    pub async fn view(&self) -> CopilotConfigView {
        let config = self.config.read().await.clone();
        CopilotConfigView {
            providers: PROVIDERS
                .iter()
                .map(|provider| ProviderStatus {
                    id: provider.id,
                    name: provider.name,
                    company: provider.company,
                    icon: provider.icon,
                    key_url: provider.key_url,
                    configured: has_key(&config, *provider),
                })
                .collect(),
            last_provider: config.last_provider,
            last_model: config.last_model,
        }
    }

    pub async fn key_for(&self, provider: &ProviderDef) -> Option<String> {
        let config = self.config.read().await;
        config
            .keys
            .get(provider.id)
            .filter(|key| !key.trim().is_empty())
            .cloned()
            .or_else(|| provider.env_key())
    }

    pub async fn set_key(
        &self,
        provider_id: &str,
        key: &str,
    ) -> Result<Vec<ModelInfo>, CopilotError> {
        let provider = catalog::find(provider_id)
            .ok_or_else(|| CopilotError::InvalidRequest("unknown provider".into()))?;
        let key = key.trim();
        if key.is_empty() {
            let mut config = self.config.write().await;
            config.keys.remove(provider.id);
            let snapshot = config.clone();
            drop(config);
            self.save(&snapshot).await;
            return Ok(Vec::new());
        }
        if key.len() > 1024 || !key.chars().all(|ch| ch.is_ascii_graphic()) {
            return Err(CopilotError::InvalidRequest(
                "the API key contains unsupported characters".into(),
            ));
        }

        // Validate before persisting: a typo should not be remembered as a
        // working configuration.
        let models = self.fetch_models(provider, key).await?;
        let mut config = self.config.write().await;
        config.keys.insert(provider.id.to_string(), key.to_string());
        let snapshot = config.clone();
        drop(config);
        self.save(&snapshot).await;
        Ok(models)
    }

    pub async fn models(&self, provider_id: &str) -> Result<Vec<ModelInfo>, CopilotError> {
        let provider = catalog::find(provider_id)
            .ok_or_else(|| CopilotError::InvalidRequest("unknown provider".into()))?;
        let key = self
            .key_for(provider)
            .await
            .ok_or(CopilotError::MissingKey)?;
        self.fetch_models(provider, &key).await
    }

    async fn fetch_models(
        &self,
        provider: &ProviderDef,
        key: &str,
    ) -> Result<Vec<ModelInfo>, CopilotError> {
        let mut request = self
            .http
            .get(provider.models_url(key))
            .header("accept", "application/json");
        request = match provider.id {
            "anthropic" => request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
            "gemini" => request,
            "openrouter" => request.bearer_auth(key).header("X-Title", "SlydeCode"),
            _ => request.bearer_auth(key),
        };

        let response = request
            .send()
            .await
            .map_err(|error| CopilotError::from_network(&error))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CopilotError::from_status(status, &body));
        }
        let payload: serde_json::Value = response.json().await.map_err(|_| {
            CopilotError::Upstream("the provider returned an unexpected model list".into())
        })?;
        let models = models::parse_models(provider, &payload);
        if models.is_empty() {
            return Err(CopilotError::Upstream(
                "the provider returned no chat-capable models".into(),
            ));
        }
        Ok(models)
    }

    pub async fn remember(&self, provider: &str, model: &str) {
        let mut config = self.config.write().await;
        config.last_provider = Some(provider.to_string());
        config.last_model = Some(model.to_string());
        let snapshot = config.clone();
        drop(config);
        self.save(&snapshot).await;
    }

    async fn save(&self, config: &CopilotConfig) {
        if let Err(error) = self.store.save(config).await {
            tracing::warn!(error = %error, "could not save copilot config");
        }
    }
}

fn has_key(config: &CopilotConfig, provider: ProviderDef) -> bool {
    config
        .keys
        .get(provider.id)
        .is_some_and(|key| !key.trim().is_empty())
        || provider.env_key().is_some()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_turn_can_be_cancelled_by_its_request_id() {
        let state = super::CopilotState::in_memory();
        let flag = state.begin_turn("turn-1");
        assert!(!super::CopilotState::was_cancelled(&flag));

        assert!(state.cancel_turn("turn-1"), "a live turn is cancellable");
        assert!(
            super::CopilotState::was_cancelled(&flag),
            "the running loop sees the cancel"
        );

        // Unknown ids and finished turns are reported honestly, not silently.
        assert!(!state.cancel_turn("missing"));
        assert!(state.end_turn("turn-1"));
        assert!(!state.end_turn("turn-1"), "a turn is only removed once");
        assert!(
            !state.cancel_turn("turn-1"),
            "a finished turn cannot be cancelled"
        );
    }

    use super::*;

    #[tokio::test]
    async fn unknown_provider_is_rejected() {
        let state = CopilotState::in_memory();
        let error = state.models("not-a-provider").await.unwrap_err();
        assert_eq!(error.code(), "invalid_request");
    }

    #[tokio::test]
    async fn missing_key_is_reported_without_touching_the_network() {
        let state = CopilotState::in_memory();
        let error = state.models("openai").await.unwrap_err();
        // A developer machine may export OPENAI_API_KEY; only assert the
        // in-memory path when no environment key is present.
        if std::env::var("OPENAI_API_KEY").is_err() {
            assert_eq!(error.code(), "missing_key");
        }
    }

    #[tokio::test]
    async fn malformed_keys_are_rejected_before_any_network_call() {
        let state = CopilotState::in_memory();
        let error = state
            .set_key("openai", "not a key\nwith a header injection")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_request");
    }

    #[tokio::test]
    async fn empty_key_removes_a_stored_key_but_never_returns_it() {
        let state = CopilotState::in_memory();
        state
            .config
            .write()
            .await
            .keys
            .insert("openai".into(), "sk-test".into());
        let view = state.view().await;
        assert!(
            view.providers
                .iter()
                .any(|p| p.id == "openai" && p.configured)
        );
        let encoded = serde_json::to_string(&view).unwrap();
        assert!(!encoded.contains("sk-test"));

        // Clearing skips validation and does not touch the network.
        state.set_key("openai", "").await.unwrap();
        if std::env::var("OPENAI_API_KEY").is_err() {
            let view = state.view().await;
            assert!(
                !view
                    .providers
                    .iter()
                    .any(|p| p.id == "openai" && p.configured)
            );
        }
    }
}
