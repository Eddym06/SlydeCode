//! Local provider configuration.
//!
//! Keys are written to the OS app-data directory with owner-only permissions
//! on Unix. They are never returned to the UI: the API exposes only whether a
//! provider is configured. Environment variables are read as a fallback, so a
//! developer can keep keys out of the app entirely.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rendezvous::app_data_dir;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CopilotConfig {
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
    #[serde(default)]
    pub last_provider: Option<String>,
    #[serde(default)]
    pub last_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: Option<PathBuf>,
}

impl ConfigStore {
    pub fn persistent() -> Self {
        Self {
            path: Some(app_data_dir().join("copilot.json")),
        }
    }

    pub fn in_memory() -> Self {
        Self { path: None }
    }

    pub fn load(&self) -> CopilotConfig {
        let Some(path) = &self.path else {
            return CopilotConfig::default();
        };
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|error| {
                tracing::warn!(error = %error, "copilot config is not valid JSON; ignoring it");
                CopilotConfig::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CopilotConfig::default(),
            Err(error) => {
                tracing::warn!(error = %error, "could not read copilot config");
                CopilotConfig::default()
            }
        }
    }

    pub async fn save(&self, config: &CopilotConfig) -> Result<(), std::io::Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let encoded = serde_json::to_vec_pretty(config)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let temp = path.with_extension("json.tmp");
        tokio::fs::write(&temp, encoded).await?;
        restrict(&temp);
        tokio::fs::rename(&temp, path).await?;
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {
    // Windows and macOS use per-user app-data directories. Windows has no
    // portable DACL API in std; the file stays under the user's profile.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_never_touches_disk() {
        let store = ConfigStore::in_memory();
        assert!(store.path().is_none());
        assert!(store.load().keys.is_empty());
    }
}
