//! Editable text for a slide.
//!
//! The renderer hands over the slide as a clean plate (the picture with its text
//! hidden) plus the text as it was laid out: one block per paragraph, split into
//! the visual lines the browser produced. Each block becomes a real PowerPoint
//! text box placed over the plate, so the user can click it, retype it and
//! restyle it, while the slide still looks exactly like the presenter.
//!
//! Lines are written as explicit paragraphs with `wrap="none"`. PowerPoint then
//! never re-wraps the text, so a missing typeface (Geist is not installed on most
//! machines) can change the glyph widths but never the line structure.

use serde::Deserialize;

use crate::xml;

/// Horizontal alignment of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    /// Text hugs the left edge of the box.
    #[default]
    Left,
    /// Lines are centred on the box.
    Center,
    /// Text hugs the right edge of the box.
    Right,
}

impl TextAlign {
    fn ooxml(self) -> &'static str {
        match self {
            Self::Left => "l",
            Self::Center => "ctr",
            Self::Right => "r",
        }
    }
}

/// A stretch of text that shares one style.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TextRun {
    /// The characters, already case-transformed.
    pub text: String,
    /// Font size in CSS pixels.
    pub size: f32,
    /// Colour as `RRGGBB`.
    pub color: String,
    /// Opacity of the colour, `0.0..=1.0`.
    #[serde(default = "one")]
    pub alpha: f32,
    /// Bold weight.
    #[serde(default)]
    pub bold: bool,
    /// Italic style.
    #[serde(default)]
    pub italic: bool,
    /// Single underline.
    #[serde(default)]
    pub underline: bool,
    /// First family of the CSS font stack.
    #[serde(default)]
    pub font: String,
}

fn one() -> f32 {
    1.0
}

/// One paragraph of the slide, positioned in slide pixels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TextBlock {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width of the widest line.
    pub w: f32,
    /// Height of all lines together.
    pub h: f32,
    /// Alignment of the lines inside the box.
    #[serde(default)]
    pub align: TextAlign,
    /// Distance between consecutive baselines in CSS pixels.
    pub line_height: f32,
    /// Visual lines, each a list of styled runs.
    pub lines: Vec<Vec<TextRun>>,
}

/// Parses the JSON the renderer produced. Malformed input yields no blocks:
/// a slide without editable text is still a valid slide.
pub fn parse_blocks(json: &str) -> Vec<TextBlock> {
    serde_json::from_str::<Vec<TextBlock>>(json)
        .map(|blocks| blocks.into_iter().filter(usable).collect())
        .unwrap_or_default()
}

fn usable(block: &TextBlock) -> bool {
    let finite = [block.x, block.y, block.w, block.h, block.line_height]
        .iter()
        .all(|value| value.is_finite());
    finite
        && block.w > 0.0
        && block.h > 0.0
        && block.line_height > 0.0
        && block
            .lines
            .iter()
            .any(|line| line.iter().any(|run| !run.text.is_empty()))
}

/// EMU for a fractional pixel length, clamped so a hostile value cannot overflow.
fn emu(px: f32) -> i64 {
    (f64::from(px.clamp(-100_000.0, 100_000.0)) * 9525.0).round() as i64
}

/// Hundredths of a point for a CSS pixel size (96 DPI: 1 px = 0.75 pt).
fn size_hundredths(px: f32) -> i64 {
    ((f64::from(px.clamp(1.0, 1000.0)) * 75.0).round() as i64).clamp(100, 400_000)
}

fn hex_color(raw: &str) -> String {
    let clean: String = raw.chars().filter(char::is_ascii_hexdigit).collect();
    if clean.len() == 6 {
        clean.to_ascii_uppercase()
    } else {
        "000000".to_string()
    }
}

/// Writes one `p:sp` text box per block. Shape ids start at `first_id`.
pub(crate) fn shapes_xml(blocks: &[TextBlock], first_id: u32, out: &mut String, depth: usize) {
    for (index, block) in blocks.iter().enumerate() {
        let id = first_id + index as u32;
        let name = format!("Text {}", index + 1);
        xml::line(out, depth, "<p:sp>");
        xml::line(out, depth + 1, "<p:nvSpPr>");
        xml::line(
            out,
            depth + 2,
            &format!("<p:cNvPr id=\"{id}\" name=\"{name}\"/>"),
        );
        xml::line(out, depth + 2, "<p:cNvSpPr txBox=\"1\"/>");
        xml::line(out, depth + 2, "<p:nvPr/>");
        xml::line(out, depth + 1, "</p:nvSpPr>");
        xml::line(out, depth + 1, "<p:spPr>");
        xml::line(out, depth + 2, "<a:xfrm>");
        xml::line(
            out,
            depth + 3,
            &format!("<a:off x=\"{}\" y=\"{}\"/>", emu(block.x), emu(block.y)),
        );
        xml::line(
            out,
            depth + 3,
            &format!(
                "<a:ext cx=\"{}\" cy=\"{}\"/>",
                emu(block.w).max(1),
                emu(block.h).max(1)
            ),
        );
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
            "<a:bodyPr wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" rtlCol=\"0\" anchor=\"t\"><a:noAutofit/></a:bodyPr>",
        );
        xml::line(out, depth + 2, "<a:lstStyle/>");
        let spacing = size_hundredths(block.line_height);
        for line in &block.lines {
            if line.iter().all(|run| run.text.is_empty()) {
                continue;
            }
            xml::line(out, depth + 2, "<a:p>");
            xml::line(
                out,
                depth + 3,
                &format!(
                    "<a:pPr algn=\"{}\"><a:lnSpc><a:spcPts val=\"{spacing}\"/></a:lnSpc><a:spcBef><a:spcPts val=\"0\"/></a:spcBef><a:spcAft><a:spcPts val=\"0\"/></a:spcAft></a:pPr>",
                    block.align.ooxml()
                ),
            );
            for run in line.iter().filter(|run| !run.text.is_empty()) {
                run_xml(run, out, depth + 3);
            }
            xml::line(out, depth + 2, "</a:p>");
        }
        xml::line(out, depth + 1, "</p:txBody>");
        xml::line(out, depth, "</p:sp>");
    }
}

fn run_xml(run: &TextRun, out: &mut String, depth: usize) {
    let alpha = (run.alpha.clamp(0.0, 1.0) * 100_000.0).round() as u32;
    let fill = if alpha >= 100_000 {
        format!("<a:srgbClr val=\"{}\"/>", hex_color(&run.color))
    } else {
        format!(
            "<a:srgbClr val=\"{}\"><a:alpha val=\"{alpha}\"/></a:srgbClr>",
            hex_color(&run.color)
        )
    };
    let font = if run.font.trim().is_empty() {
        String::new()
    } else {
        let face = xml::escape(run.font.trim());
        format!("<a:latin typeface=\"{face}\"/><a:cs typeface=\"{face}\"/>")
    };
    xml::line(out, depth, "<a:r>");
    xml::line(
        out,
        depth + 1,
        &format!(
            "<a:rPr lang=\"en-US\" sz=\"{}\" b=\"{}\" i=\"{}\"{} dirty=\"0\"><a:solidFill>{fill}</a:solidFill>{font}</a:rPr>",
            size_hundredths(run.size),
            u8::from(run.bold),
            u8::from(run.italic),
            if run.underline { " u=\"sng\"" } else { "" },
        ),
    );
    xml::line(
        out,
        depth + 1,
        &format!("<a:t>{}</a:t>", xml::escape(&run.text)),
    );
    xml::line(out, depth, "</a:r>");
}
