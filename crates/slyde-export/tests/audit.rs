//! Integration test for the layout audit, against a real browser.
//!
//! It needs a real Chromium on the machine, like the product. When none is found
//! the test explains itself and passes, so CI without a browser does not fail for
//! the wrong reason.

use std::path::PathBuf;

use slyde_core::AspectRatio;
use slyde_export::{SlideTarget, audit_slides};

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

/// A slide document shaped like the studio's: a 1280x720 `#slyde-stage`, already
/// marked ready so the audit does not wait for renderers that are not there.
fn slide(id: &str, inner: &str) -> SlideTarget {
    let html = format!(
        "<!doctype html><html data-slyde-ready=\"1\"><meta charset=utf-8>\
         <body style=\"margin:0\"><main id=\"slyde-stage\" style=\"position:relative;\
         width:1280px;height:720px;overflow:hidden;background:#17161a;color:#f7f4ef;\
         font:32px sans-serif\">{inner}</main></body></html>"
    );
    SlideTarget {
        id: id.to_string(),
        url: format!("data:text/html;charset=utf-8,{}", urlencode(&html)),
    }
}

fn kinds(audit: &slyde_export::SlideAudit) -> Vec<&str> {
    audit
        .issues
        .iter()
        .map(|issue| issue.kind.as_str())
        .collect()
}

#[tokio::test]
async fn measures_what_a_model_cannot_see() {
    let Some(chrome) = find_chrome() else {
        eprintln!("no Chromium found; skipping the audit integration test");
        return;
    };

    let clean = slide(
        "clean",
        "<h1 style=\"position:absolute;left:96px;top:120px;margin:0;font-size:72px\">A clear headline</h1>\
         <p style=\"position:absolute;left:96px;top:280px;margin:0;font-size:28px;max-width:900px\">One supporting sentence with room to breathe.</p>",
    );
    let broken = slide(
        "broken",
        "<h1 data-id=\"title\" style=\"position:absolute;left:1000px;top:100px;margin:0;white-space:nowrap;font-size:80px\">Overflowing headline</h1>\
         <p style=\"position:absolute;left:96px;top:300px;margin:0;font-size:10px\">tiny text</p>\
         <p style=\"position:absolute;left:96px;top:380px;margin:0;font-size:30px;color:#3a3a40\">Low contrast on dark</p>\
         <div style=\"position:absolute;left:96px;top:480px;width:200px;height:34px;overflow:hidden;white-space:nowrap;font-size:28px\">A very long line that gets clipped by its box</div>\
         <p style=\"position:absolute;left:640px;top:300px;margin:0\">Overlap A</p>\
         <p style=\"position:absolute;left:650px;top:306px;margin:0\">Overlap B</p>",
    );
    let empty = slide("empty", "");

    let audits = audit_slides(&chrome, &[clean, broken, empty], AspectRatio::R16x9)
        .await
        .expect("the audit runs");
    assert_eq!(audits.len(), 3, "one result per slide, in order");

    assert!(
        audits[0].issues.is_empty(),
        "a well-made slide has no findings: {:?}",
        audits[0].issues
    );

    let found = kinds(&audits[1]);
    for expected in ["overflow", "clipped", "overlap", "contrast", "small"] {
        assert!(
            found.contains(&expected),
            "missing {expected}: {:?}",
            audits[1].issues
        );
    }
    let overflow = audits[1]
        .issues
        .iter()
        .find(|issue| issue.kind == "overflow")
        .unwrap();
    assert!(overflow.element.contains("h1#title"), "{overflow:?}");
    assert!(overflow.detail.contains("right"), "{overflow:?}");
    // Worst problems come first.
    assert_eq!(found[0], "overflow");

    assert_eq!(kinds(&audits[2]), vec!["empty"]);
}

#[tokio::test]
async fn a_deck_without_slides_costs_no_browser() {
    // No Chrome is needed (or started) for nothing to audit.
    let audits = audit_slides(
        std::path::Path::new("definitely-not-a-browser"),
        &[],
        AspectRatio::R16x9,
    )
    .await
    .unwrap();
    assert!(audits.is_empty());
}
