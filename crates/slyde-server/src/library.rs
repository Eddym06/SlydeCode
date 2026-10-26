//! Deck library: many decks on disk, one active at a time.
//!
//! Before this module existed the app kept a single `current.slyde`, so starting
//! a second deck silently overwrote the first. A library makes the decks
//! directory the source of truth:
//!
//! - `<app-data>/decks/<id>.slyde` — one file per deck, same versioned format the
//!   download/upload path uses;
//! - `<app-data>/settings.json` — which deck is active (`active_deck`);
//! - the legacy `current.slyde` file is imported once by [`Library::migrate_legacy`].
//!
//! Deck ids are slugs derived from the title plus a short random suffix. They are
//! filename-safe by construction and validated on every lookup, so a request can
//! never address a file outside the library.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use slyde_core::{AspectRatio, Presentation, Theme};
use tokio::sync::watch;

use crate::error::ServerError;
use crate::storage;

/// Epoch milliseconds, used to sort the library and stamp writes.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Where the library keeps its files. `root` is the decks directory itself.
#[derive(Debug)]
pub struct Library {
    root: PathBuf,
    settings_path: PathBuf,
    /// Path of the file the autosave task must write to. Updated atomically when
    /// the active deck changes so a debounced write can never land in the wrong
    /// deck.
    path_tx: watch::Sender<PathBuf>,
}

/// One row of the deck list. Everything here is derived from the file, so the
/// list can never disagree with what is on disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeckSummary {
    pub id: String,
    pub title: String,
    pub slides: usize,
    pub updated_at_ms: u64,
    pub theme_id: String,
    pub aspect_ratio: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    active_deck: Option<String>,
}

impl Library {
    /// Opens (and lazily creates) the library under `root`.
    ///
    /// The settings file lives inside `root`: a library owns its own state, and
    /// two libraries (a test, a temporary profile, the real app-data one) can
    /// never fight over a shared pointer.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let settings_path = root.join("settings.json");
        let active = Self::load_settings(&settings_path)
            .and_then(|settings| settings.active_deck)
            .map(|id| Self::path_in(&root, &id))
            .unwrap_or_else(|| root.join("unset.slyde"));
        let (path_tx, _) = watch::channel(active);
        Self {
            root,
            settings_path,
            path_tx,
        }
    }

    /// The library that belongs to the OS app-data directory.
    pub fn from_app_data() -> Self {
        Self::new(storage::decks_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Subscribes to the active file path. The autosave task owns one of these.
    pub fn subscribe_path(&self) -> watch::Receiver<PathBuf> {
        self.path_tx.subscribe()
    }

    pub fn active(&self) -> Option<String> {
        Self::load_settings(&self.settings_path).and_then(|settings| settings.active_deck)
    }

    /// Makes `id` the deck every mutation is saved to.
    pub fn set_active(&self, id: &str) -> Result<(), ServerError> {
        validate_id(id)?;
        if !self.path_for(id).is_file() {
            return Err(ServerError::Deck(format!("deck not found: {id}")));
        }
        self.write_settings(Settings {
            active_deck: Some(id.to_string()),
        })?;
        // `send` drops the value when nobody is subscribed yet, which is exactly
        // the startup case: the library is configured before the autosave task
        // subscribes, so the write would land in the placeholder path.
        self.path_tx.send_replace(self.path_for(id));
        Ok(())
    }

    /// Every deck in the library, newest first.
    ///
    /// A file that cannot be read or parsed is skipped instead of failing the
    /// whole listing: one corrupt deck must not hide the rest.
    pub fn list(&self) -> Result<Vec<DeckSummary>, ServerError> {
        let mut decks = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(decks),
            Err(error) => return Err(ServerError::Io(error)),
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(slyde_core::FILE_EXTENSION) {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if validate_id(id).is_err() {
                continue;
            }
            let Ok(Some(deck)) = storage::load(&path) else {
                continue;
            };
            decks.push(DeckSummary {
                id: id.to_string(),
                title: deck.title.clone(),
                slides: deck.slides.len(),
                updated_at_ms: deck.updated_at_ms,
                theme_id: deck.theme.id.clone(),
                aspect_ratio: deck.aspect_ratio.as_str().to_string(),
            });
        }
        decks.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(decks)
    }

    /// Loads one deck. `Ok(None)` means the file is gone.
    pub fn load(&self, id: &str) -> Result<Option<Presentation>, ServerError> {
        validate_id(id)?;
        storage::load(&self.path_for(id))
    }

    /// Writes a brand-new deck and returns its id.
    ///
    /// The caller supplies the starter presentation so the library stays free of
    /// template policy; it only owns identity and storage.
    pub async fn create(&self, title: &str, deck: &Presentation) -> Result<String, ServerError> {
        let id = self.free_id(title);
        let mut deck = deck.clone();
        deck.title = title.trim().to_string();
        deck.updated_at_ms = now_ms();
        storage::save(&self.path_for(&id), &deck).await?;
        Ok(id)
    }

    /// Copies a deck into a new id. Returns `(new_id, presentation)`.
    pub async fn duplicate(&self, id: &str) -> Result<(String, Presentation), ServerError> {
        let Some(original) = self.load(id)? else {
            return Err(ServerError::Deck(format!("deck not found: {id}")));
        };
        let title = format!("{} (copy)", original.title);
        let new_id = self.create(&title, &original).await?;
        let copy = self
            .load(&new_id)?
            .ok_or_else(|| ServerError::Deck("the copy could not be read back".into()))?;
        Ok((new_id, copy))
    }

    /// Renames a deck. The id and the file name stay stable so open tabs, MCP
    /// clients and recent-file lists keep pointing at the same deck.
    pub async fn rename(&self, id: &str, title: &str) -> Result<DeckSummary, ServerError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(ServerError::Deck("a deck title cannot be empty".into()));
        }
        let Some(mut deck) = self.load(id)? else {
            return Err(ServerError::Deck(format!("deck not found: {id}")));
        };
        deck.title = title.chars().take(120).collect();
        deck.updated_at_ms = now_ms();
        storage::save(&self.path_for(id), &deck).await?;
        Ok(DeckSummary {
            id: id.to_string(),
            title: deck.title,
            slides: deck.slides.len(),
            updated_at_ms: deck.updated_at_ms,
            theme_id: deck.theme.id,
            aspect_ratio: deck.aspect_ratio.as_str().to_string(),
        })
    }

    /// Deletes a deck file. Refuses to leave the library without an active deck:
    /// the caller decides what to open next.
    pub fn remove(&self, id: &str) -> Result<(), ServerError> {
        validate_id(id)?;
        crate::thumbnails::forget(&self.root, id);
        match std::fs::remove_file(self.path_for(id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ServerError::Io(error)),
        }
    }

    /// Imports the pre-library `current.slyde` file as a normal deck.
    ///
    /// Runs once on startup. The legacy file is left untouched: if the migration
    /// is interrupted or the user downgrades, no work is lost.
    pub async fn migrate_legacy(&self) -> Result<Option<String>, ServerError> {
        let legacy = storage::current_path();
        if !legacy.is_file() {
            return Ok(None);
        }
        let Some(deck) = storage::load(&legacy)? else {
            return Ok(None);
        };
        let id = self.free_id(&deck.title);
        storage::save(&self.path_for(&id), &deck).await?;
        Ok(Some(id))
    }

    /// The id every mutation should be saved under: the active one, or the
    /// newest deck when the active pointer is missing or stale.
    pub fn active_or_newest(&self) -> Result<Option<String>, ServerError> {
        if let Some(id) = self.active()
            && self.path_for(&id).is_file()
        {
            return Ok(Some(id));
        }
        Ok(self.list()?.first().map(|deck| deck.id.clone()))
    }

    pub fn path_for(&self, id: &str) -> PathBuf {
        Self::path_in(&self.root, id)
    }

    fn path_in(root: &Path, id: &str) -> PathBuf {
        root.join(format!("{id}.{}", slyde_core::FILE_EXTENSION))
    }

    /// First free id for `title`: readable, stable for a human, unique on disk.
    fn free_id(&self, title: &str) -> String {
        let base = id_slug(title);
        for attempt in 0..64 {
            let suffix = short_suffix();
            let candidate = if attempt == 0 {
                base.clone()
            } else {
                format!("{base}-{attempt}")
            };
            let candidate = format!("{candidate}-{suffix}");
            if !self.path_for(&candidate).exists() {
                return candidate;
            }
        }
        format!("deck-{}", uuid_like_suffix())
    }

    fn load_settings(path: &Path) -> Option<Settings> {
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn write_settings(&self, settings: Settings) -> Result<(), ServerError> {
        if let Some(parent) = self.settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let encoded = serde_json::to_vec_pretty(&settings)
            .map_err(|error| ServerError::Deck(error.to_string()))?;
        let temp = self.settings_path.with_extension("json.tmp");
        std::fs::write(&temp, encoded)?;
        std::fs::rename(&temp, &self.settings_path)?;
        Ok(())
    }
}

/// Deck ids are file names. Only lowercase ASCII, digits and dashes are allowed,
/// which also rules out separators, drive letters and dot segments.
fn validate_id(id: &str) -> Result<(), ServerError> {
    let valid = !id.is_empty()
        && id.len() <= 96
        && id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    if valid {
        Ok(())
    } else {
        Err(ServerError::Deck(format!("invalid deck id: {id}")))
    }
}

/// Turns a human title into an ASCII id stem.
///
/// Titles are localized ("Presentación sin título"), and ids are file names:
/// transliterating the common Latin accents is what keeps `validate_id` from
/// rejecting the very titles the UI produces. Download file names keep their
/// accents (`storage::file_name_for`); only the internal id is reduced.
fn id_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    // `char::to_lowercase` handles 'Ñ' -> 'ñ'; `to_ascii_lowercase` would not.
    for ch in raw.chars().flat_map(char::to_lowercase) {
        let mapped = match ch {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        };
        if mapped.is_ascii_alphanumeric() {
            out.push(mapped);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let short: String = trimmed.chars().take(40).collect();
    let short = short.trim_end_matches('-');
    if short.is_empty() {
        "deck".to_string()
    } else {
        short.to_string()
    }
}

fn short_suffix() -> String {
    let mut bytes = [0u8; 2];
    if getrandom::fill(&mut bytes).is_err() {
        return std::process::id().to_string();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn uuid_like_suffix() -> String {
    let mut bytes = [0u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        return std::process::id().to_string();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The deck a brand-new library starts with: one welcome slide, studio theme.
pub fn welcome_deck() -> Presentation {
    let mut deck = Presentation::new("Untitled presentation", Theme::studio(), AspectRatio::R16x9);
    let mut slide = slyde_core::Slide::new(
        slyde_core::new_slide_id(),
        slyde_core::ensure_element_ids(&crate::starter::default_code("hero")),
    );
    slide.template = slyde_core::TemplateType::Hero;
    slide.animation = crate::starter::default_animation_for("hero");
    deck.slides.push(slide);
    deck.updated_at_ms = now_ms();
    deck
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_library(tag: &str) -> (Library, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "slyde-library-{tag}-{}-{}",
            std::process::id(),
            short_suffix()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (Library::new(&root), root)
    }

    #[test]
    fn ids_are_validated_against_traversal() {
        assert!(validate_id("q3-review-8f21").is_ok());
        assert!(validate_id("../escape").is_err());
        assert!(validate_id("c:\\absolute").is_err());
        assert!(validate_id("UPPER").is_err());
        assert!(validate_id("").is_err());
        assert!(validate_id("with space").is_err());
    }

    /// Localized titles are the norm ("Presentación sin título"), and an id is a
    /// file name, so the slug has to survive every accent the UI can produce.
    #[test]
    fn id_slugs_are_ascii_and_never_empty() {
        assert_eq!(id_slug("Q3 Review"), "q3-review");
        assert_eq!(
            id_slug("Presentación sin título"),
            "presentacion-sin-titulo"
        );
        assert_eq!(id_slug("Diseño 2026 — Año"), "diseno-2026-ano");
        assert_eq!(id_slug("  ???  "), "deck");
        assert_eq!(id_slug(""), "deck");
        assert_eq!(id_slug("Ñandú café"), "nandu-cafe");
        for title in ["Presentación sin título", "Diseño 2026", "日本語"] {
            let slug = id_slug(title);
            assert!(
                validate_id(&slug).is_ok(),
                "{title} produced an id the validator rejects: {slug}"
            );
        }
        assert_eq!(id_slug(&"x".repeat(200)).len(), 40);
    }

    #[tokio::test]
    async fn create_list_duplicate_and_remove() {
        let (library, root) = temp_library("crud");

        let first = library.create("Q3 Review", &welcome_deck()).await.unwrap();
        assert!(first.starts_with("q3-review-"), "{first}");
        let second = library.create("Roadmap", &welcome_deck()).await.unwrap();
        assert_ne!(first, second);

        let decks = library.list().unwrap();
        assert_eq!(decks.len(), 2, "both decks are visible");
        assert!(decks.iter().any(|deck| deck.id == first));
        assert!(decks.iter().any(|deck| deck.title == "Roadmap"));

        let (copy_id, copy) = library.duplicate(&first).await.unwrap();
        assert_eq!(copy.title, "Q3 Review (copy)");
        assert_eq!(library.list().unwrap().len(), 3);

        library.remove(&copy_id).unwrap();
        assert_eq!(library.list().unwrap().len(), 2);
        library.remove(&copy_id).unwrap();
        assert_eq!(
            library.list().unwrap().len(),
            2,
            "removing twice is a no-op"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn active_deck_round_trips_through_settings() {
        let (library, root) = temp_library("active");
        let id = library.create("Deck A", &welcome_deck()).await.unwrap();
        library.set_active(&id).unwrap();

        let reopened = Library::new(&root);
        assert_eq!(reopened.active().as_deref(), Some(id.as_str()));
        assert_eq!(
            reopened.active_or_newest().unwrap().as_deref(),
            Some(id.as_str())
        );
        let path = reopened.path_for(&id);
        assert!(path.is_file());

        // A stale pointer falls back to the newest deck instead of erroring.
        std::fs::remove_file(&path).unwrap();
        reopened.set_active(&id).unwrap_err();
        assert_eq!(reopened.active_or_newest().unwrap(), None);

        let _ = std::fs::remove_dir_all(root);
    }

    /// Regression: `watch::Sender::send` drops the value when nobody is
    /// subscribed yet. The library is configured before the autosave task
    /// subscribes, so the write ended up in the placeholder `unset.slyde` and the
    /// user's deck on disk stayed stale.
    #[tokio::test]
    async fn the_autosave_path_is_correct_even_when_nobody_is_subscribed_yet() {
        let (library, root) = temp_library("path-before-subscribe");
        let id = library.create("Deck", &welcome_deck()).await.unwrap();
        library.set_active(&id).unwrap();

        let path = library.subscribe_path();
        assert_eq!(
            path.borrow().clone(),
            library.path_for(&id),
            "a late subscriber must see the active path, not the placeholder"
        );
        assert!(!path.borrow().ends_with("unset.slyde"));

        let _ = std::fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn rename_keeps_the_id_and_updates_the_title() {
        let (library, root) = temp_library("rename");
        let id = library.create("Original", &welcome_deck()).await.unwrap();

        let summary = library.rename(&id, "  Renamed deck  ").await.unwrap();
        assert_eq!(summary.title, "Renamed deck");
        assert_eq!(summary.id, id, "the id is the stable handle");
        assert_eq!(library.load(&id).unwrap().unwrap().title, "Renamed deck");

        assert!(library.rename(&id, "   ").await.is_err());
        assert!(library.rename("missing", "x").await.is_err());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn corrupt_files_are_skipped_instead_of_hiding_the_rest() {
        let (library, root) = temp_library("corrupt");
        let good = library.create("Good", &welcome_deck()).await.unwrap();
        std::fs::write(root.join("broken.slyde"), "{not json").unwrap();

        let decks = library.list().unwrap();
        assert_eq!(decks.len(), 1);
        assert_eq!(decks[0].id, good);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn listing_is_newest_first() {
        let (library, root) = temp_library("order");
        let old = library.create("Old", &welcome_deck()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        let new = library.create("New", &welcome_deck()).await.unwrap();

        let decks = library.list().unwrap();
        assert_eq!(decks[0].id, new, "the newest deck is first");
        assert_eq!(decks[1].id, old);

        let _ = std::fs::remove_dir_all(root);
    }
}
