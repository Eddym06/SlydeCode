//! Durable deck storage.
//!
//! Two things live here:
//!
//! - `current.slyde` in the OS app-data directory: the deck the user is
//!   working on. Written by the autosave task after every mutation, restored on
//!   startup, so closing the app never loses work.
//! - The same format, used for explicit downloads and uploads (`.slyde` files).
//!
//! Writes are atomic: a temporary file is written and then renamed over the
//! target. A crash mid-write leaves the previous deck intact instead of a
//! truncated file that cannot be opened.

use std::path::{Path, PathBuf};

use slyde_core::{Presentation, ProjectFile};

use crate::error::ServerError;

/// Directory that holds the working deck and any future per-deck files.
pub fn decks_dir() -> PathBuf {
    crate::rendezvous::app_data_dir().join("decks")
}

/// The deck restored on startup.
pub fn current_path() -> PathBuf {
    decks_dir().join(format!("current.{}", slyde_core::FILE_EXTENSION))
}

/// Reads and validates a deck. `Ok(None)` means the file does not exist yet,
/// which is the normal first-run state, not an error.
pub fn load(path: &Path) -> Result<Option<Presentation>, ServerError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let file =
                ProjectFile::from_json(&raw).map_err(|err| ServerError::Deck(err.to_string()))?;
            Ok(Some(file.presentation))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ServerError::Io(err)),
    }
}

/// Writes the deck atomically.
pub async fn save(path: &Path, deck: &Presentation) -> Result<(), ServerError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = ProjectFile::new(deck.clone()).to_json()?;
    let temp = path.with_extension("slyde.tmp");
    tokio::fs::write(&temp, json.as_bytes()).await?;
    // Rename replaces the destination atomically on both Windows and Unix.
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

/// Directory where export jobs stage their files before they are handed to the
/// browser. Always inside app data, never the repository.
pub fn exports_dir() -> PathBuf {
    crate::rendezvous::app_data_dir().join("exports")
}

/// A safe, readable file stem derived from the deck title.
///
/// Every export format builds its own name from this (`q3-review.pdf`,
/// `q3-review.zip`), so the name the user downloads always matches the deck.
pub fn file_stem_for(title: &str) -> String {
    let mut slug: String = title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let stem = if slug.is_empty() { "deck" } else { slug };
    stem.chars().take(64).collect()
}

/// A safe download name derived from the deck title.
///
/// Keeps the file name readable in a downloads folder while guaranteeing it is
/// valid on Windows, macOS and Linux.
pub fn file_name_for(title: &str) -> String {
    format!("{}.{}", file_stem_for(title), slyde_core::FILE_EXTENSION)
}

/// Saves to a caller-chosen path (used by tests and future CLI save flows).
pub async fn save_to(path: &Path, deck: &Presentation) -> Result<PathBuf, ServerError> {
    save(path, deck).await?;
    Ok(path.to_path_buf())
}

/// Files bundled with a deck, as `(asset file name, bytes)`.
pub type DeckAssets = Vec<(String, Vec<u8>)>;

/// A `.slyde` file is JSON for a deck without images and a ZIP bundle when the
/// slides reference files in the asset store.
///
/// The bundle keeps the same extension: a user should not have to know which
/// shape their deck took, and the reader sniffs the ZIP signature instead of
/// trusting the name.
pub fn is_bundle(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// Reads a `.slyde` file in either shape.
///
/// Returns the deck plus the bundled assets, which the caller restores into the
/// asset store. A JSON deck simply has no assets.
pub fn read_deck_file(bytes: &[u8]) -> Result<(Presentation, DeckAssets), ServerError> {
    if !is_bundle(bytes) {
        let raw = std::str::from_utf8(bytes)
            .map_err(|_| ServerError::Deck("deck file must be UTF-8 text or a bundle".into()))?;
        let file = ProjectFile::from_json(raw).map_err(|err| ServerError::Deck(err.to_string()))?;
        return Ok((file.presentation, Vec::new()));
    }

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|err| ServerError::Deck(format!("invalid deck bundle: {err}")))?;
    let mut deck_json: Option<Vec<u8>> = None;
    let mut assets = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| ServerError::Deck(format!("invalid deck bundle: {err}")))?;
        let Some(name) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };
        let mut buffer = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buffer)
            .map_err(|err| ServerError::Deck(format!("invalid deck bundle: {err}")))?;
        match name.to_str() {
            Some("deck.json") => deck_json = Some(buffer),
            Some(other) if other.starts_with("assets/") => {
                let file = other.trim_start_matches("assets/").to_string();
                assets.push((file, buffer));
            }
            _ => {}
        }
    }

    let deck_json =
        deck_json.ok_or_else(|| ServerError::Deck("the bundle has no deck.json".into()))?;
    let raw = String::from_utf8(deck_json)
        .map_err(|_| ServerError::Deck("deck.json is not UTF-8".into()))?;
    let file = ProjectFile::from_json(&raw).map_err(|err| ServerError::Deck(err.to_string()))?;
    Ok((file.presentation, assets))
}

/// Serialises a deck, bundling the assets it references.
pub fn write_deck_file(
    deck: &Presentation,
    assets: &[(String, Vec<u8>)],
) -> Result<Vec<u8>, ServerError> {
    let json = ProjectFile::new(deck.clone()).to_json()?;
    if assets.is_empty() {
        return Ok(json.into_bytes());
    }

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let deck_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    zip.start_file("deck.json", deck_options)
        .map_err(|err| ServerError::Deck(format!("could not write the bundle: {err}")))?;
    std::io::Write::write_all(&mut zip, json.as_bytes())
        .map_err(|err| ServerError::Deck(format!("could not write the bundle: {err}")))?;

    // Images are already compressed; storing them keeps the bundle fast.
    let asset_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o644);
    for (name, bytes) in assets {
        zip.start_file(format!("assets/{name}"), asset_options)
            .map_err(|err| ServerError::Deck(format!("could not write the bundle: {err}")))?;
        std::io::Write::write_all(&mut zip, bytes)
            .map_err(|err| ServerError::Deck(format!("could not write the bundle: {err}")))?;
    }
    let cursor = zip
        .finish()
        .map_err(|err| ServerError::Deck(format!("could not finish the bundle: {err}")))?;
    Ok(cursor.into_inner())
}

/// Every `/assets/<name>` the deck's slide code references, in first-seen order.
pub fn referenced_assets(deck: &Presentation) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for slide in &deck.slides {
        let mut rest = slide.code.as_str();
        while let Some(position) = rest.find("/assets/") {
            let tail = &rest[position + "/assets/".len()..];
            // Asset names never contain quotes, spaces or markup characters, so
            // the reference ends at the first one.
            let end = tail
                .find(|ch: char| {
                    ch.is_whitespace()
                        || matches!(ch, '"' | '\'' | ')' | '(' | '<' | '>' | ';' | ',' | '\\')
                })
                .unwrap_or(tail.len());
            let candidate = &tail[..end];
            if crate::assets::name_from_slide_url(&format!("/assets/{candidate}")).is_some()
                && !names.iter().any(|name| name == candidate)
            {
                names.push(candidate.to_string());
            }
            rest = &tail[end..];
        }
    }
    names
}
/// Debounced saver for the active deck.
///
/// Every mutation bumps the watch channel; bursts (an agent writing ten slides)
/// collapse into one write. The last value always wins, so no intermediate
/// state is lost from the user's point of view.
///
/// The target path comes from a second watch channel rather than a fixed
/// argument: switching decks in the library changes where the next write goes,
/// and the debounced write must land in the deck that produced it.
pub async fn run_autosave(
    mut deck_rx: tokio::sync::watch::Receiver<Presentation>,
    mut path_rx: tokio::sync::watch::Receiver<PathBuf>,
) {
    loop {
        tokio::select! {
            changed = deck_rx.changed() => {
                if changed.is_err() {
                    return;
                }

                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let deck = deck_rx.borrow_and_update().clone();
                // Read the path after the debounce, not before: a deck switch
                // during the window must not write this state into the previous
                // file.
                let path = path_rx.borrow_and_update().clone();
                if let Err(err) = save(&path, &deck).await {
                    tracing::error!(error = %err, path = %path.display(), "deck autosave failed");
                }
            }
            changed = path_rx.changed() => {
                if changed.is_err() {
                    return;
                }

                // The path changed and the caller already saved the outgoing
                // deck; just acknowledge so the select does not spin.
                path_rx.borrow_and_update();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slyde_core::{AspectRatio, Slide, Theme};

    fn deck() -> Presentation {
        let mut deck = Presentation::new("Landing <v2>", Theme::studio(), AspectRatio::R16x9);
        deck.slides
            .push(Slide::new("sld_a".into(), "<h1>A</h1>".into()));
        deck
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("slyde-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn missing_file_is_not_an_error() {
        let dir = temp_dir("missing");
        assert!(load(&dir.join("nope.slyde")).unwrap().is_none());
    }

    #[tokio::test]
    async fn saves_and_loads_a_deck() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("deck.slyde");
        save(&path, &deck()).await.unwrap();

        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.title, "Landing <v2>");
        assert_eq!(loaded.slides.len(), 1);
        // Loading assigns a persistent handle to every block.
        assert_eq!(loaded.slides[0].code, "<h1 data-id=\"slyde-1\">A</h1>");
    }

    #[tokio::test]
    async fn saving_replaces_the_previous_file_and_leaves_no_temp_file() {
        let dir = temp_dir("replace");
        let path = dir.join("deck.slyde");
        save(&path, &deck()).await.unwrap();

        let mut next = deck();
        next.title = "Second".into();
        save(&path, &next).await.unwrap();

        assert_eq!(load(&path).unwrap().unwrap().title, "Second");
        assert!(!path.with_extension("slyde.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_reported_without_panicking() {
        let dir = temp_dir("corrupt");
        let path = dir.join("bad.slyde");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn download_names_are_safe_and_readable() {
        assert_eq!(file_name_for("Q3 Review"), "q3-review.slyde");
        assert_eq!(file_name_for("  "), "deck.slyde");
        assert_eq!(file_name_for("../../etc/passwd"), "etc-passwd.slyde");
        assert_eq!(file_name_for("Diseño 2026"), "diseño-2026.slyde");
        assert_eq!(file_name_for(&"x".repeat(200)).len(), 64 + ".slyde".len());
    }

    #[test]
    fn a_deck_without_images_stays_plain_json() {
        let bytes = write_deck_file(&deck(), &[]).unwrap();
        assert!(!is_bundle(&bytes), "no assets means no ZIP");
        let (parsed, assets) = read_deck_file(&bytes).unwrap();
        assert_eq!(parsed.title, "Landing <v2>");
        assert!(assets.is_empty());
    }

    #[test]
    fn a_deck_with_images_round_trips_as_a_bundle() {
        let mut deck = deck();
        deck.slides[0].code = format!(
            "{}<img data-id=\"image-1\" src=\"/assets/0123456789abcdef.png\" style=\"width: 100px\" />",
            deck.slides[0].code
        );
        let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        let assets = vec![("0123456789abcdef.png".to_string(), png.clone())];

        let bytes = write_deck_file(&deck, &assets).unwrap();
        assert!(is_bundle(&bytes), "a deck with images is a ZIP bundle");

        let (parsed, extracted) = read_deck_file(&bytes).unwrap();
        // Loading normalises element handles, so compare the parts that matter:
        // the reference and the image bytes.
        assert!(
            parsed.slides[0]
                .code
                .contains("/assets/0123456789abcdef.png")
        );
        assert!(parsed.slides[0].code.contains("data-id=\"image-1\""));
        assert_eq!(extracted, assets, "the image survives byte for byte");
    }

    #[test]
    fn referenced_assets_are_deduplicated_and_ignore_data_urls() {
        let mut deck = deck();
        deck.slides[0].code = concat!(
            "<img src=\"/assets/0123456789abcdef.png\">",
            "<img src='/assets/0123456789abcdef.png'>",
            "<img src=\"/assets/fedcba9876543210.webp\">",
            "<img src=\"data:image/png;base64,AAAA\">"
        )
        .to_string();
        let names = referenced_assets(&deck);
        assert_eq!(
            names,
            vec![
                "0123456789abcdef.png".to_string(),
                "fedcba9876543210.webp".to_string()
            ]
        );
    }

    #[test]
    fn a_broken_bundle_is_reported_without_panicking() {
        assert!(read_deck_file(b"PK\x03\x04not really a zip").is_err());
        assert!(read_deck_file(b"{not json").is_err());
    }
    #[test]
    fn export_stems_are_shared_across_formats() {
        assert_eq!(file_stem_for("Q3 Review"), "q3-review");
        assert_eq!(file_stem_for("  "), "deck");
        assert_eq!(file_stem_for(&"x".repeat(200)).len(), 64);
    }
}
