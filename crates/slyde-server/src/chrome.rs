//! Browser discovery and launch.
//!
//! SlydeCode renders in the user's own Chrome (see MEMORY.md). We never bundle a
//! web engine: we find an installed Chromium-family browser and open a dedicated
//! app window with its own profile, so the user's main browser session is left
//! untouched.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::error::ServerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserKind {
    Chrome,
    Chromium,
    Edge,
    Brave,
}

#[derive(Debug, Clone)]
pub struct Browser {
    pub path: PathBuf,
    pub kind: BrowserKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Chromeless app window: no tabs, no address bar.
    App,
    /// Fullscreen kiosk, used for presenting.
    Kiosk,
}

impl Browser {
    /// Opens `url` in a dedicated profile. Returns the child process so the
    /// caller can decide whether to wait or detach.
    pub fn launch(
        &self,
        url: &str,
        mode: OpenMode,
        profile_dir: &Path,
    ) -> Result<Child, ServerError> {
        std::fs::create_dir_all(profile_dir)?;

        let mut cmd = Command::new(&self.path);
        cmd.arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-features=Translate,MediaRouter")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--metrics-recording-only");

        match mode {
            OpenMode::App => {
                cmd.arg("--window-size=1440,920")
                    .arg(format!("--app={url}"));
            }
            OpenMode::Kiosk => {
                cmd.arg("--kiosk").arg(url);
            }
        }

        #[cfg(target_os = "linux")]
        {
            cmd.arg("--class=SlydeCode");
        }

        let child = cmd.spawn()?;
        Ok(child)
    }
}

/// Finds an installed Chromium-family browser.
///
/// Order: `SLYDE_BROWSER` override, Google Chrome, Chromium, Microsoft Edge,
/// Brave. Edge is a last resort: it is Chromium, so rendering is identical.
pub fn discover_browser() -> Result<Browser, ServerError> {
    if let Ok(custom) = std::env::var("SLYDE_BROWSER") {
        let path = PathBuf::from(custom);
        if path.is_file() {
            return Ok(Browser {
                path,
                kind: BrowserKind::Chrome,
            });
        }
    }

    for candidate in candidates() {
        if candidate.path.is_file() {
            return Ok(candidate);
        }
    }

    for (bin, kind) in path_candidates() {
        if let Some(path) = find_in_path(bin) {
            return Ok(Browser { path, kind });
        }
    }

    Err(ServerError::BrowserNotFound)
}

fn candidates() -> Vec<Browser> {
    #[cfg(target_os = "windows")]
    let out = windows_candidates();

    #[cfg(target_os = "macos")]
    let out = macos_candidates();

    #[cfg(target_os = "linux")]
    let out = linux_candidates();

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let out = Vec::new();

    out
}

/// Well-known Windows install locations, resolved from the standard env vars.
#[cfg(target_os = "windows")]
fn windows_candidates() -> Vec<Browser> {
    let mut out = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            roots.push(PathBuf::from(base));
        }
    }
    for root in roots {
        for (rel, kind) in [
            ("Google/Chrome/Application/chrome.exe", BrowserKind::Chrome),
            ("Chromium/Application/chrome.exe", BrowserKind::Chromium),
            ("Microsoft/Edge/Application/msedge.exe", BrowserKind::Edge),
            (
                "BraveSoftware/Brave-Browser/Application/brave.exe",
                BrowserKind::Brave,
            ),
        ] {
            out.push(Browser {
                path: root.join(rel),
                kind,
            });
        }
    }
    out
}

/// The four supported browsers as installed by a normal macOS app install.
#[cfg(target_os = "macos")]
fn macos_candidates() -> Vec<Browser> {
    let mut out = Vec::new();
    for (path, kind) in [
        (
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            BrowserKind::Chrome,
        ),
        (
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            BrowserKind::Chromium,
        ),
        (
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            BrowserKind::Edge,
        ),
        (
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            BrowserKind::Brave,
        ),
    ] {
        out.push(Browser {
            path: PathBuf::from(path),
            kind,
        });
    }
    out
}

/// Absolute paths for distro, snap and flatpak installs.
///
/// PATH lookups (see [`path_candidates`]) already cover the usual `/usr/bin`
/// case; these extra locations matter because snap and flatpak wrappers are not
/// always on the PATH a GUI-launched process inherits.
#[cfg(target_os = "linux")]
fn linux_candidates() -> Vec<Browser> {
    let mut out = Vec::new();
    for (path, kind) in [
        ("/opt/google/chrome/chrome", BrowserKind::Chrome),
        ("/snap/bin/chromium", BrowserKind::Chromium),
        (
            "/var/lib/flatpak/exports/bin/com.google.Chrome",
            BrowserKind::Chrome,
        ),
        (
            "/var/lib/flatpak/exports/bin/org.chromium.Chromium",
            BrowserKind::Chromium,
        ),
        (
            "/var/lib/flatpak/exports/bin/com.microsoft.Edge",
            BrowserKind::Edge,
        ),
        (
            "/var/lib/flatpak/exports/bin/com.brave.Browser",
            BrowserKind::Brave,
        ),
    ] {
        out.push(Browser {
            path: PathBuf::from(path),
            kind,
        });
    }
    out
}

fn path_candidates() -> Vec<(&'static str, BrowserKind)> {
    vec![
        ("google-chrome", BrowserKind::Chrome),
        ("google-chrome-stable", BrowserKind::Chrome),
        ("chromium", BrowserKind::Chromium),
        ("chromium-browser", BrowserKind::Chromium),
        ("microsoft-edge", BrowserKind::Edge),
        ("brave-browser", BrowserKind::Brave),
    ]
}

#[cfg(target_os = "linux")]
fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
}

#[cfg(not(target_os = "linux"))]
fn find_in_path(_bin: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_declares_candidate_paths() {
        // A platform whose candidate list is empty would silently fall back to a
        // PATH-only lookup, which misses snap/flatpak/app-bundle installs.
        assert!(
            !candidates().is_empty(),
            "each supported OS must contribute candidate paths"
        );
    }

    #[test]
    fn discovery_returns_an_existing_binary_or_a_clear_error() {
        match discover_browser() {
            Ok(browser) => assert!(browser.path.is_file(), "path must exist"),
            Err(err) => assert!(matches!(err, ServerError::BrowserNotFound)),
        }
    }
}
