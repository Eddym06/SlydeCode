//! Integration test for the batch renderer: one Chrome session, several pages.
//!
//! It needs a real Chromium on the machine, which is the same requirement the
//! product has. When none is found the test explains itself and passes, so CI on
//! a machine without a browser does not fail for the wrong reason.

use std::path::PathBuf;

use slyde_core::AspectRatio;
use slyde_export::{SlideTarget, export_png_batch};

fn find_chrome() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("SLYDE_BROWSER") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }
    let candidates = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| {
            let local = std::env::var("LOCALAPPDATA").ok()?;
            let path = PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe");
            path.is_file().then_some(path)
        })
}

fn page(color: &str) -> String {
    // A data: URL keeps the test independent of the studio server.
    let html = format!(
        "<!doctype html><meta charset=utf-8><body style=\"margin:0;background:{color}\"></body>"
    );
    format!("data:text/html;charset=utf-8,{}", urlencode(&html))
}

fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Width and height from the PNG IHDR chunk.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

#[tokio::test]
async fn renders_every_slide_in_a_single_chrome_session() {
    let Some(chrome) = find_chrome() else {
        eprintln!("no Chromium found; skipping the batch renderer test");
        return;
    };

    let dir = std::env::temp_dir().join(format!("slyde-cdp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let slides = vec![
        SlideTarget {
            id: "sld_a".into(),
            url: page("#ff0000"),
        },
        SlideTarget {
            id: "sld_b".into(),
            url: page("#0000ff"),
        },
    ];

    let files = export_png_batch(&chrome, &slides, AspectRatio::R16x9, &dir, 2, None)
        .await
        .expect("the batch export should render both slides");
    assert_eq!(files.len(), 2, "one file per slide");

    let first = std::fs::read(&files[0]).expect("the first slide was written");
    let (width, height) = png_size(&first).expect("a real PNG");
    assert_eq!(
        (width, height),
        (2560, 1440),
        "the capture respects the 2x device scale factor and the 16:9 canvas"
    );

    let second = std::fs::read(&files[1]).expect("the second slide was written");
    assert_ne!(
        first, second,
        "each capture is the slide that was navigated to"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
