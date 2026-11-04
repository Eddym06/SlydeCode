//! Turns an image source an agent names into a stored asset URL.
//!
//! Slides must render offline and travel inside a `.slyde` bundle, so a remote
//! link cannot be written into the slide as-is. This module is the one gate that
//! accepts what an agent or the UI can provide:
//!
//! - `/assets/<name>` must already exist in the store;
//! - a base64 `data:image/…` URL is decoded and stored;
//! - an `http(s)` link is downloaded through the SSRF-safe fetcher and stored.
//!
//! The result is always `/assets/<id>.<ext>`, the only remote form the sandbox
//! can serve from its own origin.

use crate::assets::name_from_slide_url;
use crate::state::AppState;

/// Resolves `src` into an asset-store URL, saving remote data when needed.
pub async fn resolve(state: &AppState, src: &str) -> Result<String, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err("an image source is required".into());
    }

    if let Some(name) = name_from_slide_url(src) {
        let exists = state
            .assets
            .path_for(name)
            .is_some_and(|path| path.is_file());
        if !exists {
            return Err(format!(
                "the deck has no stored image named {name}; upload it first or pass an https link"
            ));
        }
        return Ok(format!("/assets/{name}"));
    }

    if src.starts_with("data:image/") {
        return state
            .assets
            .save_data_url(src)
            .map(|info| info.url)
            .map_err(|error| error.to_string());
    }

    if src.starts_with("https://") || src.starts_with("http://") {
        let bytes = crate::copilot::webfetch::fetch_image(src)
            .await
            .map_err(|error| format!("could not download the image: {error}"))?;
        return state
            .assets
            .save_bytes(&bytes)
            .map(|info| info.url)
            .map_err(|error| error.to_string());
    }

    Err("images must be a stored /assets/ URL, a base64 image data URL, or an http(s) link".into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use slyde_core::{AspectRatio, PresentationState, Theme};

    use super::*;

    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUg==";

    fn state_with_store(tag: &str) -> (Arc<AppState>, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "slyde-intake-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let core = PresentationState::new("Images", Theme::studio(), AspectRatio::R16x9);
        let state = AppState::with_copilot(
            core,
            "tok".into(),
            std::path::PathBuf::from("ui"),
            vec![],
            "http://127.0.0.1:1".into(),
            Arc::new(crate::copilot::CopilotState::in_memory()),
            crate::state::Persistence {
                assets: Arc::new(crate::assets::AssetStore::new(&root)),
                ..crate::state::Persistence::memory()
            },
        );
        (state, root)
    }

    #[tokio::test]
    async fn a_data_url_is_stored_and_returned_as_an_asset_path() {
        let (state, root) = state_with_store("data");
        let url = resolve(&state, &format!("data:image/png;base64,{PNG_B64}"))
            .await
            .unwrap();
        assert!(url.starts_with("/assets/"), "{url}");
        assert!(url.ends_with(".png"), "{url}");
        let name = url.strip_prefix("/assets/").unwrap();
        assert!(state.assets.path_for(name).unwrap().is_file());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn an_existing_asset_is_accepted_and_a_missing_one_is_named() {
        let (state, root) = state_with_store("existing");
        let stored = state
            .assets
            .save_bytes(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
            .unwrap();
        assert_eq!(resolve(&state, &stored.url).await.unwrap(), stored.url);

        let error = resolve(&state, "/assets/0123456789abcdef.png")
            .await
            .unwrap_err();
        assert!(error.contains("0123456789abcdef.png"), "{error}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unsafe_or_unsupported_sources_are_refused_before_any_fetch() {
        let (state, root) = state_with_store("refuse");
        for src in [
            "",
            "ftp://example.com/a.png",
            "not a link",
            "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=",
            "http://127.0.0.1/photo.png",
        ] {
            assert!(resolve(&state, src).await.is_err(), "{src} must be refused");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
