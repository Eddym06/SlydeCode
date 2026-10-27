//! HTTP surface for named versions (see [`crate::versions`]).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::routes::{bad_request, forbidden, unauthorized};
use crate::state::{AppState, AuthQuery, is_authorized, origin_allowed};
use crate::versions::VersionError;

#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct IdRequest {
    pub id: String,
}

macro_rules! guard {
    ($state:expr, $headers:expr, $auth:expr) => {
        if !origin_allowed(&$state, &$headers) {
            return forbidden();
        }
        if !is_authorized(&$state, &$headers, &$auth) {
            return unauthorized();
        }
    };
}

fn listing(state: &AppState) -> serde_json::Value {
    json!({ "ok": true, "versions": state.versions.list(&state.deck_key()) })
}

fn failure(error: VersionError) -> Response {
    bad_request(&error.to_string())
}

pub async fn api_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Response {
    guard!(state, headers, auth);
    Json(listing(&state)).into_response()
}

pub async fn api_save(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<SaveRequest>,
) -> Response {
    guard!(state, headers, auth);
    let deck = state.view().await.presentation;
    match state
        .versions
        .save(&state.deck_key(), &request.name, false, &deck)
    {
        Ok(_) => Json(listing(&state)).into_response(),
        Err(error) => failure(error),
    }
}

/// Restores a version as one undoable step. The deck as it is now is saved as an
/// automatic version first, so a restore can always be taken back.
pub async fn api_restore(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<IdRequest>,
) -> Response {
    guard!(state, headers, auth);
    let key = state.deck_key();
    let target = match state.versions.load(&key, &request.id) {
        Ok(deck) => deck,
        Err(error) => return failure(error),
    };
    let current = state.view().await.presentation;
    if let Err(error) = state
        .versions
        .save(&key, "Before restoring a version", true, &current)
    {
        return failure(error);
    }
    if let Err(error) = state.core.write().await.restore(target) {
        return bad_request(&error.to_string());
    }
    state.publish_state().await;
    let mut body = listing(&state);
    body["view"] = json!(state.view().await);
    Json(body).into_response()
}

pub async fn api_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<IdRequest>,
) -> Response {
    guard!(state, headers, auth);
    match state.versions.delete(&state.deck_key(), &request.id) {
        Ok(()) => Json(listing(&state)).into_response(),
        Err(error) => failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use slyde_core::{AspectRatio, PresentationState, TemplateType, Theme};
    use tower::util::ServiceExt;

    fn state() -> Arc<AppState> {
        let mut core = PresentationState::new("Deck", Theme::studio(), AspectRatio::R16x9);
        core.upsert_slide(0, None, "<h1>One</h1>".into(), TemplateType::Hero, vec![])
            .unwrap();
        AppState::new(
            core,
            "tok".into(),
            std::path::PathBuf::from("ui"),
            vec![],
            "http://127.0.0.1:1".into(),
        )
    }

    async fn call(
        state: &Arc<AppState>,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, serde_json::Value) {
        let response = crate::routes::app_router(state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(format!("{path}?t=tok"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(json!({})))
    }

    #[tokio::test]
    async fn a_named_version_can_be_restored_and_the_restore_is_undoable() {
        let state = state();
        let (status, saved) = call(
            &state,
            "POST",
            "/api/versions",
            r#"{"name":"Before the rewrite"}"#,
        )
        .await;
        assert_eq!(status, 200);
        let id = saved["versions"][0]["id"].as_str().unwrap().to_string();

        state
            .core
            .write()
            .await
            .upsert_slide(1, None, "<h1>Two</h1>".into(), TemplateType::Hero, vec![])
            .unwrap();
        assert_eq!(state.view().await.presentation.slides.len(), 2);

        let (status, restored) = call(
            &state,
            "POST",
            "/api/versions/restore",
            &format!(r#"{{"id":"{id}"}}"#),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(state.view().await.presentation.slides.len(), 1);
        assert_eq!(
            restored["view"]["presentation"]["slides"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let versions = restored["versions"].as_array().unwrap();
        assert!(
            versions
                .iter()
                .any(|v| v["auto"] == true && v["slides"] == 2),
            "the deck before restoring is kept as an automatic version"
        );

        state.core.write().await.undo().unwrap();
        assert_eq!(
            state.view().await.presentation.slides.len(),
            2,
            "restore is one undo step"
        );

        let (status, _) = call(
            &state,
            "POST",
            "/api/versions/restore",
            r#"{"id":"missing"}"#,
        )
        .await;
        assert_eq!(status, 400);
        let (status, after) = call(
            &state,
            "POST",
            "/api/versions/delete",
            &format!(r#"{{"id":"{id}"}}"#),
        )
        .await;
        assert_eq!(status, 200);
        assert!(
            after["versions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["id"] != id.as_str())
        );
    }
}
