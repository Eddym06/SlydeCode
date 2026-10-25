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
style-src 'self' 'unsafe-inline'; \
script-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; \
font-src 'self' data:; \
media-src data: blob:; \
connect-src 'none'; \
form-action 'none'; \
base-uri 'none'";

/// Bundled typefaces for slide content. `'self'` in the CSP means the sandbox
/// origin, where `slyde-server` also mounts `/fonts`; the browser then caches a
/// single copy for every slide iframe instead of inlining the face per document.
const FONT_FACES: &str = r#"
  @font-face {
    font-family: "Geist";
    src: url("/fonts/Geist-Variable.woff2") format("woff2");
    font-weight: 100 900;
    font-style: normal;
    font-display: swap;
  }
  @font-face {
    font-family: "Geist";
    src: url("/fonts/Geist-Italic-Variable.woff2") format("woff2");
    font-weight: 100 900;
    font-style: italic;
    font-display: swap;
  }
  @font-face {
    font-family: "Geist Mono";
    src: url("/fonts/GeistMono-Variable.woff2") format("woff2");
    font-weight: 100 900;
    font-style: normal;
    font-display: swap;
  }
  @font-face {
    font-family: "Geist Mono";
    src: url("/fonts/GeistMono-Italic-Variable.woff2") format("woff2");
    font-weight: 100 900;
    font-style: italic;
    font-display: swap;
  }"#;

pub fn csp_header() -> &'static str {
    CSP
}

/// Renders a complete HTML document for one slide.
///
/// `static_mode` skips the "hide until animated" step, used for thumbnails and
/// for export, where nothing will trigger the timeline.
pub fn render_document(
    deck: &Presentation,
    slide: &Slide,
    static_mode: bool,
    edit_mode: bool,
) -> String {
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
<style>{font_faces}
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
        font_faces = FONT_FACES,
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
        runtime = runtime_script(width, height, static_mode, edit_mode),
    )
}

fn runtime_script(width: u32, height: u32, static_mode: bool, edit_mode: bool) -> String {
    // Plain ES5-ish JavaScript: no build step, no modules, works in every
    // Chromium version we support.
    let template = r#"
(function () {
  var W = __W__, H = __H__, STATIC = __STATIC__, EDIT = __EDIT__;
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

  // ---- Technical blocks: code, math and diagrams ----
  // Renderers are vendored on this origin and loaded only when the slide needs
  // them, so a deck without technical blocks pays nothing.
  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var script = document.createElement('script');
      script.src = src;
      script.onload = function () { resolve(); };
      script.onerror = function () { reject(new Error('failed to load ' + src)); };
      document.head.appendChild(script);
    });
  }

  function loadStyle(href) {
    var link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = href;
    document.head.appendChild(link);
  }

  // The token palette flips with the deck theme, not the editor theme.
  (function detectScheme() {
    var bg = getComputedStyle(document.documentElement).getPropertyValue('--slyde-bg') ||
      getComputedStyle(document.body).backgroundColor;
    var value = String(bg).trim();
    var rgb = null;
    if (value.charAt(0) === '#') {
      var hex = value.slice(1);
      if (hex.length === 3) hex = hex[0] + hex[0] + hex[1] + hex[1] + hex[2] + hex[2];
      rgb = [parseInt(hex.slice(0, 2), 16), parseInt(hex.slice(2, 4), 16), parseInt(hex.slice(4, 6), 16)];
    } else {
      var match = value.match(/rgba?\(([^)]+)\)/);
      if (match) rgb = match[1].split(',').slice(0, 3).map(Number);
    }
    var luminance = 0;
    if (rgb && rgb.length === 3 && rgb.every(function (n) { return !isNaN(n); })) {
      luminance = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255;
    }
    document.documentElement.setAttribute('data-slyde-scheme', luminance > 0.55 ? 'light' : 'dark');
  })();

  // Splits highlighted HTML into balanced per-line fragments so every line can
  // carry a number and an emphasis state without corrupting nested spans.
  function splitHighlightedLines(html) {
    var lines = [];
    var open = [];
    var buffer = '';
    var tokenizer = /<[^>]+>|[^<]+/g;
    var token;
    function closeTag(tag) {
      var name = tag.replace(/^<\s*/, '').split(/[\s/>]/)[0];
      return '</' + name + '>';
    }
    function flush() {
      var closing = open.slice().reverse().map(closeTag).join('');
      lines.push(buffer + closing);
      buffer = open.join('');
    }
    while ((token = tokenizer.exec(html))) {
      var value = token[0];
      if (value.charAt(0) === '<') {
        var closing = value.charAt(1) === '/';
        var selfClosing = /\/>$/.test(value) || /^<(br|hr|img|input|wbr)\b/i.test(value);
        if (closing) open.pop();
        else if (!selfClosing) open.push(value);
        buffer += value;
      } else {
        var parts = value.split('\n');
        for (var i = 0; i < parts.length; i += 1) {
          if (i > 0) flush();
          buffer += parts[i];
        }
      }
    }
    lines.push(buffer + open.slice().reverse().map(closeTag).join(''));
    return lines;
  }

  function emphasisSet(raw) {
    var set = {};
    String(raw || '').split(',').forEach(function (part) {
      var range = part.trim().split('-');
      if (!range[0]) return;
      var from = parseInt(range[0], 10);
      var to = range.length > 1 ? parseInt(range[1], 10) : from;
      if (isNaN(from) || isNaN(to)) return;
      for (var n = from; n <= to; n += 1) set[n] = true;
    });
    return set;
  }

  function renderCodeBlocks(blocks) {
    return loadScript('/vendor/hljs/highlight.min.js').then(function () {
      Array.prototype.forEach.call(blocks, function (block) {
        var codeEl = block.querySelector('code') || block;
        var language = (block.getAttribute('data-lang') || '').trim();
        if (!language) {
          var match = (codeEl.className || '').match(/language-([\w+-]+)/);
          if (match) language = match[1];
        }
        var html;
        try {
          if (language && window.hljs.getLanguage(language)) {
            html = window.hljs.highlight(codeEl.textContent, { language: language }).value;
          } else {
            html = window.hljs.highlightAuto(codeEl.textContent).value;
          }
        } catch (e) {
          html = null;
        }
        if (html) {
          var lines = splitHighlightedLines(html);
          var emphasis = emphasisSet(block.getAttribute('data-emphasis'));
          codeEl.innerHTML = lines.map(function (line, index) {
            var classes = 'slyde-line' + (emphasis[index + 1] ? ' is-emphasis' : '');
            return '<span class="' + classes + '"><span class="slyde-ln">' + (index + 1) +
              '</span><span class="slyde-lc">' + line + '</span></span>';
          }).join('');
        }
        block.classList.add('slyde-code-ready');
      });
    }).catch(function () {
      /* Highlighting is a progressive enhancement: the code stays readable. */
    });
  }

  function renderMath() {
    loadStyle('/vendor/katex/katex.min.css');
    return loadScript('/vendor/katex/katex.min.js')
      .then(function () { return loadScript('/vendor/katex/auto-render.min.js'); })
      .then(function () {
        if (typeof window.renderMathInElement !== 'function') return;
        window.renderMathInElement(document.body, {
          delimiters: [
            { left: '$$', right: '$$', display: true },
            { left: '\\[', right: '\\]', display: true },
            { left: '\\(', right: '\\)', display: false },
            { left: '$', right: '$', display: false }
          ],
          ignoredTags: ['style', 'script', 'pre', 'code', 'noscript', 'textarea'],
          throwOnError: false
        });
        document.body.classList.add('slyde-math-ready');
      })
      .catch(function () {
        /* Without KaTeX the source stays visible instead of disappearing. */
      });
  }

  function renderDiagrams(nodes) {
    return loadScript('/vendor/mermaid/mermaid.min.js')
      .then(function () {
        var scheme = document.documentElement.getAttribute('data-slyde-scheme');
        window.mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: scheme === 'light' ? 'default' : 'dark',
          fontFamily: 'inherit'
        });
        return window.mermaid.run({ nodes: Array.prototype.slice.call(nodes) });
      })
      .catch(function () {
        /* A broken definition must never take the whole slide down. */
        Array.prototype.forEach.call(nodes, function (node) {
          node.setAttribute('data-slyde-diagram-error', '1');
        });
      });
  }

  (function renderTechnicalBlocks() {
    // Only `pre` blocks: matching `pre > code` as well would process the same
    // block twice and destroy the line wrapper the first pass created.
    var codeBlocks = document.querySelectorAll('pre');
    var diagrams = document.querySelectorAll('.slyde-diagram');
    var text = document.body.innerText || '';
    var hasMath = /\$[^$\n]+\$|\\\(|\\\[/.test(text);
    var jobs = [];
    if (codeBlocks.length) jobs.push(renderCodeBlocks(codeBlocks));
    if (hasMath) jobs.push(renderMath());
    if (diagrams.length) jobs.push(renderDiagrams(diagrams));
    var done = function () {
      document.documentElement.setAttribute('data-slyde-ready', '1');
      if (window.parent !== window) {
        window.parent.postMessage({ __slyde: true, type: 'rendered' }, '*');
      }
    };
    if (!jobs.length) {
      done();
    } else {
      Promise.all(jobs).then(done, done);
    }
  })();

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
    typewriter: [{ opacity: 0, clipPath: 'inset(0 100% 0 0)' }, { opacity: 1, clipPath: 'inset(0 0 0 0)' }],
    spin: [{ opacity: 0, transform: 'rotate(0deg)' }, { opacity: 1, transform: 'rotate(360deg)' }],
    rotate_in: [{ opacity: 0, transform: 'rotate(-45deg) scale(.9)' }, { opacity: 1, transform: 'rotate(0deg) scale(1)' }],
    pulse: [{ transform: 'scale(1)' }, { transform: 'scale(1.06)' }, { transform: 'scale(1)' }],
    flip_up: [{ opacity: 0, transform: 'perspective(600px) rotateX(40deg)' }, { opacity: 1, transform: 'perspective(600px) rotateX(0deg)' }],
    blur_in: [{ opacity: 0, filter: 'blur(12px)' }, { opacity: 1, filter: 'blur(0)' }],
    bounce_in: [{ opacity: 0, transform: 'scale(0.3)' }, { opacity: 0.9, transform: 'scale(1.08)' }, { opacity: 1, transform: 'scale(1)' }]
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

  // The keyboard belongs to whoever is presenting, never to the slide. If a
  // slide document ends up focused, forward every keystroke to the parent so
  // navigation (arrows, B, S, F, Escape) keeps working.
  window.addEventListener('keydown', function (event) {
    if (STATIC || window.parent === window) return;
    try {
      window.parent.postMessage({
        __slyde: true,
        type: 'key',
        key: event.key,
        code: event.code,
        altKey: event.altKey,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        shiftKey: event.shiftKey
      }, '*');
    } catch (e) {}
  });

  // ---- Visual editing layer (studio stage only) ----
  if (EDIT) {
    var selection = null;
    var editingNode = null;
    var outline = null;

    var post = function (message) {
      if (window.parent === window) return;
      message.__slyde = true;
      try { window.parent.postMessage(message, '*'); } catch (e) {}
    };

    // Every block gets a stable handle `mutate_element` can target: the
    // authored id, an existing data-id, or a generated one.
    (function assignIds() {
      var nodes = stage.querySelectorAll('*');
      for (var i = 0; i < nodes.length; i += 1) {
        var el = nodes[i];
        var handle = el.getAttribute('id') || el.getAttribute('data-id');
        if (!handle) {
          handle = 'slyde-' + (i + 1);
          el.setAttribute('data-id', handle);
        }
        el.setAttribute('data-slyde-id', handle);
      }
    })();

    function ensureOutline() {
      if (outline) return outline;
      outline = document.createElement('div');
      outline.setAttribute('data-slyde-ui', '1');
      outline.style.cssText = 'position:fixed;left:0;top:0;pointer-events:none;display:none;' +
        'z-index:2147483646;box-sizing:border-box;border-radius:2px;' +
        'box-shadow:0 0 0 2px #FF6B4A, 0 0 0 3px rgba(255,107,74,.22);';
      document.body.appendChild(outline);
      return outline;
    }

    function paint() {
      var layer = ensureOutline();
      if (!selection) { layer.style.display = 'none'; return; }
      var rect = selection.getBoundingClientRect();
      if (!rect.width && !rect.height) { layer.style.display = 'none'; return; }
      layer.style.display = 'block';
      layer.style.transform = 'translate(' + Math.round(rect.left) + 'px,' + Math.round(rect.top) + 'px)';
      layer.style.width = Math.round(rect.width) + 'px';
      layer.style.height = Math.round(rect.height) + 'px';
    }

    function describe(el) {
      var rect = el.getBoundingClientRect();
      return {
        type: 'select',
        id: el.getAttribute('data-slyde-id'),
        tag: el.tagName ? el.tagName.toLowerCase() : 'div',
        text: (el.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 80),
        rect: { x: rect.left, y: rect.top, w: rect.width, h: rect.height },
        viewport: { w: window.innerWidth, h: window.innerHeight }
      };
    }

    function select(el) {
      selection = el;
      paint();
      post(el ? describe(el) : { type: 'deselect' });
    }

    function closestBlock(node) {
      var el = node;
      while (el && el !== stage) {
        if (el.getAttribute && el.getAttribute('data-slyde-id')) return el;
        el = el.parentElement;
      }
      return null;
    }

    stage.addEventListener('click', function (event) {
      if (editingNode) return;
      event.preventDefault();
      select(closestBlock(event.target));
    }, true);

    document.addEventListener('dblclick', function (event) {
      var block = closestBlock(event.target);
      if (!block || editingNode) return;
      event.preventDefault();
      beginEdit(block);
    }, true);

    function beginEdit(el) {
      editingNode = el;
      el.__slydeBackup = el.innerHTML;
      el.setAttribute('contenteditable', 'true');
      el.style.caretColor = '#FF6B4A';
      el.focus();
      try {
        var range = document.createRange();
        range.selectNodeContents(el);
        var current = window.getSelection();
        current.removeAllRanges();
        current.addRange(range);
      } catch (e) {}
      post({ type: 'editing', id: el.getAttribute('data-slyde-id') });
    }

    function finishEdit(commit) {
      if (!editingNode) return;
      var el = editingNode;
      editingNode = null;
      el.removeAttribute('contenteditable');
      el.style.caretColor = '';
      if (commit) {
        var value = (el.innerText || '').replace(/\s+/g, ' ').trim();
        post({ type: 'text', id: el.getAttribute('data-slyde-id'), value: value });
      } else {
        el.innerHTML = el.__slydeBackup || '';
      }
      paint();
    }

    document.addEventListener('keydown', function (event) {
      if (!editingNode) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        finishEdit(false);
      } else if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        finishEdit(true);
      }
    }, true);

    document.addEventListener('focusout', function (event) {
      if (editingNode && event.target === editingNode) finishEdit(true);
    }, true);

    window.addEventListener('resize', paint);
    window.addEventListener('scroll', paint, true);

    window.addEventListener('message', function (event) {
      var data = event.data;
      if (!data || data.__slyde !== true) return;
      if (data.type === 'select-by-id') {
        select(data.id ? stage.querySelector('[data-slyde-id="' + data.id + '"]') : null);
      } else if (data.type === 'clear-selection') {
        select(null);
      }
    });

    // The document tree: the studio lists it as "Structure" and every entry can
    // be selected without touching the canvas.
    (function emitTree() {
      var nodes = [];
      var all = stage.querySelectorAll('[data-slyde-id]');
      for (var i = 0; i < all.length && nodes.length < 80; i += 1) {
        var el = all[i];
        nodes.push({
          id: el.getAttribute('data-slyde-id'),
          tag: el.tagName ? el.tagName.toLowerCase() : 'div',
          text: (el.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 48)
        });
      }
      post({ type: 'tree', nodes: nodes });
    })();
  }

  if (window.parent !== window) {
    window.parent.postMessage({ __slyde: true, type: 'ready' }, '*');
  }
})();
"#;
    template
        .replace("__W__", &width.to_string())
        .replace("__H__", &height.to_string())
        .replace("__STATIC__", if static_mode { "true" } else { "false" })
        .replace("__EDIT__", if edit_mode { "true" } else { "false" })
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
        let html = render_document(&deck, &slide, true, false);
        assert!(html.contains("<h1 class=\"t\">Hello</h1>"));
        assert!(html.contains("--slyde-bg: #17161A"));
        assert!(html.contains("width: 1280px"));
        assert!(html.contains("height: 720px"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("font-src 'self' data:"));
        assert!(html.contains("@font-face"));
        assert!(html.contains("/fonts/Geist-Variable.woff2"));
    }

    #[test]
    fn title_is_escaped() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, true, false);
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
        let html = render_document(&deck, &slide, false, false);
        assert!(html.contains("\"kind\":\"fade_up\""));
        assert!(html.contains("var W = 1280"));
        assert!(html.contains("STATIC = false"));
    }

    #[test]
    fn static_mode_is_flagged_for_thumbnails() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, true, false);
        assert!(html.contains("STATIC = true"));
        assert!(html.contains("EDIT = false"));
    }

    #[test]
    fn edit_mode_enables_the_selection_layer() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, true, true);
        assert!(html.contains("EDIT = true"));
        assert!(html.contains("data-slyde-id"));
        assert!(html.contains("contenteditable"));
    }

    #[test]
    fn portrait_ratio_changes_stage_size() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let mut deck = deck_with(slide.clone());
        deck.aspect_ratio = AspectRatio::R9x16;
        let html = render_document(&deck, &slide, true, false);
        assert!(html.contains("width: 720px"));
        assert!(html.contains("height: 1280px"));
    }
}
