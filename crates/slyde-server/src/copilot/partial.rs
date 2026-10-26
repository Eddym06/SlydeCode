//! Reads finished slides out of a tool call that is still being streamed.
//!
//! A model that writes a whole deck in one `build_deck` call spends tens of
//! seconds producing the arguments. Waiting for the closing brace before showing
//! anything makes the product look frozen, so the scanner below watches the
//! argument text as it grows and reports every slide object the moment it is
//! complete. The copilot applies those slides immediately; the final tool call
//! then only has to write what was not already applied.
//!
//! The scanner is incremental (it never re-reads bytes it has seen) and only
//! understands the shape the tools publish: a top-level object holding a
//! `slides` array of objects. Anything else simply yields nothing, and the tool
//! runs normally when the call ends.

use serde_json::Value;

/// What the scanner found since the last call.
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    /// Deck-level fields (`title`, `theme`, `aspect_ratio`), once, with the first slide.
    pub header: Option<Value>,
    /// Slide objects that closed since the last call, in order.
    pub slides: Vec<Value>,
}

#[derive(Debug, Default)]
pub struct SlideScanner {
    /// Bytes of the argument string already consumed.
    pos: usize,
    depth: usize,
    in_string: bool,
    escaped: bool,
    /// Byte offset where the current slide object started, when inside one.
    object_start: Option<usize>,
    /// Byte offset of the `[` that opens the slides array.
    array_open: Option<usize>,
    header_sent: bool,
}

impl SlideScanner {
    /// Consumes the new tail of `raw` (the whole argument string so far).
    pub fn feed(&mut self, raw: &str) -> Found {
        let mut found = Found::default();
        let bytes = raw.as_bytes();
        while self.pos < bytes.len() {
            let byte = bytes[self.pos];
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.in_string = false;
                }
                self.pos += 1;
                continue;
            }
            match byte {
                b'"' => self.in_string = true,
                b'{' | b'[' => {
                    self.depth += 1;
                    // Depth 2 is the slides array, depth 3 one slide object.
                    if byte == b'[' && self.depth == 2 && self.array_open.is_none() {
                        self.array_open = Some(self.pos);
                    }
                    if byte == b'{' && self.depth == 3 && self.array_open.is_some() {
                        self.object_start = Some(self.pos);
                    }
                }
                b'}' | b']' => {
                    if byte == b'}' && self.depth == 3 {
                        if let Some(start) = self.object_start.take() {
                            // `get` keeps a multi-byte boundary mistake from panicking.
                            if let Some(slice) = raw.get(start..=self.pos)
                                && let Ok(value) = serde_json::from_str::<Value>(slice)
                            {
                                if !self.header_sent {
                                    self.header_sent = true;
                                    found.header = self.header(raw);
                                }
                                found.slides.push(value);
                            }
                        }
                    }
                    self.depth = self.depth.saturating_sub(1);
                }
                _ => {}
            }
            self.pos += 1;
        }
        found
    }

    /// The deck-level fields that were written before the slides array.
    fn header(&self, raw: &str) -> Option<Value> {
        let open = self.array_open?;
        let prefix = &raw[..open];
        let cut = prefix.rfind("\"slides\"")?;
        let head = prefix[..cut].trim_end().trim_end_matches(',');
        serde_json::from_str::<Value>(&format!("{head}}}")).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARGS: &str = r#"{"title":"Launch \"2026\"","theme":"paper","aspect_ratio":"16:9","slides":[{"html_code":"<h1>One {x}</h1>","template":"hero","animation":[{"target":".a","kind":"fade"}]},{"html_code":"<p>Two ]\"</p>","notes":"n"}]}"#;

    #[test]
    fn slides_are_reported_when_they_close_whatever_the_chunking() {
        for step in [1, 2, 7, 13, 64, ARGS.len()] {
            let mut scanner = SlideScanner::default();
            let mut slides = Vec::new();
            let mut headers = Vec::new();
            let mut end = 0;
            while end < ARGS.len() {
                end = (end + step).min(ARGS.len());
                while !ARGS.is_char_boundary(end) {
                    end += 1;
                }
                let found = scanner.feed(&ARGS[..end]);
                headers.extend(found.header);
                slides.extend(found.slides);
            }
            assert_eq!(slides.len(), 2, "step {step}");
            assert_eq!(slides[0]["html_code"], "<h1>One {x}</h1>");
            assert_eq!(slides[1]["html_code"], "<p>Two ]\"</p>");
            assert_eq!(headers.len(), 1, "the header comes once, step {step}");
            assert_eq!(headers[0]["title"], "Launch \"2026\"");
            assert_eq!(headers[0]["theme"], "paper");
        }
    }

    #[test]
    fn a_slide_is_not_reported_before_it_closes() {
        let mut scanner = SlideScanner::default();
        let cut = ARGS.find("\"notes\"").unwrap();
        let found = scanner.feed(&ARGS[..cut]);
        assert_eq!(found.slides.len(), 1, "only the first slide has closed");
        assert!(scanner.feed(ARGS).slides.len() == 1);
    }

    #[test]
    fn other_shapes_yield_nothing_and_never_panic() {
        let mut scanner = SlideScanner::default();
        assert_eq!(
            scanner.feed(r#"{"slide_id":"a","html_code":"<h1>é</h1>"}"#),
            Found::default()
        );
        let mut scanner = SlideScanner::default();
        assert_eq!(scanner.feed("not json at all }}} ]]]"), Found::default());
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn real_shape_with_long_bodies() {
        let slides: Vec<Value> = (1..=3)
            .map(|n| serde_json::json!({
                "html_code": format!("<section><h1>Slide {n}</h1><p>{}</p></section>", "body ".repeat(300)),
                "template": "bullet",
                "notes": format!("note {n}")
            }))
            .collect();
        let args = serde_json::json!({ "title": "Mock deck", "theme": "paper", "aspect_ratio": "16:9", "slides": slides }).to_string();
        let mut scanner = SlideScanner::default();
        let found = scanner.feed(&args);
        assert_eq!(found.slides.len(), 3, "{}", &args[..120]);
        assert!(found.header.is_some());
    }
}
