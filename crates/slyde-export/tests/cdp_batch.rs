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

#[tokio::test]
async fn extracts_text_blocks_and_leaves_unreadable_content_in_the_picture() {
    let Some(chrome) = find_chrome() else {
        eprintln!("no Chromium found; skipping the text extraction test");
        return;
    };
    let html = r#"<!doctype html><html data-slyde-ready="1"><meta charset=utf-8>
      <body style="margin:0;background:#101018">
      <main id="slyde-stage" style="position:absolute;inset:0">
        <h1 style="position:absolute;left:80px;top:60px;margin:0;font:700 64px sans-serif;color:#ff6b4a">
          Hello <span style="color:#ffffff">editable</span> world</h1>
        <p style="position:absolute;left:80px;top:200px;margin:0;font:400 28px sans-serif;color:#ccc;text-align:center;width:900px">Second<br>line</p>
        <pre style="position:absolute;left:80px;top:400px;margin:0;color:#0f0">let kept = true;</pre>
      </main></body></html>"#;
    let url = format!("data:text/html;charset=utf-8,{}", urlencode(html));

    let dir = std::env::temp_dir().join(format!("slyde-cdp-text-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let slides = vec![SlideTarget {
        id: "sld_text".into(),
        url,
    }];
    let captured = slyde_export::export_slides_batch(
        &chrome,
        &slides,
        AspectRatio::R16x9,
        &dir,
        1,
        None,
        true,
    )
    .await
    .expect("the export should render and extract");

    let blocks = &captured[0].text;
    assert_eq!(
        blocks.len(),
        2,
        "the heading and the paragraph, not the <pre>: {blocks:?}"
    );
    let heading = &blocks[0];
    let plain: String = heading.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(plain.trim(), "Hello editable world");
    assert!(
        heading.runs.iter().any(|run| run.color == "ffffff"),
        "the span keeps its own color"
    );
    assert!(
        heading
            .runs
            .iter()
            .all(|run| run.bold && run.size_centipx == 6400)
    );
    assert_eq!(blocks[1].align, "ctr");
    assert!(
        blocks[1].runs.iter().any(|run| run.text == "\n"),
        "<br> becomes a line break"
    );
    assert!(blocks[1].wrap, "two lines are multi-line text");

    let png = std::fs::read(&captured[0].png).expect("png written");
    assert!(png_size(&png).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
