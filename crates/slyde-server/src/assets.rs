//! Image asset store.
//!
//! Slides may only reference images from two places: a `data:` URL or the asset
//! store (`/assets/<id>.<ext>`), because the sandbox CSP allows `'self' data:
//! blob:` and nothing else. The store is what makes "paste a photo" work:
//!
//! - files live in `<app-data>/assets/`, content is never rewritten;
//! - ids are random and opaque, so a slide cannot address anything outside the
//!   store and a path traversal is impossible by construction;
//! - the same id survives an export/import round trip (the `.slyde` bundle
//!   carries the files), so a reopened deck keeps its images.
//!
//! Only raster formats are accepted. SVG is deliberately refused: an SVG is a
//! document that can carry script, and it would be served from the sandbox origin
//! where slide content lives.

use std::path::{Path, PathBuf};

use crate::error::ServerError;

/// Upload ceiling for one image. Larger than any sane slide image and small
/// enough that a paste cannot stall the server.
pub const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    pub id: String,
    /// Path the slide should use, e.g. `/assets/9f3a….png`.
    pub url: String,
    pub content_type: &'static str,
    pub bytes: usize,
}

#[derive(Debug)]
pub struct AssetStore {
    root: PathBuf,
}

impl AssetStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The store inside the OS app-data directory.
    pub fn from_app_data() -> Self {
        Self::new(crate::rendezvous::app_data_dir().join("assets"))
    }

    /// Scratch store for in-memory servers and tests: never the user's data.
    pub fn for_tests() -> Self {
        Self::new(std::env::temp_dir().join(format!("slyde-assets-scratch-{}", std::process::id())))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Saves raw bytes, naming the file after their detected format.
    pub fn save_bytes(&self, bytes: &[u8]) -> Result<AssetInfo, ServerError> {
        if bytes.is_empty() {
            return Err(ServerError::Deck("the image is empty".into()));
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ServerError::Deck(format!(
                "the image is {} bytes, over the {MAX_IMAGE_BYTES} byte limit",
                bytes.len()
            )));
        }
        let (extension, content_type) = sniff(bytes).ok_or_else(|| {
            ServerError::Deck("only PNG, JPEG, WebP and GIF images are supported".into())
        })?;

        std::fs::create_dir_all(&self.root)?;
        let id = format!("{}.{extension}", new_asset_id());
        std::fs::write(self.root.join(&id), bytes)?;
        Ok(AssetInfo {
            url: format!("/assets/{id}"),
            id,
            content_type,
            bytes: bytes.len(),
        })
    }

    /// Saves a `data:image/…;base64,…` URL, which is what the canvas sends when
    /// the user pastes an image.
    pub fn save_data_url(&self, url: &str) -> Result<AssetInfo, ServerError> {
        let (meta, payload) = url
            .split_once(',')
            .ok_or_else(|| ServerError::Deck("malformed data URL".into()))?;
        if !meta.starts_with("data:image/") || !meta.ends_with(";base64") {
            return Err(ServerError::Deck(
                "only base64 image data URLs can be stored".into(),
            ));
        }
        // 4 base64 chars per 3 bytes: reject an oversized payload before decoding.
        if payload.len() / 4 * 3 > MAX_IMAGE_BYTES + 3 {
            return Err(ServerError::Deck("the image is too large".into()));
        }
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
            .map_err(|_| ServerError::Deck("the image data is not valid base64".into()))?;
        self.save_bytes(&bytes)
    }

    /// Saves an asset under a known name, used when importing a `.slyde` bundle.
    ///
    /// The name comes from the bundle and must already be an asset id; anything
    /// else is ignored so a crafted file cannot write outside the store.
    pub fn restore(&self, name: &str, bytes: &[u8]) -> Result<(), ServerError> {
        if !is_asset_file_name(name) {
            return Err(ServerError::Deck(format!("invalid asset name: {name}")));
        }
        std::fs::create_dir_all(&self.root)?;
        std::fs::write(self.root.join(name), bytes)?;
        Ok(())
    }

    pub fn path_for(&self, name: &str) -> Option<PathBuf> {
        is_asset_file_name(name).then(|| self.root.join(name))
    }

    /// Every asset file name currently stored.
    pub fn list(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| is_asset_file_name(name))
            .collect()
    }
}

/// Parses `/assets/<name>` into `<name>`, rejecting anything else.
pub fn name_from_slide_url(url: &str) -> Option<&str> {
    let name = url.trim().strip_prefix("/assets/")?;
    is_asset_file_name(name).then_some(name)
}

fn is_asset_file_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    (8..=40).contains(&stem.len())
        && stem.chars().all(|ch| ch.is_ascii_hexdigit())
        && matches!(extension, "png" | "jpg" | "webp" | "gif")
}

fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(("png", "image/png"));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(("jpg", "image/jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("gif", "image/gif"));
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(("webp", "image/webp"));
    }
    None
}

fn new_asset_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&(nanos as u64).to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ];

    fn store(tag: &str) -> (AssetStore, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "slyde-assets-{tag}-{}-{}",
            std::process::id(),
            new_asset_id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        (AssetStore::new(&root), root)
    }

    #[test]
    fn saves_a_png_and_returns_a_usable_url() {
        let (store, root) = store("png");
        let info = store.save_bytes(PNG).unwrap();
        assert!(info.id.ends_with(".png"), "{}", info.id);
        assert_eq!(info.url, format!("/assets/{}", info.id));
        assert_eq!(info.content_type, "image/png");

        let path = store.path_for(&info.id).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), PNG);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_formats_and_oversized_payloads() {
        let (store, root) = store("reject");
        assert!(store.save_bytes(b"<svg onload=alert(1)>").is_err());
        assert!(store.save_bytes(b"").is_err());
        assert!(store.save_bytes(&[0u8; 8]).is_err());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_a_data_url_and_detects_the_real_format() {
        let (store, root) = store("dataurl");
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, PNG);
        let url = format!("data:image/png;base64,{encoded}");
        let info = store.save_data_url(&url).unwrap();
        assert_eq!(info.content_type, "image/png");

        // A mismatch between the declared mime and the bytes is caught by sniffing.
        let jpeg = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
        );
        let lying = format!("data:image/png;base64,{jpeg}");
        let info = store.save_data_url(&lying).unwrap();
        assert_eq!(info.content_type, "image/jpeg", "the bytes decide");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn asset_names_cannot_escape_the_store() {
        assert!(name_from_slide_url("/assets/0123456789abcdef.png").is_some());
        assert!(name_from_slide_url("/assets/../settings.json").is_none());
        assert!(name_from_slide_url("/assets/0123456789abcdef.svg").is_none());
        assert!(name_from_slide_url("/api/state").is_none());
        assert!(!is_asset_file_name("short.png"));

        let (store, root) = store("traversal");
        assert!(store.restore("../evil.png", PNG).is_err());
        assert!(store.path_for("../../etc/passwd").is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restoring_a_bundle_asset_keeps_its_id() {
        let (store, root) = store("restore");
        store.restore("0123456789abcdef.png", PNG).unwrap();
        assert_eq!(store.list(), vec!["0123456789abcdef.png".to_string()]);
        // Importing the same bundle twice is idempotent.
        store.restore("0123456789abcdef.png", PNG).unwrap();
        assert_eq!(store.list().len(), 1);

        let _ = std::fs::remove_dir_all(root);
    }
}
