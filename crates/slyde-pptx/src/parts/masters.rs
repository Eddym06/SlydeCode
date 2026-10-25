//! Slide master, blank layout and notes master parts.

use super::SLIDE_LAYOUT_ID;
use super::empty_group;
use crate::rels::{self, Relationships};
use crate::xml;

/// `ppt/slideMasters/slideMaster1.xml`: an intentionally empty master.
pub(crate) fn slide_master_xml() -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<p:sldMaster xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\">",
            xml::NS_A,
            xml::NS_R,
            xml::NS_P
        ),
    );
    xml::line(&mut out, 1, "<p:cSld>");
    xml::line(
        &mut out,
        2,
        "<p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>",
    );
    xml::line(&mut out, 2, "<p:spTree>");
    empty_group(&mut out, 3);
    xml::line(&mut out, 2, "</p:spTree>");
    xml::line(&mut out, 1, "</p:cSld>");
    xml::line(
        &mut out,
        1,
        "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>",
    );
    xml::line(&mut out, 1, "<p:sldLayoutIdLst>");
    xml::line(
        &mut out,
        2,
        &format!("<p:sldLayoutId id=\"{SLIDE_LAYOUT_ID}\" r:id=\"rId1\"/>"),
    );
    xml::line(&mut out, 1, "</p:sldLayoutIdLst>");
    xml::line(&mut out, 1, "<p:txStyles>");
    xml::line(
        &mut out,
        2,
        "<p:titleStyle><a:lvl1pPr algn=\"l\"><a:defRPr sz=\"4400\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mj-lt\"/><a:ea typeface=\"+mj-ea\"/><a:cs typeface=\"+mj-cs\"/></a:defRPr></a:lvl1pPr></p:titleStyle>",
    );
    xml::line(
        &mut out,
        2,
        "<p:bodyStyle><a:lvl1pPr marL=\"342900\" indent=\"-342900\" algn=\"l\"><a:defRPr sz=\"3200\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl1pPr></p:bodyStyle>",
    );
    xml::line(
        &mut out,
        2,
        "<p:otherStyle><a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr><a:lvl1pPr marL=\"0\" algn=\"l\"><a:defRPr sz=\"1800\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl1pPr></p:otherStyle>",
    );
    xml::line(&mut out, 1, "</p:txStyles>");
    xml::line(&mut out, 0, "</p:sldMaster>");
    out
}

/// `ppt/slideLayouts/slideLayout1.xml`: the blank layout every slide uses.
pub(crate) fn slide_layout_xml() -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<p:sldLayout xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\" type=\"blank\" preserve=\"1\">",
            xml::NS_A,
            xml::NS_R,
            xml::NS_P
        ),
    );
    xml::line(&mut out, 1, "<p:cSld name=\"Blank\">");
    xml::line(&mut out, 2, "<p:spTree>");
    empty_group(&mut out, 3);
    xml::line(&mut out, 2, "</p:spTree>");
    xml::line(&mut out, 1, "</p:cSld>");
    xml::line(
        &mut out,
        1,
        "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>",
    );
    xml::line(&mut out, 0, "</p:sldLayout>");
    out
}

/// `ppt/slideMasters/_rels/slideLayout1.xml.rels` and its master counterpart
/// are built here; the layout points back at the single master.
pub(crate) fn slide_layout_rels_xml() -> String {
    let mut rels = Relationships::new();
    rels.add(rels::REL_SLIDE_MASTER, "../slideMasters/slideMaster1.xml");
    rels.to_xml()
}

pub(crate) fn slide_master_rels_xml() -> String {
    let mut rels = Relationships::new();
    rels.add(rels::REL_SLIDE_LAYOUT, "../slideLayouts/slideLayout1.xml");
    rels.add(rels::REL_THEME, "../theme/theme1.xml");
    rels.to_xml()
}

pub(crate) fn notes_master_rels_xml() -> String {
    let mut rels = Relationships::new();
    rels.add(rels::REL_THEME, "../theme/theme2.xml");
    rels.to_xml()
}

/// `ppt/notesMasters/notesMaster1.xml`: the notes page every notes slide uses.
///
/// Only the slide-image and body placeholders are defined; the header, date,
/// footer and slide-number placeholders add nothing to a printed deck and are
/// intentionally absent.
pub(crate) fn notes_master_xml() -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!(
            "<p:notesMaster xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\">",
            xml::NS_A,
            xml::NS_R,
            xml::NS_P
        ),
    );
    xml::line(&mut out, 1, "<p:cSld>");
    xml::line(
        &mut out,
        2,
        "<p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg>",
    );
    xml::line(&mut out, 2, "<p:spTree>");
    empty_group(&mut out, 3);
    notes_master_shape(
        &mut out,
        3,
        2,
        "Slide Image Placeholder 1",
        "sldImg",
        2,
        1_143_000,
        685_800,
        4_572_000,
        3_429_000,
    );
    notes_master_shape(
        &mut out,
        3,
        3,
        "Notes Placeholder 2",
        "body",
        3,
        685_800,
        4_343_400,
        5_486_400,
        4_114_800,
    );
    xml::line(&mut out, 2, "</p:spTree>");
    xml::line(&mut out, 1, "</p:cSld>");
    xml::line(
        &mut out,
        1,
        "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>",
    );
    xml::line(
        &mut out,
        1,
        "<p:notesStyle><a:lvl1pPr><a:defRPr sz=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl1pPr></p:notesStyle>",
    );
    xml::line(&mut out, 0, "</p:notesMaster>");
    out
}

#[allow(clippy::too_many_arguments)]
fn notes_master_shape(
    out: &mut String,
    depth: usize,
    id: u32,
    name: &str,
    placeholder: &str,
    index: u32,
    x: i64,
    y: i64,
    cx: i64,
    cy: i64,
) {
    xml::line(out, depth, "<p:sp>");
    xml::line(out, depth + 1, "<p:nvSpPr>");
    xml::line(
        out,
        depth + 2,
        &format!("<p:cNvPr id=\"{id}\" name=\"{name}\"/>"),
    );
    xml::line(
        out,
        depth + 2,
        "<p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr>",
    );
    xml::line(
        out,
        depth + 2,
        &format!("<p:nvPr><p:ph type=\"{placeholder}\" idx=\"{index}\"/></p:nvPr>"),
    );
    xml::line(out, depth + 1, "</p:nvSpPr>");
    xml::line(out, depth + 1, "<p:spPr>");
    xml::line(out, depth + 2, "<a:xfrm>");
    xml::line(out, depth + 3, &format!("<a:off x=\"{x}\" y=\"{y}\"/>"));
    xml::line(out, depth + 3, &format!("<a:ext cx=\"{cx}\" cy=\"{cy}\"/>"));
    xml::line(out, depth + 2, "</a:xfrm>");
    xml::line(
        out,
        depth + 2,
        "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
    );
    xml::line(out, depth + 1, "</p:spPr>");
    xml::line(
        out,
        depth + 1,
        "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\"/></a:p></p:txBody>",
    );
    xml::line(out, depth, "</p:sp>");
}
