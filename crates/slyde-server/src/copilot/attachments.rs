//! Chat attachments.
//!
//! Images travel to vision models as base64 blocks. Documents are converted to
//! text on the server: the browser never needs to parse a PDF or unzip a DOCX,
//! and the provider receives a bounded amount of content.

use std::io::{Cursor, Read};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

use super::error::CopilotError;

pub const MAX_ATTACHMENTS: usize = 6;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_DOCUMENT_BYTES: usize = 12 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 30_000;
const MAX_DOCUMENT_XML_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mime: String,
    /// Base64. The UI accepts a raw payload or a `data:` URL.
    #[serde(default)]
    pub data: String,
}

#[derive(Debug, Clone)]
pub enum AttachmentContent {
    Image {
        name: String,
        mime: String,
        data: String,
    },
    Document {
        name: String,
        text: String,
    },
}

impl AttachmentContent {
    pub fn name(&self) -> &str {
        match self {
            Self::Image { name, .. } | Self::Document { name, .. } => name,
        }
    }
}

pub fn decode_all(inputs: &[AttachmentInput]) -> Result<Vec<AttachmentContent>, CopilotError> {
    if inputs.len() > MAX_ATTACHMENTS {
        return Err(CopilotError::InvalidRequest(format!(
            "at most {MAX_ATTACHMENTS} attachments are allowed per message"
        )));
    }
    inputs.iter().map(decode_one).collect()
}

fn decode_one(input: &AttachmentInput) -> Result<AttachmentContent, CopilotError> {
    let name = sanitize_name(&input.name);
    let mime = input
        .mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if input.data.trim().is_empty() {
        return Err(CopilotError::InvalidRequest(format!(
            "{name} is empty or could not be read"
        )));
    }
    let bytes = decode_base64(&input.data)?;

    if is_image_mime(&mime) {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(CopilotError::InvalidRequest(format!(
                "{name} is larger than 5 MB"
            )));
        }
        return Ok(AttachmentContent::Image {
            name,
            mime,
            data: STANDARD.encode(bytes),
        });
    }

    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(CopilotError::InvalidRequest(format!(
            "{name} is larger than 12 MB"
        )));
    }

    let text = extract_document(&name, &mime, &bytes)?;
    let text = truncate_chars(text.trim(), MAX_TEXT_CHARS);
    if text.is_empty() {
        return Err(CopilotError::InvalidRequest(format!(
            "no readable text was found in {name}"
        )));
    }
    Ok(AttachmentContent::Document { name, text })
}

fn decode_base64(raw: &str) -> Result<Vec<u8>, CopilotError> {
    let payload = raw
        .strip_prefix("data:")
        .and_then(|value| value.split_once(',').map(|(_, data)| data))
        .unwrap_or(raw);
    let compact: String = payload.chars().filter(|ch| !ch.is_whitespace()).collect();
    STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| CopilotError::InvalidRequest("an attachment is not valid base64".into()))
}

fn extract_document(name: &str, mime: &str, bytes: &[u8]) -> Result<String, CopilotError> {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();

    if extension == "pdf" || mime == "application/pdf" {
        return extract_pdf(name, bytes);
    }
    if extension == "docx" || is_zip_mime(mime, "wordprocessingml") {
        return extract_docx(name, bytes);
    }
    if extension == "pptx" || is_zip_mime(mime, "presentationml") {
        return extract_pptx(name, bytes);
    }
    if is_text_mime(&mime)
        || matches!(
            extension.as_str(),
            "txt" | "md" | "csv" | "json" | "html" | "css" | "js" | "ts" | "yaml" | "yml"
        )
    {
        return String::from_utf8(bytes.to_vec())
            .map_err(|_| CopilotError::InvalidRequest(format!("{name} is not UTF-8 text")));
    }

    Err(CopilotError::InvalidRequest(format!(
        "{name} is not a supported attachment. Use an image, PDF, DOCX, PPTX or a text file."
    )))
}

fn extract_pdf(name: &str, bytes: &[u8]) -> Result<String, CopilotError> {
    // PDF parsers are exposed to hostile input. A panic must become an error
    // for this attachment, never take down the local server.
    let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    }))
    .map_err(|_| CopilotError::InvalidRequest(format!("{name} could not be read")))?;
    extracted.map_err(|_| CopilotError::InvalidRequest(format!("{name} could not be read")))
}

fn extract_docx(name: &str, bytes: &[u8]) -> Result<String, CopilotError> {
    let xml = read_zip_entry(bytes, "word/document.xml")?;
    let text = xml_to_text(&xml);
    if text.trim().is_empty() {
        return Err(CopilotError::InvalidRequest(format!(
            "no readable text was found in {name}"
        )));
    }
    Ok(text)
}

fn extract_pptx(name: &str, bytes: &[u8]) -> Result<String, CopilotError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| CopilotError::InvalidRequest(format!("{name} is not a valid PPTX file")))?;
    let mut slides = (0..archive.len())
        .filter_map(|index| {
            let file = archive.by_index(index).ok()?;
            let path = file.name().to_string();
            if path.starts_with("ppt/slides/slide") && path.ends_with(".xml") {
                let number = path
                    .trim_start_matches("ppt/slides/slide")
                    .trim_end_matches(".xml")
                    .parse::<u32>()
                    .unwrap_or(u32::MAX);
                Some((number, path))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    slides.sort_by_key(|(number, _)| *number);
    if slides.is_empty() {
        return Err(CopilotError::InvalidRequest(format!(
            "no readable text was found in {name}"
        )));
    }

    let mut text = String::new();
    for (index, (_, path)) in slides.iter().enumerate() {
        let xml = read_zip_entry(bytes, path)?;
        text.push_str(&format!("Slide {}\n", index + 1));
        text.push_str(&xml_to_text(&xml));
        text.push_str("\n\n");
    }
    Ok(text)
}

fn read_zip_entry(bytes: &[u8], path: &str) -> Result<String, CopilotError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        CopilotError::InvalidRequest("the document is not a valid ZIP container".into())
    })?;
    let mut file = archive.by_name(path).map_err(|_| {
        CopilotError::InvalidRequest("the document is missing its text body".into())
    })?;
    let mut xml = String::new();
    file.by_ref()
        .take(MAX_DOCUMENT_XML_BYTES)
        .read_to_string(&mut xml)
        .map_err(|_| CopilotError::InvalidRequest("the document text could not be read".into()))?;
    Ok(xml)
}

fn xml_to_text(xml: &str) -> String {
    let mut with_breaks = xml
        .replace("</w:p>", "\n")
        .replace("</a:p>", "\n")
        .replace("</w:tr>", "\n")
        .replace("<w:tab/>", "\t")
        .replace("<w:br/>", "\n");
    // Keep only text nodes: a small scanner is enough for OOXML bodies and
    // avoids pulling an XML parser just to strip tags.
    let mut out = String::with_capacity(with_breaks.len() / 2);
    let mut in_tag = false;
    for ch in with_breaks.drain(..) {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn sanitize_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let name = if trimmed.is_empty() {
        "attachment"
    } else {
        trimmed
    };
    name.chars().take(120).collect()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n\n[attachment truncated]");
    out
}

fn is_image_mime(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/webp")
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/x-yaml" | "application/yaml"
        )
}

fn is_zip_mime(mime: &str, needle: &str) -> bool {
    mime.contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, mime: &str, bytes: &[u8]) -> AttachmentInput {
        AttachmentInput {
            name: name.into(),
            mime: mime.into(),
            data: STANDARD.encode(bytes),
        }
    }

    #[test]
    fn images_are_normalized_and_limited() {
        let decoded = decode_all(&[input("shot.png", "image/png", b"png-bytes")]).unwrap();
        match &decoded[0] {
            AttachmentContent::Image { mime, data, .. } => {
                assert_eq!(mime, "image/png");
                assert_eq!(STANDARD.decode(data).unwrap(), b"png-bytes");
            }
            _ => panic!("expected image"),
        }
    }

    #[test]
    fn plain_text_documents_are_accepted() {
        let decoded = decode_all(&[input("notes.md", "text/markdown", b"# Title\nBody")]).unwrap();
        match &decoded[0] {
            AttachmentContent::Document { text, .. } => assert!(text.contains("# Title")),
            _ => panic!("expected document"),
        }
    }

    #[test]
    fn unsupported_binary_documents_are_rejected() {
        let error =
            decode_all(&[input("archive.bin", "application/octet-stream", b"xx")]).unwrap_err();
        assert_eq!(error.code(), "invalid_request");
    }

    #[test]
    fn data_urls_are_accepted() {
        let raw = format!("data:image/png;base64,{}", STANDARD.encode(b"x"));
        let decoded = decode_all(&[AttachmentInput {
            name: "shot.png".into(),
            mime: "image/png".into(),
            data: raw,
        }])
        .unwrap();
        assert_eq!(decoded.len(), 1);
    }

    #[test]
    fn ooxml_text_strips_tags_and_keeps_breaks() {
        let text = xml_to_text("<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:t>World</w:t></w:p>");
        assert_eq!(text, "Hello\nWorld");
    }
}
