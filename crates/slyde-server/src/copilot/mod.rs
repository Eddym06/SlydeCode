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
pub mod partial;
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

/// One chat message as it is persisted per deck.
///
/// Only role and text: tool rows and reasoning were live UI, and storing them
/// would freeze a rendering the next session may not reproduce. The thread is
/// there to remember the conversation, not to replay animations.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredMessage {
    pub role: String,
    pub text: String,
}

/// Bounds for one deck's thread so a long session cannot grow the file forever.
const MAX_THREAD_MESSAGES: usize = 40;
const MAX_THREAD_TEXT_CHARS: usize = 20_000;
const MAX_THREAD_DECKS: usize = 100;

#[derive(Debug, Default)]
struct Threads {
    path: Option<std::path::PathBuf>,
    decks: std::collections::BTreeMap<String, Vec<StoredMessage>>,
}

impl Threads {
    fn load(path: Option<std::path::PathBuf>) -> Self {
        let mut threads = Self {
            path,
            decks: Default::default(),
        };
        let Some(file) = threads.path.clone() else {
            return threads;
        };
        let Ok(raw) = std::fs::read_to_string(&file) else {
            return threads;
        };
        match serde_json::from_str::<std::collections::BTreeMap<String, Vec<StoredMessage>>>(&raw) {
            Ok(decks) => threads.decks = decks,
            Err(error) => {
                tracing::warn!(%error, "copilot threads file is not valid JSON; ignoring it")
            }
        }
        threads
    }

    fn save(&self) -> Result<(), CopilotError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CopilotError::Cancelled)?;
        }
        let encoded = serde_json::to_vec_pretty(&self.decks)
            .map_err(|error| CopilotError::InvalidRequest(error.to_string()))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, encoded)
            .and_then(|_| std::fs::rename(&temp, path))
            .map_err(|error| {
                tracing::warn!(%error, "could not save the copilot threads file");
                CopilotError::Cancelled
            })
    }
}

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
    /// Conversations, one per deck, kept next to the key file.
    threads: std::sync::Mutex<Threads>,
    /// The deck as it was before the latest copilot turn, so the whole turn can
    /// be reverted in one step. It is only valid while the deck is still exactly
    /// as that turn left it.
    last_turn: std::sync::Mutex<Option<TurnSnapshot>>,
}

#[derive(Debug, Clone)]
pub(crate) struct TurnSnapshot {
    pub before: slyde_core::Presentation,
    /// Revision right after the turn. `None` while the turn is still running.
    pub revision_after: Option<u64>,
}

impl CopilotState {
    pub(crate) fn begin_snapshot(&self, before: slyde_core::Presentation) {
        let mut slot = self.last_turn.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(TurnSnapshot {
            before,
            revision_after: None,
        });
    }

    /// Seals the snapshot, or drops it when the turn changed nothing.
    pub(crate) fn finish_snapshot(&self, revision_before: u64, revision_after: u64) -> bool {
        let mut slot = self.last_turn.lock().unwrap_or_else(|e| e.into_inner());
        if revision_after == revision_before {
            *slot = None;
            return false;
        }
        if let Some(snapshot) = slot.as_mut() {
            snapshot.revision_after = Some(revision_after);
        }
        slot.is_some()
    }

    /// Takes the snapshot when the deck still matches the end of that turn.
    pub(crate) fn take_snapshot(
        &self,
        current_revision: u64,
    ) -> Result<slyde_core::Presentation, CopilotError> {
        let mut slot = self.last_turn.lock().unwrap_or_else(|e| e.into_inner());
        match slot.take() {
            Some(TurnSnapshot {
                before,
                revision_after: Some(after),
            }) if after == current_revision => Ok(before),
            Some(other) => {
                *slot = Some(other);
                Err(CopilotError::InvalidRequest(
                    "the deck changed after the copilot's last turn, so it can no longer be reverted in one step; use undo".into(),
                ))
            }
            None => Err(CopilotError::InvalidRequest(
                "there is nothing to revert".into(),
            )),
        }
    }
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
        let thread_path = store
            .path()
            .map(|path| path.with_file_name("copilot-threads.json"));
        let threads = Threads::load(thread_path);
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
            threads: std::sync::Mutex::new(threads),
            last_turn: std::sync::Mutex::new(None),
        }
    }

    /// The stored conversation for a deck, oldest first.
    pub fn thread(&self, deck: &str) -> Vec<StoredMessage> {
        let threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        threads.decks.get(deck).cloned().unwrap_or_default()
    }

    /// Replaces a deck's conversation, bounded in length and message size.
    pub fn save_thread(
        &self,
        deck: &str,
        messages: Vec<StoredMessage>,
    ) -> Result<usize, CopilotError> {
        if deck.trim().is_empty() {
            return Err(CopilotError::InvalidRequest("a deck id is required".into()));
        }
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut cleaned: Vec<StoredMessage> = messages
            .into_iter()
            .filter(|message| !message.text.trim().is_empty())
            .map(|message| StoredMessage {
                role: match message.role.as_str() {
                    "assistant" => "assistant".to_string(),
                    _ => "user".to_string(),
                },
                text: message.text.chars().take(MAX_THREAD_TEXT_CHARS).collect(),
            })
            .collect();
        if cleaned.len() > MAX_THREAD_MESSAGES {
            cleaned.drain(0..cleaned.len() - MAX_THREAD_MESSAGES);
        }
        if cleaned.is_empty() {
            threads.decks.remove(deck);
        } else {
            threads.decks.insert(deck.to_string(), cleaned.clone());
        }
        // Keep the file bounded: drop the oldest decks when the cap is reached.
        while threads.decks.len() > MAX_THREAD_DECKS {
            let Some(oldest) = threads.decks.keys().next().cloned() else {
                break;
            };
            threads.decks.remove(&oldest);
        }
        let stored = cleaned.len();
        threads.save()?;
        Ok(stored)
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
    fn conversations_are_stored_per_deck_and_bounded() {
        let state = super::CopilotState::in_memory();
        assert!(state.thread("deck-a").is_empty());

        state
            .save_thread(
                "deck-a",
                vec![super::StoredMessage {
                    role: "user".into(),
                    text: "make it shorter".into(),
                }],
            )
            .unwrap();
        assert_eq!(state.thread("deck-a").len(), 1);
        assert!(
            state.thread("deck-b").is_empty(),
            "decks never share a thread"
        );

        // Long threads keep the newest messages, and long text is truncated.
        let many: Vec<_> = (0..100)
            .map(|index| super::StoredMessage {
                role: "assistant".into(),
                text: format!("m{index}"),
            })
            .collect();
        assert_eq!(state.save_thread("deck-a", many).unwrap(), 40);
        assert_eq!(state.thread("deck-a").last().unwrap().text, "m99");

        let huge = super::StoredMessage {
            role: "user".into(),
            text: "x".repeat(50_000),
        };
        state.save_thread("deck-c", vec![huge]).unwrap();
        assert_eq!(
            state.thread("deck-c")[0].text.chars().count(),
            20_000,
            "a pasted document cannot bloat the file"
        );

        // An empty thread removes the entry instead of storing a blank list.
        state.save_thread("deck-a", vec![]).unwrap();
        assert!(state.thread("deck-a").is_empty());
        assert!(state.save_thread("   ", vec![]).is_err());
    }
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
