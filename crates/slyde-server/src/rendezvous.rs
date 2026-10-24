//! Single-instance coordination.
//!
//! Starting SlydeCode twice must not spawn a second server: the second launch
//! finds the running instance through `runtime.json` in the OS app-data
//! directory, checks `/health`, and simply opens another window against it.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub port: u16,
    pub sandbox_port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at_ms: u64,
}

impl RuntimeInfo {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }

    pub fn present_url(&self) -> String {
        format!("http://127.0.0.1:{}/present?t={}", self.port, self.token)
    }
}

/// `%APPDATA%\SlydeCode`, `~/Library/Application Support/SlydeCode`, or
/// `$XDG_DATA_HOME/slydecode`.
pub fn app_data_dir() -> PathBuf {
    // Explicit override: test runners and power users get an isolated profile
    // (runtime file, decks, Chrome profile) without touching the real one.
    if let Ok(dir) = std::env::var("SLYDE_DATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("SlydeCode");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Library/Application Support/SlydeCode");
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(data) = std::env::var("XDG_DATA_HOME") {
            if !data.is_empty() {
                return PathBuf::from(data).join("slydecode");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local/share/slydecode");
        }
    }

    std::env::temp_dir().join("slydecode")
}

pub fn runtime_path() -> PathBuf {
    app_data_dir().join("runtime.json")
}

pub fn read_runtime() -> Option<RuntimeInfo> {
    let raw = std::fs::read_to_string(runtime_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_runtime(info: &RuntimeInfo) -> std::io::Result<()> {
    let dir = app_data_dir();
    std::fs::create_dir_all(&dir)?;
    let path = runtime_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(info)?)?;
    std::fs::rename(tmp, path)
}

pub fn clear_runtime() {
    let _ = std::fs::remove_file(runtime_path());
}

/// Minimal HTTP probe: a full HTTP client is not worth a dependency for one
/// health check.
pub async fn probe_health(port: u16, timeout: Duration) -> bool {
    let attempt = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        stream
            .write_all(
                b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .await
            .ok()?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.ok()?;
        let text = String::from_utf8_lossy(&buf);
        Some(text.starts_with("HTTP/1.1 200"))
    };

    matches!(tokio::time::timeout(timeout, attempt).await, Ok(Some(true)))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_is_absolute_and_named() {
        let dir = app_data_dir();
        assert!(dir.is_absolute());
        let name = dir.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
        assert!(name.contains("slyde"));
    }

    #[test]
    fn runtime_info_round_trips() {
        let info = RuntimeInfo {
            port: 1234,
            sandbox_port: 1235,
            token: "abc".into(),
            pid: 99,
            started_at_ms: 1,
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: RuntimeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.port, 1234);
        assert!(back.url().contains("t=abc"));
    }

    #[tokio::test]
    async fn probe_fails_fast_on_a_closed_port() {
        // Port 1 is reserved and never bound by this process.
        assert!(!probe_health(1, Duration::from_millis(200)).await);
    }
}
