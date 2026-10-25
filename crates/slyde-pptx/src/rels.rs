//! Relationship parts.
//!
//! Every `.rels` file is a flat list of `Relationship` elements whose ids are
//! handed out in insertion order. Building the list first and printing it last
//! keeps the slide order and the ids used in `presentation.xml` in sync, which
//! is the one place where a mismatch silently produces a broken deck.

use crate::xml;

pub(crate) const REL_OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
pub(crate) const REL_CORE_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
pub(crate) const REL_SLIDE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
pub(crate) const REL_SLIDE_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
pub(crate) const REL_SLIDE_LAYOUT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
pub(crate) const REL_THEME: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
pub(crate) const REL_IMAGE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
pub(crate) const REL_NOTES_SLIDE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";
pub(crate) const REL_NOTES_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster";

/// One parsed-keyed list of relationships, printed as a standalone part.
#[derive(Debug, Default)]
pub(crate) struct Relationships {
    entries: Vec<(String, &'static str, String)>,
}

impl Relationships {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Registers a relationship and returns the id that refers to it.
    pub(crate) fn add(&mut self, rel_type: &'static str, target: impl Into<String>) -> String {
        let id = format!("rId{}", self.entries.len() + 1);
        self.entries.push((id.clone(), rel_type, target.into()));
        id
    }

    pub(crate) fn to_xml(&self) -> String {
        let mut out = String::from(xml::DECL);
        xml::line(
            &mut out,
            0,
            &format!("<Relationships xmlns=\"{}\">", xml::NS_RELATIONSHIPS),
        );
        for (id, rel_type, target) in &self.entries {
            xml::line(
                &mut out,
                1,
                &format!(
                    "<Relationship Id=\"{id}\" Type=\"{rel_type}\" Target=\"{}\"/>",
                    xml::escape(target)
                ),
            );
        }
        xml::line(&mut out, 0, "</Relationships>");
        out
    }
}
