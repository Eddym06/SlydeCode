//! Cover thumbnails for the deck library.
//!
//! A thumbnail is the first slide rendered by the same Chrome that renders every
//! other export, so it can never disagree with the presenter. Rendering is
//! strictly on demand: the library sheet asks for the covers it is about to show
//! (`POST /api/decks/thumbnails`), the server renders the missing ones in **one**
//! Chrome session and caches them as `<library>/thumbs/<id>-<updated_at>.png`.
//! The `updated_at` in the name is the invalidation rule: an edited deck has a
//! new key, so a stale cover is never served, and the old file is deleted when
//! the new one lands.
//!
//! Nothing here launches a browser speculatively (charter §9): no request, no
//! Chrome.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use slyde_core::Presentation;
use slyde_export::SlideTarget;

use crate::library::Library;
use crate::routes::{bad_request, forbidden, unauthorized};
use crate::state::{AppState, AuthQuery, is_authorized, origin_allowed};
use crate::{discover_browser, now_ms, sandbox, storage};

/// Most covers one request may render. The sheet shows a screenful at a time.
const MAX_IDS: usize = 24;
/// Covers are small previews, so 1x is plenty.
const THUMBNAIL_SCALE: u32 = 1;

static WORK_SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// One render at a time: two library sheets must not start two Chromes.
static RENDER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Deserialize)]
pub struct EnsureRequest {
    #[serde(default)]
    pub ids: Vec<String>,
}

fn thumbs_dir(library: &Library) -> PathBuf {
    library.root().join("thumbs")
}

fn cache_file(library: &Library, id: &str, updated_at_ms: u64) -> PathBuf {
    thumbs_dir(library).join(format!("{id}-{updated_at_ms}.png"))
}

/// Deletes every cached cover of a deck. Called when the deck is removed.
pub fn forget(root: &Path, id: &str) {
    let prefix = format!("{id}-");
    let Ok(entries) = std::fs::read_dir(root.join("thumbs")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `<id>-<digits>.png` exactly: another deck whose id merely starts with
        // this one (`alpha` and `alpha-beta`) must keep its covers.
        let is_cover = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".png"))
            .is_some_and(|stamp| !stamp.is_empty() && stamp.bytes().all(|b| b.is_ascii_digit()));
        if is_cover {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The deck as it is *now*: the open deck lives in memory (its autosave is
/// debounced), every other deck is read from its file.
pub(crate) async fn deck_for(
    state: &AppState,
    library: &Library,
    id: &str,
) -> Result<Option<Presentation>, crate::error::ServerError> {
    if library.active().as_deref() == Some(id) {
        return Ok(Some(state.core.read().await.presentation().clone()));
    }
    library.load(id)
}

/// Sandbox-origin page that shows the first slide of a library deck, static and
/// without the editing layer. The only reader is the headless Chrome rendering
/// a cover.
pub async fn sandbox_cover(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !crate::state::is_slide_authorized(&state, &auth) {
        return unauthorized();
    }
    let Some(library) = state.library.clone() else {
        return (StatusCode::NOT_FOUND, "no deck library").into_response();
    };
    let deck = match deck_for(&state, &library, &id).await {
        Ok(Some(deck)) => deck,
        Ok(None) => return (StatusCode::NOT_FOUND, "deck not found").into_response(),
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid deck id").into_response(),
    };
    let Some(slide) = deck.slides.first() else {
        return (StatusCode::NOT_FOUND, "the deck has no slides").into_response();
    };

    let html = sandbox::render_document(&deck, slide, true, false);
    let mut response = axum::response::Html(html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(sandbox::csp_header()),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
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

/// `POST /api/decks/thumbnails` — renders whichever of `ids` has no current
/// cover and answers with every id that now has one.
pub async fn api_ensure(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<EnsureRequest>,
) -> Response {
    if let Some(denied) = guard(&state, &headers, &auth) {
        return denied;
    }
    let Some(library) = state.library.clone() else {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "code": "no_library" })),
        )
            .into_response();
    };
    if request.ids.len() > MAX_IDS {
        return bad_request(&format!("ask for at most {MAX_IDS} covers at a time"));
    }

    let mut ready: Vec<String> = Vec::new();
    // Decks that still need a render, grouped by canvas shape.
    let mut pending: BTreeMap<String, Vec<(String, u64, slyde_core::AspectRatio)>> =
        BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for id in request.ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let deck = match deck_for(&state, &library, &id).await {
            Ok(Some(deck)) if !deck.slides.is_empty() => deck,
            Ok(_) => continue,
            Err(error) => return bad_request(&error.to_string()),
        };
        let updated = deck.updated_at_ms;
        if cache_file(&library, &id, updated).is_file() {
            ready.push(id);
        } else {
            pending
                .entry(deck.aspect_ratio.as_str().to_string())
                .or_default()
                .push((id, updated, deck.aspect_ratio));
        }
    }

    let mut browser_missing = false;
    if !pending.is_empty() {
        let _turn = RENDER_LOCK.lock().await;
        match discover_browser() {
            Err(_) => browser_missing = true,
            Ok(browser) => {
                for group in pending.into_values() {
                    ready.extend(render_group(&state, &library, &browser.path, group).await);
                }
            }
        }
    }

    Json(json!({ "ok": true, "ready": ready, "browser_missing": browser_missing })).into_response()
}

/// Renders one canvas shape's worth of covers in a single Chrome session.
async fn render_group(
    state: &AppState,
    library: &Library,
    chrome: &Path,
    group: Vec<(String, u64, slyde_core::AspectRatio)>,
) -> Vec<String> {
    let Some(ratio) = group.first().map(|entry| entry.2) else {
        return Vec::new();
    };
    let slide_token = sandbox::percent_encode(&state.sandbox_token);
    let targets: Vec<SlideTarget> = group
        .iter()
        .map(|(id, _, _)| SlideTarget {
            id: id.clone(),
            url: format!(
                "{}/library/{}/cover?t={slide_token}",
                state.sandbox_origin,
                sandbox::percent_encode(id)
            ),
        })
        .collect();

    let work = storage::exports_dir().join(format!(
        "covers-{}-{}-{}",
        now_ms(),
        std::process::id(),
        WORK_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if tokio::fs::create_dir_all(&work).await.is_err() {
        return Vec::new();
    }

    let mut done = Vec::new();
    match slyde_export::export_png_batch(chrome, &targets, ratio, &work, THUMBNAIL_SCALE, None)
        .await
    {
        Ok(files) => {
            let _ = tokio::fs::create_dir_all(thumbs_dir(library)).await;
            for ((id, updated, _), file) in group.iter().zip(files) {
                forget(library.root(), id);
                let target = cache_file(library, id, *updated);
                match tokio::fs::copy(&file, &target).await {
                    Ok(_) => done.push(id.clone()),
                    Err(error) => tracing::warn!(%error, deck = %id, "could not cache a cover"),
                }
            }
        }
        Err(error) => tracing::warn!(%error, "could not render deck covers"),
    }
    let _ = tokio::fs::remove_dir_all(&work).await;
    done
}

/// `GET /api/decks/{id}/thumbnail` — serves a cached cover, never renders one.
pub async fn api_thumbnail(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if let Some(denied) = guard(&state, &headers, &auth) {
        return denied;
    }
    let Some(library) = state.library.clone() else {
        return (StatusCode::NOT_FOUND, "no deck library").into_response();
    };
    let deck = match deck_for(&state, &library, &id).await {
        Ok(Some(deck)) => deck,
        Ok(None) => return (StatusCode::NOT_FOUND, "deck not found").into_response(),
        Err(error) => return bad_request(&error.to_string()),
    };
    match tokio::fs::read(cache_file(&library, &id, deck.updated_at_ms)).await {
        Ok(bytes) => {
            let mut response = bytes.into_response();
            let out = response.headers_mut();
            out.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            out.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=300"),
            );
            response
        }
        Err(_) => (StatusCode::NOT_FOUND, "the cover has not been rendered yet").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forget_removes_only_the_covers_of_that_deck() {
        let root = std::env::temp_dir().join(format!("slyde-thumbs-{}", std::process::id()));
        let thumbs = root.join("thumbs");
        std::fs::create_dir_all(&thumbs).unwrap();
        for name in [
            "alpha-1.png",
            "alpha-2.png",
            "alpha-beta-1.png",
            "beta-1.png",
        ] {
            std::fs::write(thumbs.join(name), b"png").unwrap();
        }
        forget(&root, "alpha");
        let mut left: Vec<String> = std::fs::read_dir(&thumbs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec!["alpha-beta-1.png".to_string(), "beta-1.png".to_string()],
            "another deck whose id starts with this one keeps its cover"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
