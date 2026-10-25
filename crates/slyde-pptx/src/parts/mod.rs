//! Builders for the package-level parts and the shared part helpers.
//!
//! The slide, layout and notes builders live in `masters` and `slides`; this
//! module owns `[Content_Types].xml`, the root relationships, core properties
//! and `presentation.xml`, plus the two helpers every part uses.
//!
//! User text enters only through [`crate::xml::escape`]; nothing here can panic
//! on deck content.

mod masters;
mod slides;

pub(crate) use masters::{
    notes_master_rels_xml, notes_master_xml, slide_layout_rels_xml, slide_layout_xml,
    slide_master_rels_xml, slide_master_xml,
};
pub(crate) use slides::{notes_slide_rels_xml, notes_slide_xml, slide_rels_xml, slide_xml};

use crate::rels::{self, Relationships};
use crate::theme::theme_xml;
use crate::xml;

pub(crate) const CT_PRESENTATION: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
pub(crate) const CT_SLIDE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
pub(crate) const CT_SLIDE_MASTER: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml";
pub(crate) const CT_SLIDE_LAYOUT: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
pub(crate) const CT_NOTES_SLIDE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
pub(crate) const CT_NOTES_MASTER: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml";
pub(crate) const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
pub(crate) const CT_CORE_PROPERTIES: &str =
    "application/vnd.openxmlformats-package.core-properties+xml";
pub(crate) const CT_RELATIONSHIPS: &str =
    "application/vnd.openxmlformats-package.relationships+xml";
pub(crate) const CT_XML: &str = "application/xml";
pub(crate) const CT_PNG: &str = "image/png";

/// `p:notesSz` is the paper size for notes pages; the renderer never prints
/// them, so the conventional 4:3 value is enough.
const NOTES_PAGE_WIDTH_EMU: i64 = 6_858_000;
const NOTES_PAGE_HEIGHT_EMU: i64 = 9_144_000;

/// First id used for `p:sldId`; the schema requires every id to be at least 256.
const FIRST_SLIDE_ID: u32 = 256;
/// Fixed id of the single slide master, matching PowerPoint's own numbering.
const SLIDE_MASTER_ID: u32 = 2_147_483_648;
/// Fixed id of the single layout inside that master.
const SLIDE_LAYOUT_ID: u32 = 2_147_483_649;

/// `[Content_Types].xml` for a deck with `notes.len() == slide_count`.
pub(crate) fn content_types_xml(notes: &[bool]) -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!("<Types xmlns=\"{}\">", xml::NS_CONTENT_TYPES),
    );
    default_type(&mut out, "rels", CT_RELATIONSHIPS);
    default_type(&mut out, "xml", CT_XML);
    default_type(&mut out, "png", CT_PNG);
    override_part(&mut out, "/ppt/presentation.xml", CT_PRESENTATION);
    override_part(&mut out, "/docProps/core.xml", CT_CORE_PROPERTIES);
    override_part(
        &mut out,
        "/ppt/slideMasters/slideMaster1.xml",
        CT_SLIDE_MASTER,
    );
    override_part(
        &mut out,
        "/ppt/slideLayouts/slideLayout1.xml",
        CT_SLIDE_LAYOUT,
    );
    override_part(&mut out, "/ppt/theme/theme1.xml", CT_THEME);
    for (index, has_notes) in notes.iter().enumerate() {
        let number = index + 1;
        override_part(
            &mut out,
            &format!("/ppt/slides/slide{number}.xml"),
            CT_SLIDE,
        );
        if *has_notes {
            override_part(
                &mut out,
                &format!("/ppt/notesSlides/notesSlide{number}.xml"),
                CT_NOTES_SLIDE,
            );
        }
    }
    if notes.iter().any(|has_notes| *has_notes) {
        override_part(
            &mut out,
            "/ppt/notesMasters/notesMaster1.xml",
            CT_NOTES_MASTER,
        );
        override_part(&mut out, "/ppt/theme/theme2.xml", CT_THEME);
    }
    xml::line(&mut out, 0, "</Types>");
    out
}

fn default_type(out: &mut String, extension: &str, content_type: &str) {
    xml::line(
        out,
        1,
        &format!("<Default Extension=\"{extension}\" ContentType=\"{content_type}\"/>"),
    );
}

fn override_part(out: &mut String, part: &str, content_type: &str) {
    xml::line(
        out,
        1,
        &format!("<Override PartName=\"{part}\" ContentType=\"{content_type}\"/>"),
    );
}

/// Package-level relationships: the main document plus core properties.
pub(crate) fn root_rels_xml() -> String {
    let mut rels = Relationships::new();
    rels.add(rels::REL_OFFICE_DOCUMENT, "ppt/presentation.xml");
    rels.add(rels::REL_CORE_PROPERTIES, "docProps/core.xml");
    rels.to_xml()
}

/// `docProps/core.xml`, the only part allowed to carry a timestamp.
pub(crate) fn core_properties_xml(title: &str, created: &str) -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<cp:coreProperties xmlns:cp=\"{}\" xmlns:dc=\"{}\" xmlns:dcterms=\"{}\" xmlns:dcmitype=\"{}\" xmlns:xsi=\"{}\">",
            xml::NS_CORE_PROPERTIES,
            xml::NS_DC,
            xml::NS_DCTERMS,
            xml::NS_DCMITYPE,
            xml::NS_XSI
        ),
    );
    xml::line(
        &mut out,
        1,
        &format!("<dc:title>{}</dc:title>", xml::escape(title)),
    );
    xml::line(&mut out, 1, "<dc:creator>SlydeCode</dc:creator>");
    xml::line(
        &mut out,
        1,
        "<cp:lastModifiedBy>SlydeCode</cp:lastModifiedBy>",
    );
    xml::line(
        &mut out,
        1,
        &format!("<dcterms:created xsi:type=\"dcterms:W3CDTF\">{created}</dcterms:created>"),
    );
    xml::line(
        &mut out,
        1,
        &format!("<dcterms:modified xsi:type=\"dcterms:W3CDTF\">{created}</dcterms:modified>"),
    );
    xml::line(&mut out, 0, "</cp:coreProperties>");
    out
}

/// `ppt/presentation.xml`: slide size, slide order and the master lists.
pub(crate) fn presentation_xml(
    width_emu: i64,
    height_emu: i64,
    slide_master_r_id: &str,
    slide_r_ids: &[String],
    notes_master_r_id: Option<&str>,
) -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<p:presentation xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\">",
            xml::NS_A,
            xml::NS_R,
            xml::NS_P
        ),
    );
    xml::line(&mut out, 1, "<p:sldMasterIdLst>");
    xml::line(
        &mut out,
        2,
        &format!("<p:sldMasterId id=\"{SLIDE_MASTER_ID}\" r:id=\"{slide_master_r_id}\"/>"),
    );
    xml::line(&mut out, 1, "</p:sldMasterIdLst>");
    if let Some(notes_master_r_id) = notes_master_r_id {
        xml::line(&mut out, 1, "<p:notesMasterIdLst>");
        xml::line(
            &mut out,
            2,
            &format!("<p:notesMasterId r:id=\"{notes_master_r_id}\"/>"),
        );
        xml::line(&mut out, 1, "</p:notesMasterIdLst>");
    }
    xml::line(&mut out, 1, "<p:sldIdLst>");
    for (index, r_id) in slide_r_ids.iter().enumerate() {
        let id = FIRST_SLIDE_ID + index as u32;
        xml::line(
            &mut out,
            2,
            &format!("<p:sldId id=\"{id}\" r:id=\"{r_id}\"/>"),
        );
    }
    xml::line(&mut out, 1, "</p:sldIdLst>");
    xml::line(
        &mut out,
        1,
        &format!("<p:sldSz cx=\"{width_emu}\" cy=\"{height_emu}\"/>"),
    );
    xml::line(
        &mut out,
        1,
        &format!("<p:notesSz cx=\"{NOTES_PAGE_WIDTH_EMU}\" cy=\"{NOTES_PAGE_HEIGHT_EMU}\"/>"),
    );
    xml::line(&mut out, 1, "<p:defaultTextStyle>");
    xml::line(
        &mut out,
        2,
        "<a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr>",
    );
    xml::line(&mut out, 1, "</p:defaultTextStyle>");
    xml::line(&mut out, 0, "</p:presentation>");
    out
}

/// The empty `p:nvGrpSpPr`/`p:grpSpPr` pair every shape tree starts with.
pub(super) fn empty_group(out: &mut String, depth: usize) {
    xml::line(out, depth, "<p:nvGrpSpPr>");
    xml::line(out, depth + 1, "<p:cNvPr id=\"1\" name=\"\"/>");
    xml::line(out, depth + 1, "<p:cNvGrpSpPr/>");
    xml::line(out, depth + 1, "<p:nvPr/>");
    xml::line(out, depth, "</p:nvGrpSpPr>");
    xml::line(out, depth, "<p:grpSpPr>");
    xml::line(out, depth + 1, "<a:xfrm>");
    xml::line(out, depth + 2, "<a:off x=\"0\" y=\"0\"/>");
    xml::line(out, depth + 2, "<a:ext cx=\"0\" cy=\"0\"/>");
    xml::line(out, depth + 2, "<a:chOff x=\"0\" y=\"0\"/>");
    xml::line(out, depth + 2, "<a:chExt cx=\"0\" cy=\"0\"/>");
    xml::line(out, depth + 1, "</a:xfrm>");
    xml::line(out, depth, "</p:grpSpPr>");
}

/// The theme is shared by both masters; this keeps `lib.rs` from knowing which
/// module owns it.
pub(crate) fn theme_part_xml() -> String {
    theme_xml()
}
