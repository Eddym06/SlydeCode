//! Image asset endpoints.
//!
//! Uploads arrive as a base64 data URL, which is exactly what the canvas has when
//! the user pastes an image; the server stores the bytes and answers with the
//! `/assets/<id>.<ext>` path a slide can reference. Downloads serve those bytes
//! back on **both** origins, because a slide document renders from the sandbox
//! origin and only `'self'` is allowed by its CSP.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::assets::name_from_slide_url;
use crate::routes::{bad_request, forbidden, unauthorized};
use crate::state::{AppState, AuthQuery, is_authorized, origin_allowed};

#[derive(Debug, Deserialize)]
pub struct UploadRequest {
    /// `data:image/png;base64,…`
    pub data: String,
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

/// Stores one pasted or uploaded image.
pub async fn api_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(request): Json<UploadRequest>,
) -> Response {
    if let Some(response) = guard(&state, &headers, &auth) {
        return response;
    }
    let info = match state.assets.save_data_url(&request.data) {
        Ok(info) => info,
        Err(error) => return bad_request(&error.to_string()),
    };
    tracing::info!(url = %info.url, bytes = info.bytes, "stored an image asset");
    Json(json!({
        "ok": true,
        "id": info.id,
        "url": info.url,
        "content_type": info.content_type,
        "bytes": info.bytes,
    }))
    .into_response()
}

/// Serves a stored image. The path is `/assets/<name>`; the name is validated
/// against the store's naming rule, so no other file can be reached.
pub async fn serve_asset(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let Some(path) = state.assets.path_for(&name) else {
        return (StatusCode::NOT_FOUND, "asset not found").into_response();
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::NOT_FOUND, "asset not found").into_response(),
    };
    let content_type = match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "application/octet-stream",
    };
    let mut response = Body::from(bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// `/assets/<name>` parsed the same way a slide URL would be, used by the bundle
/// writer to know which files to include.
pub fn referenced_asset(value: &str) -> Option<String> {
    name_from_slide_url(value).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use slyde_core::{AspectRatio, PresentationState, Theme};
    use tower::util::ServiceExt;

    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUg==";

    fn state_with_store(tag: &str) -> (Arc<AppState>, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "slyde-asset-api-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let core = PresentationState::new("Assets", Theme::studio(), AspectRatio::R16x9);
        let state = AppState::with_copilot(
            core,
            "tok_test".into(),
            std::path::PathBuf::from("ui"),
            vec!["http://127.0.0.1:4444".into()],
            "http://127.0.0.1:4445".into(),
            Arc::new(crate::copilot::CopilotState::in_memory()),
            crate::state::Persistence {
                assets: Arc::new(crate::assets::AssetStore::new(&root)),
                ..crate::state::Persistence::memory()
            },
        );
        (state, root)
    }

    fn upload_request(body: &str, query: &str) -> Request<Body> {
        Request::post(format!("/api/assets{query}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn an_uploaded_image_is_served_back_on_both_origins() {
        let (state, root) = state_with_store("roundtrip");
        let app = crate::routes::app_router(state.clone());
        let sandbox = crate::routes::sandbox_router(state.clone());

        let response = app
            .clone()
            .oneshot(upload_request(
                &format!(r#"{{"data":"data:image/png;base64,{PNG_B64}"}}"#),
                "?t=tok_test",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(payload["content_type"], "image/png");
        let url = payload["url"].as_str().unwrap().to_string();
        assert!(url.starts_with("/assets/"), "{url}");

        for router in [app, sandbox] {
            let response = router
                .clone()
                .oneshot(Request::get(url.clone()).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "served on both origins");
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "image/png"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(
                &body[..8],
                &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn uploads_require_a_token_and_reject_non_images() {
        let (state, root) = state_with_store("guards");
        let app = crate::routes::app_router(state);

        let response = app
            .clone()
            .oneshot(upload_request(
                &format!(r#"{{"data":"data:image/png;base64,{PNG_B64}"}}"#),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(upload_request(
                r#"{"data":"data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="}"#,
                "?t=tok_test",
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "SVG is refused: it is a document, not a picture"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unknown_asset_names_are_404_not_a_file_read() {
        let (state, root) = state_with_store("404");
        let app = crate::routes::app_router(state);
        for name in ["nope.png", "../settings.json", "0123456789abcdef.svg"] {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!("/assets/{name}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
