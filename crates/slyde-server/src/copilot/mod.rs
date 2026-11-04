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
pub mod image_service;
pub mod imagegen;
pub mod models;
pub mod partial;
pub mod prompt;
pub mod review;
pub mod routes;
pub mod stock;
pub mod tools;
pub mod transport;
pub mod vault;
pub mod webfetch;
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
    /// The image model behind the same key, when the provider offers one.
    /// `None` tells the UI that this provider cannot draw pictures.
    pub image_model: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    Off,
    Ai,
    Stock,
}

impl ImageMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "ai" => Some(Self::Ai),
            "stock" => Some(Self::Stock),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ai => "ai",
            Self::Stock => "stock",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StockStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CopilotConfigView {
    pub providers: Vec<ProviderStatus>,
    pub stock: Vec<StockStatus>,
    pub image_mode: ImageMode,
    pub last_provider: Option<String>,
    pub last_model: Option<String>,
    pub last_image_provider: Option<String>,
    pub last_image_model: Option<String>,
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
    /// Whether slides the model writes are opened in a browser and measured (see
    /// `review`). On for the real app; off for in-memory states, so tests that
    /// script a provider never start a browser unless they ask to.
    review: std::sync::atomic::AtomicBool,
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
        Self::with_store(store, config, true)
    }

    pub fn in_memory() -> Self {
        Self::with_store(ConfigStore::in_memory(), CopilotConfig::default(), false)
    }

    pub(crate) fn review_enabled(&self) -> bool {
        self.review.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn set_review(&self, on: bool) {
        self.review.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    fn with_store(store: ConfigStore, config: CopilotConfig, review: bool) -> Self {
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
            review: std::sync::atomic::AtomicBool::new(review),
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
        let stock = stock::StockKeys::from_lookup(|id| config.keys.get(id).cloned());
        let stock = stock.providers();
        let image_mode = effective_image_mode(&config);
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
                    image_model: provider.image.map(|image| image.model),
                })
                .collect(),
            stock: [
                ("unsplash", "Unsplash", stock.contains(&"unsplash")),
                ("pexels", "Pexels", stock.contains(&"pexels")),
                ("vecteezy", "Vecteezy", stock.contains(&"vecteezy")),
            ]
            .into_iter()
            .map(|(id, name, configured)| StockStatus {
                id,
                name,
                configured,
            })
            .collect(),
            image_mode,
            last_provider: config.last_provider,
            last_model: config.last_model,
            last_image_provider: config.last_image_provider,
            last_image_model: config.last_image_model,
        }
    }

    /// The effective stock-image keys for this profile, with stored values
    /// taking precedence over environment variables.
    pub async fn stock_keys(&self) -> stock::StockKeys {
        let config = self.config.read().await;
        stock::StockKeys::from_lookup(|id| config.keys.get(id).cloned())
    }

    /// The current image source for the copilot: off, AI or stock search.
    pub async fn image_mode(&self) -> ImageMode {
        let config = self.config.read().await;
        effective_image_mode(&config)
    }

    pub async fn set_image_mode(&self, mode: &str) -> Result<(), CopilotError> {
        let mode = ImageMode::parse(mode).ok_or_else(|| {
            CopilotError::InvalidRequest("the image mode must be off, ai or stock".into())
        })?;
        {
            let config = self.config.read().await;
            match mode {
                ImageMode::Ai
                    if !PROVIDERS.iter().any(|provider| {
                        provider.image.is_some() && has_key(&config, *provider)
                    }) =>
                {
                    return Err(CopilotError::InvalidRequest(
                        "connect an image model before selecting AI images".into(),
                    ));
                }
                ImageMode::Stock
                    if stock::StockKeys::from_lookup(|id| config.keys.get(id).cloned())
                        .providers()
                        .is_empty() =>
                {
                    return Err(CopilotError::InvalidRequest(
                        "add a stock image key before selecting stock images".into(),
                    ));
                }
                _ => {}
            }
        }
        let mut config = self.config.write().await;
        config.image_mode = Some(mode.as_str().to_string());
        let snapshot = config.clone();
        drop(config);
        self.save(&snapshot).await;
        Ok(())
    }

    /// Saves or removes one stock service key. No network call is made: a stock
    /// key is small and the failure is visible on the next search, unlike a
    /// chat provider key where a bad key would break the whole copilot.
    pub async fn set_stock_key(
        &self,
        provider: &str,
        key: &str,
        account: Option<&str>,
    ) -> Result<(), CopilotError> {
        let (key_id, account_id) = match provider {
            "unsplash" => (stock::UNSPLASH_KEY, None),
            "pexels" => (stock::PEXELS_KEY, None),
            "vecteezy" => (stock::VECTEEZY_KEY, Some(stock::VECTEEZY_ACCOUNT)),
            _ => {
                return Err(CopilotError::InvalidRequest(
                    "unknown stock image provider".into(),
                ));
            }
        };
        let key = key.trim();
        let account = account.unwrap_or("").trim();
        if key.is_empty() {
            let mut config = self.config.write().await;
            config.keys.remove(key_id);
            if let Some(account_id) = account_id {
                config.keys.remove(account_id);
            }
            let snapshot = config.clone();
            drop(config);
            self.save(&snapshot).await;
            return Ok(());
        }
        if key.len() > 1024 || !key.chars().all(|ch| ch.is_ascii_graphic()) {
            return Err(CopilotError::InvalidRequest(
                "the stock API key contains unsupported characters".into(),
            ));
        }
        if let Some(account_id) = account_id {
            if account.is_empty() {
                return Err(CopilotError::InvalidRequest(
                    "the Vecteezy account id is required".into(),
                ));
            }
            if account.len() > 64
                || !account
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            {
                return Err(CopilotError::InvalidRequest(
                    "the Vecteezy account id is not valid".into(),
                ));
            }
            let mut config = self.config.write().await;
            config.keys.insert(key_id.to_string(), key.to_string());
            config
                .keys
                .insert(account_id.to_string(), account.to_string());
            let snapshot = config.clone();
            drop(config);
            self.save(&snapshot).await;
            return Ok(());
        }
        let mut config = self.config.write().await;
        config.keys.insert(key_id.to_string(), key.to_string());
        let snapshot = config.clone();
        drop(config);
        self.save(&snapshot).await;
        Ok(())
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

    /// The models a provider can draw with. OpenRouter returns its full routed
    /// image catalog here; direct vendors return the image models their model
    /// list exposes, or their catalog default when the list has no metadata.
    pub async fn image_models(&self, provider_id: &str) -> Result<Vec<ModelInfo>, CopilotError> {
        let provider = catalog::find(provider_id)
            .ok_or_else(|| CopilotError::InvalidRequest("unknown provider".into()))?;
        let spec = provider.image.ok_or_else(|| {
            CopilotError::InvalidRequest(format!(
                "{} does not offer image generation",
                provider.name
            ))
        })?;
        let key = self
            .key_for(provider)
            .await
            .ok_or(CopilotError::MissingKey)?;
        let payload = self
            .fetch_payload(provider, &key, provider.image_models_url(&key))
            .await?;
        let mut models = models::parse_image_models(provider, &payload);
        if provider.id == "openrouter"
            && let Ok(general) = self.fetch_model_payload(provider, &key).await
        {
            models::merge_prices(&mut models, &general);
        }
        if models.is_empty() {
            // A vendor that exposes no image metadata still has the one model
            // the catalog knows how to call.
            models.push(ModelInfo {
                id: spec.model.to_string(),
                name: spec.model.to_string(),
                free: false,
                vision: None,
                context_window: None,
                input_price: None,
                output_price: None,
                image_price: Some(spec.cost_usd),
            });
        }
        Ok(models)
    }

    async fn fetch_models(
        &self,
        provider: &ProviderDef,
        key: &str,
    ) -> Result<Vec<ModelInfo>, CopilotError> {
        let payload = self.fetch_model_payload(provider, key).await?;
        let models = models::parse_models(provider, &payload);
        if models.is_empty() {
            return Err(CopilotError::Upstream(
                "the provider returned no chat-capable models".into(),
            ));
        }
        Ok(models)
    }

    async fn fetch_model_payload(
        &self,
        provider: &ProviderDef,
        key: &str,
    ) -> Result<serde_json::Value, CopilotError> {
        let url = provider.models_url(key);
        self.fetch_payload(provider, key, url).await
    }

    async fn fetch_payload(
        &self,
        provider: &ProviderDef,
        key: &str,
        url: String,
    ) -> Result<serde_json::Value, CopilotError> {
        let mut request = self.http.get(url).header("accept", "application/json");
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
        response.json().await.map_err(|_| {
            CopilotError::Upstream("the provider returned an unexpected model list".into())
        })
    }

    /// One small request for the selected model's per-picture estimate. It is
    /// best effort: a failure keeps the catalog estimate instead of blocking the
    /// user's choice.
    async fn image_price_for(&self, provider: &ProviderDef, key: &str, model: &str) -> Option<f64> {
        let url = provider.image_endpoints_url(model)?;
        let response = self
            .http
            .get(url)
            .bearer_auth(key)
            .header("accept", "application/json")
            .header("X-Title", "SlydeCode")
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let payload: serde_json::Value = response.json().await.ok()?;
        models::parse_image_price(&payload)
    }

    pub async fn remember(&self, provider: &str, model: &str) {
        let mut config = self.config.write().await;
        config.last_provider = Some(provider.to_string());
        config.last_model = Some(model.to_string());
        let snapshot = config.clone();
        drop(config);
        self.save(&snapshot).await;
    }

    /// Remembers the provider/model used for pictures. It deliberately does not
    /// touch the chat selection: DeepSeek can write the deck while OpenAI draws
    /// it.
    pub async fn remember_image(&self, provider: &str, model: &str) -> Result<(), CopilotError> {
        let def = catalog::find(provider)
            .ok_or_else(|| CopilotError::InvalidRequest("unknown provider".into()))?;
        let image = def.image.ok_or_else(|| {
            CopilotError::InvalidRequest(format!("{} does not offer image generation", def.name))
        })?;
        let model = model.trim();
        if model.is_empty()
            || model.len() > 200
            || !model
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | ':'))
        {
            return Err(CopilotError::InvalidRequest(
                "the image model id is not valid".into(),
            ));
        }

        let mut cost = image.cost_usd;
        if model != image.model {
            if self.key_for(def).await.is_none() {
                return Err(CopilotError::InvalidRequest(format!(
                    "connect {} before choosing another image model",
                    def.name
                )));
            }
            let models = self.image_models(provider).await?;
            let selected = models
                .iter()
                .find(|candidate| candidate.id == model)
                .ok_or_else(|| {
                    CopilotError::InvalidRequest(format!(
                        "{} does not offer the image model {model}",
                        def.name
                    ))
                })?;
            cost = selected.image_price.unwrap_or(image.cost_usd);
        }
        if def.id == "openrouter"
            && let Some(key) = self.key_for(def).await
            && let Some(price) = self.image_price_for(def, &key, model).await
        {
            cost = price;
        }

        let mut config = self.config.write().await;
        config.last_image_provider = Some(provider.to_string());
        config.last_image_model = Some(model.to_string());
        config.last_image_cost = Some(cost);
        let snapshot = config.clone();
        drop(config);
        self.save(&snapshot).await;
        Ok(())
    }

    /// The model the image service should call for `provider`, honoring the
    /// user's selection only when it belongs to that provider.
    pub async fn image_model_for(&self, provider: &ProviderDef) -> String {
        let config = self.config.read().await;
        if config.last_image_provider.as_deref() == Some(provider.id)
            && let Some(model) = config.last_image_model.as_deref()
            && !model.trim().is_empty()
        {
            return model.to_string();
        }
        provider
            .image
            .map(|spec| spec.model.to_string())
            .unwrap_or_default()
    }

    /// Conservative per-picture price for the selected model, used to reserve
    /// budget before the provider reports the real charge.
    pub async fn image_model_cost(&self, provider: &ProviderDef) -> f64 {
        let config = self.config.read().await;
        if config.last_image_provider.as_deref() == Some(provider.id) {
            return config
                .last_image_cost
                .filter(|cost| *cost >= 0.0)
                .unwrap_or_else(|| provider.image.map(|spec| spec.cost_usd).unwrap_or(0.0));
        }
        provider.image.map(|spec| spec.cost_usd).unwrap_or(0.0)
    }

    /// The provider the user picked for pictures, when its key is still present
    /// and it still offers an image model.
    pub async fn preferred_image_provider(&self) -> Option<&'static ProviderDef> {
        let config = self.config.read().await;
        let id = config.last_image_provider.as_deref()?;
        let provider = catalog::find(id)?;
        (provider.image.is_some() && has_key(&config, *provider)).then_some(provider)
    }

    /// The provider the image service should use: the user's explicit choice,
    /// or the first connected provider that can draw.
    pub async fn image_provider(&self) -> Option<&'static ProviderDef> {
        let config = self.config.read().await;
        if let Some(id) = config.last_image_provider.as_deref()
            && let Some(provider) = catalog::find(id)
            && provider.image.is_some()
            && has_key(&config, *provider)
        {
            return Some(provider);
        }
        catalog::PROVIDERS
            .iter()
            .find(|provider| provider.image.is_some() && has_key(&config, **provider))
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

/// Explicit user choice first; otherwise prefer an AI image model and fall back
/// to the stock services when only those are configured.
fn effective_image_mode(config: &CopilotConfig) -> ImageMode {
    if let Some(mode) = config.image_mode.as_deref().and_then(ImageMode::parse) {
        return mode;
    }
    if PROVIDERS
        .iter()
        .any(|provider| provider.image.is_some() && has_key(config, *provider))
    {
        return ImageMode::Ai;
    }
    if !stock::StockKeys::from_lookup(|id| config.keys.get(id).cloned())
        .providers()
        .is_empty()
    {
        return ImageMode::Stock;
    }
    ImageMode::Off
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
    async fn the_image_role_is_remembered_apart_from_chat() {
        let state = CopilotState::in_memory();

        let error = state.remember_image("deepseek", "any").await.unwrap_err();
        assert_eq!(
            error.code(),
            "invalid_request",
            "no image model behind that key"
        );

        let error = state
            .remember_image("openai", "not-gpt-image-1")
            .await
            .unwrap_err();
        assert_eq!(
            error.code(),
            "invalid_request",
            "the model must be the offered one"
        );

        state.remember_image("openai", "gpt-image-1").await.unwrap();
        let view = state.view().await;
        assert_eq!(view.last_image_provider.as_deref(), Some("openai"));
        assert_eq!(view.last_image_model.as_deref(), Some("gpt-image-1"));
        assert_eq!(view.last_provider, None, "the chat selection is untouched");
        assert_eq!(view.last_model, None);
        assert!(
            state.preferred_image_provider().await.is_none(),
            "without a key the preference is not usable"
        );
    }

    #[tokio::test]
    async fn stock_keys_are_stored_and_reported_without_leaving_the_server() {
        let state = CopilotState::in_memory();
        state
            .set_stock_key("unsplash", "unsplash-key", None)
            .await
            .unwrap();
        state
            .set_stock_key("vecteezy", "vecteezy-key", Some("acct_9"))
            .await
            .unwrap();
        let view = state.view().await;
        assert!(
            view.stock
                .iter()
                .any(|source| source.id == "unsplash" && source.configured)
        );
        assert!(
            view.stock
                .iter()
                .any(|source| source.id == "vecteezy" && source.configured)
        );
        assert!(state.stock_keys().await.providers().contains(&"vecteezy"));

        let config = state.config.read().await;
        assert_eq!(
            config.keys.get(stock::UNSPLASH_KEY).map(String::as_str),
            Some("unsplash-key")
        );
        assert_eq!(
            config.keys.get(stock::VECTEEZY_ACCOUNT).map(String::as_str),
            Some("acct_9")
        );

        let error = state
            .set_stock_key("vecteezy", "only-key", None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_request");
        assert!(error.public_message().contains("account"));
    }

    #[tokio::test]
    async fn the_image_mode_is_validated_and_remembered() {
        let state = CopilotState::in_memory();
        let error = state.set_image_mode("sometimes").await.unwrap_err();
        assert_eq!(error.code(), "invalid_request");

        state.set_image_mode("off").await.unwrap();
        assert_eq!(state.image_mode().await, ImageMode::Off);

        state
            .config
            .write()
            .await
            .keys
            .insert("openai".into(), "sk-test".into());
        state.set_image_mode("ai").await.unwrap();
        assert_eq!(state.image_mode().await, ImageMode::Ai);
        assert_eq!(state.view().await.image_mode, ImageMode::Ai);

        state
            .set_stock_key("unsplash", "stock-test", None)
            .await
            .unwrap();
        state.set_image_mode("stock").await.unwrap();
        assert_eq!(state.image_mode().await, ImageMode::Stock);
        assert_eq!(state.view().await.image_mode, ImageMode::Stock);
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
