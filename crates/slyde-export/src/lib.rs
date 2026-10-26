//! PDF and PNG export through headless Chrome.
//!
//! SlydeCode has exactly one renderer, so export uses it too: a headless Chrome
//! instance loads the same slide documents the presenter shows and prints them.
//! No second layout engine means no WYSIWYG drift.
//!
//! PDF is one document with one page per slide (`@page` sized to the deck
//! aspect ratio). PNG is one file per slide at 1280px wide.
//!
//! This crate knows nothing about the server, the CLI or the app data layout:
//! callers hand it a browser executable, fully built URLs and output paths.
//! That keeps it usable from the running server (the UI export buttons) and
//! from the `slyde export` command alike.

mod cdp;

pub use cdp::{RenderedSlide, export_png_batch, export_slides_with_text};

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use slyde_core::AspectRatio;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

const CHROME_TIMEOUT: Duration = Duration::from_secs(90);
const SLIDE_WIDTH: u32 = 1280;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("the deck has no slides to export")]
    EmptyDeck,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("chrome exited with {status}: {details}")]
    Chrome { status: String, details: String },
}

/// One slide URL plus the id used to name its PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideTarget {
    pub id: String,
    pub url: String,
}

/// Emitted before the first slide and after every written file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    pub completed: usize,
    pub total: usize,
}

/// Prints the whole deck as one paginated PDF.
pub async fn export_pdf(
    chrome: &Path,
    print_url: &str,
    output: &Path,
    progress: Option<&UnboundedSender<ExportProgress>>,
) -> Result<PathBuf, ExportError> {
    report(progress, 0, 1);

    // Chrome resolves `--print-to-pdf` against its own working directory, not
    // always ours, so hand it an absolute path.
    let output = std::path::absolute(output)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&output);

    let profile = export_profile_dir("pdf");
    let mut command = tokio::process::Command::new(chrome);
    command
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--no-pdf-header-footer")
        .arg("--run-all-compositor-stages-before-draw")
        // Math, diagrams and syntax highlighting render after the load event;
        // the virtual time budget gives them room before the page is captured.
        .arg("--virtual-time-budget=15000")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--print-to-pdf={}", output.display()))
        .arg(print_url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // A timeout drops the future; without this, Chrome would outlive the
        // failed export and keep the profile directory locked.
        .kill_on_drop(true);

    let outcome = run_chrome(command, &output).await;
    let _ = std::fs::remove_dir_all(&profile);
    outcome?;

    report(progress, 1, 1);
    Ok(output)
}

/// Renders one PNG per slide into `dir`, in deck order.
pub async fn export_png(
    chrome: &Path,
    slides: &[SlideTarget],
    aspect_ratio: AspectRatio,
    dir: &Path,
    progress: Option<&UnboundedSender<ExportProgress>>,
) -> Result<Vec<PathBuf>, ExportError> {
    if slides.is_empty() {
        return Err(ExportError::EmptyDeck);
    }

    let dir = std::path::absolute(dir)?;
    std::fs::create_dir_all(&dir)?;

    let (width, height) = png_dimensions(aspect_ratio);
    let profile = export_profile_dir("png");
    let total = slides.len();
    let mut written = Vec::with_capacity(total);

    report(progress, 0, total);

    let outcome: Result<(), ExportError> = async {
        for (index, slide) in slides.iter().enumerate() {
            let file = dir.join(format!("{:02}-{}.png", index + 1, safe_id(&slide.id)));
            let _ = std::fs::remove_file(&file);

            let mut command = tokio::process::Command::new(chrome);
            command
                .arg("--headless=new")
                .arg("--disable-gpu")
                .arg("--hide-scrollbars")
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--virtual-time-budget=15000")
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(format!("--window-size={width},{height}"))
                .arg(format!("--screenshot={}", file.display()))
                .arg(&slide.url)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            run_chrome(command, &file).await?;
            written.push(file);
            report(progress, index + 1, total);
        }
        Ok(())
    }
    .await;

    let _ = std::fs::remove_dir_all(&profile);
    outcome?;
    Ok(written)
}

/// Pixels for a screenshot at a fixed 1280px width, matching the print scale.
pub fn png_dimensions(aspect_ratio: AspectRatio) -> (u32, u32) {
    let (base_w, base_h) = aspect_ratio.size_px();
    let height = (SLIDE_WIDTH as f64 * base_h as f64 / base_w as f64).round() as u32;
    (SLIDE_WIDTH, height.max(1))
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
            details: String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(600)
                .collect(),
        });
    }

    Ok(())
}

fn report(progress: Option<&UnboundedSender<ExportProgress>>, completed: usize, total: usize) {
    if let Some(tx) = progress {
        // A closed channel only means nobody is watching (CLI, tests).
        let _ = tx.send(ExportProgress { completed, total });
    }
}

/// A profile per export attempt: two exports at once must never fight over the
/// same Chrome profile lock.
pub(crate) fn export_profile_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("slyde-export-{tag}-{}-{nanos}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn safe_id(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_width_is_fixed_and_height_follows_the_ratio() {
        assert_eq!(png_dimensions(AspectRatio::R16x9), (1280, 720));
        assert_eq!(png_dimensions(AspectRatio::R4x3), (1280, 960));
        assert_eq!(png_dimensions(AspectRatio::R9x16), (1280, 2276));
    }

    #[test]
    fn slide_ids_are_made_filename_safe() {
        assert_eq!(safe_id("sld_abc-123"), "sld_abc-123");
        assert_eq!(safe_id("../../etc/passwd"), "------etc-passwd");
    }

    #[tokio::test]
    async fn exporting_an_empty_deck_fails_before_launching_chrome() {
        let dir = std::env::temp_dir().join(format!("slyde-export-empty-{}", std::process::id()));
        let err = export_png(Path::new("chrome"), &[], AspectRatio::R16x9, &dir, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ExportError::EmptyDeck));
    }
}
