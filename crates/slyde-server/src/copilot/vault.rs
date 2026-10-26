//! Where provider API keys live.
//!
//! On Windows and macOS the OS credential store holds them (Credential Manager,
//! Keychain), so the app-data directory never contains a readable secret. Linux
//! keeps the owner-only file: the kernel keyring the pure-Rust backend would use
//! does not survive a reboot, and the Secret Service backend needs `libdbus`,
//! which is not worth a system dependency for a local tool.
//!
//! The vault is a trait so the migration and fallback rules are tested without
//! touching the real credential store.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Mutex;

/// A per-user secret store keyed by provider id.
pub trait Vault: Debug + Send + Sync {
    fn get(&self, provider: &str) -> Result<Option<String>, String>;
    fn set(&self, provider: &str, key: &str) -> Result<(), String>;
    fn delete(&self, provider: &str) -> Result<(), String>;
}

/// The OS credential store, where one is supported.
pub fn system_vault() -> Option<Box<dyn Vault>> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        Some(Box::new(os::KeychainVault))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod os {
    use super::Vault;

    const SERVICE: &str = "SlydeCode";

    #[derive(Debug)]
    pub struct KeychainVault;

    fn entry(provider: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, &format!("copilot:{provider}")).map_err(|e| e.to_string())
    }

    impl Vault for KeychainVault {
        fn get(&self, provider: &str) -> Result<Option<String>, String> {
            match entry(provider)?.get_password() {
                Ok(key) => Ok(Some(key)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        }

        fn set(&self, provider: &str, key: &str) -> Result<(), String> {
            entry(provider)?
                .set_password(key)
                .map_err(|e| e.to_string())
        }

        fn delete(&self, provider: &str) -> Result<(), String> {
            match entry(provider)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        }
    }
}

/// In-memory vault for tests; `fail_writes` simulates a locked credential store.
#[derive(Debug, Default)]
pub struct MemoryVault {
    entries: Mutex<BTreeMap<String, String>>,
    pub fail_writes: bool,
}

impl MemoryVault {
    /// A vault that refuses every write, like a locked credential store.
    pub fn locked() -> Self {
        Self {
            entries: Mutex::default(),
            fail_writes: true,
        }
    }

    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.entries.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

impl Vault for MemoryVault {
    fn get(&self, provider: &str) -> Result<Option<String>, String> {
        let entries = self.entries.lock().map_err(|e| e.to_string())?;
        Ok(entries.get(provider).cloned())
    }

    fn set(&self, provider: &str, key: &str) -> Result<(), String> {
        if self.fail_writes {
            return Err("the credential store is locked".into());
        }
        let mut entries = self.entries.lock().map_err(|e| e.to_string())?;
        entries.insert(provider.to_string(), key.to_string());
        Ok(())
    }

    fn delete(&self, provider: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().map_err(|e| e.to_string())?;
        entries.remove(provider);
        Ok(())
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod system_tests {
    use super::*;

    /// Touches the real credential store, so it only runs on request:
    /// `cargo test -p slyde-server system_vault -- --ignored`.
    #[test]
    #[ignore = "writes to the user's real credential store"]
    fn system_vault_round_trips_a_secret() {
        let vault = system_vault().expect("this platform has a credential store");
        let id = format!("selftest-{}", std::process::id());
        assert_eq!(vault.get(&id).unwrap(), None);
        vault.set(&id, "sk-roundtrip").unwrap();
        assert_eq!(vault.get(&id).unwrap().as_deref(), Some("sk-roundtrip"));
        vault.delete(&id).unwrap();
        assert_eq!(vault.get(&id).unwrap(), None);
        vault.delete(&id).unwrap();
    }
}
