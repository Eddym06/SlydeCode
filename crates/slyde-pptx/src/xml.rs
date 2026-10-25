//! The few XML rules the crate needs to keep for itself.
//!
//! There is no serializer dependency here on purpose: the parts are small and
//! hand-written, so escaping, indentation and the single UTC timestamp used by
//! `docProps/core.xml` live in this module. Everything that reaches a part goes
//! through [`escape`], which is the only defence user text has.

/// XML declaration shared by every part in the package.
pub(crate) const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

pub(crate) const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub(crate) const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
pub(crate) const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(crate) const NS_CONTENT_TYPES: &str =
    "http://schemas.openxmlformats.org/package/2006/content-types";
pub(crate) const NS_RELATIONSHIPS: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships";
pub(crate) const NS_CORE_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
pub(crate) const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub(crate) const NS_DCTERMS: &str = "http://purl.org/dc/terms/";
pub(crate) const NS_DCMITYPE: &str = "http://purl.org/dc/dcmitype/";
pub(crate) const NS_XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Appends `text` as a single line indented `depth` levels (two spaces each).
pub(crate) fn line(out: &mut String, depth: usize, text: &str) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push_str(text);
    out.push('\n');
}

/// Escapes text for use in XML character data or an attribute value.
///
/// Characters that XML 1.0 cannot represent at all (most C0 controls and the
/// two non-characters `U+FFFE`/`U+FFFF`) are dropped instead of being written,
/// because a deck must never be rejected for a stray control byte in a title
/// or in the speaker notes.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            '\u{FFFE}' | '\u{FFFF}' => {}
            control if (control as u32) < 0x20 => {}
            other => out.push(other),
        }
    }
    out
}

/// Current UTC time as `yyyy-mm-ddThh:mm:ssZ`.
///
/// The timestamp is only ever used in `docProps/core.xml`; slide and package
/// structure stay byte-stable between runs.
pub(crate) fn now_utc_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    iso8601_utc(seconds)
}

/// Formats seconds since the Unix epoch as `yyyy-mm-ddThh:mm:ssZ`.
pub(crate) fn iso8601_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = remainder / 3_600;
    let minute = (remainder % 3_600) / 60;
    let second = remainder % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 to a civil date, after Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}
