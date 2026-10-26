//! Integration check that the crate produces the deck the python-pptx
//! validator consumes.
//!
//! The test writes `target/tmp/pptx-check.pptx` (three 1280x720 slides, one of
//! them with speaker notes). From the repository root:
//!
//! ```text
//! cargo test -p slyde-pptx
//! python scripts/validate-pptx.py target/tmp/pptx-check.pptx --expect-slides 3
//! ```

#[path = "support/png.rs"]
mod png;

use std::path::PathBuf;

use slyde_pptx::{PptxDeck, PptxSlide, write_pptx};

/// The deck the python validator is pointed at.
pub const CHECK_DECK_TITLE: &str = "SlydeCode validation deck";
/// Slide size of the deck, in CSS pixels at 96 DPI.
pub const CHECK_DECK_SIZE: (u32, u32) = (1280, 720);

fn output_path() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.join("../../target/tmp/pptx-check.pptx")
}

#[test]
fn writes_the_deck_checked_by_the_python_validator() {
    let slides = vec![
        PptxSlide {
            title: "Cover: SlydeCode export".to_string(),
            png: png::solid(1280, 720, [0x1F, 0x24, 0x30]),
            notes: None,
            texts: Vec::new(),
        },
        PptxSlide {
            title: "Second slide, with notes".to_string(),
            png: png::solid(1280, 720, [0xE8, 0x5D, 0x3B]),
            notes: Some(
                "Speaker notes for the second slide.\nThey are exported as a notes page."
                    .to_string(),
            ),
            texts: Vec::new(),
        },
        PptxSlide {
            title: "Escaping < & > \"quoted\" 'single'".to_string(),
            png: png::solid(1280, 720, [0xF4, 0xF1, 0xEA]),
            notes: None,
            texts: Vec::new(),
        },
    ];
    let (width_px, height_px) = CHECK_DECK_SIZE;
    let deck = PptxDeck {
        title: CHECK_DECK_TITLE,
        width_px,
        height_px,
        slides: &slides,
    };

    let path = output_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create target/tmp");
    }
    write_pptx(&deck, &path).expect("write pptx");

    let written = std::fs::metadata(&path).expect("written deck exists");
    assert!(
        written.len() > 1024,
        "deck is suspiciously small: {} bytes",
        written.len()
    );
    assert_eq!(
        std::fs::read(&path).expect("read deck").first(),
        Some(&b'P'),
        "a .pptx is a ZIP, so it starts with the PK signature"
    );
}
