//! The deck theme.
//!
//! Slides only carry a full-bleed picture, so the theme exists to make the
//! package structurally valid rather than to style text. It is still a real
//! theme: colour scheme, font scheme and a format scheme with the three fill,
//! line, effect and background-fill entries the schema requires.

use crate::xml;

/// One theme part. The notes master gets its own copy so each master owns a
/// theme, which is what every PowerPoint-authored deck looks like.
pub(crate) fn theme_xml() -> String {
    let mut out = String::from(xml::DECL);
    xml::line(
        &mut out,
        0,
        &format!("<a:theme xmlns:a=\"{}\" name=\"SlydeCode\">", xml::NS_A),
    );
    xml::line(&mut out, 1, "<a:themeElements>");

    xml::line(&mut out, 2, "<a:clrScheme name=\"SlydeCode\">");
    xml::line(
        &mut out,
        3,
        "<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>",
    );
    xml::line(
        &mut out,
        3,
        "<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>",
    );
    xml::line(&mut out, 3, "<a:dk2><a:srgbClr val=\"1F2430\"/></a:dk2>");
    xml::line(&mut out, 3, "<a:lt2><a:srgbClr val=\"F4F1EA\"/></a:lt2>");
    for (name, value) in [
        ("accent1", "4F81BD"),
        ("accent2", "C0504D"),
        ("accent3", "9BBB59"),
        ("accent4", "8064A2"),
        ("accent5", "4BACC6"),
        ("accent6", "F79646"),
        ("hlink", "0000FF"),
        ("folHlink", "800080"),
    ] {
        xml::line(
            &mut out,
            3,
            &format!("<a:{name}><a:srgbClr val=\"{value}\"/></a:{name}>"),
        );
    }
    xml::line(&mut out, 2, "</a:clrScheme>");

    xml::line(&mut out, 2, "<a:fontScheme name=\"SlydeCode\">");
    xml::line(
        &mut out,
        3,
        "<a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>",
    );
    xml::line(
        &mut out,
        3,
        "<a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont>",
    );
    xml::line(&mut out, 2, "</a:fontScheme>");

    xml::line(&mut out, 2, "<a:fmtScheme name=\"SlydeCode\">");
    write_fill_styles(&mut out);
    write_line_styles(&mut out);
    write_effect_styles(&mut out);
    write_background_fills(&mut out);
    xml::line(&mut out, 2, "</a:fmtScheme>");

    xml::line(&mut out, 1, "</a:themeElements>");
    xml::line(&mut out, 0, "</a:theme>");
    out
}

fn write_fill_styles(out: &mut String) {
    xml::line(out, 3, "<a:fillStyleLst>");
    xml::line(
        out,
        4,
        "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    );
    xml::line(
        out,
        4,
        "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:tint val=\"50000\"/><a:satMod val=\"300000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:shade val=\"15000\"/><a:satMod val=\"350000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"16200000\" scaled=\"1\"/></a:gradFill>",
    );
    xml::line(
        out,
        4,
        "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:tint val=\"100000\"/><a:shade val=\"100000\"/><a:satMod val=\"130000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:tint val=\"50000\"/><a:shade val=\"100000\"/><a:satMod val=\"350000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"16200000\" scaled=\"0\"/></a:gradFill>",
    );
    xml::line(out, 3, "</a:fillStyleLst>");
}

fn write_line_styles(out: &mut String) {
    xml::line(out, 3, "<a:lnStyleLst>");
    xml::line(
        out,
        4,
        "<a:ln w=\"9525\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"><a:shade val=\"95000\"/><a:satMod val=\"105000\"/></a:schemeClr></a:solidFill><a:prstDash val=\"solid\"/></a:ln>",
    );
    for width in [25_400, 38_100] {
        xml::line(
            out,
            4,
            &format!(
                "<a:ln w=\"{width}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>"
            ),
        );
    }
    xml::line(out, 3, "</a:lnStyleLst>");
}

fn write_effect_styles(out: &mut String) {
    xml::line(out, 3, "<a:effectStyleLst>");
    for (distance, alpha) in [(20_000, 38_000), (23_000, 35_000), (23_000, 35_000)] {
        xml::line(
            out,
            4,
            &format!(
                "<a:effectStyle><a:effectLst><a:outerShdw blurRad=\"40000\" dist=\"{distance}\" dir=\"5400000\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"{alpha}\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle>"
            ),
        );
    }
    xml::line(out, 3, "</a:effectStyleLst>");
}

fn write_background_fills(out: &mut String) {
    xml::line(out, 3, "<a:bgFillStyleLst>");
    xml::line(
        out,
        4,
        "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    );
    xml::line(
        out,
        4,
        "<a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/><a:satMod val=\"170000\"/></a:schemeClr></a:solidFill>",
    );
    xml::line(
        out,
        4,
        "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:tint val=\"80000\"/><a:satMod val=\"300000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:shade val=\"30000\"/><a:satMod val=\"200000\"/></a:schemeClr></a:gs></a:gsLst><a:path path=\"circle\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path></a:gradFill>",
    );
    xml::line(out, 3, "</a:bgFillStyleLst>");
}
