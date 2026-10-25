//! Element handles for the bimodal canvas.
//!
//! Every block inside a slide gets a stable `data-id`, so the studio (click to
//! select), the MCP tools (`get_selected_element`, `mutate_element`) and the AI
//! refer to the same thing. Ids are part of the stored code — not DOM state —
//! because every mutation is a string edit over `Slide::code`.
//!
//! Handles are unique **within one slide document**, which is what a DOM needs.
//! They are not unique across the deck (`slyde-1` exists in every slide), so
//! deck-level edits must be scoped with a `slide_id`; `mutate_element` rejects an
//! unscoped id that matches more than one slide.
//!
//! The scan is deliberately small: it only looks for element tags, skips
//! comments, and skips the text inside `<style>`/`<script>` so CSS selectors and
//! JS strings can never be mistaken for markup.

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Adds a `data-id="slyde-N"` to every element that has neither an `id` nor a
/// `data-id`. Idempotent: running it twice changes nothing.
pub fn ensure_element_ids(code: &str) -> String {
    if !code.contains('<') {
        return code.to_string();
    }
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len() + 32);
    let mut cursor = 0usize;
    let mut counter = 0usize;
    let mut skip_until: Option<usize> = None;

    while cursor < bytes.len() {
        if let Some(end) = skip_until {
            // Inside <style>/<script>: copy verbatim up to the closing tag.
            out.push_str(&code[cursor..end]);
            cursor = end;
            skip_until = None;
            continue;
        }

        let Some(open_rel) = code[cursor..].find('<') else {
            out.push_str(&code[cursor..]);
            break;
        };
        let open = cursor + open_rel;
        out.push_str(&code[cursor..open]);

        // Comment: copy it whole.
        if code[open..].starts_with("<!--") {
            let end = code[open..]
                .find("-->")
                .map(|i| open + i + 3)
                .unwrap_or(bytes.len());
            out.push_str(&code[open..end]);
            cursor = end;
            continue;
        }

        // Closing tag or doctype/declaration: copy up to `>`.
        let next = code[open + 1..].chars().next().unwrap_or(' ');
        if next == '/' || next == '!' || next == '?' {
            let end = tag_end(code, open).unwrap_or(bytes.len());
            out.push_str(&code[open..end]);
            cursor = end;
            continue;
        }
        if !next.is_ascii_alphabetic() {
            out.push('<');
            cursor = open + 1;
            continue;
        }

        let name_end = code[open + 1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
            .map(|i| open + 1 + i)
            .unwrap_or(bytes.len());
        let name = code[open + 1..name_end].to_ascii_lowercase();
        let end = match tag_end(code, open) {
            Some(end) => end,
            None => {
                out.push_str(&code[open..]);
                break;
            }
        };

        let tag = &code[open..end];
        counter += 1;
        let handle = if has_handle(tag) {
            None
        } else {
            Some(format!("slyde-{counter}"))
        };

        out.push('<');
        out.push_str(&code[open + 1..name_end]);
        if let Some(handle) = handle {
            out.push_str(&format!(" data-id=\"{handle}\""));
        }
        out.push_str(&code[name_end..end]);

        cursor = end;

        // Copy the body of style/script verbatim.
        if (name == "style" || name == "script") && !tag.trim_end().ends_with("/>") {
            let close = format!("</{name}");
            let body_end = code[cursor..]
                .to_ascii_lowercase()
                .find(&close)
                .map(|i| cursor + i)
                .unwrap_or(bytes.len());
            out.push_str(&code[cursor..body_end]);
            cursor = body_end;
        }
    }

    out
}

fn has_handle(tag: &str) -> bool {
    ["id=", "data-id="].iter().any(|needle| {
        tag.to_ascii_lowercase()
            .match_indices(needle)
            .any(|(index, _)| {
                // `id=` must start an attribute, not sit inside another name.
                let before = tag.as_bytes().get(index.wrapping_sub(1)).copied();
                let starts_attribute = match before {
                    Some(byte) => byte.is_ascii_whitespace(),
                    None => false,
                };
                starts_attribute && value_follows(tag, index + needle.len())
            })
    })
}

fn value_follows(tag: &str, index: usize) -> bool {
    match tag.as_bytes().get(index).copied() {
        Some(byte) if byte == b' ' || byte == b'\t' => value_follows(tag, index + 1),
        Some(b'"') | Some(b'\'') => true,
        _ => false,
    }
}

/// Index just past the `>` closing the tag that starts at `open`, ignoring `>`
/// inside quoted attribute values.
fn tag_end(code: &str, open: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, ch) in code[open..].char_indices() {
        match (quote, ch) {
            (None, '"') | (None, '\'') => quote = Some(ch),
            (Some(active), current) if current == active => quote = None,
            (None, '>') => return Some(open + offset + 1),
            _ => {}
        }
    }
    None
}

/// True for elements that never have a closing tag.
pub fn is_void(tag_name: &str) -> bool {
    VOID_TAGS.contains(&tag_name.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_ids_in_document_order() {
        let out = ensure_element_ids("<div class=\"slide\"><h1>Hi</h1><p>Body</p></div>");
        assert!(
            out.contains("<div data-id=\"slyde-1\" class=\"slide\">"),
            "{out}"
        );
        assert!(out.contains("<h1 data-id=\"slyde-2\">Hi</h1>"), "{out}");
        assert!(out.contains("<p data-id=\"slyde-3\">Body</p>"), "{out}");
    }

    #[test]
    fn keeps_authored_handles_and_does_not_renumber_them() {
        let out = ensure_element_ids("<h1 id=\"title-1\">A</h1><p data-id=\"body\">B</p>");
        assert_eq!(out, "<h1 id=\"title-1\">A</h1><p data-id=\"body\">B</p>");
    }

    #[test]
    fn is_idempotent() {
        let once = ensure_element_ids("<section><span>x</span></section>");
        let twice = ensure_element_ids(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn skips_style_and_script_bodies() {
        let out = ensure_element_ids(
            "<style>.a > .b { color: red }</style><script>if (a < b) {}</script><p>x</p>",
        );
        assert!(out.contains(".a > .b"), "{out}");
        assert!(out.contains("if (a < b)"), "{out}");
        assert!(out.contains("<p data-id=\"slyde-3\">"), "{out}");
    }

    #[test]
    fn ignores_comments_and_doctype() {
        let out = ensure_element_ids("<!-- <div>no</div> --><div>x</div>");
        assert!(out.contains("<!-- <div>no</div> -->"), "{out}");
        assert!(out.contains("<div data-id=\"slyde-1\">"), "{out}");
    }

    #[test]
    fn marks_void_elements_too() {
        let out = ensure_element_ids("<img src=\"data:image/png;base64,AA\">");
        assert_eq!(
            out,
            "<img data-id=\"slyde-1\" src=\"data:image/png;base64,AA\">"
        );
        assert!(is_void("img"));
        assert!(!is_void("div"));
    }

    #[test]
    fn does_not_treat_substrings_as_handles() {
        let out = ensure_element_ids("<div class=\"id=foo\" title=\"a\">x</div>");
        assert!(out.contains("data-id=\"slyde-1\""), "{out}");
    }
}
