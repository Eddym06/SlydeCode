//! Local provider configuration.
//!
//! Keys go to the OS credential store where there is one (see `vault`). Where
//! there is none, or when it refuses a write, they fall back to the app-data
//! file, locked down: `0600` on Unix and an explicit owner-only DACL on Windows
//! (the file would otherwise inherit whatever the profile grants). A key found
//! in the file by an older version is moved into the credential store on load.
//! Keys are never returned to the UI: the API exposes only whether a provider is
//! configured. Environment variables are read as a fallback, so a developer can
//! keep keys out of the app entirely.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::catalog::PROVIDERS;
use super::vault::{Vault, system_vault};
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
    vault: Option<Arc<dyn Vault>>,
}

impl ConfigStore {
    pub fn persistent() -> Self {
        Self {
            path: Some(app_data_dir().join("copilot.json")),
            // The credential store is per user, not per data directory. A custom
            // `SLYDE_DATA_DIR` (tests, probes, a second profile) must stay
            // isolated, so it keeps its keys in its own file.
            vault: if std::env::var_os("SLYDE_DATA_DIR").is_some() {
                None
            } else {
                system_vault().map(Arc::from)
            },
        }
    }

    pub fn in_memory() -> Self {
        Self {
            path: None,
            vault: None,
        }
    }

    /// A store at an explicit path with an explicit vault (or none). Tests use
    /// it so they never touch the real credential store.
    pub fn with_vault(path: impl Into<PathBuf>, vault: Option<Arc<dyn Vault>>) -> Self {
        Self {
            path: Some(path.into()),
            vault,
        }
    }

    /// A store at an explicit path. Used by tests so they never touch the real
    /// app-data file.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::with_vault(path, None)
    }

    pub fn load(&self) -> CopilotConfig {
        let Some(path) = &self.path else {
            return CopilotConfig::default();
        };
        let mut config = read_file(path);
        let Some(vault) = &self.vault else {
            return config;
        };

        // Keys an older version left in the file move into the credential store;
        // the file is only rewritten once every one of them is safely stored.
        let legacy = std::mem::take(&mut config.keys);
        let mut migrated_all = true;
        for (provider, key) in &legacy {
            let stored = matches!(vault.get(provider), Ok(Some(_)));
            if !stored && let Err(error) = vault.set(provider, key) {
                tracing::warn!(%provider, %error, "could not move a key into the credential store");
                migrated_all = false;
            }
        }
        if !legacy.is_empty() && migrated_all {
            if let Err(error) = write_file_sync(path, &config) {
                tracing::warn!(%error, "could not clear migrated keys from the config file");
            }
        } else {
            config.keys = legacy;
        }

        for provider in PROVIDERS {
            match vault.get(provider.id) {
                Ok(Some(key)) => {
                    config.keys.insert(provider.id.to_string(), key);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(provider = provider.id, %error, "could not read a key from the credential store");
                }
            }
        }
        config
    }

    pub async fn save(&self, config: &CopilotConfig) -> Result<(), std::io::Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut on_disk = config.clone();
        if let Some(vault) = &self.vault {
            // Whatever the credential store accepts leaves the file; whatever it
            // refuses stays in the file, because losing a key the user just typed
            // is worse than keeping it in the owner-only fallback.
            let mut refused = BTreeMap::new();
            for provider in PROVIDERS {
                match config
                    .keys
                    .get(provider.id)
                    .filter(|k| !k.trim().is_empty())
                {
                    Some(key) => {
                        if let Err(error) = vault.set(provider.id, key) {
                            tracing::warn!(provider = provider.id, %error, "credential store refused a key; keeping it in the config file");
                            refused.insert(provider.id.to_string(), key.clone());
                        }
                    }
                    None => {
                        let _ = vault.delete(provider.id);
                    }
                }
            }
            on_disk.keys = refused;
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let encoded = encode(&on_disk)?;
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

fn read_file(path: &Path) -> CopilotConfig {
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

fn encode(config: &CopilotConfig) -> Result<Vec<u8>, std::io::Error> {
    serde_json::to_vec_pretty(config)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn write_file_sync(path: &Path, config: &CopilotConfig) -> Result<(), std::io::Error> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, encode(config)?)?;
    restrict(&temp);
    std::fs::rename(&temp, path)
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
    use super::super::vault::MemoryVault;
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
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slyde-vault-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn keys_live_in_the_vault_and_never_in_the_file() {
        let dir = temp_dir("save");
        let vault = Arc::new(MemoryVault::default());
        let store = ConfigStore::with_vault(dir.join("copilot.json"), Some(vault.clone()));
        let mut config = CopilotConfig::default();
        config.keys.insert("deepseek".into(), "sk-secret".into());
        config.last_provider = Some("deepseek".into());
        store.save(&config).await.unwrap();

        let raw = std::fs::read_to_string(dir.join("copilot.json")).unwrap();
        assert!(
            !raw.contains("sk-secret"),
            "the file must hold no secret: {raw}"
        );
        assert!(
            raw.contains("deepseek"),
            "non-secret settings stay in the file"
        );
        assert_eq!(
            vault.snapshot().get("deepseek").map(String::as_str),
            Some("sk-secret")
        );
        assert_eq!(
            store.load().keys.get("deepseek").map(String::as_str),
            Some("sk-secret")
        );

        // Removing a key removes it from the vault too.
        config.keys.clear();
        store.save(&config).await.unwrap();
        assert!(vault.snapshot().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_file_keys_move_into_the_vault_on_load() {
        let dir = temp_dir("migrate");
        let path = dir.join("copilot.json");
        std::fs::write(
            &path,
            r#"{"keys":{"openai":"sk-old"},"last_provider":"openai","last_model":"m"}"#,
        )
        .unwrap();
        let vault = Arc::new(MemoryVault::default());
        let store = ConfigStore::with_vault(&path, Some(vault.clone()));

        let config = store.load();
        assert_eq!(
            config.keys.get("openai").map(String::as_str),
            Some("sk-old")
        );
        assert_eq!(config.last_model.as_deref(), Some("m"));
        assert_eq!(
            vault.snapshot().get("openai").map(String::as_str),
            Some("sk-old")
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("sk-old"),
            "migrated keys leave the file: {raw}"
        );
        assert!(raw.contains("last_provider"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_locked_vault_never_loses_the_key() {
        let dir = temp_dir("locked");
        let path = dir.join("copilot.json");
        let vault = Arc::new(MemoryVault::locked());
        let store = ConfigStore::with_vault(&path, Some(vault));
        let mut config = CopilotConfig::default();
        config.keys.insert("groq".into(), "gsk-typed".into());
        store.save(&config).await.unwrap();

        assert_eq!(
            store.load().keys.get("groq").map(String::as_str),
            Some("gsk-typed")
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("gsk-typed")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_memory_store_never_touches_disk() {
        let store = ConfigStore::in_memory();
        assert!(store.path().is_none());
        assert!(store.load().keys.is_empty());
    }
}
