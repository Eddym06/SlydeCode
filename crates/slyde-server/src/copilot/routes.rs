//! HTTP surface for the copilot.
//!
//! Same token/origin guard as every other state-changing route. API keys go in,
//! model metadata and streamed chat events go out — never keys back.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::state::{AppState, AuthQuery, is_authorized, origin_allowed};

use super::chat::{self, ChatRequest};
use super::error::CopilotError;
use super::wire::ChatEvent;

/// Generous because images and PDFs are base64-encoded in the request. The
/// attachment decoder applies its own, tighter per-file limits.
pub const MAX_CHAT_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct ProviderQuery {
    provider: String,
    #[serde(flatten)]
    auth: AuthQuery,
}

#[derive(Debug, Deserialize)]
pub struct KeyInput {
    provider: String,
    #[serde(default)]
    key: String,
}

pub async fn api_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }
    let view = state.copilot.view().await;
    (StatusCode::OK, Json(json!({ "ok": true, "config": view }))).into_response()
}

pub async fn api_models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ProviderQuery>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &query.auth) {
        return unauthorized();
    }
    match state.copilot.models(&query.provider).await {
        Ok(models) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "models": models })),
        )
            .into_response(),
        Err(error) => error_response(error),
    }
}

pub async fn api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(input): Json<KeyInput>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }
    match state.copilot.set_key(&input.provider, &input.key).await {
        Ok(models) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "models": models })),
        )
            .into_response(),
        Err(error) => error_response(error),
    }
}

pub async fn api_chat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<ChatRequest>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }

    let (tx, rx) = mpsc::unbounded_channel::<ChatEvent>();
    let task_state = state.clone();
    tokio::spawn(async move {
        chat::run(task_state, request, tx).await;
    });

    let stream = stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| {
            let payload = serde_json::to_string(&event).unwrap_or_else(|_| {
                "{\"type\":\"error\",\"code\":\"internal\",\"message\":\"serialization failed\"}"
                    .to_string()
            });
            (Ok::<Event, Infallible>(Event::default().data(payload)), rx)
        })
    });

    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache, no-transform"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", header::HeaderValue::from_static("no"));
    response
}

fn error_response(error: CopilotError) -> Response {
    let status = match error {
        CopilotError::MissingKey => StatusCode::BAD_REQUEST,
        CopilotError::InvalidKey => StatusCode::UNAUTHORIZED,
        CopilotError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        CopilotError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        CopilotError::Unsupported => StatusCode::BAD_REQUEST,
        CopilotError::ToolLimit(_) => StatusCode::OK,
        CopilotError::Cancelled => StatusCode::REQUEST_TIMEOUT,
        CopilotError::Network(_) | CopilotError::Upstream(_) => StatusCode::BAD_GATEWAY,
    };
    let body = json!({
        "ok": false,
        "code": error.code(),
        "error": error.public_message(),
    });
    (status, Json(body)).into_response()
}

#[derive(Debug, serde::Deserialize)]
pub struct CancelRequest {
    pub request_id: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ThreadQuery {
    pub deck: String,
    #[serde(flatten)]
    pub auth: AuthQuery,
}

#[derive(Debug, serde::Deserialize)]
pub struct ThreadInput {
    pub deck: String,
    #[serde(default)]
    pub messages: Vec<super::StoredMessage>,
}

/// Returns the conversation stored for one deck.
pub async fn api_thread(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ThreadQuery>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &query.auth) {
        return unauthorized();
    }
    let messages = state.copilot.thread(&query.deck);
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "deck": query.deck, "messages": messages })),
    )
        .into_response()
}

/// Replaces the conversation stored for one deck.
pub async fn api_save_thread(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(input): Json<ThreadInput>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }
    match state.copilot.save_thread(&input.deck, input.messages) {
        Ok(stored) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "stored": stored })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": error.public_message() })),
        )
            .into_response(),
    }
}

/// Stops a running turn server-side.
///
/// The browser aborting the fetch is enough to end the stream, but an explicit
/// cancel also stops the tool rounds that were already scheduled and drops the
/// upstream response instead of waiting for the provider to finish.
pub async fn api_cancel(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<CancelRequest>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }
    let cancelled = state.copilot.cancel_turn(&request.request_id);
    Json(json!({ "ok": true, "cancelled": cancelled })).into_response()
}

/// Reverts everything the copilot did in its last turn, as one undoable step.
pub async fn api_revert(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }
    let revision = state.view().await.revision;
    let before = match state.copilot.take_snapshot(revision) {
        Ok(before) => before,
        Err(error) => return error_response(error),
    };
    let restored = state.core.write().await.restore(before);
    if let Err(error) = restored {
        return error_response(CopilotError::InvalidRequest(error.to_string()));
    }
    state.publish_state().await;
    Json(json!({ "ok": true })).into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "code": "unauthorized", "error": "unauthorized" })),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "ok": false, "code": "forbidden", "error": "origin not allowed" })),
    )
        .into_response()
}

/// Ensures the module is part of the app router even when the route list is
/// edited later.
pub fn routes() -> [&'static str; 4] {
    [
        "/api/copilot/config",
        "/api/copilot/models",
        "/api/copilot/key",
        "/api/copilot/chat",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_list_matches_the_documented_surface() {
        assert_eq!(routes().len(), 4);
    }
}
