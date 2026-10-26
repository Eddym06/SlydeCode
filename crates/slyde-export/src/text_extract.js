(() => {
  // Reads every visible text block of the slide, as it was laid out, and hides
  // that text so the following screenshot is a clean plate. The result is a JSON
  // string: [{ x, y, w, h, align, line_height, lines: [[run]] }] in stage pixels.
  const stage = document.getElementById('slyde-stage');
  if (!stage) return '[]';

  const SKIP = 'svg, canvas, pre, video, img, script, style, noscript, [data-slyde-ui], ' +
    '.slyde-diagram, .katex, .katex-display, .mermaid';
  const origin = stage.getBoundingClientRect();
  const k = stage.offsetWidth ? origin.width / stage.offsetWidth : 1;

  const toHex = (v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0');
  const parseColor = (value) => {
    if (!value) return null;
    let m = value.match(/^rgba?\(([^)]+)\)/);
    if (m) {
      const p = m[1].split(/[\s,\/]+/).filter(Boolean).map(Number);
      return { hex: (toHex(p[0]) + toHex(p[1]) + toHex(p[2])).toUpperCase(), a: p.length > 3 ? p[3] : 1 };
    }
    m = value.match(/^color\(srgb ([^)]+)\)/);
    if (m) {
      const [rgb, alpha] = m[1].split('/');
      const p = rgb.trim().split(/\s+/).map(Number);
      return {
        hex: (toHex(p[0] * 255) + toHex(p[1] * 255) + toHex(p[2] * 255)).toUpperCase(),
        a: alpha === undefined ? 1 : Number(alpha),
      };
    }
    return null;
  };

  const blockOf = (node) => {
    let el = node.parentElement;
    while (el && el !== stage) {
      const d = getComputedStyle(el).display;
      if (d !== 'inline' && d !== 'contents') return el;
      el = el.parentElement;
    }
    return stage;
  };

  const effectiveOpacity = (el) => {
    let o = 1;
    for (let n = el; n && n !== document.documentElement; n = n.parentElement) {
      o *= Number(getComputedStyle(n).opacity);
    }
    return o;
  };

  const fillOf = (el) => {
    const cs = getComputedStyle(el);
    let c = parseColor(cs.webkitTextFillColor) || parseColor(cs.color);
    if (c && c.a === 0) {
      // Gradient text paints through the background; take its first stop.
      const stop = (cs.backgroundImage || '').match(/rgba?\([^)]+\)|color\(srgb[^)]+\)/);
      c = (stop && parseColor(stop[0])) ||
        parseColor(getComputedStyle(stage).getPropertyValue('--slyde-accent').trim()) ||
        { hex: 'FFFFFF', a: 1 };
    }
    return c || { hex: '000000', a: 1 };
  };

  const styleOf = (el) => {
    const cs = getComputedStyle(el);
    const fill = fillOf(el);
    const family = (cs.fontFamily || '').split(',')[0].replace(/["']/g, '').trim() || 'Geist';
    return {
      size: parseFloat(cs.fontSize) || 16,
      color: fill.hex,
      alpha: Math.max(0, Math.min(1, fill.a * effectiveOpacity(el))),
      bold: Number(cs.fontWeight) >= 600,
      italic: cs.fontStyle === 'italic' || cs.fontStyle === 'oblique',
      underline: (cs.textDecorationLine || '').includes('underline'),
      font: family,
      upper: cs.textTransform === 'uppercase',
      lower: cs.textTransform === 'lowercase',
    };
  };
  const keyOf = (s) => [s.size, s.color, s.alpha, s.bold, s.italic, s.underline, s.font].join('|');

  // 1. Group the visible text nodes by their nearest block-level ancestor.
  const groups = new Map();
  const walker = document.createTreeWalker(stage, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (!/\S/.test(node.nodeValue)) continue;
    const parent = node.parentElement;
    if (!parent || parent.closest(SKIP)) continue;
    const cs = getComputedStyle(parent);
    if (cs.visibility === 'hidden' || cs.display === 'none') continue;
    const block = blockOf(node);
    if (!groups.has(block)) groups.set(block, []);
    groups.get(block).push(node);
  }

  const blocks = [];
  const marked = new Set();
  for (const [block, nodes] of groups) {
    if (blocks.length >= 200) break;
    const bcs = getComputedStyle(block);
    const lines = [];
    let current = null;
    let box = null;

    for (const node of nodes) {
      const style = styleOf(node.parentElement);
      const text = node.nodeValue;
      const range = document.createRange();
      for (let i = 0; i < text.length; i += 1) {
        let ch = text[i];
        const isSpace = /\s/.test(ch);
        range.setStart(node, i);
        range.setEnd(node, i + 1);
        const rect = range.getClientRects()[0];
        if (isSpace) {
          if (current && current.runs.length) current.pendingSpace = true;
          continue;
        }
        if (!rect || (!rect.width && !rect.height)) continue;
        if (style.upper) ch = ch.toUpperCase();
        else if (style.lower) ch = ch.toLowerCase();
        const cy = rect.top + rect.height / 2;
        if (!current || Math.abs(cy - current.cy) > style.size * k * 0.6) {
          current = { cy, runs: [], key: null, pendingSpace: false };
          lines.push(current);
        }
        const key = keyOf(style);
        if (current.pendingSpace) {
          const last = current.runs[current.runs.length - 1];
          if (last) last.text += ' ';
          current.pendingSpace = false;
        }
        let run = current.runs[current.runs.length - 1];
        if (!run || current.key !== key) {
          run = {
            text: '', size: style.size, color: style.color, alpha: style.alpha,
            bold: style.bold, italic: style.italic, underline: style.underline, font: style.font,
          };
          current.runs.push(run);
          current.key = key;
        }
        run.text += ch;
        const r = {
          l: (rect.left - origin.left) / k, t: (rect.top - origin.top) / k,
          rt: (rect.right - origin.left) / k, b: (rect.bottom - origin.top) / k,
        };
        current.box = current.box
          ? {
              l: Math.min(current.box.l, r.l), t: Math.min(current.box.t, r.t),
              rt: Math.max(current.box.rt, r.rt), b: Math.max(current.box.b, r.b),
            }
          : r;
        box = box
          ? { l: Math.min(box.l, r.l), t: Math.min(box.t, r.t), rt: Math.max(box.rt, r.rt), b: Math.max(box.b, r.b) }
          : r;
      }
      marked.add(node.parentElement);
    }
    if (!box || !lines.length) continue;

    let lineHeight = parseFloat(bcs.lineHeight);
    if (lines.length > 1) {
      lineHeight = (lines[lines.length - 1].cy - lines[0].cy) / (lines.length - 1) / k;
    } else if (!(lineHeight > 0)) {
      lineHeight = lines[0].runs[0].size * 1.2;
    }
    const align = bcs.textAlign === 'center' ? 'center'
      : (bcs.textAlign === 'right' || bcs.textAlign === 'end') ? 'right' : 'left';
    blocks.push({
      x: box.l, y: box.t, w: box.rt - box.l, h: box.b - box.t,
      align, line_height: lineHeight,
      lines: lines.map((l) => l.runs.filter((r) => r.text.length)),
    });
  }

  // 2. Hide the text that was extracted so the screenshot is a clean plate.
  const style = document.createElement('style');
  style.setAttribute('data-slyde-ui', '1');
  style.textContent =
    '[data-pptx-text]{color:transparent!important;-webkit-text-fill-color:transparent!important;' +
    'text-shadow:none!important;-webkit-text-stroke:0!important}' +
    '[data-pptx-clip]{background:none!important}';
  document.head.appendChild(style);
  for (const el of marked) {
    el.setAttribute('data-pptx-text', '1');
    if (getComputedStyle(el).webkitBackgroundClip === 'text') el.setAttribute('data-pptx-clip', '1');
  }
  return JSON.stringify(blocks);
})()
