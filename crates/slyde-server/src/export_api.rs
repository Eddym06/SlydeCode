//! `POST /api/export`: export the open deck for the user, without leaving the
//! studio.
//!
//! The heavy lifting lives in `slyde-export`; this module owns the HTTP shape,
//! progress events and the packaging the browser downloads:
//!
//! - `format=pdf` returns one PDF.
//! - `format=png` returns a ZIP with one PNG per slide (a browser will not let
//!   a page trigger N downloads, and a folder of loose files is worse UX).

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use slyde_export::{SlideTarget, export_pdf, export_png};

use crate::state::{AppState, AuthQuery, ServerEvent, is_authorized, origin_allowed};
use crate::{discover_browser, now_ms, sandbox, storage};

/// Distinguishes export jobs that start in the same millisecond.
/// PNGs are rendered at 2x so they stay sharp on a retina projector.
const PNG_SCALE: u32 = 2;
static EXPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    pub t: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Pdf,
    Png,
    /// PowerPoint: one rendered picture per slide, wrapped in a real `.pptx`.
    Pptx,
    /// Standalone self-contained HTML deck.
    Html,
}

impl ExportFormat {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("pdf") => Some(Self::Pdf),
            Some("png") | Some("images") => Some(Self::Png),
            Some("pptx") | Some("powerpoint") => Some(Self::Pptx),
            Some("html") | Some("web") => Some(Self::Html),
            _ => None,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Pptx => "pptx",
            Self::Html => "html",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "zip",
            Self::Pptx => "pptx",
            Self::Html => "html",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Png => "application/zip",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Html => "text/html; charset=utf-8",
        }
    }
}

pub async fn api_export(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return api_error(StatusCode::FORBIDDEN, "forbidden", "origin not allowed");
    }
    let auth = AuthQuery { t: query.t.clone() };
    if !is_authorized(&state, &headers, &auth) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid token",
        );
    }

    let Some(format) = ExportFormat::parse(query.format.as_deref()) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_format",
            "format must be pdf, png, pptx or html",
        );
    };

    if format == ExportFormat::Html {
        let (deck_copy, deck_title) = {
            let core = state.core.read().await;
            (
                core.presentation().clone(),
                core.presentation().title.clone(),
            )
        };
        let html_string = sandbox::render_standalone_presentation(&deck_copy);
        let filename = format!("{}.html", storage::file_stem_for(&deck_title));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")).unwrap(),
        );
        let _ = state.events.send(ServerEvent::ExportProgress {
            format: "html".to_string(),
            completed: 1,
            total: 1,
        });
        return (StatusCode::OK, headers, html_string.into_bytes()).into_response();
    }

    // Snapshot the deck, then drop the lock: an export must never block the
    // editor (or an AI agent) while Chrome renders.
    let (title, aspect_ratio, slide_ids, slide_notes) = {
        let core = state.core.read().await;
        let deck = core.presentation();
        (
            deck.title.clone(),
            deck.aspect_ratio,
            deck.slides
                .iter()
                .map(|slide| slide.id.clone())
                .collect::<Vec<_>>(),
            deck.slides
                .iter()
                .map(|slide| slide.notes.clone())
                .collect::<Vec<_>>(),
        )
    };

    if slide_ids.is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "empty_deck",
            "add a slide before exporting",
        );
    }

    let browser = match discover_browser() {
        Ok(browser) => browser,
        Err(_) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "browser_not_found",
                "Google Chrome was not found; install it and try again",
            );
        }
    };

    let dir = storage::exports_dir().join(format!(
        "{}-{}-{}-{}",
        now_ms(),
        std::process::id(),
        EXPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        format.tag()
    ));
    if let Err(err) = tokio::fs::create_dir_all(&dir).await {
        tracing::error!(error = %err, dir = %dir.display(), "could not create export workspace");
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "export_failed",
            "the export could not start",
        );
    }

    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<slyde_export::ExportProgress>();
    let broadcaster = state.clone();
    let format_tag = format.tag().to_string();
    let pump = tokio::spawn(async move {
        while let Some(step) = progress_rx.recv().await {
            broadcaster.broadcast(ServerEvent::ExportProgress {
                format: format_tag.clone(),
                completed: step.completed,
                total: step.total,
            });
        }
    });

    let sandbox_origin = state.sandbox_origin.clone();
    let token = sandbox::percent_encode(&state.token);
    // Slide documents carry their own token: the sandbox origin accepts it and
    // the print page's iframes need it to load at all.
    let slide_token = sandbox::percent_encode(&state.sandbox_token);

    let result = match format {
        ExportFormat::Pdf => {
            let output = dir.join("deck.pdf");
            let url = format!("{sandbox_origin}/print?t={token}");
            export_pdf(&browser.path, &url, &output, Some(&progress_tx))
                .await
                .map(|written| vec![written])
        }
        ExportFormat::Html => unreachable!("html export returns before chrome render"),
        ExportFormat::Png | ExportFormat::Pptx => {
            let targets: Vec<SlideTarget> = slide_ids
                .iter()
                .map(|id| SlideTarget {
                    id: id.clone(),
                    url: format!(
                        "{sandbox_origin}/s/{}?static=1&t={slide_token}",
                        sandbox::percent_encode(id)
                    ),
                })
                .collect();
            // One Chrome session for the whole deck, at 2x. If the DevTools
            // handshake fails on an unusual Chrome build, fall back to the
            // sequential CLI path instead of failing the export.
            match slyde_export::export_png_batch(
                &browser.path,
                &targets,
                aspect_ratio,
                &dir,
                PNG_SCALE,
                Some(&progress_tx),
            )
            .await
            {
                Ok(files) => Ok(files),
                Err(error) => {
                    tracing::warn!(%error, "batch PNG export failed; using one Chrome per slide");
                    export_png(
                        &browser.path,
                        &targets,
                        aspect_ratio,
                        &dir,
                        Some(&progress_tx),
                    )
                    .await
                }
            }
        }
    };

    drop(progress_tx);
    let _ = pump.await;

    let files = match result {
        Ok(files) => files,
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            tracing::error!(error = %err, format = format.tag(), "export failed");
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "export_failed",
                "the export could not finish",
            );
        }
    };

    let packaged = match format {
        ExportFormat::Pdf => match files.first() {
            Some(path) => tokio::fs::read(path).await.map_err(|err| err.to_string()),
            None => Err("chrome produced no PDF".to_string()),
        },
        ExportFormat::Png => {
            let files = files.clone();
            match tokio::task::spawn_blocking(move || bundle_pngs(&files)).await {
                Ok(result) => result,
                Err(err) => Err(format!("packaging task failed: {err}")),
            }
        }
        ExportFormat::Html => unreachable!("html export returns before packaging"),
        ExportFormat::Pptx => {
            let files = files.clone();
            let notes = slide_notes.clone();
            let deck_title = title.clone();
            match tokio::task::spawn_blocking(move || {
                bundle_pptx(&deck_title, &notes, &files, aspect_ratio)
            })
            .await
            {
                Ok(result) => result,
                Err(err) => Err(format!("packaging task failed: {err}")),
            }
        }
    };

    let _ = tokio::fs::remove_dir_all(&dir).await;

    let body = match packaged {
        Ok(body) => body,
        Err(details) => {
            tracing::error!(error = %details, "could not package export");
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "export_failed",
                "the export could not finish",
            );
        }
    };

    attachment(&title, format, body)
}

fn attachment(title: &str, format: ExportFormat, body: Vec<u8>) -> Response {
    let name = format!("{}.{}", storage::file_stem_for(title), format.extension());
    // RFC 5987: the plain `filename` is ASCII-only, the UTF-8 form carries the
    // real (possibly accented) name. Browsers prefer the latter.
    let disposition = format!(
        "attachment; filename=\"deck.{extension}\"; filename*=UTF-8''{encoded}",
        extension = format.extension(),
        encoded = sandbox::percent_encode(&name)
    );

    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.content_type()),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Builds the `.pptx` bundle from the rendered PNGs.
///
/// The deck is already rendered by Chrome, so PowerPoint gets one full-bleed
/// picture per slide: exactly what the presenter showed, no second layout
/// engine and no HTML-to-shape translation to get wrong. Slide notes travel as
/// real notes, so the speaker view still works.
fn bundle_pptx(
    deck_title: &str,
    notes: &[Option<String>],
    files: &[PathBuf],
    aspect_ratio: slyde_core::AspectRatio,
) -> Result<Vec<u8>, String> {
    if files.is_empty() {
        return Err("no slides were rendered".to_string());
    }
    let (width_px, height_px) = slyde_export::png_dimensions(aspect_ratio);
    let mut slides = Vec::with_capacity(files.len());
    for (index, path) in files.iter().enumerate() {
        let png = std::fs::read(path)
            .map_err(|err| format!("could not read {}: {err}", path.display()))?;
        slides.push(slyde_pptx::PptxSlide {
            title: format!("Slide {}", index + 1),
            png,
            notes: notes.get(index).and_then(|value| value.clone()),
        });
    }

    let deck = slyde_pptx::PptxDeck {
        title: deck_title,
        width_px,
        height_px,
        slides: &slides,
    };
    let target = files[0]
        .parent()
        .map(|dir| dir.join("deck.pptx"))
        .unwrap_or_else(|| std::env::temp_dir().join("deck.pptx"));
    slyde_pptx::write_pptx(&deck, &target).map_err(|err| err.to_string())?;
    let bytes = std::fs::read(&target)
        .map_err(|err| format!("could not read {}: {err}", target.display()))?;
    let _ = std::fs::remove_file(&target);
    Ok(bytes)
}
/// PNGs are already compressed; storing them keeps the ZIP step near-instant.
fn bundle_pngs(files: &[PathBuf]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));

    for path in files {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("invalid export file name: {}", path.display()))?;
        let bytes = std::fs::read(path)
            .map_err(|err| format!("could not read {}: {err}", path.display()))?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o644);

        zip.start_file(name, options)
            .map_err(|err| format!("could not start {name}: {err}"))?;
        zip.write_all(&bytes)
            .map_err(|err| format!("could not write {name}: {err}"))?;
    }

    let cursor = zip
        .finish()
        .map_err(|err| format!("could not finish the zip: {err}"))?;
    Ok(cursor.into_inner())
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "ok": false, "code": code, "error": message })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use slyde_core::{AspectRatio, PresentationState, Theme};
    use tower::util::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let mut core = PresentationState::new("Deck", Theme::studio(), AspectRatio::R16x9);
        core.upsert_slide(
            0,
            Some("sld_a".into()),
            "<h1>A</h1>".into(),
            slyde_core::TemplateType::Hero,
            vec![],
        )
        .unwrap();
        AppState::new(
            core,
            "tok_test".into(),
            PathBuf::from("ui"),
            vec!["http://127.0.0.1:4444".into()],
            "http://127.0.0.1:4445".into(),
        )
    }

    #[test]
    fn format_parsing_is_case_insensitive_and_rejects_unknown_values() {
        assert_eq!(ExportFormat::parse(Some("PDF")), Some(ExportFormat::Pdf));
        assert_eq!(ExportFormat::parse(Some("png")), Some(ExportFormat::Png));
        assert_eq!(ExportFormat::parse(Some("images")), Some(ExportFormat::Png));
        assert_eq!(ExportFormat::parse(Some("pptx")), Some(ExportFormat::Pptx));
        assert_eq!(
            ExportFormat::parse(Some("POWERPOINT")),
            Some(ExportFormat::Pptx)
        );
        assert_eq!(ExportFormat::parse(Some("html")), Some(ExportFormat::Html));
        assert_eq!(ExportFormat::parse(Some("WEB")), Some(ExportFormat::Html));
        assert_eq!(ExportFormat::parse(None), None);
    }

    #[tokio::test]
    async fn html_export_returns_standalone_document_without_chrome() {
        use tower::ServiceExt;
        let app = crate::routes::app_router(test_state());
        let res = app
            .oneshot(
                axum::http::Request::post("/api/export?t=tok_test&format=html")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let content_type = res.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(content_type, "text/html; charset=utf-8");
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("slyde-offline-slide"));
        assert!(html.contains("slyde-laser"));
    }

    #[test]
    fn png_bundle_is_a_zip_with_one_entry_per_slide() {
        let dir = std::env::temp_dir().join(format!(
            "slyde-zip-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("01-sld_a.png");
        let second = dir.join("02-sld_b.png");
        std::fs::write(&first, b"png-a").unwrap();
        std::fs::write(&second, b"png-b").unwrap();

        let bytes = bundle_pngs(&[first.clone(), second.clone()]).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        assert!(archive.by_name("01-sld_a.png").is_ok());
        assert!(archive.by_name("02-sld_b.png").is_ok());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn export_requires_a_valid_token() {
        let app = crate::routes::app_router(test_state());
        let res = app
            .oneshot(
                Request::post("/api/export?format=pdf")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_formats_fail_before_chrome_is_considered() {
        let app = crate::routes::app_router(test_state());
        let res = app
            .oneshot(
                Request::post("/api/export?t=tok_test&format=gif")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["code"], "invalid_format");
    }
}
