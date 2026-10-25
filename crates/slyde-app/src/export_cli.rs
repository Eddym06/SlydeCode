//! `slyde export`: export the deck that is already open in the studio.
//!
//! The command talks to the running instance over its loopback API, hands the
//! URLs to `slyde-export`, and prints the files it wrote. It deliberately owns
//! the CLI concerns (arguments, runtime discovery, progress lines) so the
//! export crate stays transport-agnostic.

use std::path::PathBuf;
use std::time::Duration;

use slyde_core::AspectRatio;
use slyde_export::{ExportProgress, SlideTarget, export_pdf, export_png};
use slyde_server::{discover_browser, http::get_json, probe_health, read_runtime};

const NOT_RUNNING: &str = "SlydeCode is not running. Open the app first, then export.";
const NO_BROWSER: &str = "no supported browser found; install Google Chrome or set SLYDE_BROWSER";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pdf,
    Png,
}

impl Format {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.to_ascii_lowercase().as_str() {
            "pdf" => Ok(Self::Pdf),
            "png" | "images" => Ok(Self::Png),
            other => Err(format!("unsupported format: {other} (use pdf or png)")),
        }
    }
}

pub async fn run(args: &[String]) -> Result<(), String> {
    let mut format = Format::Pdf;
    let mut output: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" | "-f" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--format needs a value".to_string())?;
                format = Format::parse(value)?;
            }
            "--out" | "-o" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--out needs a path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown option: {other}")),
        }
        index += 1;
    }

    let runtime = read_runtime().ok_or_else(|| NOT_RUNNING.to_string())?;
    if !probe_health(runtime.port, Duration::from_millis(600)).await {
        return Err(NOT_RUNNING.to_string());
    }

    let state = get_json(
        &format!(
            "http://127.0.0.1:{}/api/state?t={}",
            runtime.port, runtime.token
        ),
        Duration::from_secs(5),
    )
    .await
    .map_err(|err| format!("could not read the deck: {err}"))?;

    let deck = state
        .pointer("/view/presentation")
        .ok_or_else(|| "the running instance returned no deck".to_string())?;
    let title = deck
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("deck");
    let aspect_ratio = deck
        .get("aspect_ratio")
        .and_then(|value| value.as_str())
        .and_then(AspectRatio::parse)
        .unwrap_or_default();
    let slide_ids: Vec<String> = deck
        .get("slides")
        .and_then(|value| value.as_array())
        .map(|slides| {
            slides
                .iter()
                .filter_map(|slide| slide.get("id").and_then(|value| value.as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    if slide_ids.is_empty() {
        return Err("the deck has no slides to export".to_string());
    }

    let browser = discover_browser().map_err(|_| NO_BROWSER.to_string())?;
    let stem = slyde_server::storage::file_stem_for(title);
    let sandbox = format!("http://127.0.0.1:{}", runtime.sandbox_port);

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<ExportProgress>();
    let reporter = tokio::spawn(async move {
        while let Some(step) = progress_rx.recv().await {
            if step.total > 1 {
                eprintln!("exporting {}/{}", step.completed, step.total);
            }
        }
    });

    let result = match format {
        Format::Pdf => {
            let path =
                output.unwrap_or_else(|| PathBuf::from("exports").join(format!("{stem}.pdf")));
            let url = format!("{sandbox}/print?t={}", runtime.token);
            export_pdf(&browser.path, &url, &path, Some(&progress_tx))
                .await
                .map(|written| vec![written])
        }
        Format::Png => {
            let dir = output.unwrap_or_else(|| PathBuf::from("exports").join(&stem));
            let targets: Vec<SlideTarget> = slide_ids
                .iter()
                .map(|id| SlideTarget {
                    id: id.clone(),
                    // Slide documents are authenticated: the CLI already holds the
                    // session token, and `/s/{id}` accepts it.
                    url: format!("{sandbox}/s/{id}?static=1&t={}", runtime.token),
                })
                .collect();
            export_png(
                &browser.path,
                &targets,
                aspect_ratio,
                &dir,
                Some(&progress_tx),
            )
            .await
        }
    };

    drop(progress_tx);
    let _ = reporter.await;
    let written = result.map_err(|err| err.to_string())?;

    for file in written {
        println!("{}", file.display());
    }
    Ok(())
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

    #[tokio::test]
    async fn unknown_options_are_rejected_before_touching_the_runtime() {
        let err = run(&["--nope".to_string()]).await.unwrap_err();
        assert!(err.contains("unknown option"));
    }
}
