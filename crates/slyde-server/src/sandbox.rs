//! Slide document rendering for the sandbox origin.
//!
//! Slides are served from a **second** listener, on a different port, so the
//! document has a different origin than the editor. The editor embeds it with
//! `sandbox="allow-scripts"` (no `allow-same-origin`), which gives the frame an
//! opaque origin: AI-authored JavaScript cannot read the editor's DOM, storage
//! or token.
//!
//! Inside the sandbox we also lock the document down with CSP: no network
//! requests, no forms, no navigation. Only inline styles/scripts and local
//! image data are allowed.

use slyde_core::{Presentation, Slide};

const CSP: &str = "default-src 'none'; \
style-src 'unsafe-inline'; \
script-src 'unsafe-inline'; \
img-src data: blob:; \
font-src data:; \
media-src data: blob:; \
connect-src 'none'; \
form-action 'none'; \
base-uri 'none'";

pub fn csp_header() -> &'static str {
    CSP
}

/// Renders a complete HTML document for one slide.
///
/// `static_mode` skips the "hide until animated" step, used for thumbnails and
/// for export, where nothing will trigger the timeline.
pub fn render_document(deck: &Presentation, slide: &Slide, static_mode: bool) -> String {
    let (width, height) = deck.aspect_ratio.size_px();
    let steps_json = serde_json::to_string(&slide.animation)
        .unwrap_or_else(|_| "[]".to_string())
        .replace("</", "<\\/");

    let title = escape_html(&deck.title);
    let theme = &deck.theme;

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<title>{title}</title>
<style>
  :root {{
    --slyde-bg: {bg};
    --slyde-fg: {fg};
    --slyde-muted: {muted};
    --slyde-accent: {accent};
    --slyde-font: {font};
  }}
  * {{ box-sizing: border-box; }}
  html, body {{
    margin: 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--slyde-bg);
    color: var(--slyde-fg);
    font-family: var(--slyde-font);
    -webkit-font-smoothing: antialiased;
  }}
  body {{ display: block; }}
  #slyde-stage {{
    position: absolute;
    top: 0;
    left: 0;
    width: {width}px;
    height: {height}px;
    transform-origin: top left;
    overflow: hidden;
    background: var(--slyde-bg);
    color: var(--slyde-fg);
  }}
  #slyde-stage img, #slyde-stage svg, #slyde-stage video, #slyde-stage canvas {{
    max-width: 100%;
  }}
  {base_css}
</style>
</head>
<body>
<main id="slyde-stage">{code}</main>
<script type="application/json" id="slyde-steps">{steps}</script>
<script>{runtime}</script>
</body>
</html>"#,
        csp = CSP,
        title = title,
        bg = theme.background,
        fg = theme.foreground,
        muted = theme.muted,
        accent = theme.accent,
        font = theme.font_family,
        width = width,
        height = height,
        code = slide.code,
        steps = steps_json,
        base_css = crate::starter::base_css(),
        runtime = runtime_script(width, height, static_mode),
    )
}

fn runtime_script(width: u32, height: u32, static_mode: bool) -> String {
    // Plain ES5-ish JavaScript: no build step, no modules, works in every
    // Chromium version we support.
    let template = r#"
(function () {
  var W = __W__, H = __H__, STATIC = __STATIC__;
  var stage = document.getElementById('slyde-stage');

  function fit() {
    var w = window.innerWidth;
    var h = window.innerHeight;
    // Inside a freshly created iframe the viewport can still be 0x0 when the
    // load event fires; scaling to 0 would hide the slide until a resize.
    if (!w || !h) return;
    var s = Math.min(w / W, h / H);
    var x = (w - W * s) / 2;
    var y = (h - H * s) / 2;
    stage.style.transform = 'translate(' + x + 'px,' + y + 'px) scale(' + s + ')';
  }
  window.addEventListener('resize', fit);
  if (window.ResizeObserver) {
    new ResizeObserver(fit).observe(document.documentElement);
  }
  requestAnimationFrame(fit);
  fit();

  var steps = [];
  try { steps = JSON.parse(document.getElementById('slyde-steps').textContent || '[]'); } catch (e) {}

  if (!STATIC) {
    steps.forEach(function (step) {
      var el = document.querySelector(step.target);
      if (el) { el.style.opacity = '0'; }
    });
  }

  var KEYFRAMES = {
    fade_up: [{ opacity: 0, transform: 'translateY(18px)' }, { opacity: 1, transform: 'translateY(0)' }],
    fade: [{ opacity: 0 }, { opacity: 1 }],
    scale_in: [{ opacity: 0, transform: 'scale(.96)' }, { opacity: 1, transform: 'scale(1)' }],
    slide_in_left: [{ opacity: 0, transform: 'translateX(-28px)' }, { opacity: 1, transform: 'translateX(0)' }],
    slide_in_right: [{ opacity: 0, transform: 'translateX(28px)' }, { opacity: 1, transform: 'translateX(0)' }],
    typewriter: [{ opacity: 0, clipPath: 'inset(0 100% 0 0)' }, { opacity: 1, clipPath: 'inset(0 0 0 0)' }]
  };

  function play(step) {
    if (!step || !step.target) return;
    var el = document.querySelector(step.target);
    if (!el) return;
    el.style.opacity = '';
    var frames = KEYFRAMES[step.kind] || KEYFRAMES.fade;
    try {
      el.animate(frames, {
        duration: step.duration_ms || 520,
        delay: step.delay_ms || 0,
        easing: step.easing || 'cubic-bezier(.16,1,.3,1)',
        fill: 'both'
      });
    } catch (e) {
      el.style.opacity = '1';
    }
  }

  window.addEventListener('message', function (event) {
    var data = event.data;
    if (!data || data.__slyde !== true) return;
    if (data.type === 'animate') play(data.step);
    if (data.type === 'reset') location.reload();
  });

  if (window.parent !== window) {
    window.parent.postMessage({ __slyde: true, type: 'ready' }, '*');
  }
})();
"#;

    template
        .replace("__W__", &width.to_string())
        .replace("__H__", &height.to_string())
        .replace("__STATIC__", if static_mode { "true" } else { "false" })
}

/// One document, one page per slide, each slide in its own iframe so slide CSS
/// never leaks between pages. Printed by headless Chrome with
/// `--print-to-pdf`; `@page size` makes the PDF match the deck aspect ratio
/// exactly.
pub fn render_print_document(deck: &Presentation, sandbox_origin: &str, token: &str) -> String {
    let (width, height) = deck.aspect_ratio.size_px();
    let token = percent_encode(token);

    let pages = deck
        .slides
        .iter()
        .map(|slide| {
            format!(
                r#"<section class="page"><iframe src="{origin}/s/{id}?static=1" title=""></iframe></section>"#,
                origin = sandbox_origin,
                id = percent_encode(&slide.id)
            )
        })
        .collect::<String>();

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>
  @page {{ size: {width}px {height}px; margin: 0; }}
  html, body {{ margin: 0; padding: 0; background: #fff; }}
  .page {{
    width: {width}px;
    height: {height}px;
    overflow: hidden;
    page-break-after: always;
    break-after: page;
  }}
  .page:last-child {{ page-break-after: auto; break-after: auto; }}
  iframe {{ width: {width}px; height: {height}px; border: 0; display: block; }}
</style>
</head>
<body data-token="{token}">
{pages}
</body>
</html>"#,
        title = escape_html(&deck.title),
        width = width,
        height = height,
        token = token,
        pages = pages,
    )
}

pub(crate) fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slyde_core::{AnimationKind, AnimationStep, AspectRatio, Theme};

    fn deck_with(slide: Slide) -> Presentation {
        let mut deck = Presentation::new("Q3 <Review>", Theme::studio(), AspectRatio::R16x9);
        deck.slides.push(slide);
        deck
    }

    #[test]
    fn document_contains_slide_markup_and_theme_tokens() {
        let slide = Slide::new("a".into(), "<h1 class=\"t\">Hello</h1>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, true);
        assert!(html.contains("<h1 class=\"t\">Hello</h1>"));
        assert!(html.contains("--slyde-bg: #17161A"));
        assert!(html.contains("width: 1280px"));
        assert!(html.contains("height: 720px"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("connect-src 'none'"));
    }

    #[test]
    fn title_is_escaped() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, true);
        assert!(html.contains("<title>Q3 &lt;Review&gt;</title>"));
        assert!(!html.contains("<title>Q3 <Review>"));
    }

    #[test]
    fn animation_steps_are_embedded_and_escaped() {
        let mut slide = Slide::new("a".into(), "<p>x</p>".into());
        slide.animation.push(AnimationStep {
            target: ".title".into(),
            kind: AnimationKind::FadeUp,
            delay_ms: 40,
            duration_ms: 500,
            easing: "ease-out".into(),
        });
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, false);
        assert!(html.contains("\"kind\":\"fade_up\""));
        assert!(html.contains("var W = 1280"));
        assert!(html.contains("STATIC = false"));
    }

    #[test]
    fn static_mode_is_flagged_for_thumbnails() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, true);
        assert!(html.contains("STATIC = true"));
    }

    #[test]
    fn portrait_ratio_changes_stage_size() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let mut deck = deck_with(slide.clone());
        deck.aspect_ratio = AspectRatio::R9x16;
        let html = render_document(&deck, &slide, true);
        assert!(html.contains("width: 720px"));
        assert!(html.contains("height: 1280px"));
    }
}
