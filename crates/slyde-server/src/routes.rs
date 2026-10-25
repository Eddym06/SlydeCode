use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use slyde_core::{AspectRatio, CoreError, ProjectFile, Theme};
use tower_http::services::ServeDir;

use crate::agent::{self, AgentCommand};
use crate::sandbox;
use crate::state::{AppState, AuthQuery, ServerEvent, UiCommand, is_authorized, origin_allowed};

/// Ceiling for an uploaded `.slyde` file. Core limits already cap slides and
/// per-slide code; this stops an oversized body before it is parsed.
pub const MAX_DECK_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default, Deserialize)]
pub struct SandboxQuery {
    /// `static=1` renders the slide fully revealed (thumbnails, export).
    #[serde(default, rename = "static")]
    pub static_mode: Option<String>,
    /// `edit=1` enables the studio selection and inline-editing layer.
    #[serde(default)]
    pub edit: Option<String>,
    #[serde(default)]
    pub t: Option<String>,
}

async fn no_cache_header(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    response
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
        .route("/api/export", post(crate::export_api::api_export))
        .route("/api/deck", get(api_deck_download))
        .route(
            "/api/deck/open",
            post(api_deck_open).layer(DefaultBodyLimit::max(MAX_DECK_UPLOAD_BYTES)),
        )
        .route(
            "/api/copilot/config",
            get(crate::copilot::routes::api_config),
        )
        .route(
            "/api/copilot/models",
            get(crate::copilot::routes::api_models),
        )
        .route("/api/copilot/key", post(crate::copilot::routes::api_key))
        .route(
            "/api/copilot/chat",
            post(crate::copilot::routes::api_chat).layer(DefaultBodyLimit::max(
                crate::copilot::routes::MAX_CHAT_BODY_BYTES,
            )),
        )
        .route("/ws", get(crate::ws::upgrade))
        .nest_service("/static", static_dir)
        .layer(axum::middleware::from_fn(no_cache_header))
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
        Ok(html) => ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response(),
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
) -> Result<Option<ServerEvent>, CoreError> {
    let mut core = state.core.write().await;
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
        UiCommand::AddSlide { template, index } => {
            let code = crate::starter::default_code(&template);
            let target_index = index.unwrap_or_else(|| core.presentation().slides.len());
            core.upsert_slide(
                target_index,
                None,
                code,
                crate::starter::template_type(&template),
                crate::starter::default_animation_for(&template),
            )
            .map(|_| None)
        }
        UiCommand::DuplicateSlide { slide_id } => {
            let (current_idx, slide) = core
                .presentation()
                .slides
                .iter()
                .enumerate()
                .find(|(_, s)| s.id == slide_id)
                .map(|(idx, s)| (idx, s.clone()))
                .ok_or_else(|| CoreError::SlideNotFound(slide_id))?;
            core.upsert_slide(
                current_idx + 1,
                None,
                slide.code,
                slide.template,
                slide.animation,
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
        UiCommand::MutateElement {
            element_id,
            text,
            style,
            html,
        } => core
            .mutate_element(&element_id, text, style, html)
            .map(|_| None),
        UiCommand::SetAnimation {
            slide_id,
            animation,
        } => core.set_animation(&slide_id, animation).map(|_| None),
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

    match agent::run(&state, command).await {
        Ok(outcome) => Json(json!({
            "ok": true,
            "created": outcome.created,
            "upsert": outcome.upsert,
            "sandbox_origin": state.sandbox_origin,
            "view": state.view().await
        }))
        .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// Downloads the current deck as a `.slyde` file.
///
/// Served with `Content-Disposition: attachment`, so the browser writes it to
/// the downloads folder; the app never needs a native save dialog.
async fn api_deck_download(
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

    let deck = state.core.read().await.presentation().clone();
    let body = match ProjectFile::new(deck.clone()).to_json() {
        Ok(body) => body,
        Err(err) => return bad_request(&err.to_string()),
    };

    let name = crate::storage::file_name_for(&deck.title);
    // RFC 5987: the plain `filename` is ASCII-only, the UTF-8 form carries the
    // real (possibly accented) name. Browsers prefer the latter.
    let disposition = format!(
        "attachment; filename=\"deck.{extension}\"; filename*=UTF-8''{encoded}",
        extension = slyde_core::FILE_EXTENSION,
        encoded = sandbox::percent_encode(&name)
    );

    let mut response = body.into_response();
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        out.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Opens a `.slyde` file uploaded from the UI, replacing the current deck.
///
/// The upload is validated by the core (`ProjectFile::from_json`) before it
/// touches the state, so a corrupt or hostile file leaves the open deck alone.
async fn api_deck_open(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
    body: Bytes,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return forbidden();
    }
    if !is_authorized(&state, &headers, &auth) {
        return unauthorized();
    }

    let raw = match std::str::from_utf8(&body) {
        Ok(raw) => raw,
        Err(_) => return bad_request("deck file must be UTF-8 text"),
    };
    let file = match ProjectFile::from_json(raw) {
        Ok(file) => file,
        Err(err) => return bad_request(&err.to_string()),
    };

    {
        let mut core = state.core.write().await;
        if let Err(err) = core.replace(file.presentation) {
            return bad_request(&err.to_string());
        }
    }

    state.publish_state().await;
    Json(json!({ "ok": true, "view": state.view().await })).into_response()
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": message })),
    )
        .into_response()
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

async fn sandbox_slide(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<SandboxQuery>,
) -> Response {
    let core = state.core.read().await;
    let deck = core.presentation();
    let Some(slide) = deck.slide(&id) else {
        return (StatusCode::NOT_FOUND, "slide not found").into_response();
    };

    let is_static = query.static_mode.as_deref() == Some("1");
    let is_edit = query.edit.as_deref() == Some("1");
    let html = sandbox::render_document(deck, slide, is_static, is_edit);

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

    use crate::state::{AgentAction, AgentPhase};

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

    type ActivityRow = (AgentPhase, AgentAction, Option<String>, Option<usize>);

    fn agent_activity(
        events: &mut tokio::sync::broadcast::Receiver<ServerEvent>,
    ) -> Vec<ActivityRow> {
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let ServerEvent::AgentActivity {
                phase,
                action,
                slide_id,
                slide_index,
            } = event
            {
                seen.push((phase, action, slide_id, slide_index));
            }
        }
        seen
    }

    #[tokio::test]
    async fn agent_calls_announce_writing_and_done() {
        let state = test_state();
        let mut events = state.events.subscribe();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/agent?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"action":"upsert_slide","index":0,"slide_id":"sld_a","html_code":"<h1>New</h1>"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let seen = agent_activity(&mut events);
        assert_eq!(seen.len(), 2, "one active event and one done event");
        assert_eq!(seen[0].0, AgentPhase::Writing);
        assert_eq!(seen[0].1, AgentAction::UpsertSlide);
        assert_eq!(seen[0].2.as_deref(), Some("sld_a"));
        assert_eq!(seen[0].3, Some(0));
        assert_eq!(seen[1].0, AgentPhase::Done);
        assert_eq!(seen[1].2.as_deref(), Some("sld_a"));
    }

    #[tokio::test]
    async fn a_failed_agent_call_announces_an_error() {
        let state = test_state();
        let mut events = state.events.subscribe();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/agent?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"action":"upsert_slide","index":0,"html_code":""}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let seen = agent_activity(&mut events);
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, AgentPhase::Writing);
        assert_eq!(seen[1].0, AgentPhase::Error);
    }

    #[tokio::test]
    async fn ui_commands_do_not_claim_agent_activity() {
        let state = test_state();
        let mut events = state.events.subscribe();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/command?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"action":"set_notes","slide_id":"sld_a","notes":"Mine"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(agent_activity(&mut events).is_empty());
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
    async fn duplicate_slide_clones_content_and_inserts_after() {
        let state = test_state();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/command?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"duplicate_slide","slide_id":"sld_a"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let core = state.core.read().await;
        assert_eq!(core.presentation().slides.len(), 2);
        assert_ne!(core.presentation().slides[0].id, core.presentation().slides[1].id);
        assert_eq!(core.presentation().slides[0].code, core.presentation().slides[1].code);
    }

    #[tokio::test]
    async fn add_slide_supports_explicit_index() {
        let state = test_state();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/command?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"add_slide","template":"bullets","index":0}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let core = state.core.read().await;
        assert_eq!(core.presentation().slides.len(), 2);
        assert_eq!(core.presentation().slides[0].template, slyde_core::TemplateType::Bullet);
    }

    #[tokio::test]
    async fn deck_download_returns_an_attachment_with_the_project() {
        let app = app_router(test_state());
        let res = app
            .oneshot(
                Request::get("/api/deck?t=tok_test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let disposition = res
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(disposition.starts_with("attachment"));
        assert!(disposition.contains("filename*=UTF-8''deck.slyde"));

        let json = body_json(res).await;
        assert_eq!(json["format"], "slydecore");
        assert_eq!(json["presentation"]["slides"][0]["id"], "sld_a");
    }

    #[tokio::test]
    async fn deck_open_replaces_the_deck() {
        let state = test_state();
        let app = app_router(state.clone());

        let mut incoming = PresentationState::new("Opened", Theme::paper(), AspectRatio::R4x3);
        incoming
            .upsert_slide(
                0,
                Some("sld_x".into()),
                "<h1>Opened</h1>".into(),
                TemplateType::Hero,
                vec![],
            )
            .unwrap();
        let body = ProjectFile::new(incoming.presentation().clone())
            .to_json()
            .unwrap();

        let res = app
            .oneshot(
                Request::post("/api/deck/open?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json = body_json(res).await;
        assert_eq!(json["view"]["presentation"]["title"], "Opened");

        let core = state.core.read().await;
        assert_eq!(core.presentation().aspect_ratio, AspectRatio::R4x3);
        assert_eq!(core.presentation().slides[0].id, "sld_x");
        assert!(!core.can_undo(), "opening a file starts a fresh history");
    }

    #[tokio::test]
    async fn deck_open_rejects_a_corrupt_file_and_keeps_the_deck() {
        let state = test_state();
        let app = app_router(state.clone());
        let res = app
            .oneshot(
                Request::post("/api/deck/open?t=tok_test")
                    .header("content-type", "application/json")
                    .body(Body::from("{not a deck"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.core.read().await.presentation().title, "Deck");
    }

    #[tokio::test]
    async fn deck_endpoints_require_the_token() {
        let app = app_router(test_state());
        let res = app
            .oneshot(Request::get("/api/deck").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
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
