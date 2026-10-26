//! Unit tests for the writer.
//!
//! The tests read the produced archive back, check every required part, walk
//! the XML with a tiny stack-based well-formedness checker (deliberately not a
//! parser dependency) and pin the geometry rules: slide size in EMU, full-bleed
//! picture frame, unique slide ids.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::xml;
use crate::{
    EMU_PER_PIXEL, PptxAlign, PptxDeck, PptxError, PptxRun, PptxSlide, PptxText, build_package,
};

#[path = "../tests/support/png.rs"]
mod png;

const WIDTH_PX: u32 = 1280;
const HEIGHT_PX: u32 = 720;

fn slide(title: &str) -> PptxSlide {
    PptxSlide {
        title: title.to_string(),
        png: png::solid(16, 9, [0x1F, 0x24, 0x30]),
        notes: None,
        texts: Vec::new(),
    }
}

fn notes_slide(title: &str, notes: &str) -> PptxSlide {
    PptxSlide {
        title: title.to_string(),
        png: png::solid(16, 9, [0xE8, 0x5D, 0x3B]),
        notes: Some(notes.to_string()),
        texts: Vec::new(),
    }
}

fn deck<'a>(slides: &'a [PptxSlide]) -> PptxDeck<'a> {
    PptxDeck {
        title: "SlydeCode test deck",
        width_px: WIDTH_PX,
        height_px: HEIGHT_PX,
        slides,
    }
}

fn package(slides: &[PptxSlide]) -> BTreeMap<String, Vec<u8>> {
    let bytes = build_package(&deck(slides)).expect("the package builds");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("the package is a ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry can be read");
        let name = entry.name().to_string();
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer).expect("entry bytes");
        entries.insert(name, buffer);
    }
    entries
}

fn text(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    let bytes = parts
        .get(name)
        .unwrap_or_else(|| panic!("part {name} is missing"));
    String::from_utf8(bytes.clone())
        .unwrap_or_else(|error| panic!("part {name} is not UTF-8: {error}"))
}

// ---------------------------------------------------------------------------
// 1. Every required part exists and holds the expected content.
// ---------------------------------------------------------------------------

#[test]
fn writes_every_required_part() {
    let slides = [slide("Cover"), slide("Second")];
    let parts = package(&slides);
    let required = [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "ppt/presentation.xml",
        "ppt/_rels/presentation.xml.rels",
        "ppt/theme/theme1.xml",
        "ppt/slideMasters/slideMaster1.xml",
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        "ppt/slideLayouts/slideLayout1.xml",
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        "ppt/slides/slide1.xml",
        "ppt/slides/_rels/slide1.xml.rels",
        "ppt/slides/slide2.xml",
        "ppt/slides/_rels/slide2.xml.rels",
        "ppt/media/image1.png",
        "ppt/media/image2.png",
    ];
    for name in required {
        assert!(parts.contains_key(name), "missing required part {name}");
    }
    assert!(
        !parts.keys().any(|name| name.contains("notesSlide")),
        "notes parts must not appear when no slide has notes"
    );
    assert!(
        !parts.keys().any(|name| name.contains("notesMaster")),
        "the notes master must not appear when no slide has notes"
    );
}

#[test]
fn content_types_cover_png_and_presentation() {
    let parts = package(&[slide("Cover")]);
    let content_types = text(&parts, "[Content_Types].xml");
    assert!(content_types.contains("<Default Extension=\"png\" ContentType=\"image/png\"/>"));
    assert!(content_types.contains("<Default Extension=\"rels\""));
    assert!(content_types.contains("<Default Extension=\"xml\""));
    assert!(content_types.contains(
        "/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\""
    ));
    assert!(content_types.contains("presentationml.slide+xml"));
    assert!(content_types.contains("presentationml.slideMaster+xml"));
    assert!(content_types.contains("presentationml.slideLayout+xml"));
    assert!(content_types.contains("openxmlformats-officedocument.theme+xml"));
}

#[test]
fn one_slide_part_and_one_media_part_per_slide() {
    let slides = [slide("One"), slide("Two"), slide("Three")];
    let parts = package(&slides);
    let slide_parts: Vec<&String> = parts
        .keys()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .collect();
    assert_eq!(slide_parts.len(), slides.len());
    let media_parts: Vec<&String> = parts
        .keys()
        .filter(|name| name.starts_with("ppt/media/image") && name.ends_with(".png"))
        .collect();
    assert_eq!(media_parts.len(), slides.len());

    for (index, slide) in slides.iter().enumerate() {
        let number = index + 1;
        assert_eq!(
            parts
                .get(&format!("ppt/media/image{number}.png"))
                .expect("media part"),
            &slide.png,
            "the PNG bytes must be embedded untouched"
        );
        let slide_xml = text(&parts, &format!("ppt/slides/slide{number}.xml"));
        assert!(
            slide_xml.contains("<p:pic>"),
            "slide {number} has no picture"
        );
        assert_eq!(
            slide_xml.matches("<p:pic>").count(),
            1,
            "slide {number} must hold exactly one picture"
        );
        assert!(slide_xml.contains("<a:blip r:embed=\"rId2\"/>"));
        let rels = text(&parts, &format!("ppt/slides/_rels/slide{number}.xml.rels"));
        assert!(rels.contains("Target=\"../media/image"));
    }
}

// ---------------------------------------------------------------------------
// 2. Every XML part is well formed, checked without a parser dependency.
// ---------------------------------------------------------------------------

#[test]
fn every_xml_part_is_well_formed() {
    let slides = [
        slide("Cover"),
        notes_slide("Notes & <escapes>", "Line one & line two\nSecond <line>"),
    ];
    let parts = package(&slides);
    let mut checked = 0;
    for (name, bytes) in &parts {
        if !(name.ends_with(".xml") || name.ends_with(".rels")) {
            continue;
        }
        let document = std::str::from_utf8(bytes).expect("XML parts are UTF-8");
        assert_well_formed(name, document);
        checked += 1;
    }
    assert!(
        checked >= 12,
        "expected to check every XML part, saw {checked}"
    );
}

#[test]
#[should_panic]
fn well_formedness_checker_rejects_unbalanced_tags() {
    assert_well_formed("broken.xml", "<a><b></a>");
}

#[test]
#[should_panic]
fn well_formedness_checker_rejects_bare_ampersands() {
    assert_well_formed("broken.xml", "<a>R&D</a>");
}

// ---------------------------------------------------------------------------
// 3. Errors and escaping.
// ---------------------------------------------------------------------------

#[test]
fn empty_deck_is_rejected() {
    let result = build_package(&deck(&[]));
    assert!(matches!(result, Err(PptxError::Empty)));
}

#[test]
fn zero_slide_size_is_rejected() {
    let slides = [slide("Cover")];
    let zero_width = PptxDeck {
        title: "Zero",
        width_px: 0,
        height_px: HEIGHT_PX,
        slides: &slides,
    };
    assert!(matches!(
        build_package(&zero_width),
        Err(PptxError::SlideSize { width_px: 0, .. })
    ));
}

#[test]
fn non_png_bytes_are_rejected() {
    let mut broken = slide("Cover");
    broken.png = b"GIF89a not a png".to_vec();
    assert!(matches!(
        build_package(&deck(&[broken])),
        Err(PptxError::Image { slide: 1, .. })
    ));

    let mut empty_image = slide("Cover");
    empty_image.png = Vec::new();
    assert!(matches!(
        build_package(&deck(&[empty_image])),
        Err(PptxError::Image { slide: 1, .. })
    ));
}

#[test]
fn title_characters_are_escaped() {
    let title = "R&D <b> \"quoted\" & 'single'";
    let parts = package(&[slide(title)]);
    let slide_xml = text(&parts, "ppt/slides/slide1.xml");
    assert!(
        slide_xml.contains("R&amp;D &lt;b&gt; &quot;quoted&quot; &amp; &apos;single&apos;"),
        "escaped title missing from slide XML"
    );
    assert!(!slide_xml.contains("<b>"), "raw markup leaked into the XML");
    assert_well_formed("slide1.xml", &slide_xml);

    let core = text(&parts, "docProps/core.xml");
    assert!(core.contains("SlydeCode test deck"));
}

#[test]
fn editable_text_becomes_real_text_boxes_with_escaped_content() {
    let mut with_text = slide("Editable");
    with_text.texts = vec![PptxText {
        x_px: 96,
        y_px: -4,
        w_px: 640,
        h_px: 120,
        align: PptxAlign::Center,
        wrap: true,
        line_spacing_pct: 82,
        runs: vec![
            PptxRun {
                text: "R&D <plan>".to_string(),
                color: "ff6b4a".to_string(),
                opacity: 60,
                size_centipx: 4800,
                bold: true,
                italic: false,
                font: "Geist".to_string(),
            },
            PptxRun {
                text: "\nSecond \"line\"".to_string(),
                color: "not-a-color".to_string(),
                opacity: 100,
                size_centipx: 2000,
                bold: false,
                italic: true,
                font: "A&B".to_string(),
            },
        ],
    }];
    let parts = package(&[with_text]);
    let slide_xml = text(&parts, "ppt/slides/slide1.xml");
    assert_well_formed("slide1.xml", &slide_xml);
    assert!(slide_xml.contains("txBox=\"1\""));
    assert!(slide_xml.contains("R&amp;D &lt;plan&gt;"));
    assert!(slide_xml.contains("algn=\"ctr\""));
    assert!(
        slide_xml.contains("sz=\"3600\""),
        "48px is 36pt: {slide_xml}"
    );
    assert!(slide_xml.contains("<a:alpha val=\"60000\"/>"));
    assert!(slide_xml.contains("<a:spcPct val=\"82000\"/>"));
    assert!(slide_xml.contains("srgbClr val=\"FF6B4A\""));
    assert!(
        slide_xml.contains("srgbClr val=\"000000\""),
        "an invalid color must never reach the XML"
    );
    assert_eq!(
        slide_xml.matches("<a:p>").count(),
        2,
        "a newline starts a paragraph"
    );
    assert!(
        slide_xml.contains("y=\"-38100\""),
        "negative offsets survive"
    );
}

#[test]
fn notes_are_escaped_and_split_into_paragraphs() {
    let parts = package(&[notes_slide(
        "Cover",
        "Line one & <tag>\nSecond line with \"quotes\"",
    )]);
    let notes_xml = text(&parts, "ppt/notesSlides/notesSlide1.xml");
    assert_well_formed("notesSlide1.xml", &notes_xml);
    assert!(notes_xml.contains("Line one &amp; &lt;tag&gt;"));
    assert!(notes_xml.contains("Second line with &quot;quotes&quot;"));
    assert_eq!(notes_xml.matches("<a:p>").count(), 2);
    assert!(notes_xml.contains("<p:ph type=\"body\" idx=\"3\"/>"));
}

// ---------------------------------------------------------------------------
// 4. Geometry: the picture covers the slide exactly, in EMU.
// ---------------------------------------------------------------------------

#[test]
fn picture_extent_matches_the_slide_size_in_emu() {
    let parts = package(&[slide("Cover")]);
    let width_emu = i64::from(WIDTH_PX) * i64::from(EMU_PER_PIXEL);
    let height_emu = i64::from(HEIGHT_PX) * i64::from(EMU_PER_PIXEL);
    assert_eq!(width_emu, 12_192_000);
    assert_eq!(height_emu, 6_858_000);

    let presentation = text(&parts, "ppt/presentation.xml");
    assert!(presentation.contains(&format!(
        "<p:sldSz cx=\"{width_emu}\" cy=\"{height_emu}\"/>"
    )));

    let slide_xml = text(&parts, "ppt/slides/slide1.xml");
    assert!(slide_xml.contains("<a:off x=\"0\" y=\"0\"/>"));
    assert!(slide_xml.contains(&format!("<a:ext cx=\"{width_emu}\" cy=\"{height_emu}\"/>")));
    assert!(slide_xml.contains("<a:stretch><a:fillRect/></a:stretch>"));
}

#[test]
fn slide_ids_are_unique_and_start_at_256() {
    let slides = [slide("One"), slide("Two"), slide("Three")];
    let parts = package(&slides);
    let presentation = text(&parts, "ppt/presentation.xml");
    let ids: Vec<u32> = presentation
        .match_indices("<p:sldId id=\"")
        .map(|(index, marker)| {
            let rest = &presentation[index + marker.len()..];
            let end = rest.find('"').expect("closing quote");
            rest[..end].parse().expect("numeric slide id")
        })
        .collect();
    assert_eq!(ids, vec![256, 257, 258]);
    let unique: std::collections::BTreeSet<u32> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "slide ids must be unique");
    assert!(ids.iter().all(|id| *id >= 256));
}

// ---------------------------------------------------------------------------
// Notes parts are only written when they are needed.
// ---------------------------------------------------------------------------

#[test]
fn notes_add_the_notes_slide_master_and_relationship() {
    let slides = [notes_slide("Cover", "Hello notes"), slide("Two")];
    let parts = package(&slides);
    for name in [
        "ppt/notesSlides/notesSlide1.xml",
        "ppt/notesSlides/_rels/notesSlide1.xml.rels",
        "ppt/notesMasters/notesMaster1.xml",
        "ppt/notesMasters/_rels/notesMaster1.xml.rels",
        "ppt/theme/theme2.xml",
    ] {
        assert!(parts.contains_key(name), "missing notes part {name}");
    }
    assert!(
        !parts.contains_key("ppt/notesSlides/notesSlide2.xml"),
        "a slide without notes must not get a notes part"
    );
    let content_types = text(&parts, "[Content_Types].xml");
    assert!(content_types.contains("presentationml.notesSlide+xml"));
    assert!(content_types.contains("presentationml.notesMaster+xml"));
    assert!(!content_types.contains("notesSlide2.xml"));

    let presentation = text(&parts, "ppt/presentation.xml");
    assert!(presentation.contains("<p:notesMasterIdLst>"));
    assert!(presentation.contains("<p:notesMasterId r:id=\"rId"));
    let slide_rels = text(&parts, "ppt/slides/_rels/slide1.xml.rels");
    assert!(slide_rels.contains("notesSlide1.xml"));
    let notes_rels = text(&parts, "ppt/notesSlides/_rels/notesSlide1.xml.rels");
    assert!(notes_rels.contains("../slides/slide1.xml"));
    assert!(notes_rels.contains("../notesMasters/notesMaster1.xml"));
}

// ---------------------------------------------------------------------------
// Timestamp formatting.
// ---------------------------------------------------------------------------

#[test]
fn utc_timestamps_use_the_w3cdtf_shape() {
    assert_eq!(xml::iso8601_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(xml::iso8601_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(xml::iso8601_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    assert_eq!(xml::iso8601_utc(1_719_792_000), "2024-07-01T00:00:00Z");
}

// ---------------------------------------------------------------------------
// The stack-based well-formedness checker used by the tests above.
// ---------------------------------------------------------------------------

/// Panics with `name` context when `document` is not well-formed XML.
///
/// This is intentionally a checker, not a parser: it validates entity
/// references, tag nesting, matching names and a single root element — enough
/// to catch an unescaped `&`, a mismatched tag or a truncated part, which are
/// the realistic ways hand-written XML breaks.
fn assert_well_formed(name: &str, document: &str) {
    assert_entities(name, document);

    let bytes = document.as_bytes();
    let mut index = 0;
    let mut stack: Vec<String> = Vec::new();
    let mut roots = 0;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let mut cursor = index + 1;
        let mut quote: Option<u8> = None;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            match quote {
                Some(active) if byte == active => quote = None,
                Some(_) => {}
                None if byte == b'"' || byte == b'\'' => quote = Some(byte),
                None if byte == b'>' => break,
                None => {}
            }
            cursor += 1;
        }
        assert!(
            cursor < bytes.len(),
            "{name}: unterminated tag starting at byte {index}"
        );
        let raw = &document[index + 1..cursor];
        if let Some(closing) = raw.strip_prefix('/') {
            let expected = closing.trim();
            let opened = stack.pop();
            assert_eq!(
                opened.as_deref(),
                Some(expected),
                "{name}: closing </{expected}> does not match the open element"
            );
        } else if !(raw.starts_with('?') || raw.starts_with('!')) {
            let self_closing = raw.ends_with('/');
            let body = if self_closing {
                &raw[..raw.len() - 1]
            } else {
                raw
            };
            let tag = body.split_whitespace().next().unwrap_or_default();
            assert!(!tag.is_empty(), "{name}: tag without a name");
            if stack.is_empty() {
                roots += 1;
            }
            if !self_closing {
                stack.push(tag.to_string());
            }
        }
        index = cursor + 1;
    }

    assert!(stack.is_empty(), "{name}: unclosed elements {stack:?}");
    assert_eq!(roots, 1, "{name}: expected exactly one root element");
}

/// Fails on any `&` that is not a predefined or numeric character reference.
fn assert_entities(name: &str, document: &str) {
    let bytes = document.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'&' {
            continue;
        }
        let rest = &document[index..];
        let end = rest
            .find(';')
            .unwrap_or_else(|| panic!("{name}: entity at byte {index} is not terminated"));
        let entity = &rest[1..end];
        let known = matches!(entity, "amp" | "lt" | "gt" | "quot" | "apos")
            || (entity.starts_with('#') && entity.len() > 1);
        assert!(known, "{name}: unknown entity &{entity};");
    }
}
