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
    if (event.source !== window.parent) return;
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
    var selection = null;      // the primary block (last one picked)
    var group = [];            // every selected block, primary included
    var editingNode = null;
    var outlines = [];

    // Direct manipulation: eight resize handles around the selected block and
    // magnetic guides while a block moves or resizes. All maths run in stage
    // pixels (the document is CSS-scaled to fit the iframe).
    var HANDLES = {
      nw: [0, 0, 'nwse-resize'], n: [50, 0, 'ns-resize'], ne: [100, 0, 'nesw-resize'],
      e: [100, 50, 'ew-resize'], se: [100, 100, 'nwse-resize'], s: [50, 100, 'ns-resize'],
      sw: [0, 100, 'nesw-resize'], w: [0, 50, 'ew-resize']
    };
    var SNAP_PX = 6;       // snap distance, in stage pixels
    var SAFE_MARGIN = 64;  // safe-area guide, in stage pixels
    var MIN_SIZE = 16;
    var handleNodes = null;
    var guideNodes = null;
    var snapTargets = null;

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

    function outlineFor(index) {
      if (outlines[index]) return outlines[index];
      var node = document.createElement('div');
      node.setAttribute('data-slyde-ui', '1');
      node.style.cssText = 'position:fixed;left:0;top:0;pointer-events:none;display:none;' +
        'z-index:2147483646;box-sizing:border-box;border-radius:2px;' +
        'box-shadow:0 0 0 2px #FF6B4A, 0 0 0 3px rgba(255,107,74,.22);';
      document.body.appendChild(node);
      outlines[index] = node;
      return node;
    }

    function paint() {
      var shown = group.length ? group : (selection ? [selection] : []);
      for (var i = 0; i < Math.max(outlines.length, shown.length); i += 1) {
        var layer = outlineFor(i);
        var el = shown[i];
        if (!el) { layer.style.display = 'none'; continue; }
        var rect = el.getBoundingClientRect();
        if (!rect.width && !rect.height) { layer.style.display = 'none'; continue; }
        layer.style.display = 'block';
        layer.style.transform = 'translate(' + Math.round(rect.left) + 'px,' + Math.round(rect.top) + 'px)';
        layer.style.width = Math.round(rect.width) + 'px';
        layer.style.height = Math.round(rect.height) + 'px';
        // The primary block is solid; the others of a multi-selection are dashed.
        layer.style.boxShadow = shown.length > 1 && el !== selection
          ? '0 0 0 1.5px #7C3AED' : '0 0 0 2px #FF6B4A, 0 0 0 3px rgba(255,107,74,.22)';
      }
      paintHandles();
    }

    function ensureHandles() {
      if (handleNodes) return handleNodes;
      handleNodes = {};
      Object.keys(HANDLES).forEach(function (name) {
        var node = document.createElement('div');
        node.setAttribute('data-slyde-ui', '1');
        node.setAttribute('data-slyde-handle', name);
        node.style.cssText = 'position:fixed;left:0;top:0;width:10px;height:10px;margin:-5px 0 0 -5px;' +
          'display:none;z-index:2147483647;box-sizing:border-box;border-radius:3px;' +
          'background:#fff;border:2px solid #FF6B4A;touch-action:none;cursor:' + HANDLES[name][2] + ';';
        document.body.appendChild(node);
        handleNodes[name] = node;
      });
      return handleNodes;
    }

    // Handles belong to a single selected block; a multi-selection moves as a
    // group and has no handles.
    function paintHandles() {
      var nodes = ensureHandles();
      var el = group.length <= 1 ? selection : null;
      var rect = el && !editingNode ? el.getBoundingClientRect() : null;
      var visible = rect && (rect.width || rect.height);
      Object.keys(nodes).forEach(function (name) {
        var node = nodes[name];
        if (!visible) { node.style.display = 'none'; return; }
        var spec = HANDLES[name];
        node.style.display = 'block';
        node.style.transform = 'translate(' + Math.round(rect.left + rect.width * spec[0] / 100) + 'px,' +
          Math.round(rect.top + rect.height * spec[1] / 100) + 'px)';
      });
    }

    function ensureGuides() {
      if (guideNodes) return guideNodes;
      guideNodes = ['x', 'y'].map(function () {
        var node = document.createElement('div');
        node.setAttribute('data-slyde-ui', '1');
        node.style.cssText = 'position:fixed;left:0;top:0;display:none;pointer-events:none;' +
          'z-index:2147483645;background:#FF6B4A;opacity:.85;';
        document.body.appendChild(node);
        return node;
      });
      return guideNodes;
    }

    function showGuides(gx, gy) {
      var nodes = ensureGuides();
      var box = stage.getBoundingClientRect();
      var scale = box.width ? box.width / W : 1;
      if (gx === null) nodes[0].style.display = 'none';
      else {
        nodes[0].style.display = 'block';
        nodes[0].style.transform = 'translate(' + Math.round(box.left + gx * scale) + 'px,' + Math.round(box.top) + 'px)';
        nodes[0].style.width = '1px';
        nodes[0].style.height = Math.round(box.height) + 'px';
      }
      if (gy === null) nodes[1].style.display = 'none';
      else {
        nodes[1].style.display = 'block';
        nodes[1].style.transform = 'translate(' + Math.round(box.left) + 'px,' + Math.round(box.top + gy * scale) + 'px)';
        nodes[1].style.width = Math.round(box.width) + 'px';
        nodes[1].style.height = '1px';
      }
    }

    // Where a moving edge may snap: the stage edges and centre, the safe-area
    // margins and the edges and centres of every other visible block.
    function collectSnapTargets(skip) {
      var xs = [0, W / 2, W, SAFE_MARGIN, W - SAFE_MARGIN];
      var ys = [0, H / 2, H, SAFE_MARGIN, H - SAFE_MARGIN];
      var nodes = stage.querySelectorAll('[data-slyde-id]');
      var seen = 0;
      for (var i = 0; i < nodes.length && seen < 80; i += 1) {
        var node = nodes[i];
        if (node === skip || skip.contains(node) || node.contains(skip)) continue;
        if (node.tagName === 'STYLE' || node.tagName === 'SCRIPT' || node.tagName === 'BR') continue;
        var g = stageGeometry(node);
        if (g.w < 4 || g.h < 4) continue;
        xs.push(g.x, g.x + g.w / 2, g.x + g.w);
        ys.push(g.y, g.y + g.h / 2, g.y + g.h);
        seen += 1;
      }
      return { xs: xs, ys: ys };
    }

    // Smallest correction that pulls any of `values` onto a target, or null.
    function snapDelta(values, targets) {
      var best = null;
      for (var i = 0; i < values.length; i += 1) {
        for (var j = 0; j < targets.length; j += 1) {
          var delta = targets[j] - values[i];
          if (Math.abs(delta) <= SNAP_PX && (best === null || Math.abs(delta) < Math.abs(best.delta))) {
            best = { delta: delta, at: targets[j] };
          }
        }
      }
      return best;
    }

    function toHex(color) {
      var m = /rgba?\(([^)]+)\)/.exec(color || '');
      if (!m) return '#000000';
      var p = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
      return '#' + p.slice(0, 3).map(function (v) {
        var h = Math.round(v).toString(16);
        return h.length < 2 ? '0' + h : h;
      }).join('');
    }

    function describe(el) {
      var rect = el.getBoundingClientRect();
      var cs = getComputedStyle(el);
      return {
        type: 'select',
        id: el.getAttribute('data-slyde-id'),
        tag: el.tagName ? el.tagName.toLowerCase() : 'div',
        text: (el.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 80),
        rect: { x: rect.left, y: rect.top, w: rect.width, h: rect.height },
        geom: stageGeometry(el),
        viewport: { w: window.innerWidth, h: window.innerHeight },
        style: {
          bold: parseInt(cs.fontWeight, 10) >= 600,
          italic: cs.fontStyle === 'italic',
          size: Math.round(parseFloat(cs.fontSize) * 10) / 10,
          color: toHex(cs.color),
          align: /center/.test(cs.textAlign) ? 'center' : (/right|end/.test(cs.textAlign) ? 'right' : 'left'),
          list: !!el.querySelector(':scope > ul, :scope > ol') || /^(ul|ol)$/i.test(el.tagName)
        }
      };
    }

    function select(el, additive) {
      if (additive && el) {
        var at = group.indexOf(el);
        if (!group.length && selection && selection !== el) group = [selection];
        if (at >= 0) group.splice(at, 1);
        else group.push(el);
        selection = group.length ? group[group.length - 1] : null;
      } else {
        group = [];
        selection = el;
      }
      paint();
      if (group.length > 1) {
        post({ type: 'multi', items: group.map(describe) });
      } else {
        if (group.length === 1) selection = group[0];
        post(selection ? describe(selection) : { type: 'deselect' });
      }
    }

    function closestBlock(node) {
      var el = node;
      while (el && el !== stage) {
        if (el.getAttribute && el.getAttribute('data-slyde-id')) return el;
        el = el.parentElement;
      }
      return null;
    }

    // ---- Images: paste, drag, resize -------------------------------------
    // The stored geometry is the source of truth, so the layer always works in
    // stage pixels (the document is CSS-scaled to fit the iframe) and reports
    // the final rectangle to the studio, which commits it through the state.

    function stageGeometry(el) {
      var r = el.getBoundingClientRect();
      var s = stage.getBoundingClientRect();
      var scale = s.width ? s.width / W : 1;
      return {
        x: Math.round((r.left - s.left) / scale),
        y: Math.round((r.top - s.top) / scale),
        w: Math.round(r.width / scale),
        h: Math.round(r.height / scale)
      };
    }

    function applyGeometry(el, g) {
      el.style.position = 'absolute';
      el.style.left = g.x + 'px';
      el.style.top = g.y + 'px';
      el.style.width = g.w + 'px';
      el.style.height = g.h + 'px';
      paint();
    }

    var drag = null;

    var DRAG_THRESHOLD = 4;

    stage.addEventListener('pointerdown', function (event) {
      if (editingNode || event.button !== 0) return;
      // Shift/Ctrl/Cmd-click builds a multi-selection; it must not also paint a
      // text selection across the slide.
      if (event.shiftKey || event.ctrlKey || event.metaKey) {
        event.preventDefault();
        try { window.getSelection().removeAllRanges(); } catch (e) {}
      }
      var target = closestBlock(event.target);
      if (!target) return;
      var isImage = target.tagName === 'IMG';
      // A picture moves as soon as it is grabbed. Any other block must already be
      // selected: the first click selects, dragging an already selected block
      // moves it, so plain text selection and double-click editing are untouched.
      var armed = isImage || (group.length <= 1 && selection === target && !event.shiftKey);
      if (!armed) return;
      if (isImage) event.preventDefault();
      drag = {
        el: target,
        id: target.getAttribute('data-slyde-id'),
        mode: event.shiftKey && isImage ? 'resize' : 'move',
        startX: event.clientX,
        startY: event.clientY,
        geometry: stageGeometry(target),
        started: isImage,
        pointerId: event.pointerId
      };
      if (isImage) beginDrag();
    }, true);

    document.addEventListener('pointerdown', function (event) {
      var name = event.target && event.target.getAttribute && event.target.getAttribute('data-slyde-handle');
      if (!name || !selection || editingNode || event.button !== 0) return;
      event.preventDefault();
      // The stage prevents the default focus change, so without this the frame
      // never receives keydown and the arrow-key nudge would be dead.
      try { window.focus(); } catch (e) {}
      drag = {
        el: selection,
        id: selection.getAttribute('data-slyde-id'),
        mode: 'resize',
        handle: name,
        startX: event.clientX,
        startY: event.clientY,
        geometry: stageGeometry(selection),
        started: false,
        pointerId: event.pointerId
      };
    }, true);

    function beginDrag() {
      var d = drag;
      d.started = true;
      snapTargets = collectSnapTargets(d.el);
      // Freeze the element where it is so it can move without reflowing the slide.
      applyGeometry(d.el, d.geometry);
      select(d.el);
      // The stage prevents the default focus change, so without this the frame
      // never receives keydown and the arrow-key nudge would be dead.
      try { window.focus(); } catch (e) {}
      if (d.el.setPointerCapture) {
        try { d.el.setPointerCapture(d.pointerId); } catch (e) {}
      }
    }

    window.addEventListener('pointermove', function (event) {
      if (!drag) return;
      if (!drag.started) {
        if (Math.abs(event.clientX - drag.startX) + Math.abs(event.clientY - drag.startY) < DRAG_THRESHOLD) return;
        beginDrag();
      }
      event.preventDefault();
      var s = stage.getBoundingClientRect();
      var scale = s.width ? s.width / W : 1;
      var dx = (event.clientX - drag.startX) / scale;
      var dy = (event.clientY - drag.startY) / scale;
      var next = {
        x: drag.geometry.x,
        y: drag.geometry.y,
        w: drag.geometry.w,
        h: drag.geometry.h
      };
      var guideX = null;
      var guideY = null;
      var snapping = snapTargets && !event.altKey;
      var g0 = drag.geometry;
      if (drag.mode === 'resize') {
        // A picture grabbed with Shift resizes from its corner, like a handle.
        var handle = drag.handle || 'se';
        var west = handle.indexOf('w') >= 0, east = handle.indexOf('e') >= 0;
        var north = handle.indexOf('n') >= 0, south = handle.indexOf('s') >= 0;
        var x1 = g0.x, y1 = g0.y, x2 = g0.x + g0.w, y2 = g0.y + g0.h;
        if (west) x1 = g0.x + dx;
        if (east) x2 = g0.x + g0.w + dx;
        if (north) y1 = g0.y + dy;
        if (south) y2 = g0.y + g0.h + dy;
        if (snapping) {
          if (west || east) {
            var sx = snapDelta([west ? x1 : x2], snapTargets.xs);
            if (sx) { if (west) x1 += sx.delta; else x2 += sx.delta; guideX = sx.at; }
          }
          if (north || south) {
            var sy = snapDelta([north ? y1 : y2], snapTargets.ys);
            if (sy) { if (north) y1 += sy.delta; else y2 += sy.delta; guideY = sy.at; }
          }
        }
        if (x2 - x1 < MIN_SIZE) { if (west) x1 = x2 - MIN_SIZE; else x2 = x1 + MIN_SIZE; }
        if (y2 - y1 < MIN_SIZE) { if (north) y1 = y2 - MIN_SIZE; else y2 = y1 + MIN_SIZE; }
        next.x = Math.round(x1);
        next.y = Math.round(y1);
        next.w = Math.round(x2 - x1);
        next.h = Math.round(y2 - y1);
      } else {
        var nx = g0.x + dx;
        var ny = g0.y + dy;
        if (snapping) {
          var mx = snapDelta([nx, nx + g0.w / 2, nx + g0.w], snapTargets.xs);
          if (mx) { nx += mx.delta; guideX = mx.at; }
          var my = snapDelta([ny, ny + g0.h / 2, ny + g0.h], snapTargets.ys);
          if (my) { ny += my.delta; guideY = my.at; }
        }
        next.x = Math.round(nx);
        next.y = Math.round(ny);
      }
      showGuides(guideX, guideY);
      applyGeometry(drag.el, next);
    });

    var justDragged = false;

    function endDrag() {
      if (!drag) return;
      showGuides(null, null);
      snapTargets = null;
      if (!drag.started) { drag = null; return; }
      justDragged = true;
      setTimeout(function () { justDragged = false; }, 0);
      var el = drag.el;
      var id = drag.id;
      var geometry = stageGeometry(el);
      drag = null;
      post({ type: 'image-geometry', id: id, rect: geometry });
    }
    window.addEventListener('pointerup', endDrag);
    window.addEventListener('pointercancel', endDrag);

    // Arrow keys nudge the selection: 1px, 10px with Shift. Without
    // stopImmediatePropagation the document keydown forwarder would also send
    // the arrow to the studio and change the slide.
    function nudge(dx, dy) {
      var targets = group.length > 1 ? group : (selection ? [selection] : []);
      if (!targets.length) return false;
      var changed = [];
      for (var i = 0; i < targets.length; i += 1) {
        var g = stageGeometry(targets[i]);
        g.x += dx;
        g.y += dy;
        applyGeometry(targets[i], g);
        changed.push({ id: targets[i].getAttribute('data-slyde-id'), rect: g });
      }
      post({ type: 'geometry-many', items: changed });
      return true;
    }

    document.addEventListener('keydown', function (event) {
      if (editingNode || (!selection && !group.length)) return;
      var step = event.shiftKey ? 10 : 1;
      var dx = 0;
      var dy = 0;
      if (event.key === 'ArrowLeft') dx = -step;
      else if (event.key === 'ArrowRight') dx = step;
      else if (event.key === 'ArrowUp') dy = -step;
      else if (event.key === 'ArrowDown') dy = step;
      else return;
      event.preventDefault();
      event.stopImmediatePropagation();
      nudge(dx, dy);
    }, true);

    function readImage(file) {
      var reader = new FileReader();
      reader.onload = function () {
        var image = new Image();
        image.onload = function () {
          post({
            type: 'image-paste',
            dataUrl: String(reader.result),
            natural: {
              w: image.naturalWidth || 1024,
              h: image.naturalHeight || 576
            }
          });
        };
        image.src = String(reader.result);
      };
      reader.readAsDataURL(file);
    }

    document.addEventListener('paste', function (event) {
      if (editingNode) return;
      var items = (event.clipboardData && event.clipboardData.items) || [];
      for (var i = 0; i < items.length; i += 1) {
        if (items[i].type && items[i].type.indexOf('image/') === 0) {
          var file = items[i].getAsFile();
          if (!file) continue;
          event.preventDefault();
          readImage(file);
          return;
        }
      }
    }, true);

    document.addEventListener('dragover', function (event) {
      if (!event.dataTransfer) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = 'copy';
    }, true);

    document.addEventListener('drop', function (event) {
      var files = event.dataTransfer && event.dataTransfer.files;
      if (!files || !files.length) return;
      for (var i = 0; i < files.length; i += 1) {
        if (files[i].type && files[i].type.indexOf('image/') === 0) {
          event.preventDefault();
          readImage(files[i]);
          return;
        }
      }
    }, true);
    stage.addEventListener('click', function (event) {
      if (editingNode) return;
      event.preventDefault();
      if (justDragged) return;
      var block = closestBlock(event.target);
      select(block, !!(block && (event.shiftKey || event.ctrlKey || event.metaKey)));
    }, true);

    document.addEventListener('dblclick', function (event) {
      var block = closestBlock(event.target);
      if (!block || editingNode) return;
      event.preventDefault();
      beginEdit(block);
    }, true);

    // Bullets without code: a block of lines becomes a list, and a list goes back
    // to plain lines. The studio commits the markup through `mutate_element`.
    function toggleList(el) {
      var id = el.getAttribute('data-slyde-id');
      var existing = el.querySelector(':scope > ul, :scope > ol');
      if (existing) {
        var lines = [].slice.call(existing.querySelectorAll('li')).map(function (li) {
          return (li.innerText || '').trim();
        });
        post({ type: 'html', id: id, value: lines.join('<br>') });
        return;
      }
      var raw = (el.innerText || '').trim();
      var parts = raw.indexOf('\n') >= 0 ? raw.split(/\n+/) : raw.split(/(?<=[.!?])\s+/);
      var items = parts.map(function (part) { return part.trim(); }).filter(Boolean);
      if (!items.length) return;
      var esc = function (text) {
        return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
      };
      post({
        type: 'html',
        id: id,
        value: '<ul style="margin:0;padding-left:1.2em">' +
          items.map(function (item) { return '<li>' + esc(item) + '</li>'; }).join('') + '</ul>'
      });
    }

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

    // Looks a block up by comparing the attribute instead of splicing the id
    // into a selector, so a hostile id can never change what the query means.
    function findById(id) {
      var all = stage.querySelectorAll('[data-slyde-id]');
      for (var i = 0; i < all.length; i += 1) {
        if (all[i].getAttribute('data-slyde-id') === id) return all[i];
      }
      return null;
    }

    window.addEventListener('message', function (event) {
      if (event.source !== window.parent) return;
      var data = event.data;
      if (!data || data.__slyde !== true) return;
      if (data.type === 'select-by-id') {
        select(typeof data.id === 'string' && data.id ? findById(data.id) : null);
      } else if (data.type === 'clear-selection') {
        select(null);
      } else if (data.type === 'select-step') {
        // Tab / Shift+Tab from the studio walks the blocks in reading order.
        var blocks = [].slice.call(stage.querySelectorAll('[data-slyde-id]')).filter(function (node) {
          var box = node.getBoundingClientRect();
          return box.width > 4 && box.height > 4 && node.tagName !== 'STYLE' && node.tagName !== 'BR';
        });
        if (!blocks.length) return;
        var at = blocks.indexOf(selection);
        var next = at < 0 ? (data.delta < 0 ? blocks.length - 1 : 0)
          : (at + data.delta + blocks.length) % blocks.length;
        select(blocks[next]);
      } else if (data.type === 'nudge') {
        nudge(data.dx || 0, data.dy || 0);
      } else if (data.type === 'edit-selected') {
        if (selection && !editingNode) beginEdit(selection);
      } else if (data.type === 'toggle-list') {
        if (selection) toggleList(selection);
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

/// Renders a single self-contained interactive HTML document for the whole deck.
///
/// Works without any server or network: double-clicking opens an interactive
/// presentation with slide navigation, fragments, fullscreen and the laser pointer.
pub fn render_standalone_presentation(deck: &Presentation) -> String {
    let (width, height) = deck.aspect_ratio.size_px();
    let title = escape_html(&deck.title);
    let theme = &deck.theme;
    let base_css = crate::starter::base_css();

    let slides_html: String = deck
        .slides
        .iter()
        .enumerate()
        .map(|(index, slide)| {
            let steps_json = serde_json::to_string(&slide.animation)
                .unwrap_or_else(|_| "[]".to_string())
                .replace("</", "<\\/");
            format!(
                r#"<section class="slyde-offline-slide" data-index="{index}" data-id="{id}" style="display: {display}">
  <div class="slyde-slide-content">{code}</div>
  <script type="application/json" class="slyde-slide-steps">{steps}</script>
</section>"#,
                index = index,
                id = escape_html(&slide.id),
                display = if index == 0 { "block" } else { "none" },
                code = slide.code,
                steps = steps_json,
            )
        })
        .collect();

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
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
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: #000;
    color: var(--slyde-fg);
    font-family: var(--slyde-font), -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    -webkit-font-smoothing: antialiased;
  }}
  #slyde-viewport {{
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }}
  #slyde-stage {{
    position: relative;
    width: {width}px;
    height: {height}px;
    background: var(--slyde-bg);
    overflow: hidden;
    transform-origin: center center;
    box-shadow: 0 10px 40px rgba(0,0,0,0.6);
  }}
  .slyde-offline-slide {{
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }}
  .slyde-slide-content {{
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }}
  #slyde-hud {{
    position: fixed;
    bottom: 20px;
    left: 50%;
    transform: translateX(-50%);
    background: rgba(18,18,20,0.85);
    backdrop-filter: blur(12px);
    border: 1px solid rgba(255,255,255,0.12);
    border-radius: 999px;
    padding: 6px 16px;
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 13px;
    color: #F4F3F1;
    z-index: 1000;
    opacity: 0;
    transition: opacity 220ms ease;
    user-select: none;
  }}
  body:hover #slyde-hud, body.hud-visible #slyde-hud {{ opacity: 1; }}
  #slyde-progress {{
    position: fixed;
    bottom: 0;
    left: 0;
    height: 3px;
    background: linear-gradient(90deg, #7C3AED, #FF6B4A);
    transition: width 240ms cubic-bezier(.16,1,.3,1);
    z-index: 1001;
  }}
  #slyde-laser {{
    position: absolute;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: radial-gradient(circle, #fff 20%, #FF6B4A 60%, rgba(124,58,237,0) 100%);
    box-shadow: 0 0 12px #FF6B4A;
    pointer-events: none;
    transform: translate(-50%, -50%);
    display: none;
    z-index: 999;
  }}
  {base_css}
</style>
</head>
<body>
<div id="slyde-viewport">
  <main id="slyde-stage">
    {slides}
    <div id="slyde-laser"></div>
  </main>
</div>
<div id="slyde-hud">
  <span id="slyde-count">1 / {total}</span>
  <span style="opacity:.4">?</span>
  <span><kbd>?</kbd><kbd>?</kbd> navigate</span>
  <span style="opacity:.4">?</span>
  <span><kbd>F</kbd> fullscreen</span>
  <span style="opacity:.4">?</span>
  <span><kbd>L</kbd> laser</span>
</div>
<div id="slyde-progress" style="width: {initial_pct}%"></div>

<script>
(function() {{
  var W = {width}, H = {height}, total = {total};
  var current = 0;
  var laserActive = false;
  var stage = document.getElementById('slyde-stage');
  var slides = document.querySelectorAll('.slyde-offline-slide');
  var countEl = document.getElementById('slyde-count');
  var progressEl = document.getElementById('slyde-progress');
  var laserEl = document.getElementById('slyde-laser');
  var hud = document.getElementById('slyde-hud');
  var hudTimer = null;

  function fit() {{
    var s = Math.min(window.innerWidth / W, window.innerHeight / H);
    stage.style.transform = 'scale(' + s + ')';
  }}
  window.addEventListener('resize', fit);
  fit();

  function showHud() {{
    document.body.classList.add('hud-visible');
    clearTimeout(hudTimer);
    hudTimer = setTimeout(function() {{ document.body.classList.remove('hud-visible'); }}, 2500);
  }}
  window.addEventListener('mousemove', showHud);

  function showSlide(idx) {{
    if (idx < 0 || idx >= total) return;
    current = idx;
    for (var i = 0; i < slides.length; i++) {{
      slides[i].style.display = i === current ? 'block' : 'none';
    }}
    countEl.textContent = (current + 1) + ' / ' + total;
    progressEl.style.width = Math.round(((current + 1) / total) * 100) + '%';
    location.hash = '#' + (current + 1);
  }}

  var initial = parseInt(location.hash.replace('#', ''), 10);
  if (!isNaN(initial) && initial >= 1 && initial <= total) showSlide(initial - 1);

  window.addEventListener('keydown', function(e) {{
    var k = e.key;
    if (k === 'ArrowRight' || k === ' ' || k === 'PageDown' || k === 'Enter') {{
      e.preventDefault();
      showSlide(current + 1);
    }} else if (k === 'ArrowLeft' || k === 'PageUp' || k === 'Backspace') {{
      e.preventDefault();
      showSlide(current - 1);
    }} else if (k === 'Home') {{
      e.preventDefault();
      showSlide(0);
    }} else if (k === 'End') {{
      e.preventDefault();
      showSlide(total - 1);
    }} else if (k === 'f' || k === 'F') {{
      if (!document.fullscreenElement) document.documentElement.requestFullscreen().catch(function(){{}});
      else document.exitFullscreen().catch(function(){{}});
    }} else if (k === 'l' || k === 'L') {{
      laserActive = !laserActive;
      laserEl.style.display = laserActive ? 'block' : 'none';
      stage.style.cursor = laserActive ? 'none' : '';
    }}
  }});

  stage.addEventListener('mousemove', function(e) {{
    if (!laserActive) return;
    var r = stage.getBoundingClientRect();
    var s = Math.min(window.innerWidth / W, window.innerHeight / H);
    var x = (e.clientX - r.left) / s;
    var y = (e.clientY - r.top) / s;
    laserEl.style.left = x + 'px';
    laserEl.style.top = y + 'px';
  }});
}})();
</script>
</body>
</html>"#,
        title = title,
        bg = theme.background,
        fg = theme.foreground,
        muted = theme.muted,
        accent = theme.accent,
        font = theme.font_family,
        width = width,
        height = height,
        base_css = base_css,
        slides = slides_html,
        total = deck.slides.len(),
        initial_pct = if deck.slides.is_empty() {
            0
        } else {
            100 / deck.slides.len()
        },
    )
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
                r#"<section class="page"><iframe src="{origin}/s/{id}?static=1&amp;t={token}" title=""></iframe></section>"#,
                origin = sandbox_origin,
                id = percent_encode(&slide.id),
                token = token
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
<body>
{pages}
</body>
</html>"#,
        title = escape_html(&deck.title),
        width = width,
        height = height,
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
    fn edit_mode_ships_resize_handles_and_snapping_but_static_mode_does_not() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let editing = render_document(&deck, &slide, true, true);
        for needle in [
            "data-slyde-handle",
            "nesw-resize",
            "collectSnapTargets",
            "snapDelta",
            "SAFE_MARGIN",
        ] {
            assert!(editing.contains(needle), "edit layer is missing {needle}");
        }
        assert!(editing.contains("EDIT = true"));
        // Presenting and exporting ship the same script with the layer switched off.
        let presenting = render_document(&deck, &slide, true, false);
        assert!(presenting.contains("EDIT = false"));
    }

    #[test]
    fn runtime_only_trusts_its_parent_and_never_splices_ids_into_selectors() {
        let slide = Slide::new("a".into(), "<p>x</p>".into());
        let deck = deck_with(slide.clone());
        let html = render_document(&deck, &slide, false, true);
        assert_eq!(html.matches("event.source !== window.parent").count(), 2);
        assert!(html.contains("function findById"));
        assert!(!html.contains("'[data-slyde-id=\"' + data.id"));
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

    #[test]
    fn standalone_presentation_embeds_all_slides_and_laser() {
        let slide1 = Slide::new("s1".into(), "<h1>First</h1>".into());
        let slide2 = Slide::new("s2".into(), "<p>Second</p>".into());
        let mut deck = deck_with(slide1);
        deck.slides.push(slide2);
        let html = render_standalone_presentation(&deck);
        assert!(html.contains("<h1>First</h1>"));
        assert!(html.contains("<p>Second</p>"));
        assert!(html.contains("slyde-laser"));
        assert!(html.contains("slyde-offline-slide"));
        assert!(html.contains("1 / 2"));
    }
}
