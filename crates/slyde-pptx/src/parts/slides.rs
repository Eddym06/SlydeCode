//! Slide and notes-slide parts, plus their relationship files.

use super::empty_group;
use crate::rels::{self, Relationships};
use crate::xml;
use crate::{PptxAlign, PptxSlide, PptxText};

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
    for (index, text) in slide.texts.iter().enumerate() {
        text_shape(&mut out, 3, index + 3, text);
    }
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

/// One editable text box. Shape ids start at 3: 1 is the group, 2 the picture.
fn text_shape(out: &mut String, depth: usize, id: usize, text: &PptxText) {
    let x = crate::px_to_emu_signed(text.x_px);
    let y = crate::px_to_emu_signed(text.y_px);
    let w = crate::px_to_emu(text.w_px);
    let h = crate::px_to_emu(text.h_px);
    let wrap = if text.wrap { "square" } else { "none" };
    xml::line(out, depth, "<p:sp>");
    xml::line(out, depth + 1, "<p:nvSpPr>");
    xml::line(
        out,
        depth + 2,
        &format!("<p:cNvPr id=\"{id}\" name=\"Text {}\"/>", id - 2),
    );
    xml::line(out, depth + 2, "<p:cNvSpPr txBox=\"1\"/>");
    xml::line(out, depth + 2, "<p:nvPr/>");
    xml::line(out, depth + 1, "</p:nvSpPr>");
    xml::line(out, depth + 1, "<p:spPr>");
    xml::line(out, depth + 2, "<a:xfrm>");
    xml::line(out, depth + 3, &format!("<a:off x=\"{x}\" y=\"{y}\"/>"));
    xml::line(out, depth + 3, &format!("<a:ext cx=\"{w}\" cy=\"{h}\"/>"));
    xml::line(out, depth + 2, "</a:xfrm>");
    xml::line(
        out,
        depth + 2,
        "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
    );
    xml::line(out, depth + 2, "<a:noFill/>");
    xml::line(out, depth + 1, "</p:spPr>");
    xml::line(out, depth + 1, "<p:txBody>");
    xml::line(
        out,
        depth + 2,
        &format!(
            "<a:bodyPr wrap=\"{wrap}\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"t\"><a:noAutofit/></a:bodyPr>"
        ),
    );
    xml::line(out, depth + 2, "<a:lstStyle/>");
    let algn = match text.align {
        PptxAlign::Left => "l",
        PptxAlign::Center => "ctr",
        PptxAlign::Right => "r",
    };
    // Split runs into paragraphs at every newline.
    let mut paragraphs: Vec<Vec<(&crate::PptxRun, &str)>> = vec![Vec::new()];
    for run in &text.runs {
        for (position, piece) in run.text.split('\n').enumerate() {
            if position > 0 {
                paragraphs.push(Vec::new());
            }
            if !piece.is_empty() {
                if let Some(current) = paragraphs.last_mut() {
                    current.push((run, piece));
                }
            }
        }
    }
    for paragraph in &paragraphs {
        let spacing = u32::from(text.line_spacing_pct.clamp(50, 300)) * 1000;
        let mut line = format!(
            "<a:p><a:pPr algn=\"{algn}\"><a:lnSpc><a:spcPct val=\"{spacing}\"/></a:lnSpc></a:pPr>"
        );
        for (run, piece) in paragraph {
            let size = (u64::from(run.size_centipx) * 3 / 4).clamp(100, 400_000);
            let alpha = if run.opacity < 100 {
                format!("<a:alpha val=\"{}\"/>", u32::from(run.opacity) * 1000)
            } else {
                String::new()
            };
            line.push_str(&format!(
                "<a:r><a:rPr lang=\"en-US\" sz=\"{size}\" b=\"{}\" i=\"{}\" dirty=\"0\"><a:solidFill><a:srgbClr val=\"{}\">{alpha}</a:srgbClr></a:solidFill><a:latin typeface=\"{}\"/></a:rPr><a:t>{}</a:t></a:r>",
                u8::from(run.bold),
                u8::from(run.italic),
                hex_or_black(&run.color),
                xml::escape(&run.font),
                xml::escape(piece)
            ));
        }
        line.push_str("</a:p>");
        xml::line(out, depth + 2, &line);
    }
    xml::line(out, depth + 1, "</p:txBody>");
    xml::line(out, depth, "</p:sp>");
}

/// Only six hex digits may reach an attribute.
fn hex_or_black(raw: &str) -> String {
    if raw.len() == 6 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        raw.to_ascii_uppercase()
    } else {
        "000000".to_string()
    }
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
