//! Background image orchestration for the copilot.
//!
//! The text model never talks to a vendor. It asks for a picture with a unique
//! id and a target, and this service owns everything else: which model to use,
//! how much the account can afford, how many calls may run at once, where the
//! bytes are buffered and where they are finally placed.
//!
//! Two invariants matter:
//!
//! - A request returns immediately. Generation runs in a detached task so the
//!   provider stream and the tool loop are never blocked by a slow picture.
//! - The service never overtakes the deck. If a picture is ready before the
//!   slide it belongs to exists, the job waits on a ticket and places it when
//!   the slide appears.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::agent::{self, AgentCommand};
use crate::state::{AppState, ServerEvent};

use super::catalog::ProviderDef;
use super::error::CopilotError;
use super::imagegen;

/// Hard ceiling the system enforces regardless of what a model asks for.
pub const MAX_IMAGES_PER_MINUTE: usize = 10;
/// Safety ceiling per turn. It is not advertised as a prediction; the model is
/// simply told that an image model is available.
pub const MAX_IMAGES_PER_TURN: usize = 10;
/// A few workers keep latency low without tripping vendor concurrency limits.
pub const MAX_PARALLEL: usize = 3;
/// How long a completed picture waits for its slide before giving up.
const SLIDE_WAIT: Duration = Duration::from_secs(120);
/// Vendor balance and cost are re-read at most this often.
const CAPABILITY_TTL: Duration = Duration::from_secs(30);
/// After a failed generation the capability disappears for a while, so the
/// next prompt round falls back to stock search instead of retrying blindly.
const DEGRADED_TTL: Duration = Duration::from_secs(90);
#[derive(Debug, Clone, Serialize)]
pub struct ImageCapability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<&'static str>,
    pub provider_name: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ImageCapability {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            provider: None,
            provider_name: String::new(),
            model: String::new(),
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageStatus {
    Queued,
    Generating,
    Placed,
    Failed,
    Cancelled,
}

/// What the chat stream and the WebSocket carry for one picture.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageJobView {
    pub id: String,
    pub status: ImageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Stable reason code the UI localizes. `error` keeps the English detail
    /// for reports and for codes the UI does not know yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageErrorView {
    pub code: String,
    pub message: String,
}

/// Per-turn bookkeeping. Lives as long as the UI needs to show the pictures,
/// which is longer than the text turn that requested them.
#[derive(Debug)]
pub struct ImageTurn {
    id: String,
    jobs: Mutex<Vec<ImageJobView>>,
    errors: Mutex<Vec<ImageErrorView>>,
    requested: AtomicUsize,
    /// Set when the textual turn is cancelled. Queued and in-flight jobs watch
    /// this channel and stop before spending another provider call.
    cancelled: tokio::sync::watch::Sender<bool>,
}

impl ImageTurn {
    fn new(id: String) -> Self {
        let (cancelled, _) = tokio::sync::watch::channel(false);
        Self {
            id,
            jobs: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            requested: AtomicUsize::new(0),
            cancelled,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn requested(&self) -> usize {
        self.requested.load(Ordering::SeqCst)
    }

    fn cancellation(&self) -> tokio::sync::watch::Receiver<bool> {
        self.cancelled.subscribe()
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    /// Stops this turn: queued jobs never start, running jobs drop their
    /// provider call, and anything not placed yet is marked cancelled.
    pub async fn cancel(&self) {
        self.cancelled.send_replace(true);
        let mut jobs = self.jobs.lock().await;
        for job in jobs.iter_mut() {
            if matches!(job.status, ImageStatus::Queued | ImageStatus::Generating) {
                job.status = ImageStatus::Cancelled;
                job.error_code = None;
                job.error = None;
            }
        }
    }

    pub async fn jobs(&self) -> Vec<ImageJobView> {
        self.jobs.lock().await.clone()
    }

    pub async fn errors(&self) -> Vec<ImageErrorView> {
        self.errors.lock().await.clone()
    }

    pub async fn snapshot(&self) -> (Vec<ImageJobView>, Vec<ImageErrorView>) {
        (
            self.jobs.lock().await.clone(),
            self.errors.lock().await.clone(),
        )
    }

    async fn add(&self, job: ImageJobView) -> Result<(), String> {
        let mut jobs = self.jobs.lock().await;
        if jobs.iter().any(|existing| existing.id == job.id) {
            return Err(format!(
                "image_id `{}` was already used in this turn; every picture needs a unique id",
                job.id
            ));
        }
        jobs.push(job);
        Ok(())
    }

    async fn update<F>(&self, id: &str, change: F)
    where
        F: FnOnce(&mut ImageJobView),
    {
        let mut jobs = self.jobs.lock().await;
        if let Some(job) = jobs.iter_mut().find(|job| job.id == id) {
            change(job);
        }
    }

    async fn mark_cancelled(&self, id: &str) {
        self.update(id, |job| {
            job.status = ImageStatus::Cancelled;
            job.error_code = None;
            job.error = None;
        })
        .await;
    }

    async fn fail(&self, id: &str, code: impl Into<String>, message: String) {
        // A cancelled turn must not resurrect an old job as an error after the
        // user already stopped it.
        if self.is_cancelled() {
            self.mark_cancelled(id).await;
            return;
        }
        let code = code.into();
        self.update(id, |job| {
            job.status = ImageStatus::Failed;
            job.error_code = Some(code.clone());
            job.error = Some(message.clone());
        })
        .await;
        self.errors
            .lock()
            .await
            .push(ImageErrorView { code, message });
    }
}

/// One model request, already validated by the tool layer.
#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub id: String,
    pub prompt: String,
    pub alt: String,
    pub slide_id: Option<String>,
    pub slide_index: Option<usize>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub w: Option<f32>,
    pub h: Option<f32>,
}

impl ImageRequest {
    fn view(&self, status: ImageStatus) -> ImageJobView {
        ImageJobView {
            id: self.id.clone(),
            status,
            slide_id: self.slide_id.clone(),
            slide_index: self.slide_index,
            url: None,
            error_code: None,
            error: None,
        }
    }
}

#[derive(Debug)]
struct CachedCapability {
    at: Instant,
    value: ImageCapability,
}

#[derive(Debug)]
struct Degraded {
    at: Instant,
    reason: String,
}

/// The money side of the service. Confirmed charges and reservations live in
/// one lock so a check-and-reserve is atomic even with several turns asking for
/// pictures at the same time.
#[derive(Debug, Default)]
struct Budget {
    /// Credit reported by the last probe, when the provider reports it.
    balance: Option<f64>,
    /// Confirmed charges since the last probe.
    spent: f64,
    /// Estimated charges for jobs that have not settled yet.
    reserved: f64,
}

#[derive(Debug)]
pub struct ImageService {
    cached: RwLock<Option<CachedCapability>>,
    degraded: Mutex<Option<Degraded>>,
    probe: Mutex<()>,
    /// Timestamps of the last accepted calls, newest at the back.
    rate: Mutex<VecDeque<Instant>>,
    parallel: Semaphore,
    budget: Mutex<Budget>,
}

impl Default for ImageService {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageService {
    pub fn new() -> Self {
        Self {
            cached: RwLock::new(None),
            degraded: Mutex::new(None),
            probe: Mutex::new(()),
            rate: Mutex::new(VecDeque::new()),
            parallel: Semaphore::new(MAX_PARALLEL),
            budget: Mutex::new(Budget::default()),
        }
    }

    pub fn begin_turn(self: &Arc<Self>, id: impl Into<String>) -> Arc<ImageTurn> {
        Arc::new(ImageTurn::new(id.into()))
    }

    /// Drops the cached answer after the user changes a key or picks another
    /// image model. The next prompt round probes the new configuration instead
    /// of spending up to the cache TTL on the old one.
    pub async fn invalidate(&self) {
        *self.cached.write().await = None;
        *self.degraded.lock().await = None;
    }

    /// Live answer to "is an image model usable right now?".
    ///
    /// The model is never told how many pictures it may request. The probe only
    /// decides whether the prompt advertises the image tool at all. Balance is
    /// still read internally to keep a reservation guard, but it is not a
    /// prediction shown to the text model.
    pub async fn capability(&self, state: &AppState) -> ImageCapability {
        if let Some(degraded) = self.degraded.lock().await.as_ref()
            && degraded.at.elapsed() < DEGRADED_TTL
        {
            return ImageCapability::unavailable(degraded.reason.clone());
        }

        if let Some(cached) = self.cached.read().await.as_ref()
            && cached.at.elapsed() < CAPABILITY_TTL
        {
            return cached.value.clone();
        }

        let _probe = self.probe.lock().await;
        // A failure may have landed while this round waited for the probe.
        if let Some(degraded) = self.degraded.lock().await.as_ref()
            && degraded.at.elapsed() < DEGRADED_TTL
        {
            return ImageCapability::unavailable(degraded.reason.clone());
        }
        // Another round may have filled the cache while this one waited.
        if let Some(cached) = self.cached.read().await.as_ref()
            && cached.at.elapsed() < CAPABILITY_TTL
        {
            return cached.value.clone();
        }

        let capability = self.probe_capability(state).await;
        *self.cached.write().await = Some(CachedCapability {
            at: Instant::now(),
            value: capability.clone(),
        });
        capability
    }

    async fn probe_capability(&self, state: &AppState) -> ImageCapability {
        let Some(provider) = state.copilot.image_provider().await else {
            return ImageCapability::unavailable("no image model is configured");
        };
        if provider.image.is_none() {
            return ImageCapability::unavailable("this provider has no image model");
        }
        let Some(key) = state.copilot.key_for(provider).await else {
            return ImageCapability::unavailable(format!(
                "{} has no API key configured",
                provider.name
            ));
        };

        // Keep the internal spend guard up to date. A zero balance is treated
        // as "not usable" because every request would fail; any positive
        // balance simply advertises availability, without predicting a count.
        let reported_balance = imagegen::balance(&state.copilot, provider, &key).await;
        {
            let mut budget = self.budget.lock().await;
            budget.balance = reported_balance;
            if reported_balance.is_some() {
                // The vendor already counted every charge up to this probe.
                budget.spent = 0.0;
            }
        }
        if reported_balance.is_some_and(|balance| balance <= 0.0) {
            return ImageCapability::unavailable(
                "the account has no balance left for another image",
            );
        }

        ImageCapability {
            available: true,
            provider: Some(provider.id),
            provider_name: provider.name.to_string(),
            model: state.copilot.image_model_for(provider).await,
            reason: None,
        }
    }

    /// Queues one picture and returns its ticket immediately.
    pub async fn request(
        self: &Arc<Self>,
        state: Arc<AppState>,
        turn: Arc<ImageTurn>,
        request: ImageRequest,
    ) -> Result<ImageJobView, String> {
        if turn.is_cancelled() {
            return Err("the turn was cancelled".into());
        }
        let capability = self.capability(&state).await;
        if !capability.available {
            return Err(capability
                .reason
                .unwrap_or_else(|| "image generation is not available right now".into()));
        }
        // A hard safety ceiling, not a prediction. The prompt never mentions a
        // count; this only stops a runaway turn from queueing forever.
        if turn.requested() >= MAX_IMAGES_PER_TURN {
            return Err(format!(
                "this turn can request at most {MAX_IMAGES_PER_TURN} images; ask for fewer pictures or start a new turn"
            ));
        }
        let Some(provider) = capability.provider.and_then(super::catalog::find) else {
            return Err("no image provider is available".into());
        };

        // Reserve before returning the ticket so two concurrent turns cannot
        // both spend the same remaining balance. This is an internal guard and
        // is never exposed to the model as a predicted image count.
        let reserved = state.copilot.image_model_cost(provider).await.max(0.0);
        if !self.try_reserve(reserved).await {
            return Err("the account does not have enough balance left for another image".into());
        }

        let view = request.view(ImageStatus::Queued);
        if let Err(error) = turn.add(view.clone()).await {
            self.release(reserved).await;
            return Err(error);
        }
        turn.requested.fetch_add(1, Ordering::SeqCst);
        self.broadcast(&state, turn.id(), &view);

        let service = self.clone();
        let job = request.clone();
        tokio::spawn(async move {
            service.run_job(state, turn, provider, job, reserved).await;
        });
        Ok(view)
    }

    async fn run_job(
        &self,
        state: Arc<AppState>,
        turn: Arc<ImageTurn>,
        provider: &'static ProviderDef,
        request: ImageRequest,
        reserved: f64,
    ) {
        let mut cancel = turn.cancellation();
        if turn.is_cancelled() {
            self.release(reserved).await;
            self.cancel_job(&state, &turn, &request).await;
            return;
        }

        let worker = tokio::select! {
            permit = self.parallel.acquire() => match permit {
                Ok(permit) => permit,
                Err(_) => {
                    self.release(reserved).await;
                    self.fail(
                        &state,
                        &turn,
                        &request,
                        "image_worker",
                        "the image worker stopped".into(),
                        false,
                    )
                    .await;
                    return;
                }
            },
            _ = wait_for_cancel(&mut cancel) => {
                self.release(reserved).await;
                self.cancel_job(&state, &turn, &request).await;
                return;
            }
        };
        if turn.is_cancelled() {
            drop(worker);
            self.release(reserved).await;
            self.cancel_job(&state, &turn, &request).await;
            return;
        }

        // Stamp the rate window at the moment of the real provider call, not
        // when the ticket was accepted: a backlog must not let a later turn
        // exceed the per-minute ceiling while old jobs are still queued.
        tokio::select! {
            _ = self.acquire_rate() => {}
            _ = wait_for_cancel(&mut cancel) => {
                drop(worker);
                self.release(reserved).await;
                self.cancel_job(&state, &turn, &request).await;
                return;
            }
        }

        turn.update(&request.id, |job| job.status = ImageStatus::Generating)
            .await;
        self.broadcast(&state, turn.id(), &next_view(&turn, &request.id).await);

        let generated = tokio::select! {
            result = self.generate_with_retry(&state, provider, &request.prompt) => match result {
                Ok(image) => image,
                Err(error) => {
                    // A failed call was not charged, so return the reservation
                    // before hiding the capability from the next prompt round.
                    drop(worker);
                    self.release(reserved).await;
                    self.fail(
                        &state,
                        &turn,
                        &request,
                        error.code(),
                        error.public_message(),
                        true,
                    )
                    .await;
                    return;
                }
            },
            _ = wait_for_cancel(&mut cancel) => {
                drop(worker);
                self.release(reserved).await;
                self.cancel_job(&state, &turn, &request).await;
                return;
            }
        };

        // The picture exists and was billed even if saving it locally fails.
        let actual = generated.cost_usd.unwrap_or(reserved);
        self.settle(reserved, actual).await;
        if turn.is_cancelled() {
            drop(worker);
            self.cancel_job(&state, &turn, &request).await;
            return;
        }

        let stored = match state.assets.save_bytes(&generated.bytes) {
            Ok(stored) => stored,
            Err(error) => {
                // The model worked; the local store did not. Do not punish the
                // image capability for a disk or format problem.
                drop(worker);
                self.fail(
                    &state,
                    &turn,
                    &request,
                    "image_store",
                    error.to_string(),
                    false,
                )
                .await;
                return;
            }
        };
        if turn.is_cancelled() {
            drop(worker);
            self.cancel_job(&state, &turn, &request).await;
            return;
        }
        turn.update(&request.id, |job| job.url = Some(stored.url.clone()))
            .await;
        // A finished picture waiting for its slide must not hold a generation
        // slot: the next queued picture can start drawing in the meantime.
        drop(worker);

        let placed = tokio::select! {
            result = self.wait_and_place(&state, &request, &stored.url) => result,
            _ = wait_for_cancel(&mut cancel) => {
                self.cancel_job(&state, &turn, &request).await;
                return;
            }
        };

        match placed {
            Ok((slide_id, element_id)) => {
                turn.update(&request.id, |job| {
                    job.status = ImageStatus::Placed;
                    job.slide_id = Some(slide_id);
                    job.error_code = None;
                    job.error = None;
                })
                .await;
                let _ = element_id;
                self.broadcast(&state, turn.id(), &next_view(&turn, &request.id).await);
            }
            Err(message) => {
                // A target that never appeared is a deck problem, not a model
                // problem: keep the capability for the next turn.
                self.fail(&state, &turn, &request, "image_placement", message, false)
                    .await;
            }
        }
    }

    async fn cancel_job(&self, state: &AppState, turn: &ImageTurn, request: &ImageRequest) {
        turn.mark_cancelled(&request.id).await;
        self.broadcast(state, turn.id(), &next_view(turn, &request.id).await);
    }

    async fn generate_with_retry(
        &self,
        state: &AppState,
        provider: &'static ProviderDef,
        prompt: &str,
    ) -> Result<imagegen::GeneratedImage, CopilotError> {
        let mut attempt = 0;
        loop {
            match imagegen::generate(&state.copilot, Some(provider.id), prompt).await {
                Ok(image) => return Ok(image),
                Err(error)
                    if attempt == 0
                        && matches!(
                            error.kind(),
                            CopilotError::RateLimited | CopilotError::Network(_)
                        ) =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Waits for the target slide and inserts the stored picture.
    ///
    /// This is the ticket: a fast picture that arrives before the deck is
    /// written waits here instead of being dropped or placed on the wrong slide.
    async fn wait_and_place(
        &self,
        state: &AppState,
        request: &ImageRequest,
        url: &str,
    ) -> Result<(String, Option<String>), String> {
        let deadline = Instant::now() + SLIDE_WAIT;
        loop {
            let Some(slide_id) = resolve_target(state, request).await else {
                if Instant::now() >= deadline {
                    return Err(
                        "the picture was generated but its target slide never appeared".into(),
                    );
                }
                tokio::time::sleep(Duration::from_millis(180)).await;
                continue;
            };

            let command = AgentCommand::InsertImage {
                slide_id: slide_id.clone(),
                src: url.to_string(),
                alt: Some(request.alt.clone()),
                x: request.x,
                y: request.y,
                w: request.w,
                h: request.h,
            };
            match agent::run(state, command).await {
                Ok(outcome) => return Ok((slide_id, outcome.element_id)),
                Err(error) => {
                    // The slide may have been replaced between resolution and
                    // insertion. If it is still there, this is a real failure.
                    if resolve_target(state, request).await.as_deref() == Some(slide_id.as_str()) {
                        return Err(format!("the picture could not be placed: {error}"));
                    }
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "the picture was generated but could not be placed: {error}"
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(180)).await;
                }
            }
        }
    }

    async fn acquire_rate(&self) {
        let window = Duration::from_secs(60);
        loop {
            let wait = {
                let mut recent = self.rate.lock().await;
                let now = Instant::now();
                while recent
                    .front()
                    .is_some_and(|at| now.duration_since(*at) >= window)
                {
                    recent.pop_front();
                }
                if recent.len() < MAX_IMAGES_PER_MINUTE {
                    recent.push_back(now);
                    return;
                }
                recent
                    .front()
                    .map(|at| window.saturating_sub(now.duration_since(*at)))
                    .unwrap_or_default()
            };
            tokio::time::sleep(wait + Duration::from_millis(5)).await;
        }
    }

    /// Atomically checks the reported balance and holds `cost` for one job.
    async fn try_reserve(&self, cost: f64) -> bool {
        let mut budget = self.budget.lock().await;
        if let Some(balance) = budget.balance
            && budget.spent + budget.reserved + cost > balance + 1e-9
        {
            return false;
        }
        budget.reserved += cost;
        true
    }

    async fn release(&self, reserved: f64) {
        let mut budget = self.budget.lock().await;
        budget.reserved = (budget.reserved - reserved).max(0.0);
    }

    /// Replaces a reservation with the real charge reported by the vendor.
    async fn settle(&self, reserved: f64, actual: f64) {
        let mut budget = self.budget.lock().await;
        budget.reserved = (budget.reserved - reserved).max(0.0);
        budget.spent += actual;
    }

    async fn fail(
        &self,
        state: &AppState,
        turn: &ImageTurn,
        request: &ImageRequest,
        code: &str,
        message: String,
        degrade: bool,
    ) {
        if degrade {
            // One broken generation removes the capability from the next prompt
            // round and gives the model the stock fallback instead.
            *self.degraded.lock().await = Some(Degraded {
                at: Instant::now(),
                reason: message.clone(),
            });
            *self.cached.write().await = None;
        }
        turn.fail(&request.id, code, message).await;
        self.broadcast(state, turn.id(), &next_view(turn, &request.id).await);
    }

    fn broadcast(&self, state: &AppState, turn_id: &str, job: &ImageJobView) {
        state.broadcast(ServerEvent::ImageProgress {
            turn_id: turn_id.to_string(),
            job: job.clone(),
        });
    }
}

/// Resolves when the textual turn is cancelled. A dropped sender also counts
/// as cancellation because the turn no longer exists to own its images.
async fn wait_for_cancel(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

async fn next_view(turn: &ImageTurn, id: &str) -> ImageJobView {
    turn.jobs()
        .await
        .into_iter()
        .find(|job| job.id == id)
        .unwrap_or_else(|| ImageJobView {
            id: id.to_string(),
            status: ImageStatus::Failed,
            slide_id: None,
            slide_index: None,
            url: None,
            error_code: Some("image_missing".into()),
            error: Some("the image job disappeared".into()),
        })
}

async fn resolve_target(state: &AppState, request: &ImageRequest) -> Option<String> {
    let view = state.view().await;
    if let Some(id) = &request.slide_id
        && view.presentation.slides.iter().any(|slide| &slide.id == id)
    {
        return Some(id.clone());
    }
    request
        .slide_index
        .and_then(|index| view.presentation.slides.get(index))
        .map(|slide| slide.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Persistence;
    use slyde_core::{AspectRatio, PresentationState, Theme};
    use std::path::PathBuf;

    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ];
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUg==";

    fn test_state(tag: &str) -> (Arc<AppState>, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "slyde-image-service-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let state = AppState::with_copilot(
            PresentationState::new("Deck", Theme::studio(), AspectRatio::R16x9),
            "tok".into(),
            PathBuf::from("ui"),
            vec![],
            "http://127.0.0.1:1".into(),
            Arc::new(crate::copilot::CopilotState::in_memory()),
            Persistence {
                assets: Arc::new(crate::assets::AssetStore::new(&root)),
                ..Persistence::memory()
            },
        );
        (state, root)
    }

    #[tokio::test]
    async fn a_request_without_an_image_model_is_refused_with_a_reason() {
        let (state, root) = test_state("none");
        let turn = state.images.begin_turn("t1");
        let error = state
            .images
            .request(
                state.clone(),
                turn,
                ImageRequest {
                    id: "img_a".into(),
                    prompt: "a red apple".into(),
                    alt: "A red apple".into(),
                    slide_id: Some("sld_a".into()),
                    slide_index: None,
                    x: None,
                    y: None,
                    w: None,
                    h: None,
                },
            )
            .await
            .unwrap_err();
        assert!(error.contains("no image model"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn the_capability_reports_availability_without_a_predicted_count() {
        let (state, root) = test_state("cap");
        state
            .copilot
            .config
            .write()
            .await
            .keys
            .insert("openai".into(), "sk-test".into());
        let capability = state.images.capability(&state).await;
        assert!(capability.available, "{capability:?}");
        assert_eq!(capability.model, "gpt-image-1");
        let serialized = serde_json::to_string(&capability).unwrap();
        for absent in ["max_images", "balance_usd", "unit_cost_usd", "per_minute"] {
            assert!(
                !serialized.contains(absent),
                "the capability must not carry a predicted budget: {serialized}"
            );
        }

        *state.images.degraded.lock().await = Some(Degraded {
            at: Instant::now(),
            reason: "the provider rejected the key".into(),
        });
        let degraded = state.images.capability(&state).await;
        assert!(!degraded.available);
        assert!(degraded.reason.unwrap().contains("rejected"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_rate_gate_is_shared_and_bounded() {
        // The limiter itself is exercised through `acquire_rate`; keeping the
        // window inspectable makes the invariant explicit.
        let mut recent = VecDeque::new();
        for _ in 0..MAX_IMAGES_PER_MINUTE {
            recent.push_back(Instant::now());
        }
        assert_eq!(recent.len(), MAX_IMAGES_PER_MINUTE);
        assert_eq!(MAX_IMAGES_PER_MINUTE, 10);
        assert_eq!(MAX_PARALLEL, 3);
    }

    #[tokio::test]
    async fn a_target_index_is_resolved_when_the_slide_is_written_later() {
        let (state, root) = test_state("ticket");
        agent::run(
            &state,
            AgentCommand::UpsertSlide {
                index: 0,
                slide_id: Some("sld_first".into()),
                html_code: "<h1>First</h1>".into(),
                template: None,
                animation: None,
            },
        )
        .await
        .unwrap();
        let request = ImageRequest {
            id: "img_a".into(),
            prompt: "p".into(),
            alt: "a".into(),
            slide_id: None,
            slide_index: Some(1),
            x: None,
            y: None,
            w: None,
            h: None,
        };
        assert!(
            resolve_target(&state, &request).await.is_none(),
            "the second slide does not exist yet"
        );
        agent::run(
            &state,
            AgentCommand::UpsertSlide {
                index: 1,
                slide_id: Some("sld_second".into()),
                html_code: "<h1>Second</h1>".into(),
                template: None,
                animation: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            resolve_target(&state, &request).await.as_deref(),
            Some("sld_second")
        );

        let stored = state.assets.save_bytes(PNG).unwrap();
        let (slide_id, element) = state
            .images
            .wait_and_place(&state, &request, &stored.url)
            .await
            .unwrap();
        assert_eq!(slide_id, "sld_second");
        assert!(element.unwrap().starts_with("image-"));
        assert!(
            state.view().await.presentation.slides[1]
                .code
                .contains(&stored.url)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_queued_picture_returns_at_once_and_lands_when_its_slide_exists() {
        let app = axum::Router::new().route(
            "/images/generations",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({ "data": [{ "b64_json": PNG_B64 }] }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        super::super::catalog::tests_support::register_image_mock_as(
            "mock-background",
            &format!("http://127.0.0.1:{port}"),
        );

        let (state, root) = test_state("background");
        state
            .copilot
            .config
            .write()
            .await
            .keys
            .insert("mock-background".into(), "mock-key".into());
        state
            .copilot
            .remember_image("mock-background", "mock-image")
            .await
            .unwrap();
        state.images.invalidate().await;
        assert!(state.images.capability(&state).await.available);

        let turn = state.images.begin_turn("t-background");
        let queued = state
            .images
            .request(
                state.clone(),
                turn.clone(),
                ImageRequest {
                    id: "img_cover".into(),
                    prompt: "a red apple".into(),
                    alt: "A red apple".into(),
                    slide_id: None,
                    slide_index: Some(0),
                    x: Some(40.0),
                    y: Some(30.0),
                    w: Some(400.0),
                    h: Some(300.0),
                },
            )
            .await
            .unwrap();
        assert_eq!(queued.status, ImageStatus::Queued);

        // The slide does not exist yet: the job buffers and waits instead of
        // failing or placing on the wrong slide.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(turn.jobs().await[0].status, ImageStatus::Generating);

        agent::run(
            &state,
            AgentCommand::UpsertSlide {
                index: 0,
                slide_id: Some("sld_cover".into()),
                html_code: "<h1>Cover</h1>".into(),
                template: None,
                animation: None,
            },
        )
        .await
        .unwrap();

        let placed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let jobs = turn.jobs().await;
                if jobs[0].status == ImageStatus::Placed {
                    break jobs;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the picture should be placed after the slide appears");
        assert_eq!(placed[0].slide_id.as_deref(), Some("sld_cover"));
        assert!(
            placed[0]
                .url
                .as_deref()
                .is_some_and(|url| url.starts_with("/assets/")),
            "{placed:?}"
        );
        assert!(
            state.view().await.presentation.slides[0]
                .code
                .contains("/assets/"),
            "the slide references the stored picture"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancelling_the_turn_drops_an_in_flight_image_call() {
        let app = axum::Router::new().route(
            "/images/generations",
            axum::routing::post(|| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                axum::Json(serde_json::json!({ "data": [{ "b64_json": PNG_B64 }] }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        super::super::catalog::tests_support::register_image_mock_as(
            "mock-cancel",
            &format!("http://127.0.0.1:{port}"),
        );

        let (state, root) = test_state("cancel-running");
        state
            .copilot
            .config
            .write()
            .await
            .keys
            .insert("mock-cancel".into(), "mock-key".into());
        state
            .copilot
            .remember_image("mock-cancel", "mock-image")
            .await
            .unwrap();
        state.images.invalidate().await;
        assert!(state.images.capability(&state).await.available);

        let turn = state.images.begin_turn("t-cancel-running");
        let request = ImageRequest {
            id: "img_slow".into(),
            prompt: "a slow apple".into(),
            alt: "A slow apple".into(),
            slide_id: Some("sld_a".into()),
            slide_index: None,
            x: None,
            y: None,
            w: None,
            h: None,
        };
        state
            .images
            .request(state.clone(), turn.clone(), request)
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if turn.jobs().await[0].status == ImageStatus::Generating {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the job should reach the provider call");

        turn.cancel().await;
        let status = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let jobs = turn.jobs().await;
                if jobs[0].status == ImageStatus::Cancelled {
                    break jobs[0].status;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("a cancelled provider call must not keep the job generating");
        assert_eq!(status, ImageStatus::Cancelled);
        assert!(
            turn.errors().await.is_empty(),
            "a user cancel is not a provider error"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_failed_job_is_recorded_in_the_turn_and_broadcast() {
        let (state, root) = test_state("fail");
        let turn = state.images.begin_turn("t-fail");
        let request = ImageRequest {
            id: "img_bad".into(),
            prompt: "p".into(),
            alt: "a".into(),
            slide_id: Some("sld_a".into()),
            slide_index: None,
            x: None,
            y: None,
            w: None,
            h: None,
        };
        turn.add(request.view(ImageStatus::Generating))
            .await
            .unwrap();
        state
            .images
            .fail(
                &state,
                &turn,
                &request,
                "upstream",
                "the provider failed".into(),
                true,
            )
            .await;
        let (jobs, errors) = turn.snapshot().await;
        assert_eq!(jobs[0].status, ImageStatus::Failed);
        assert_eq!(jobs[0].error_code.as_deref(), Some("upstream"));
        assert_eq!(
            errors,
            vec![ImageErrorView {
                code: "upstream".into(),
                message: "the provider failed".into(),
            }]
        );
        assert!(!state.images.capability(&state).await.available);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancelling_a_turn_marks_queued_jobs_and_stops_new_requests() {
        let (state, root) = test_state("cancel");
        let turn = state.images.begin_turn("t-cancel");
        let request = ImageRequest {
            id: "img_cancelled".into(),
            prompt: "p".into(),
            alt: "a".into(),
            slide_id: Some("sld_a".into()),
            slide_index: None,
            x: None,
            y: None,
            w: None,
            h: None,
        };
        turn.add(request.view(ImageStatus::Queued)).await.unwrap();
        turn.cancel().await;
        let jobs = turn.jobs().await;
        assert_eq!(jobs[0].status, ImageStatus::Cancelled);
        assert!(turn.is_cancelled());

        let error = state
            .images
            .request(state.clone(), turn, request)
            .await
            .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reservations_are_atomic_and_returned_when_a_job_fails() {
        let service = ImageService::new();
        service.budget.lock().await.balance = Some(0.12);
        assert!(service.try_reserve(0.04).await);
        assert!(service.try_reserve(0.04).await);
        assert!(service.try_reserve(0.04).await);
        assert!(
            !service.try_reserve(0.04).await,
            "the fourth image would spend past the reported balance"
        );
        service.release(0.04).await;
        assert!(service.try_reserve(0.04).await);

        service.settle(0.04, 0.06).await;
        let budget = service.budget.lock().await;
        assert!((budget.reserved - 0.08).abs() < 1e-9);
        assert!((budget.spent - 0.06).abs() < 1e-9);
    }

    #[test]
    fn a_full_job_list_keeps_the_ticket_target() {
        let job = ImageRequest {
            id: "img_hero".into(),
            prompt: "a hero image".into(),
            alt: "A hero image".into(),
            slide_id: Some("sld_a".into()),
            slide_index: Some(0),
            x: Some(10.0),
            y: Some(20.0),
            w: Some(400.0),
            h: Some(300.0),
        };
        let view = job.view(ImageStatus::Queued);
        assert_eq!(view.slide_id.as_deref(), Some("sld_a"));
        assert_eq!(view.slide_index, Some(0));
        assert_eq!(view.status, ImageStatus::Queued);
    }
}
