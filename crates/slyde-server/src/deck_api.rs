//! Deck library endpoints.
//!
//! The UI, the MCP bridge and the copilot all edit the *active* deck, so a
//! library operation is an ordinary state change plus a pointer move: flush the
//! outgoing deck, point the library at the new file, replace the state and
//! publish. Nothing else in the server needs to know a library exists.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use slyde_core::PresentationState;

use crate::library::{self, Library};
use crate::routes::{bad_request, forbidden, unauthorized};
use crate::state::{AppState, AuthQuery, is_authorized, origin_allowed};

#[derive(Debug, Default, Deserialize)]
pub struct NewDeckRequest {
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeckIdRequest {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub id: String,
    pub title: String,
}

#[allow(clippy::result_large_err)] // a 422 body is the error here; boxing it buys nothing on this path
fn library_of(state: &AppState) -> Result<Arc<Library>, Response> {
    state.library.clone().ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "code": "no_library",
                "error": "this server runs in memory and has no deck library"
            })),
        )
            .into_response()
    })
}

fn guard(state: &AppState, headers: &HeaderMap, auth: &AuthQuery) -> Option<Response> {
    if !origin_allowed(state, headers) {
        return Some(forbidden());
    }
    if !is_authorized(state, headers, auth) {
        return Some(unauthorized());
    }
    None
}

/// Persists the deck currently in memory to the active file.
///
/// Called before switching decks: the debounced autosave may have a pending
/// write, and switching first would send that write to the new deck's file.
async fn flush_active(state: &AppState, library: &Library) {
    let Some(active) = library.active() else {
        return;
    };
    let deck = state.core.read().await.presentation().clone();
    let path = library.path_for(&active);
    if let Err(error) = crate::storage::save(&path, &deck).await {
        tracing::error!(error = %error, path = %path.display(), "could not flush the outgoing deck");
    }
}

#[allow(clippy::result_large_err)] // same: the Err side is a ready-to-send response
async fn activate(state: &AppState, library: &Library, id: &str) -> Result<(), Response> {
    let deck = match library.load(id) {
        Ok(Some(deck)) => deck,
        Ok(None) => return Err(bad_request("that deck does not exist")),
        Err(error) => return Err(bad_request(&error.to_string())),
    };
    flush_active(state, library).await;
    if let Err(error) = library.set_active(id) {
        return Err(bad_request(&error.to_string()));
    }
    let mut core =
        PresentationState::new(deck.title.clone(), deck.theme.clone(), deck.aspect_ratio);
    if let Err(error) = core.replace(deck) {
        return Err(bad_request(&error.to_string()));
    }
    *state.core.write().await = core;
    state.publish_state().await;
    Ok(())
}

fn decks_payload(library: &Library) -> serde_json::Value {
    let decks = library.list().unwrap_or_default();
    json!({
        "ok": true,
        "active": library.active(),
        "decks": decks,
    })
}

pub async fn api_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let library = match library_of(&state) {
        Ok(library) => library,
        Err(response) => return response,
    };
    Json(decks_payload(&library)).into_response()
}

pub async fn api_new(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<NewDeckRequest>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let library = match library_of(&state) {
        Ok(library) => library,
        Err(response) => return response,
    };

    let deck = library::welcome_deck();
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| deck.title.clone());
    let id = match library.create(&title, &deck).await {
        Ok(id) => id,
        Err(error) => return bad_request(&error.to_string()),
    };
    if let Err(response) = activate(&state, &library, &id).await {
        return response;
    }
    let mut payload = decks_payload(&library);
    payload["view"] = json!(state.view().await);
    Json(payload).into_response()
}

pub async fn api_open(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<DeckIdRequest>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let library = match library_of(&state) {
        Ok(library) => library,
        Err(response) => return response,
    };
    if let Err(response) = activate(&state, &library, &request.id).await {
        return response;
    }
    let mut payload = decks_payload(&library);
    payload["view"] = json!(state.view().await);
    Json(payload).into_response()
}

pub async fn api_rename(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<RenameRequest>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let library = match library_of(&state) {
        Ok(library) => library,
        Err(response) => return response,
    };
    if let Err(error) = library.rename(&request.id, &request.title).await {
        return bad_request(&error.to_string());
    }
    // Renaming the open deck has to show up in the header now, not after the
    // next reload. `set_title` keeps it undoable like any other edit.
    if library.active().as_deref() == Some(request.id.as_str())
        && let Err(error) = state
            .core
            .write()
            .await
            .set_title(request.title.trim().to_string())
    {
        return bad_request(&error.to_string());
    }
    state.publish_state().await;
    Json(decks_payload(&library)).into_response()
}

pub async fn api_duplicate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<DeckIdRequest>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let library = match library_of(&state) {
        Ok(library) => library,
        Err(response) => return response,
    };
    let id = match library.duplicate(&request.id).await {
        Ok((id, _)) => id,
        Err(error) => return bad_request(&error.to_string()),
    };
    if let Err(response) = activate(&state, &library, &id).await {
        return response;
    }
    let mut payload = decks_payload(&library);
    payload["view"] = json!(state.view().await);
    Json(payload).into_response()
}

pub async fn api_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<DeckIdRequest>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let library = match library_of(&state) {
        Ok(library) => library,
        Err(response) => return response,
    };

    let deleting_active = library.active().as_deref() == Some(request.id.as_str());
    if !deleting_active {
        if let Err(error) = library.remove(&request.id) {
            return bad_request(&error.to_string());
        }
        return Json(decks_payload(&library)).into_response();
    }

    // Deleting the open deck: open the newest survivor, or seed a fresh deck so
    // the user is never left with an empty workspace.
    let next = library
        .list()
        .unwrap_or_default()
        .into_iter()
        .find(|deck| deck.id != request.id)
        .map(|deck| deck.id);
    let next = match next {
        Some(id) => id,
        None => {
            let deck = library::welcome_deck();
            match library.create(&deck.title.clone(), &deck).await {
                Ok(id) => id,
                Err(error) => return bad_request(&error.to_string()),
            }
        }
    };
    if let Err(response) = activate(&state, &library, &next).await {
        return response;
    }
    if let Err(error) = library.remove(&request.id) {
        return bad_request(&error.to_string());
    }
    let mut payload = decks_payload(&library);
    payload["view"] = json!(state.view().await);
    Json(payload).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use slyde_core::{AspectRatio, Presentation, Slide, TemplateType, Theme};
    use tower::util::ServiceExt;

    fn library_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slyde-deck-api-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A server whose in-memory deck and active library file describe the same
    /// deck, like the running app. `flush_active` relies on that invariant, so a
    /// test that breaks it would be testing a state the product never reaches.
    async fn state_with_library(dir: &std::path::Path) -> (Arc<AppState>, Arc<Library>) {
        let library = Arc::new(Library::new(dir));
        let mut deck = library::welcome_deck();
        deck.title = "Library test".into();
        let id = library.create("Library test", &deck).await.unwrap();
        library.set_active(&id).unwrap();

        let state = AppState::with_copilot(
            state_for(library.load(&id).unwrap().unwrap()),
            "tok_test".into(),
            std::path::PathBuf::from("ui"),
            vec!["http://127.0.0.1:4444".into()],
            "http://127.0.0.1:4445".into(),
            Arc::new(crate::copilot::CopilotState::in_memory()),
            crate::state::Persistence {
                library: Some(library.clone()),
                ..crate::state::Persistence::memory()
            },
        );
        (state, library)
    }

    fn state_for(deck: Presentation) -> PresentationState {
        let mut core =
            PresentationState::new(deck.title.clone(), deck.theme.clone(), deck.aspect_ratio);
        core.replace(deck).unwrap();
        core
    }

    /// Switches the open deck the way an endpoint does: pointer, file and memory.
    async fn make_active(state: &AppState, library: &Library, id: &str) {
        library.set_active(id).unwrap();
        let deck = library.load(id).unwrap().unwrap();
        *state.core.write().await = state_for(deck);
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn post(path: &str, body: &str) -> Request<Body> {
        Request::post(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn library_requires_a_token() {
        let dir = library_dir("token");
        let (state, _) = state_with_library(&dir).await;
        let app = crate::routes::app_router(state);

        let response = app
            .clone()
            .oneshot(Request::get("/api/decks").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(post("/api/decks/new?t=wrong", "{}"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn creating_a_deck_keeps_the_previous_one_on_disk() {
        let dir = library_dir("create");
        let (state, library) = state_with_library(&dir).await;
        let first = library.active().unwrap();

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/new?t=tok_test",
                r#"{"title":"Second deck"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = body_json(response).await;
        assert_eq!(payload["decks"].as_array().unwrap().len(), 2);
        let active = payload["active"].as_str().unwrap().to_string();
        assert_ne!(active, first, "the new deck becomes active");

        // Both files exist: starting a deck no longer overwrites the other one.
        assert!(library.path_for(&first).is_file());
        assert!(library.path_for(&active).is_file());
        assert_eq!(
            library.load(&first).unwrap().unwrap().title,
            "Library test",
            "the outgoing deck keeps its content"
        );
        assert_eq!(state.core.read().await.presentation().title, "Second deck");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn opening_a_deck_replaces_the_state_and_the_pointer() {
        let dir = library_dir("open");
        let (state, library) = state_with_library(&dir).await;
        let mut other = Presentation::new("Deck B", Theme::paper(), AspectRatio::R4x3);
        other
            .slides
            .push(Slide::new("sld_b".into(), "<h1>B</h1>".into()));
        let b = library.create("Deck B", &other).await.unwrap();

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/open?t=tok_test",
                &format!(r#"{{"id":"{b}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(library.active().as_deref(), Some(b.as_str()));
        let view = state.view().await;
        assert_eq!(view.presentation.title, "Deck B");
        assert_eq!(view.presentation.slides.len(), 1);
        assert_eq!(view.presentation.aspect_ratio, AspectRatio::R4x3);
        assert!(!view.can_undo, "opening a deck restarts history");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deleting_the_active_deck_opens_another_one() {
        let dir = library_dir("delete");
        let (state, library) = state_with_library(&dir).await;
        let keep = library.active().unwrap();
        let doomed = library
            .create("Delete me", &library::welcome_deck())
            .await
            .unwrap();
        make_active(&state, &library, &doomed).await;

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/delete?t=tok_test",
                &format!(r#"{{"id":"{doomed}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(library.active().as_deref(), Some(keep.as_str()));
        assert!(!library.path_for(&doomed).exists());
        assert_eq!(state.core.read().await.presentation().title, "Library test");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deleting_the_last_deck_seeds_a_fresh_one() {
        let dir = library_dir("last");
        let (state, library) = state_with_library(&dir).await;
        let only = library.active().unwrap();

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/delete?t=tok_test",
                &format!(r#"{{"id":"{only}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let payload = body_json(response).await;
        let decks = payload["decks"].as_array().unwrap();
        assert_eq!(decks.len(), 1, "the workspace is never left empty");
        assert_ne!(payload["active"].as_str().unwrap(), only);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn rename_updates_the_file_and_the_open_deck() {
        let dir = library_dir("rename");
        let (state, library) = state_with_library(&dir).await;
        let id = library.active().unwrap();

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/rename?t=tok_test",
                &format!(r#"{{"id":"{id}","title":"New name"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            library.load(&id).unwrap().unwrap().title,
            "New name",
            "the file is renamed"
        );
        assert_eq!(
            state.core.read().await.presentation().title,
            "New name",
            "the open deck is renamed"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn rename_rejects_an_empty_title() {
        let dir = library_dir("rename-empty");
        let (state, library) = state_with_library(&dir).await;
        let id = library.active().unwrap();

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/rename?t=tok_test",
                &format!(r#"{{"id":"{id}","title":"   "}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            library.load(&id).unwrap().unwrap().title,
            "Library test",
            "a rejected rename changes nothing"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn duplicate_opens_a_copy_and_leaves_the_original_alone() {
        let dir = library_dir("duplicate");
        let (state, library) = state_with_library(&dir).await;
        let id = library.active().unwrap();

        let app = crate::routes::app_router(state.clone());
        let response = app
            .oneshot(post(
                "/api/decks/duplicate?t=tok_test",
                &format!(r#"{{"id":"{id}"}}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = body_json(response).await;

        let decks = payload["decks"].as_array().unwrap();
        assert_eq!(decks.len(), 2);
        let active = payload["active"].as_str().unwrap();
        assert_ne!(active, id);
        assert_eq!(
            library.load(&id).unwrap().unwrap().title,
            "Library test",
            "the original keeps its title"
        );
        assert_eq!(
            library.load(active).unwrap().unwrap().title,
            "Library test (copy)"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn listing_reports_the_active_deck_and_its_metadata() {
        let dir = library_dir("list");
        let (state, library) = state_with_library(&dir).await;
        let id = library.active().unwrap();

        let app = crate::routes::app_router(state);
        let response = app
            .oneshot(
                Request::get("/api/decks?t=tok_test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload = body_json(response).await;
        assert_eq!(payload["active"], id.as_str());
        assert_eq!(payload["decks"][0]["title"], "Library test");
        assert_eq!(payload["decks"][0]["slides"], 1);
        assert!(payload["decks"][0]["updated_at_ms"].as_u64().unwrap() > 0);
        assert_eq!(payload["decks"][0]["theme_id"], "studio");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deck_covers_render_the_first_slide_and_need_the_sandbox_token() {
        let dir = library_dir("cover");
        let (state, library) = state_with_library(&dir).await;
        let id = library.active().unwrap();
        let token = state.sandbox_token.clone();

        let denied = crate::routes::sandbox_router(state.clone())
            .oneshot(
                Request::get(format!("/d/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let ok = crate::routes::sandbox_router(state.clone())
            .oneshot(
                Request::get(format!("/d/{id}?t={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let body = axum::body::to_bytes(ok.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("slyde-stage"));

        let missing = crate::routes::sandbox_router(state)
            .oneshot(
                Request::get(format!("/d/..%2Fsecret?t={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unknown_deck_ids_are_rejected() {
        let dir = library_dir("unknown");
        let (state, _) = state_with_library(&dir).await;
        let app = crate::routes::app_router(state);

        let response = app
            .clone()
            .oneshot(post("/api/decks/open?t=tok_test", r#"{"id":"nope"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Path traversal never reaches the filesystem.
        let response = app
            .oneshot(post(
                "/api/decks/open?t=tok_test",
                r#"{"id":"../../settings"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_server_without_a_library_says_so() {
        let mut core = PresentationState::new("Memory", Theme::studio(), AspectRatio::R16x9);
        core.upsert_slide(
            0,
            Some("sld_a".into()),
            "<h1>Hello</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        let state = AppState::new(
            core,
            "tok_test".into(),
            std::path::PathBuf::from("ui"),
            vec!["http://127.0.0.1:4444".into()],
            "http://127.0.0.1:4445".into(),
        );
        let response = crate::routes::app_router(state)
            .oneshot(
                Request::get("/api/decks?t=tok_test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = body_json(response).await;
        assert_eq!(payload["code"], "no_library");
    }
}
