//! Local provider configuration.
//!
//! Keys are written to the OS app-data directory and locked down: `0600` on
//! Unix, and an explicit owner-only DACL on Windows (the file would otherwise
//! inherit whatever the profile grants). They are never returned to the UI: the
//! API exposes only whether a provider is configured. Environment variables are
//! read as a fallback, so a developer can keep keys out of the app entirely.

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

    /// A store at an explicit path. Used by tests so they never touch the real
    /// app-data file.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
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

#[cfg(windows)]
fn restrict(path: &Path) {
    // No portable DACL API in std, and pulling in the `windows` crate for one
    // call is not worth it: `icacls` is part of Windows. Best effort by design —
    // a machine where this fails still keeps the file under the user profile, and
    // failing the save would lose the key the user just typed.
    let user = std::env::var("USERNAME").unwrap_or_default();
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    if user.is_empty() {
        tracing::warn!(path = %path.display(), "could not determine the user for the key file ACL");
        return;
    }
    let grant = if domain.is_empty() {
        format!("{user}:F")
    } else {
        format!("{domain}\\{user}:F")
    };
    match std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(&grant)
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => tracing::warn!(
            path = %path.display(),
            status = %output.status,
            "could not tighten the key file ACL; it stays inside the user profile"
        ),
        Err(error) => tracing::warn!(
            path = %path.display(),
            %error,
            "could not run icacls for the key file"
        ),
    }
}

#[cfg(all(not(unix), not(windows)))]
fn restrict(_path: &Path) {
    // Other platforms: the OS app-data directory is per-user by contract.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saving_a_key_tightens_the_file_permissions() {
        let dir = std::env::temp_dir().join(format!(
            "slyde-keys-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ConfigStore::at(dir.join("copilot.json"));
        let mut config = CopilotConfig::default();
        config.keys.insert("openai".into(), "sk-test".into());
        store.save(&config).await.unwrap();

        let path = store.path().unwrap();
        assert!(path.is_file(), "the key file was written");

        #[cfg(windows)]
        {
            // The file must not inherit the profile's broad ACL.
            let output = std::process::Command::new("icacls")
                .arg(path)
                .output()
                .expect("icacls is part of Windows");
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "icacls failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                !text.contains("BUILTIN\\Users"),
                "the key file must not be readable by every local user: {text}"
            );
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "owner-only on Unix");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn in_memory_store_never_touches_disk() {
        let store = ConfigStore::in_memory();
        assert!(store.path().is_none());
        assert!(store.load().keys.is_empty());
    }
}
