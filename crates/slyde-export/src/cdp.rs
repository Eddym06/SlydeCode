//! Batch PNG rendering over the DevTools protocol.
//!
//! The CLI (`--screenshot`) can only capture one URL per process, so exporting a
//! deck meant launching Chrome once per slide: ~1–2 s each, and a 50-slide deck
//! took over a minute. This module keeps **one** Chrome alive and drives it over
//! CDP: create a target, then per slide resize the viewport, navigate, wait for
//! the document to settle and capture. It also renders at 2x by default, which is
//! what makes the PNGs usable on a retina projector instead of soft 1280px.
//!
//! The module stays transport-agnostic like the rest of this crate: it takes a
//! browser path, URLs, an output directory and a scale factor.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

use crate::{ExportError, ExportProgress, SlideTarget, png_dimensions};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(25);
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(20);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(15);
/// One slide, end to end: prepare, navigate, settle, capture.
const SLIDE_TIMEOUT: Duration = Duration::from_secs(45);
/// One protocol command. A response that never arrives is a dead session.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// Script that measures the slide's text and hides it for the next capture.
const TEXT_EXTRACT_JS: &str = include_str!("text_extract.js");

/// One rendered slide: the PNG on disk and, when text was extracted, the JSON
/// description of every text block (see `text_extract.js`).
#[derive(Debug, Clone)]
pub struct RenderedSlide {
    /// The screenshot. When `text_json` is set the text is hidden in it, so it is
    /// a clean plate meant to sit under real text boxes.
    pub png: PathBuf,
    /// JSON array of text blocks, or `None` when nothing could be extracted.
    pub text_json: Option<String>,
}

/// Renders every slide with a single Chrome session.
pub async fn export_png_batch(
    chrome: &Path,
    slides: &[SlideTarget],
    aspect_ratio: slyde_core::AspectRatio,
    dir: &Path,
    scale: u32,
    progress: Option<&tokio::sync::mpsc::UnboundedSender<ExportProgress>>,
) -> Result<Vec<PathBuf>, ExportError> {
    let rendered = render_batch(chrome, slides, aspect_ratio, dir, scale, progress, false).await?;
    Ok(rendered.into_iter().map(|slide| slide.png).collect())
}

/// Like [`export_png_batch`], but also reads each slide's text and returns the
/// screenshot with that text hidden. Extraction is best effort: a slide whose
/// text cannot be read comes back with `text_json: None` and a normal picture.
pub async fn export_slides_with_text(
    chrome: &Path,
    slides: &[SlideTarget],
    aspect_ratio: slyde_core::AspectRatio,
    dir: &Path,
    scale: u32,
    progress: Option<&tokio::sync::mpsc::UnboundedSender<ExportProgress>>,
) -> Result<Vec<RenderedSlide>, ExportError> {
    render_batch(chrome, slides, aspect_ratio, dir, scale, progress, true).await
}

async fn render_batch(
    chrome: &Path,
    slides: &[SlideTarget],
    aspect_ratio: slyde_core::AspectRatio,
    dir: &Path,
    scale: u32,
    progress: Option<&tokio::sync::mpsc::UnboundedSender<ExportProgress>>,
    extract_text: bool,
) -> Result<Vec<RenderedSlide>, ExportError> {
    if slides.is_empty() {
        return Err(ExportError::EmptyDeck);
    }

    let dir = std::path::absolute(dir)?;
    std::fs::create_dir_all(&dir)?;
    let (width, height) = png_dimensions(aspect_ratio);
    let scale = scale.clamp(1, 3);
    let profile = crate::export_profile_dir("cdp");

    let outcome = async {
        let mut chrome = tokio::process::Command::new(chrome);
        chrome
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--hide-scrollbars")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = chrome.spawn()?;
        let port = wait_for_port(&profile).await?;
        let ws_url = browser_ws_url(port).await?;
        let mut session = Session::connect(&ws_url).await?;

        let total = slides.len();
        let mut written = Vec::with_capacity(total);
        if let Some(tx) = progress {
            let _ = tx.send(ExportProgress {
                completed: 0,
                total,
            });
        }

        for (index, slide) in slides.iter().enumerate() {
            // A wedged DevTools session must not hang the export: every slide has
            // a hard budget, and the caller falls back to the CLI path on error.
            let (png, text_json) = tokio::time::timeout(SLIDE_TIMEOUT, async {
                session
                    .prepare(&format!(
                        r#"{{"width":{width},"height":{height},"deviceScaleFactor":{scale},"mobile":false}}"#
                    ))
                    .await?;
                session
                    .capture(&slide.url, NAVIGATION_TIMEOUT, SETTLE_TIMEOUT, extract_text)
                    .await
            })
            .await
            .map_err(|_| ExportError::Chrome {
                status: "timeout".into(),
                details: format!("slide {} did not render in {SLIDE_TIMEOUT:?}", index + 1),
            })??;

            let file = dir.join(format!("{:02}-{}.png", index + 1, safe_id(&slide.id)));
            tokio::fs::write(&file, png).await?;
            written.push(RenderedSlide {
                png: file,
                text_json,
            });
            if let Some(tx) = progress {
                let _ = tx.send(ExportProgress {
                    completed: index + 1,
                    total,
                });
            }
        }

        let _ = session.close().await;
        let _ = child.kill().await;
        Ok::<_, ExportError>(written)
    }
    .await;

    let _ = std::fs::remove_dir_all(&profile);
    outcome
}

/// Chrome writes the port it picked into `<profile>/DevToolsActivePort`.
async fn wait_for_port(profile: &Path) -> Result<u16, ExportError> {
    let path = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(raw) = tokio::fs::read_to_string(&path).await
            && let Some(port) = raw.lines().next().and_then(|line| line.trim().parse().ok())
        {
            return Ok(port);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ExportError::Chrome {
        status: "startup".into(),
        details: format!("chrome did not open a debugging port in {STARTUP_TIMEOUT:?}"),
    })
}

/// Minimal HTTP GET for the loopback DevTools endpoint (`/json/version`).
async fn browser_ws_url(port: u16) -> Result<String, ExportError> {
    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| ExportError::Chrome {
            status: "startup".into(),
            details: format!("could not reach the DevTools endpoint: {error}"),
        })?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(io_error)?;
    let mut raw = Vec::new();
    // `Connection: close` is a request, not a promise: read what arrives within a
    // short budget instead of waiting for EOF that may never come.
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw)).await;
    match read {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return Err(io_error(error)),
        Err(_) if raw.is_empty() => {
            return Err(ExportError::Chrome {
                status: "startup".into(),
                details: "the DevTools endpoint did not answer".into(),
            });
        }
        Err(_) => {}
    }

    let text = String::from_utf8_lossy(&raw);
    let body_start = text.find("\r\n\r\n").map(|index| index + 4).unwrap_or(0);
    let payload: Value =
        serde_json::from_str(&text[body_start..]).map_err(|error| ExportError::Chrome {
            status: "startup".into(),
            details: format!("the DevTools endpoint answered with something else: {error}"),
        })?;
    payload
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ExportError::Chrome {
            status: "startup".into(),
            details: "the DevTools endpoint exposed no browser target".into(),
        })
}

fn io_error(error: std::io::Error) -> ExportError {
    ExportError::Io(error)
}

fn safe_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// One CDP connection with a single flattened page session.
struct Session {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
    session_id: String,
}

impl Session {
    async fn connect(url: &str) -> Result<Self, ExportError> {
        let (socket, _response) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| ExportError::Chrome {
                status: "cdp".into(),
                details: format!("could not open the DevTools socket: {error}"),
            })?;
        let mut session = Self {
            socket,
            next_id: 0,
            session_id: String::new(),
        };

        let target = session
            .send("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| ExportError::Chrome {
                status: "cdp".into(),
                details: "Target.createTarget returned no targetId".into(),
            })?
            .to_string();

        let attached = session
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        session.session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| ExportError::Chrome {
                status: "cdp".into(),
                details: "Target.attachToTarget returned no sessionId".into(),
            })?
            .to_string();

        session.send("Page.enable", json!({}), Some(&())).await?;
        session.send("Page.enable", json!({}), None).await.ok();
        Ok(session)
    }

    /// Sets the viewport for the next capture (CSS pixels plus the device scale).
    async fn prepare(&mut self, params: &str) -> Result<(), ExportError> {
        let params: Value = serde_json::from_str(params).map_err(|error| ExportError::Chrome {
            status: "cdp".into(),
            details: format!("invalid metrics: {error}"),
        })?;
        self.send("Emulation.setDeviceMetricsOverride", params, Some(&()))
            .await?;
        Ok(())
    }

    /// Navigates to `url` and captures the visible viewport as PNG bytes. With
    /// `extract_text` it first reads the slide's text and hides it, so the
    /// screenshot is a clean plate.
    async fn capture(
        &mut self,
        url: &str,
        navigation_timeout: Duration,
        settle_timeout: Duration,
        extract_text: bool,
    ) -> Result<(Vec<u8>, Option<String>), ExportError> {
        self.send("Page.navigate", json!({ "url": url }), Some(&()))
            .await?;

        // Wait for the load event through a promise: no CDP event plumbing, and
        // the promise cannot be missed the way a one-shot event listener can.
        self.wait_for_js(
            "new Promise((resolve) => { \
               if (document.readyState === 'complete') resolve(1); \
               else window.addEventListener('load', () => resolve(1), { once: true }); \
             })",
            navigation_timeout,
        )
        .await?;

        // Fonts and the technical renderers (KaTeX, highlight.js, Mermaid) finish
        // after load; the sandbox flips `data-slyde-ready` when they are done.
        let ready = format!(
            "document.fonts.ready.then(() => new Promise((resolve) => {{ \
               var started = Date.now(); \
               (function poll() {{ \
                 if (document.documentElement.getAttribute('data-slyde-ready') === '1') return resolve(1); \
                 if (Date.now() - started > {}) return resolve(0); \
                 setTimeout(poll, 50); \
               }})(); \
             }}))",
            settle_timeout.as_millis()
        );
        self.wait_for_js(&ready, settle_timeout + Duration::from_secs(2))
            .await?;

        let text_json = if extract_text {
            self.extract_text().await
        } else {
            None
        };

        let shot = self
            .send(
                "Page.captureScreenshot",
                json!({ "format": "png", "fromSurface": true, "captureBeyondViewport": false }),
                Some(&()),
            )
            .await?;
        let encoded =
            shot.get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| ExportError::Chrome {
                    status: "cdp".into(),
                    details: "Page.captureScreenshot returned no data".into(),
                })?;
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .map_err(|error| ExportError::Chrome {
                status: "cdp".into(),
                details: format!("the screenshot was not valid base64: {error}"),
            })?;
        Ok((png, text_json))
    }

    /// Runs the text extraction script. Any failure means "no editable text for
    /// this slide", never a failed export.
    async fn extract_text(&mut self) -> Option<String> {
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": TEXT_EXTRACT_JS,
                    "returnByValue": true
                }),
                Some(&()),
            )
            .await
            .ok()?;
        if result.get("exceptionDetails").is_some() {
            return None;
        }
        result
            .get("result")?
            .get("value")?
            .as_str()
            .map(str::to_string)
    }

    async fn wait_for_js(
        &mut self,
        expression: &str,
        timeout: Duration,
    ) -> Result<(), ExportError> {
        let started = Instant::now();
        loop {
            let result = self
                .send(
                    "Runtime.evaluate",
                    json!({
                        "expression": expression,
                        "awaitPromise": true,
                        "returnByValue": true
                    }),
                    Some(&()),
                )
                .await?;
            if result.get("exceptionDetails").is_none() {
                return Ok(());
            }
            if started.elapsed() > timeout {
                return Err(ExportError::Chrome {
                    status: "timeout".into(),
                    details: "the slide never finished rendering".into(),
                });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn close(&mut self) -> Result<(), ExportError> {
        self.send("Browser.close", json!({}), None).await.ok();
        self.socket.close(None).await.ok();
        Ok(())
    }

    /// Sends one command and resolves with its `result` object.
    ///
    /// `target` is `Some` for page-session commands and `None` for browser-level
    /// ones (`Target.*`, `Browser.close`).
    async fn send(
        &mut self,
        method: &str,
        params: Value,
        target: Option<&()>,
    ) -> Result<Value, ExportError> {
        self.next_id += 1;
        let id = self.next_id;
        let mut payload = json!({ "id": id, "method": method, "params": params });
        if target.is_some() && !self.session_id.is_empty() {
            payload["sessionId"] = json!(self.session_id);
        }
        self.socket
            .send(Message::Text(payload.to_string().into()))
            .await
            .map_err(|error| ExportError::Chrome {
                status: "cdp".into(),
                details: format!("could not send {method}: {error}"),
            })?;

        // Events are interleaved with responses; skip anything without our id.
        let round_trip = async {
            loop {
                let Some(message) = self.socket.next().await else {
                    return Err(ExportError::Chrome {
                        status: "cdp".into(),
                        details: format!("the DevTools socket closed during {method}"),
                    });
                };
                let message = message.map_err(|error| ExportError::Chrome {
                    status: "cdp".into(),
                    details: format!("DevTools protocol error during {method}: {error}"),
                })?;
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    Message::Close(_) => {
                        return Err(ExportError::Chrome {
                            status: "cdp".into(),
                            details: format!("the DevTools socket closed during {method}"),
                        });
                    }
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    return Err(ExportError::Chrome {
                        status: "cdp".into(),
                        details: format!("{method} failed: {error}"),
                    });
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
        };
        match tokio::time::timeout(COMMAND_TIMEOUT, round_trip).await {
            Ok(result) => result,
            Err(_) => Err(ExportError::Chrome {
                status: "timeout".into(),
                details: format!("{method} got no answer in {COMMAND_TIMEOUT:?}"),
            }),
        }
    }
}
