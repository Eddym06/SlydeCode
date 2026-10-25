//! Writes PowerPoint `.pptx` decks without an OOXML library.
//!
//! SlydeCode renders every slide with Chrome and treats the rendered PNG as the
//! source of truth. The deck this crate writes is therefore one full-bleed
//! picture per slide: visual fidelity is guaranteed by the renderer, the writer
//! never has to model shapes, and the package is still a regular OOXML
//! presentation that PowerPoint, Keynote and LibreOffice open and annotate.
//!
//! The layout of the package:
//!
//! ```text
//! [Content_Types].xml
//! _rels/.rels                     officeDocument + core properties
//! docProps/core.xml               deck title + created timestamp
//! ppt/presentation.xml            slide size (EMU), slide order
//! ppt/_rels/presentation.xml.rels
//! ppt/theme/theme1.xml
//! ppt/slideMasters/slideMaster1.xml        (+ .rels)
//! ppt/slideLayouts/slideLayout1.xml        (+ .rels, type="blank")
//! ppt/slides/slideN.xml                    (+ .rels: layout, image, notes)
//! ppt/media/imageN.png                     raw renderer bytes, never re-encoded
//! ppt/notesSlides/notesSlideN.xml          only when a slide has notes
//! ppt/notesMasters/notesMaster1.xml        (+ .rels, theme2.xml)
//! ```
//!
//! Nothing in this crate panics on input: titles and notes are XML-escaped,
//! every failure is a [`PptxError`], and the archive is assembled in memory
//! before the output file is touched, so a failed write cannot leave a
//! half-written deck behind.

mod parts;
mod rels;
mod theme;
mod xml;

#[cfg(test)]
mod tests;

use std::io::{Cursor, Seek, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// English Metric Units per CSS pixel at the 96 DPI the renderer uses.
pub const EMU_PER_PIXEL: u32 = 9525;

/// Converts a CSS pixel length to English Metric Units.
pub fn px_to_emu(px: u32) -> i64 {
    i64::from(px) * i64::from(EMU_PER_PIXEL)
}

/// One slide: the rendered picture plus the text that goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PptxSlide {
    /// Slide title. Stored as the slide name and the picture alt text.
    pub title: String,
    /// Raw `image/png` bytes from the renderer. Never decoded or re-encoded.
    pub png: Vec<u8>,
    /// Speaker notes, when the deck has them.
    pub notes: Option<String>,
}

/// A whole deck, borrowed from the caller for the duration of the write.
#[derive(Debug, Clone, Copy)]
pub struct PptxDeck<'a> {
    /// Deck title, written to `docProps/core.xml`.
    pub title: &'a str,
    /// Slide width in CSS pixels at 96 DPI.
    pub width_px: u32,
    /// Slide height in CSS pixels at 96 DPI.
    pub height_px: u32,
    /// Slides in presentation order.
    pub slides: &'a [PptxSlide],
}

/// Everything that can stop a deck from being written.
#[derive(Debug, thiserror::Error)]
pub enum PptxError {
    /// A presentation must contain at least one slide.
    #[error("the deck has no slides to write")]
    Empty,
    /// A zero-sized slide produces an invalid `p:sldSz`.
    #[error("the slide size must be positive, got {width_px}x{height_px} px")]
    SlideSize {
        /// Requested width in pixels.
        width_px: u32,
        /// Requested height in pixels.
        height_px: u32,
    },
    /// The renderer output is not something a PNG part can hold.
    #[error("slide {slide} has an unusable image: {reason}")]
    Image {
        /// One-based slide number, so the message points at the deck.
        slide: usize,
        /// Why the bytes were rejected.
        reason: &'static str,
    },
    /// The archive or the output file could not be written.
    #[error("could not write {path}: {source}")]
    Io {
        /// Part name or output path involved in the failure.
        path: String,
        /// Underlying filesystem or writer error.
        source: std::io::Error,
    },
    /// The ZIP layer rejected the archive.
    #[error("could not build the pptx archive: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// Writes `deck` to `out` as a `.pptx` file.
///
/// The caller keeps ownership of the rendered pictures; this function only
/// reads them. On error nothing is written to `out`.
pub fn write_pptx(deck: &PptxDeck<'_>, out: &Path) -> Result<(), PptxError> {
    let bytes = build_package(deck)?;
    std::fs::write(out, bytes).map_err(|source| PptxError::Io {
        path: out.display().to_string(),
        source,
    })
}

/// Builds the whole archive in memory.
///
/// Split from [`write_pptx`] so tests can read the parts back without touching
/// the filesystem, and so an error can never leave a partial file behind.
pub(crate) fn build_package(deck: &PptxDeck<'_>) -> Result<Vec<u8>, PptxError> {
    if deck.slides.is_empty() {
        return Err(PptxError::Empty);
    }
    if deck.width_px == 0 || deck.height_px == 0 {
        return Err(PptxError::SlideSize {
            width_px: deck.width_px,
            height_px: deck.height_px,
        });
    }
    for (index, slide) in deck.slides.iter().enumerate() {
        validate_png(index + 1, &slide.png)?;
    }

    let width_emu = px_to_emu(deck.width_px);
    let height_emu = px_to_emu(deck.height_px);
    let notes: Vec<bool> = deck
        .slides
        .iter()
        .map(|slide| slide.notes.is_some())
        .collect();
    let has_notes = notes.iter().any(|present| *present);
    let slide_count = deck.slides.len();

    // Presentation relationships decide the ids used by `p:sldId` and the
    // master lists, so they are built first and printed from the same list.
    let mut presentation_rels = rels::Relationships::new();
    let slide_master_r_id =
        presentation_rels.add(rels::REL_SLIDE_MASTER, "slideMasters/slideMaster1.xml");
    let slide_r_ids: Vec<String> = (1..=slide_count)
        .map(|number| presentation_rels.add(rels::REL_SLIDE, format!("slides/slide{number}.xml")))
        .collect();
    let notes_master_r_id = has_notes
        .then(|| presentation_rels.add(rels::REL_NOTES_MASTER, "notesMasters/notesMaster1.xml"));
    presentation_rels.add(rels::REL_THEME, "theme/theme1.xml");

    let text_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    // PNGs are already compressed; storing them keeps the write near-instant
    // and leaves the renderer bytes untouched.
    let media_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);

    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    add_part(
        &mut archive,
        "[Content_Types].xml",
        parts::content_types_xml(&notes).as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "_rels/.rels",
        parts::root_rels_xml().as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "docProps/core.xml",
        parts::core_properties_xml(deck.title, &xml::now_utc_iso8601()).as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/presentation.xml",
        parts::presentation_xml(
            width_emu,
            height_emu,
            &slide_master_r_id,
            &slide_r_ids,
            notes_master_r_id.as_deref(),
        )
        .as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/_rels/presentation.xml.rels",
        presentation_rels.to_xml().as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/theme/theme1.xml",
        parts::theme_part_xml().as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/slideMasters/slideMaster1.xml",
        parts::slide_master_xml().as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        parts::slide_master_rels_xml().as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/slideLayouts/slideLayout1.xml",
        parts::slide_layout_xml().as_bytes(),
        text_options,
    )?;
    add_part(
        &mut archive,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        parts::slide_layout_rels_xml().as_bytes(),
        text_options,
    )?;

    if has_notes {
        add_part(
            &mut archive,
            "ppt/theme/theme2.xml",
            parts::theme_part_xml().as_bytes(),
            text_options,
        )?;
        add_part(
            &mut archive,
            "ppt/notesMasters/notesMaster1.xml",
            parts::notes_master_xml().as_bytes(),
            text_options,
        )?;
        add_part(
            &mut archive,
            "ppt/notesMasters/_rels/notesMaster1.xml.rels",
            parts::notes_master_rels_xml().as_bytes(),
            text_options,
        )?;
    }

    for (index, slide) in deck.slides.iter().enumerate() {
        let number = index + 1;
        add_part(
            &mut archive,
            &format!("ppt/slides/slide{number}.xml"),
            parts::slide_xml(slide, width_emu, height_emu).as_bytes(),
            text_options,
        )?;
        add_part(
            &mut archive,
            &format!("ppt/slides/_rels/slide{number}.xml.rels"),
            parts::slide_rels_xml(number, notes[index]).as_bytes(),
            text_options,
        )?;
        add_part(
            &mut archive,
            &format!("ppt/media/image{number}.png"),
            &slide.png,
            media_options,
        )?;
        if let Some(notes_text) = slide.notes.as_deref() {
            add_part(
                &mut archive,
                &format!("ppt/notesSlides/notesSlide{number}.xml"),
                parts::notes_slide_xml(notes_text).as_bytes(),
                text_options,
            )?;
            add_part(
                &mut archive,
                &format!("ppt/notesSlides/_rels/notesSlide{number}.xml.rels"),
                parts::notes_slide_rels_xml(number).as_bytes(),
                text_options,
            )?;
        }
    }

    let cursor = archive.finish()?;
    Ok(cursor.into_inner())
}

/// Rejects bytes that no PNG part can hold.
///
/// The signature check is deliberately shallow: SlydeCode embeds whatever the
/// renderer produced, and a full decode would mean a decoder dependency. The
/// check still catches the realistic mistake of handing the writer a JPEG, a
/// WebP or an empty buffer.
fn validate_png(slide: usize, png: &[u8]) -> Result<(), PptxError> {
    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if png.is_empty() {
        return Err(PptxError::Image {
            slide,
            reason: "the rendered image is empty",
        });
    }
    if !png.starts_with(&PNG_SIGNATURE) {
        return Err(PptxError::Image {
            slide,
            reason: "the rendered image is not a PNG (missing PNG signature)",
        });
    }
    Ok(())
}

fn add_part<W: Write + Seek>(
    archive: &mut ZipWriter<W>,
    name: &str,
    bytes: &[u8],
    options: SimpleFileOptions,
) -> Result<(), PptxError> {
    archive.start_file(name, options)?;
    archive.write_all(bytes).map_err(|source| PptxError::Io {
        path: name.to_string(),
        source,
    })?;
    Ok(())
}
