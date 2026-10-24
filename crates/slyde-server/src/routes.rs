use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use slyde_core::{AspectRatio, CoreError, Theme};
use tower_http::services::ServeDir;

use crate::agent::{self, AgentCommand};
use crate::sandbox;
use crate::state::{AppState, AuthQuery, ServerEvent, UiCommand, is_authorized, origin_allowed};

#[derive(Debug, Default, Deserialize)]
pub struct SandboxQuery {
    /// `static=1` renders the slide fully revealed (thumbnails, export).
    #[serde(default, rename = "static")]
    pub static_mode: Option<String>,
    #[serde(default)]
    pub t: Option<String>,
}

pub fn app_router(state: Arc<AppState>) -> Router {
    let static_dir = ServeDir::new(state.ui_dir.join("static"));
    Router::new()
        .route("/", get(index_page))
        .route("/present", get(present_page))
        .route("/health", get(health))
        .route("/favicon.ico", get(favicon))
        .route("/api/state", get(api_state))
        .route("/api/command", post(api_command))
        .route("/api/agent", post(api_agent))
        .route("/ws", get(crate::ws::upgrade))
        .nest_service("/static", static_dir)
        .with_state(state)
}

pub fn sandbox_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/print", get(print_document))
        .route("/s/{id}", get(sandbox_slide))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let revision = state.core.read().await.revision();
    Json(json!({ "status": "ok", "app": "slyde", "revision": revision }))
}

async fn favicon() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn index_page(State(state): State<Arc<AppState>>) -> Response {
    serve_ui_file(&state, "index.html").await
}

async fn present_page(State(state): State<Arc<AppState>>) -> Response {
    serve_ui_file(&state, "present.html").await
}

async fn serve_ui_file(state: &AppState, file: &str) -> Response {
    match tokio::fs::read_to_string(state.ui_dir.join(file)).await {
        Ok(html) => (
            [(header::CACHE_CONTROL, "no-store")],
            Html(html),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(missing_ui_page(&state.ui_dir, &err.to_string())),
        )
            .into_response(),
    }
}

fn missing_ui_page(ui_dir: &std::path::Path, error: &str) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>SlydeCode</title>\
         <body style=\"font-family:system-ui;background:#17161A;color:#F7F4EF;padding:48px\">\
         <h1>UI not found</h1><p>Expected the interface at <code>{}</code>.</p>\
         <p style=\"color:#FF8265\">{}</p>\
         <p>Run from the repository root, or set <code>SLYDE_UI_DIR</code>.</p></body>",
        ui_dir.display(),
        sandbox::escape_html(error)
    )
}

async fn api_state(
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
    Json(json!({
        "ok": true,
        "sandbox_origin": state.sandbox_origin,
        "view": state.view().await
    }))
    .into_response()
}

async fn api_command(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(command): Json<UiCommand>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }

    match apply_command(&state, command).await {
        Ok(event) => {
            if let Some(event) = event {
                state.broadcast(event);
            }
            state.publish_state().await;
            Json(json!({ "ok": true, "view": state.view().await })).into_response()
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn apply_command(
    state: &AppState,
    command: UiCommand,
) -> Result<Option<ServerEvent>, CoreError> {    let mut core = state.core.write().await;
    match command {
        UiCommand::SetTitle { title } => core.set_title(title).map(|_| None),
        UiCommand::SetTheme { theme_id } => {
            let theme = Theme::resolve(&theme_id)
                .ok_or_else(|| CoreError::InvalidTheme(format!("unknown theme: {theme_id}")))?;
            core.set_theme(theme).map(|_| None)
        }
        UiCommand::SetAspectRatio { ratio } => {
            let parsed = AspectRatio::parse(&ratio)
                .ok_or_else(|| CoreError::InvalidAspectRatio(ratio.clone()))?;
            core.set_aspect_ratio(parsed).map(|_| None)
        }
        UiCommand::SetNotes { slide_id, notes } => core.set_notes(&slide_id, notes).map(|_| None),
        UiCommand::AddSlide { template } => {
            let code = crate::starter::default_code(&template);
            let index = core.presentation().slides.len();
            core.upsert_slide(
                index,
                None,
                code,
                crate::starter::template_type(&template),
                crate::starter::default_animation_for(&template),
            )
            .map(|_| None)
        }
        UiCommand::MoveSlide { from, to } => core.move_slide(from, to).map(|_| None),
        UiCommand::RemoveSlide { slide_id } => core.remove_slide(&slide_id).map(|_| None),
        UiCommand::Undo => core.undo().map(|_| None),
        UiCommand::Redo => core.redo().map(|_| None),
        UiCommand::TriggerAnimation {
            slide_id,
            step_index,
        } => {
            let step = core.animation_step(&slide_id, step_index)?;
            Ok(Some(ServerEvent::Animation {
                slide_id,
                step_index,
                step,
            }))
        }
    }
}

async fn api_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    Json(command): Json<AgentCommand>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }

    match agent::handle(&state, command).await {
        Ok(outcome) => {
            if let Some(event) = outcome.animation.clone() {
                state.broadcast(event);
            }
            state.publish_state().await;
            Json(json!({
                "ok": true,
                "created": outcome.created,
                "upsert": outcome.upsert,
                "sandbox_origin": state.sandbox_origin,
                "view": state.view().await
            }))
            .into_response()
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn print_document(
    State(state): State<Arc<AppState>>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !is_authorized(&state, &HeaderMap::new(), &auth) {
        return unauthorized();
    }

    let html = {
        let core = state.core.read().await;
        sandbox::render_print_document(core.presentation(), &state.sandbox_origin, &state.token)
    };

    let mut response = Html(html).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn sandbox_slide(    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<SandboxQuery>,
) -> Response {
    let core = state.core.read().await;
    let deck = core.presentation();
    let Some(slide) = deck.slide(&id) else {
        return (StatusCode::NOT_FOUND, "slide not found").into_response();
    };

    let is_static = query.static_mode.as_deref() == Some("1");
    let html = sandbox::render_document(deck, slide, is_static);

    let mut response = Html(html).into_response();
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

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "error": "missing or invalid token" })),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "ok": false, "error": "origin not allowed" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use slyde_core::{PresentationState, TemplateType};
    use tower::util::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let mut core = PresentationState::new("Deck", Theme::studio(), AspectRatio::R16x9);
        core.upsert_slide(
            0,
            Some("sld_a".into()),
            "<h1 class=\"title\">Hello</h1>".into(),
            TemplateType::Hero,
            vec![],
        )
        .unwrap();
        AppState::new(
            core,
            "tok_test".into(),
            std::path::PathBuf::from("ui"),
            vec!["http://127.0.0.1:4444".into()],
            "http://127.0.0.1:4445".into(),
        )
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn api_state_requires_a_valid_token() {
        let app = app_router(test_state());

        let res = app
            .clone()
            .oneshot(Request::get("/api/state").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app
            .clone()
            .oneshot(
                Request::get("/api/state?t=wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app
            .oneshot(
                Request::get("/api/state?t=tok_test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json = body_json(res).await;
        assert_eq!(json["view"]["presentation"]["title"], "Deck");
        assert_eq!(json["view"]["revision"], 1);
    }

    #[tokio::test]
    async fn foreign_origin_is_rejected_even_with_a_token() {
        let app = app_router(test_state());
        let res = app
            .oneshot(
                Request::get("/api/state?t=tok_test")
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn commands_mutate_state_and_return_the_new_view() {
        let state = test_state();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/command?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"set_title","title":"Renamed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json = body_json(res).await;
        assert_eq!(json["view"]["presentation"]["title"], "Renamed");
        assert_eq!(state.core.read().await.presentation().title, "Renamed");
    }

    #[tokio::test]
    async fn invalid_command_reports_a_readable_error() {
        let app = app_router(test_state());
        let res = app
            .oneshot(
                Request::post("/api/command?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"set_theme","theme_id":"neon"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let json = body_json(res).await;
        assert!(json["error"].as_str().unwrap().contains("neon"));
    }

    #[tokio::test]
    async fn health_needs_no_token() {
        let app = app_router(test_state());
        let res = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json = body_json(res).await;
        assert_eq!(json["app"], "slyde");
    }

    #[tokio::test]
    async fn add_slide_inserts_a_starter_with_animation() {
        let state = test_state();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/command?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"add_slide","template":"hero"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let core = state.core.read().await;
        assert_eq!(core.presentation().slides.len(), 2);
        let added = &core.presentation().slides[1];
        assert!(added.code.contains("class=\"title\""));
        assert_eq!(added.animation.len(), 2);
    }

    #[tokio::test]
    async fn sandbox_serves_the_slide_with_csp() {
        let app = sandbox_router(test_state());
        let res = app
            .oneshot(Request::get("/s/sld_a").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().contains_key("content-security-policy"));
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains("Hello"));
    }

    #[tokio::test]
    async fn sandbox_returns_404_for_unknown_slide() {
        let app = sandbox_router(test_state());
        let res = app
            .oneshot(Request::get("/s/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
