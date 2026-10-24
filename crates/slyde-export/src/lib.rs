//! PDF and PNG export through headless Chrome.
//!
//! SlydeCode has exactly one renderer, so export uses it too: a headless Chrome
//! instance loads the same slide documents the presenter shows and prints them.
//! No second layout engine means no WYSIWYG drift.
//!
//! PDF is one document with one page per slide (`@page` sized to the deck
//! aspect ratio). PNG is one file per slide at 1x.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use slyde_server::http::get_json;
use slyde_server::{RuntimeInfo, discover_browser, read_runtime};
use thiserror::Error;

const CHROME_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("SlydeCode is not running. Open the app first, then export.")]
    NotRunning,
    #[error("no supported browser found; install Google Chrome or set SLYDE_BROWSER")]
    BrowserNotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("request failed: {0}")]
    Http(#[from] slyde_server::http::HttpError),
    #[error("chrome exited with {status}: {details}")]
    Chrome { status: String, details: String },
    #[error("chrome produced no output at {0}")]
    NoOutput(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Png,
}

impl Format {
    fn parse(raw: &str) -> Result<Self, ExportError> {
        match raw.to_ascii_lowercase().as_str() {
            "pdf" => Ok(Self::Pdf),
            "png" | "images" => Ok(Self::Png),
            other => Err(ExportError::InvalidArgument(format!(
                "unsupported format: {other} (use pdf or png)"
            ))),
        }
    }
}

/// Entry point for `slyde export`.
pub async fn run_cli(args: &[String]) -> Result<(), ExportError> {
    let mut format = Format::Pdf;
    let mut output: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" | "-f" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| ExportError::InvalidArgument("--format needs a value".into()))?;
                format = Format::parse(value)?;
            }
            "--out" | "-o" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| ExportError::InvalidArgument("--out needs a path".into()))?;
                output = Some(PathBuf::from(value));
            }
            other => {
                return Err(ExportError::InvalidArgument(format!(
                    "unknown option: {other}"
                )));
            }
        }
        index += 1;
    }

    let runtime = read_runtime().ok_or(ExportError::NotRunning)?;
    if !slyde_server::probe_health(runtime.port, Duration::from_millis(600)).await {
        return Err(ExportError::NotRunning);
    }

    match format {
        Format::Pdf => {
            let path = output.unwrap_or_else(|| default_pdf_path(&runtime));
            let written = export_pdf(&runtime, &path).await?;
            println!("{}", written.display());
        }
        Format::Png => {
            let dir = output.unwrap_or_else(|| PathBuf::from("exports").join("slides"));
            let files = export_png(&runtime, &dir).await?;
            for file in files {
                println!("{}", file.display());
            }
        }
    }

    Ok(())
}

pub async fn export_pdf(runtime: &RuntimeInfo, output: &Path) -> Result<PathBuf, ExportError> {
    // Chrome resolves `--print-to-pdf` against its own working directory, not
    // always ours, so hand it an absolute path.
    let output = std::path::absolute(output)?;
    let url = format!(
        "http://127.0.0.1:{}/print?t={}",
        runtime.sandbox_port, runtime.token
    );

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&output);

    let profile = export_profile_dir("pdf");
    let browser = discover_browser().map_err(|_| ExportError::BrowserNotFound)?;

    let mut command = tokio::process::Command::new(&browser.path);
    command
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--no-pdf-header-footer")
        .arg("--run-all-compositor-stages-before-draw")
        .arg("--virtual-time-budget=5000")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--print-to-pdf={}", output.display()))
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    run_chrome(command, &output).await?;
    Ok(output)
}

pub async fn export_png(runtime: &RuntimeInfo, dir: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let dir = std::path::absolute(dir)?;
    std::fs::create_dir_all(&dir)?;

    let state = get_json(
        &format!("http://127.0.0.1:{}/api/state?t={}", runtime.port, runtime.token),
        Duration::from_secs(5),
    )
    .await?;

    let slides = state
        .pointer("/view/presentation/slides")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let ratio = state
        .pointer("/view/presentation/aspect_ratio")
        .and_then(|value| value.as_str())
        .unwrap_or("16:9");
    let (width, height) = ratio
        .split_once(':')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .map(|(w, h)| {
            let scale = 1280.0 / w as f64;
            (1280u32, (h as f64 * scale).round() as u32)
        })
        .unwrap_or((1280, 720));

    let browser = discover_browser().map_err(|_| ExportError::BrowserNotFound)?;
    let profile = export_profile_dir("png");
    let mut written = Vec::new();

    for (index, slide) in slides.iter().enumerate() {
        let Some(id) = slide.get("id").and_then(|value| value.as_str()) else {
            continue;
        };
        let file = dir.join(format!("{:02}-{}.png", index + 1, safe_id(id)));
        let _ = std::fs::remove_file(&file);

        let url = format!(
            "http://127.0.0.1:{}/s/{}?static=1",
            runtime.sandbox_port, id
        );

        let mut command = tokio::process::Command::new(&browser.path);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--hide-scrollbars")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--window-size={width},{height}"))
            .arg(format!("--screenshot={}", file.display()))
            .arg(&url)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        run_chrome(command, &file).await?;
        written.push(file);
    }

    Ok(written)
}

async fn run_chrome(
    mut command: tokio::process::Command,
    expected_output: &Path,
) -> Result<(), ExportError> {
    let child = command.spawn()?;
    let output = tokio::time::timeout(CHROME_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ExportError::Chrome {
            status: "timeout".into(),
            details: format!("no result after {}s", CHROME_TIMEOUT.as_secs()),
        })??;

    let exists = expected_output.is_file()
        && std::fs::metadata(expected_output)
            .map(|meta| meta.len() > 0)
            .unwrap_or(false);

    if !exists {
        return Err(ExportError::Chrome {
            status: output.status.to_string(),
            details: String::from_utf8_lossy(&output.stderr).chars().take(600).collect(),
        });
    }

    Ok(())
}

fn export_profile_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slyde-export-{tag}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn safe_id(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

fn default_pdf_path(_runtime: &RuntimeInfo) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from("exports").join(format!("deck-{stamp}.pdf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parsing_accepts_aliases_and_rejects_unknown() {
        assert_eq!(Format::parse("PDF").unwrap(), Format::Pdf);
        assert_eq!(Format::parse("png").unwrap(), Format::Png);
        assert_eq!(Format::parse("images").unwrap(), Format::Png);
        assert!(Format::parse("pptx").is_err());
    }

    #[test]
    fn slide_ids_are_made_filename_safe() {
        assert_eq!(safe_id("sld_abc-123"), "sld_abc-123");
        assert_eq!(safe_id("../../etc/passwd"), "------etc-passwd");
    }

    #[tokio::test]
    async fn cli_requires_a_running_instance() {
        // No runtime file in a clean test env means a clear, actionable error.
        if slyde_server::read_runtime().is_none() {
            let err = run_cli(&[]).await.unwrap_err();
            assert!(matches!(err, ExportError::NotRunning));
        }
    }
}
