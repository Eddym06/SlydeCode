//! Slide and notes-slide parts, plus their relationship files.

use super::empty_group;
use crate::PptxSlide;
use crate::rels::{self, Relationships};
use crate::xml;

/// `ppt/slides/slideN.xml`: one full-bleed picture and nothing else.
///
/// The picture frame starts at the origin and uses the slide size, so the PNG
/// covers the slide exactly while `a:stretch` keeps the caller's aspect ratio.
pub(crate) fn slide_xml(slide: &PptxSlide, width_emu: i64, height_emu: i64) -> String {
    let title = xml::escape(&slide.title);
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<p:sld xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\">",
            xml::NS_A,
            xml::NS_R,
            xml::NS_P
        ),
    );
    xml::line(&mut out, 1, &format!("<p:cSld name=\"{title}\">"));
    xml::line(&mut out, 2, "<p:spTree>");
    empty_group(&mut out, 3);
    xml::line(&mut out, 3, "<p:pic>");
    xml::line(&mut out, 4, "<p:nvPicPr>");
    xml::line(
        &mut out,
        5,
        &format!("<p:cNvPr id=\"2\" name=\"Slide image\" descr=\"{title}\"/>"),
    );
    xml::line(
        &mut out,
        5,
        "<p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr>",
    );
    xml::line(&mut out, 5, "<p:nvPr/>");
    xml::line(&mut out, 4, "</p:nvPicPr>");
    xml::line(&mut out, 4, "<p:blipFill>");
    xml::line(&mut out, 5, "<a:blip r:embed=\"rId2\"/>");
    xml::line(&mut out, 5, "<a:stretch><a:fillRect/></a:stretch>");
    xml::line(&mut out, 4, "</p:blipFill>");
    xml::line(&mut out, 4, "<p:spPr>");
    xml::line(&mut out, 5, "<a:xfrm>");
    xml::line(&mut out, 6, "<a:off x=\"0\" y=\"0\"/>");
    xml::line(
        &mut out,
        6,
        &format!("<a:ext cx=\"{width_emu}\" cy=\"{height_emu}\"/>"),
    );
    xml::line(&mut out, 5, "</a:xfrm>");
    xml::line(
        &mut out,
        5,
        "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
    );
    xml::line(&mut out, 4, "</p:spPr>");
    xml::line(&mut out, 3, "</p:pic>");
    crate::text::shapes_xml(&slide.text, 3, &mut out, 3);
    xml::line(&mut out, 2, "</p:spTree>");
    xml::line(&mut out, 1, "</p:cSld>");
    xml::line(
        &mut out,
        1,
        "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>",
    );
    xml::line(&mut out, 0, "</p:sld>");
    out
}

/// `ppt/slides/_rels/slideN.xml.rels`: layout, image and optionally notes.
pub(crate) fn slide_rels_xml(slide_number: usize, has_notes: bool) -> String {
    let mut rels = Relationships::new();
    rels.add(rels::REL_SLIDE_LAYOUT, "../slideLayouts/slideLayout1.xml");
    rels.add(rels::REL_IMAGE, format!("../media/image{slide_number}.png"));
    if has_notes {
        rels.add(
            rels::REL_NOTES_SLIDE,
            format!("../notesSlides/notesSlide{slide_number}.xml"),
        );
    }
    rels.to_xml()
}

/// `ppt/notesSlides/notesSlideN.xml`: one body placeholder with the notes text.
pub(crate) fn notes_slide_xml(notes: &str) -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<p:notes xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\">",
            xml::NS_A,
            xml::NS_R,
            xml::NS_P
        ),
    );
    xml::line(&mut out, 1, "<p:cSld>");
    xml::line(&mut out, 2, "<p:spTree>");
    empty_group(&mut out, 3);
    xml::line(&mut out, 3, "<p:sp>");
    xml::line(&mut out, 4, "<p:nvSpPr>");
    xml::line(
        &mut out,
        5,
        "<p:cNvPr id=\"2\" name=\"Notes Placeholder 1\"/>",
    );
    xml::line(
        &mut out,
        5,
        "<p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>",
    );
    xml::line(
        &mut out,
        5,
        "<p:nvPr><p:ph type=\"body\" idx=\"3\"/></p:nvPr>",
    );
    xml::line(&mut out, 4, "</p:nvSpPr>");
    xml::line(&mut out, 4, "<p:spPr/>");
    xml::line(&mut out, 4, "<p:txBody>");
    xml::line(&mut out, 5, "<a:bodyPr/>");
    xml::line(&mut out, 5, "<a:lstStyle/>");
    let mut wrote_paragraph = false;
    for text in notes.lines() {
        wrote_paragraph = true;
        xml::line(
            &mut out,
            5,
            &format!(
                "<a:p><a:r><a:rPr lang=\"en-US\" dirty=\"0\"/><a:t>{}</a:t></a:r></a:p>",
                xml::escape(text)
            ),
        );
    }
    if !wrote_paragraph {
        xml::line(&mut out, 5, "<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
    }
    xml::line(&mut out, 4, "</p:txBody>");
    xml::line(&mut out, 3, "</p:sp>");
    xml::line(&mut out, 2, "</p:spTree>");
    xml::line(&mut out, 1, "</p:cSld>");
    xml::line(
        &mut out,
        1,
        "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>",
    );
    xml::line(&mut out, 0, "</p:notes>");
    out
}

/// `ppt/notesSlides/_rels/notesSlideN.xml.rels`: the owning slide and the
/// notes master, in the order PowerPoint itself writes them.
pub(crate) fn notes_slide_rels_xml(slide_number: usize) -> String {
    let mut rels = Relationships::new();
    rels.add(
        rels::REL_SLIDE,
        format!("../slides/slide{slide_number}.xml"),
    );
    rels.add(rels::REL_NOTES_MASTER, "../notesMasters/notesMaster1.xml");
    rels.to_xml()
}
