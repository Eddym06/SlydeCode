(() => {
  // Layout audit of one rendered slide. It runs inside the slide document, after
  // fonts and the technical renderers settled, and returns JSON: the number of
  // words and a list of measured problems. Every number is in stage pixels.
  const stage = document.getElementById('slyde-stage');
  const empty = (detail) => JSON.stringify({ words: 0, issues: [{ kind: 'broken', element: '', detail: detail }] });
  if (!stage) return empty('the slide did not render');

  const box = stage.getBoundingClientRect();
  const k = box.width / (stage.offsetWidth || box.width || 1); // stage scale (1 in the audit viewport)
  const px = (value) => Math.round(value / (k || 1));
  const issues = [];

  const describe = (el) => {
    const id = el.getAttribute('data-id') || el.id || '';
    const text = (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 34);
    return el.tagName.toLowerCase() + (id ? '#' + id : '') + (text ? ' "' + text + '"' : '');
  };
  const add = (kind, el, detail) => issues.push({ kind: kind, element: describe(el), detail: detail });

  // ---- colors ----
  const parseColor = (value) => {
    const match = /rgba?\(([^)]+)\)/.exec(value || '');
    if (!match) return null;
    const parts = match[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
    return { r: parts[0], g: parts[1], b: parts[2], a: parts.length > 3 ? parts[3] : 1 };
  };
  const over = (fg, bg) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });
  const luminance = (c) => {
    const channel = (v) => {
      const s = v / 255;
      return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
  };
  // The color behind an element, or null when a gradient or a picture sits there
  // (its contrast cannot be judged from a single color).
  const backdrop = (el) => {
    const layers = [];
    for (let node = el; node; node = node.parentElement) {
      const cs = getComputedStyle(node);
      if (cs.backgroundImage && cs.backgroundImage !== 'none') return null;
      const c = parseColor(cs.backgroundColor);
      if (c && c.a > 0) {
        layers.push(c);
        if (c.a >= 0.99) break;
      }
    }
    let base = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = layers.length - 1; i >= 0; i -= 1) base = over(layers[i], base);
    return base;
  };

  // ---- the text the slide actually shows ----
  const owners = [];
  const seen = new Set();
  let words = 0;
  const walker = document.createTreeWalker(stage, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const text = (walker.currentNode.nodeValue || '').trim();
    if (!text) continue;
    const el = walker.currentNode.parentElement;
    if (!el || /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE)$/.test(el.tagName)) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || parseFloat(cs.opacity) === 0) continue;
    words += text.split(/\s+/).length;
    if (!seen.has(el)) {
      seen.add(el);
      owners.push(el);
    }
  }

  const hasPicture = !!stage.querySelector('img, svg, canvas, video, .slyde-diagram');
  if (!words && !hasPicture) {
    issues.push({ kind: 'empty', element: '', detail: 'the slide shows nothing: no text, picture or diagram' });
  }
  if (words > 120) {
    issues.push({ kind: 'dense', element: '', detail: words + ' words on one slide; a projected slide reads best under ~80' });
  }

  stage.querySelectorAll('img').forEach((img) => {
    if (img.complete && img.naturalWidth === 0) add('broken', img, 'the image did not load');
  });

  const rects = [];
  owners.slice(0, 80).forEach((el) => {
    const rect = el.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) return;
    const cs = getComputedStyle(el);
    const technical = !!el.closest('pre, code, svg, .slyde-diagram');
    rects.push({ el: el, rect: rect });

    // Outside the canvas.
    const sides = [];
    if (rect.left < box.left - 2) sides.push('left by ' + px(box.left - rect.left) + 'px');
    if (rect.right > box.right + 2) sides.push('right by ' + px(rect.right - box.right) + 'px');
    if (rect.top < box.top - 2) sides.push('top by ' + px(box.top - rect.top) + 'px');
    if (rect.bottom > box.bottom + 2) sides.push('bottom by ' + px(rect.bottom - box.bottom) + 'px');
    if (sides.length) {
      add('overflow', el, 'extends beyond the canvas ' + sides.join(', '));
    } else {
      // Too close to the edge to look intentional.
      const margin = Math.min(rect.left - box.left, box.right - rect.right, rect.top - box.top, box.bottom - rect.bottom);
      if (px(margin) < 24 && !technical) {
        add('margin', el, 'only ' + px(margin) + 'px from the canvas edge; keep at least 40px');
      }
    }

    // Cut off by its own box.
    if ((cs.overflowX === 'hidden' || cs.overflowX === 'clip') && el.scrollWidth > el.clientWidth + 2) {
      add('clipped', el, 'text is cut off horizontally (' + el.scrollWidth + 'px of content in ' + el.clientWidth + 'px)');
    }
    if ((cs.overflowY === 'hidden' || cs.overflowY === 'clip') && el.scrollHeight > el.clientHeight + 2) {
      add('clipped', el, 'text is cut off vertically (' + el.scrollHeight + 'px of content in ' + el.clientHeight + 'px)');
    }

    // Type that cannot be read from the back of a room.
    const size = parseFloat(cs.fontSize);
    const floor = technical ? 12 : 14;
    if (size < floor) add('small', el, 'type is ' + Math.round(size * 10) / 10 + 'px; use at least ' + (technical ? 12 : 18) + 'px');

    // Contrast against the color behind it (skipped over gradients and pictures).
    const fill = (cs.webkitTextFillColor || '').replace(/\s/g, '');
    const color = parseColor(cs.color);
    if (color && color.a >= 0.1 && fill !== 'rgba(0,0,0,0)' && !technical) {
      const back = backdrop(el);
      if (back) {
        const fg = over(color, back);
        const a = luminance(fg);
        const b = luminance(back);
        const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
        const large = size >= 24 || (size >= 18.66 && parseInt(cs.fontWeight, 10) >= 700);
        const need = large ? 3 : 4.5;
        if (ratio < need) {
          add('contrast', el, 'contrast is ' + Math.round(ratio * 10) / 10 + ':1; text this size needs ' + need + ':1');
        }
      }
    }
  });

  // Text boxes sitting on top of each other.
  for (let i = 0; i < rects.length && issues.length < 40; i += 1) {
    for (let j = i + 1; j < rects.length; j += 1) {
      const a = rects[i];
      const b = rects[j];
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const w = Math.min(a.rect.right, b.rect.right) - Math.max(a.rect.left, b.rect.left);
      const h = Math.min(a.rect.bottom, b.rect.bottom) - Math.max(a.rect.top, b.rect.top);
      if (w <= 0 || h <= 0) continue;
      const smaller = Math.min(a.rect.width * a.rect.height, b.rect.width * b.rect.height);
      if ((w * h) / smaller > 0.3) add('overlap', a.el, 'overlaps ' + describe(b.el));
    }
  }

  // Worst first, bounded: the model gets a short list, not a wall of text.
  const order = ['broken', 'empty', 'overflow', 'clipped', 'overlap', 'contrast', 'small', 'dense', 'margin'];
  issues.sort((x, y) => order.indexOf(x.kind) - order.indexOf(y.kind));
  return JSON.stringify({ words: words, issues: issues.slice(0, 12) });
})()
