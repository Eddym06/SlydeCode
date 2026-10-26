//! Named versions of a deck.
//!
//! Undo covers the last few edits of a session; a version is a deliberate
//! checkpoint that survives restarts ("before the AI rewrote everything"). Two
//! kinds exist: the ones a person names, and the automatic ones the copilot saves
//! before a turn that changed the deck. Restoring never destroys anything: the
//! current deck is saved as an automatic version first, and the restore itself
//! is one undoable step.
//!
//! Storage: `<app-data>/versions/<deck-id>/<version-id>.json`, one file per
//! version holding the whole presentation. A server without a library keeps them
//! in memory and never touches the disk.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use slyde_core::Presentation;

/// Automatic versions kept per deck; the oldest are dropped first.
const MAX_AUTO_VERSIONS: usize = 30;
/// Named versions are only removed by their owner.
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionMeta {
    pub id: String,
    pub name: String,
    /// Saved by the studio, not named by a person.
    pub auto: bool,
    pub created_at_ms: u64,
    pub slides: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredVersion {
    #[serde(flatten)]
    meta: VersionMeta,
    presentation: Presentation,
}

#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("that version does not exist")]
    NotFound,
    #[error("the version could not be stored: {0}")]
    Io(String),
}

#[derive(Debug)]
pub struct Versions {
    dir: Option<PathBuf>,
    memory: Mutex<HashMap<String, Vec<StoredVersion>>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Ids and deck keys end up in file names: ASCII, digits, `-` and `_` only.
fn safe(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 96
        && raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

impl Versions {
    pub fn in_memory() -> Self {
        Self {
            dir: None,
            memory: Mutex::new(HashMap::new()),
        }
    }

    pub fn persistent(dir: PathBuf) -> Self {
        Self {
            dir: Some(dir),
            memory: Mutex::new(HashMap::new()),
        }
    }

    fn deck_dir(&self, deck: &str) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .filter(|_| safe(deck))
            .map(|dir| dir.join(deck))
    }

    /// Saves a checkpoint and returns its metadata.
    pub fn save(
        &self,
        deck: &str,
        name: &str,
        auto: bool,
        presentation: &Presentation,
    ) -> Result<VersionMeta, VersionError> {
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        let created = now_ms();
        let meta = VersionMeta {
            id: format!("{created}-{:04x}", rand_suffix()),
            name: if name.is_empty() {
                "Untitled version".to_string()
            } else {
                name
            },
            auto,
            created_at_ms: created,
            slides: presentation.slides.len(),
        };
        let stored = StoredVersion {
            meta: meta.clone(),
            presentation: presentation.clone(),
        };
        if let Some(dir) = self.deck_dir(deck) {
            std::fs::create_dir_all(&dir).map_err(|e| VersionError::Io(e.to_string()))?;
            let bytes = serde_json::to_vec(&stored).map_err(|e| VersionError::Io(e.to_string()))?;
            let target = dir.join(format!("{}.json", meta.id));
            let temp = dir.join(format!("{}.json.tmp", meta.id));
            std::fs::write(&temp, bytes)
                .and_then(|_| std::fs::rename(&temp, &target))
                .map_err(|e| VersionError::Io(e.to_string()))?;
        } else {
            let mut memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            memory.entry(deck.to_string()).or_default().push(stored);
        }
        if auto {
            self.prune(deck);
        }
        Ok(meta)
    }

    /// Newest first.
    pub fn list(&self, deck: &str) -> Vec<VersionMeta> {
        let mut metas: Vec<VersionMeta> = match self.deck_dir(deck) {
            Some(dir) => std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .filter_map(|entry| std::fs::read(entry.path()).ok())
                .filter_map(|bytes| serde_json::from_slice::<StoredVersion>(&bytes).ok())
                .map(|stored| stored.meta)
                .collect(),
            None => self
                .memory
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(deck)
                .map(|list| list.iter().map(|stored| stored.meta.clone()).collect())
                .unwrap_or_default(),
        };
        metas.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(b.id.cmp(&a.id)));
        metas
    }

    pub fn load(&self, deck: &str, id: &str) -> Result<Presentation, VersionError> {
        if !safe(id) {
            return Err(VersionError::NotFound);
        }
        match self.deck_dir(deck) {
            Some(dir) => {
                let bytes = std::fs::read(dir.join(format!("{id}.json")))
                    .map_err(|_| VersionError::NotFound)?;
                serde_json::from_slice::<StoredVersion>(&bytes)
                    .map(|stored| stored.presentation)
                    .map_err(|_| VersionError::NotFound)
            }
            None => self
                .memory
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(deck)
                .and_then(|list| list.iter().find(|stored| stored.meta.id == id))
                .map(|stored| stored.presentation.clone())
                .ok_or(VersionError::NotFound),
        }
    }

    pub fn delete(&self, deck: &str, id: &str) -> Result<(), VersionError> {
        if !safe(id) {
            return Err(VersionError::NotFound);
        }
        match self.deck_dir(deck) {
            Some(dir) => std::fs::remove_file(dir.join(format!("{id}.json")))
                .map_err(|_| VersionError::NotFound),
            None => {
                let mut memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
                let list = memory.get_mut(deck).ok_or(VersionError::NotFound)?;
                let before = list.len();
                list.retain(|stored| stored.meta.id != id);
                if list.len() == before {
                    Err(VersionError::NotFound)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn prune(&self, deck: &str) {
        let autos: Vec<VersionMeta> = self
            .list(deck)
            .into_iter()
            .filter(|meta| meta.auto)
            .collect();
        for old in autos.iter().skip(MAX_AUTO_VERSIONS) {
            let _ = self.delete(deck, &old.id);
        }
    }
}

fn rand_suffix() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static COUNTER: AtomicU16 = AtomicU16::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slyde_core::{AspectRatio, Theme};

    fn deck(title: &str) -> Presentation {
        Presentation::new(title, Theme::studio(), AspectRatio::R16x9)
    }

    #[test]
    fn versions_round_trip_in_memory_newest_first() {
        let versions = Versions::in_memory();
        let first = versions
            .save("d", "First draft", false, &deck("A"))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        versions
            .save("d", "Before: rewrite", true, &deck("B"))
            .unwrap();
        let list = versions.list("d");
        assert_eq!(list.len(), 2);
        assert!(list[0].auto && !list[1].auto, "newest first");
        assert_eq!(versions.load("d", &first.id).unwrap().title, "A");
        assert!(
            versions.list("other").is_empty(),
            "decks do not share versions"
        );
        versions.delete("d", &first.id).unwrap();
        assert!(matches!(
            versions.load("d", &first.id),
            Err(VersionError::NotFound)
        ));
    }

    #[test]
    fn versions_persist_on_disk_and_refuse_path_tricks() {
        let dir = std::env::temp_dir().join(format!("slyde-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let versions = Versions::persistent(dir.clone());
        let meta = versions
            .save("deck-1", "Pitch v1", false, &deck("Pitch"))
            .unwrap();
        assert!(
            dir.join("deck-1")
                .join(format!("{}.json", meta.id))
                .is_file()
        );

        let reopened = Versions::persistent(dir.clone());
        assert_eq!(reopened.list("deck-1")[0].name, "Pitch v1");
        assert_eq!(reopened.load("deck-1", &meta.id).unwrap().title, "Pitch");
        assert!(matches!(
            reopened.load("deck-1", "../../etc/passwd"),
            Err(VersionError::NotFound)
        ));
        assert!(reopened.list("../escape").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn automatic_versions_are_pruned_but_named_ones_stay() {
        let versions = Versions::in_memory();
        versions.save("d", "Keep me", false, &deck("N")).unwrap();
        for index in 0..(MAX_AUTO_VERSIONS + 5) {
            versions
                .save("d", &format!("auto {index}"), true, &deck("A"))
                .unwrap();
        }
        let list = versions.list("d");
        assert_eq!(
            list.iter().filter(|meta| meta.auto).count(),
            MAX_AUTO_VERSIONS
        );
        assert!(list.iter().any(|meta| meta.name == "Keep me"));
    }
}
