//! Integration test for the batch renderer: one Chrome session, several pages.
//!
//! It needs a real Chromium on the machine, which is the same requirement the
//! product has. When none is found the test explains itself and passes, so CI on
//! a machine without a browser does not fail for the wrong reason.

use std::path::PathBuf;

use slyde_core::AspectRatio;
use slyde_export::{SlideTarget, export_png_batch, export_slides_with_text};

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

/// A page shaped like the sandbox document: `#slyde-stage` plus the ready flag.
fn stage_page() -> String {
    let html = r##"<!doctype html><html data-slyde-ready="1"><meta charset=utf-8>
<body style="margin:0"><div id="slyde-stage" style="position:absolute;left:0;top:0;width:1280px;height:720px;background:#102030">
<h1 style="position:absolute;left:100px;top:80px;width:500px;margin:0;font:700 48px/60px Arial;color:#ff6b4a">Hello brave new world of slides</h1>
<p style="position:absolute;left:100px;top:400px;margin:0;font:400 24px/32px Arial;color:#ffffff">Plain <b style="color:#00ff00">green</b> text</p>
</div></body></html>"##;
    format!("data:text/html;charset=utf-8,{}", urlencode(html))
}

#[tokio::test]
async fn reads_the_text_and_returns_a_clean_plate() {
    let Some(chrome) = find_chrome() else {
        eprintln!("no Chromium found; skipping the text extraction test");
        return;
    };

    let dir = std::env::temp_dir().join(format!("slyde-cdp-text-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let slides = vec![SlideTarget {
        id: "sld_text".into(),
        url: stage_page(),
    }];

    let rendered = export_slides_with_text(&chrome, &slides, AspectRatio::R16x9, &dir, 1, None)
        .await
        .expect("the render should succeed");
    let json = rendered[0]
        .text_json
        .as_deref()
        .expect("text was extracted");
    let blocks: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
    let blocks = blocks.as_array().expect("an array of blocks");
    assert_eq!(blocks.len(), 2, "heading and paragraph: {json}");

    let heading = &blocks[0];
    let lines = heading["lines"].as_array().unwrap();
    assert!(
        lines.len() >= 2,
        "the heading wraps like the browser: {json}"
    );
    assert_eq!(lines[0][0]["color"], "FF6B4A");
    assert_eq!(lines[0][0]["bold"], true);
    assert!(
        (heading["x"].as_f64().unwrap() - 100.0).abs() < 3.0,
        "{json}"
    );
    let words: String = lines
        .iter()
        .map(|line| {
            line.as_array()
                .unwrap()
                .iter()
                .map(|run| run["text"].as_str().unwrap())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(words, "Hello brave new world of slides");

    let paragraph = &blocks[1];
    let runs = paragraph["lines"][0].as_array().unwrap();
    assert_eq!(runs.len(), 3, "plain, green bold, plain: {json}");
    assert_eq!(runs[1]["color"], "00FF00");
    assert_eq!(runs[1]["bold"], true);
    assert_eq!(paragraph["align"], "left");

    // The plate must differ from a normal capture of the same page.
    let plain = export_png_batch(
        &chrome,
        &slides,
        AspectRatio::R16x9,
        &dir.join("plain"),
        1,
        None,
    )
    .await
    .expect("plain render");
    assert_ne!(
        std::fs::read(&rendered[0].png).unwrap(),
        std::fs::read(&plain[0]).unwrap(),
        "the plate has the text hidden"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
