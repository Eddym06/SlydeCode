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
            let file = ProjectFile::from_json(&raw).map_err(|err| ServerError::Deck(err.to_string()))?;
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

/// A safe download name derived from the deck title.
///
/// Keeps the file name readable in a downloads folder while guaranteeing it is
/// valid on Windows, macOS and Linux.
pub fn file_name_for(title: &str) -> String {
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
    let capped: String = stem.chars().take(64).collect();
    format!("{capped}.{}", slyde_core::FILE_EXTENSION)
}

/// Saves to a caller-chosen path (used by tests and future CLI save flows).
pub async fn save_to(path: &Path, deck: &Presentation) -> Result<PathBuf, ServerError> {
    save(path, deck).await?;
    Ok(path.to_path_buf())
}

/// Debounced saver.
///
/// Every mutation bumps the watch channel; bursts (an agent writing ten slides)
/// collapse into one write. The last value always wins, so no intermediate
/// state is lost from the user's point of view.
pub async fn run_autosave(mut rx: tokio::sync::watch::Receiver<Presentation>, path: PathBuf) {
    while rx.changed().await.is_ok() {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let deck = rx.borrow_and_update().clone();
        if let Err(err) = save(&path, &deck).await {
            tracing::error!(error = %err, path = %path.display(), "deck autosave failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slyde_core::{AspectRatio, Slide, Theme};

    fn deck() -> Presentation {
        let mut deck = Presentation::new("Landing <v2>", Theme::studio(), AspectRatio::R16x9);
        deck.slides.push(Slide::new("sld_a".into(), "<h1>A</h1>".into()));
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
        assert_eq!(loaded.slides[0].code, "<h1>A</h1>");
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
}
