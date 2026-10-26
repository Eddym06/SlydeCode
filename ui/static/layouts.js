// Slide layouts that keep a slide's words.
//
// Picking a layout does not start a blank slide: the title, the paragraphs, the
// bullet points and the first picture already on the slide are read out and laid
// out again. The same layout can then be applied to every slide of the deck, so
// the deck stays coherent without anyone writing CSS.

export const LAYOUTS = ["hero", "bullet", "image_layout", "two_columns", "quote", "big_number", "section"];

const esc = (text) =>
  String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const clean = (text) => String(text || "").replace(/\s+/g, " ").trim();

/**
 * Reads the content of a slide's markup.
 * @param {string} code the slide source
 * @param {(html: string) => Document} [parse] injectable for tests
 */
export function extractContent(code, parse = (html) => new DOMParser().parseFromString(html, "text/html")) {
  const doc = parse(`<body>${code}</body>`);
  doc.querySelectorAll("style, script").forEach((node) => node.remove());
  const text = (node) => clean(node && node.textContent);

  const eyebrow = text(doc.querySelector(".eyebrow"));
  const heading = doc.querySelector(".title") || doc.querySelector("h1, h2, h3");
  const title = text(heading);
  const sub = doc.querySelector(".sub");
  const used = new Set([heading, sub, doc.querySelector(".eyebrow")].filter(Boolean));
  const subtitle = text(sub);

  const bullets = [...doc.querySelectorAll("li")].map(text).filter(Boolean);
  const paragraphs = [...doc.querySelectorAll("p, blockquote")]
    .filter((node) => !used.has(node) && !node.closest("li"))
    .map(text)
    .filter(Boolean);
  const image = doc.querySelector("img");

  let headline = title;
  if (!headline) {
    // No heading: the first block of text plays that role.
    headline = paragraphs.shift() || subtitle || bullets[0] || "";
  }
  return {
    eyebrow,
    title: headline,
    subtitle,
    paragraphs,
    bullets,
    image: image && image.getAttribute("src") ? image.getAttribute("src") : "",
  };
}

/** The first number-looking token in the slide's words ("72%", "$1.2M", "3x"). */
export function leadNumber(content) {
  const haystack = [content.title, content.subtitle, ...content.paragraphs, ...content.bullets].join(" ");
  const match = /[$€£]?\d[\d.,]*\s?(?:%|[kKmMbB]\b|x\b|×)?/.exec(haystack);
  return match ? match[0].trim() : "";
}

const BASE = `.slide{position:absolute;inset:0;display:flex;flex-direction:column;box-sizing:border-box}`;

function shell(name, css, body, size) {
  const { w, h } = size;
  const u = Math.min(w / 1280, h / 720);
  // Pixel sizes below are authored for a 1280x720 canvas and scaled by `u`.
  const px = (value) => `${Math.round(value * u * 10) / 10}px`;
  return `<div class="slide ${name}">${body}</div>
<style>
  ${BASE}
  .${name}{padding:${px(84)} ${px(92)};}
  ${css(px, w < h)}
</style>`;
}

const list = (items, cls) =>
  items.length ? `<ul class="${cls}">${items.map((item) => `<li>${esc(item)}</li>`).join("")}</ul>` : "";

const eyebrow = (content) => (content.eyebrow ? `<span class="eyebrow">${esc(content.eyebrow)}</span>` : "");

/**
 * Lays `content` out as `layout`. The result is a slide fragment: markup plus
 * one `<style>` block, painted only with the deck theme variables.
 */
export function renderLayout(layout, content, size = { w: 1280, h: 720 }) {
  const title = esc(content.title);
  const subtitle = content.subtitle ? `<p class="sub">${esc(content.subtitle)}</p>` : "";
  const points = content.bullets.length ? content.bullets : content.paragraphs;

  switch (layout) {
    case "bullet": {
      const body = `${eyebrow(content)}<h1 class="title">${title}</h1>${subtitle}${list(points, "points")}`;
      return shell(
        "layout-bullet",
        (px) => `
  .layout-bullet{gap:${px(22)};justify-content:center;}
  .layout-bullet .title{font-size:${px(60)};}
  .layout-bullet .points{margin:${px(12)} 0 0;padding:0;list-style:none;display:grid;gap:${px(18)};}
  .layout-bullet .points li{position:relative;padding-left:${px(34)};font-size:${px(32)};line-height:1.35;}
  .layout-bullet .points li::before{content:"";position:absolute;left:0;top:.55em;width:${px(14)};height:${px(14)};border-radius:50%;background:var(--slyde-accent);}`,
        body,
        size
      );
    }
    case "image_layout": {
      const media = content.image
        ? `<img class="media" src="${esc(content.image)}" alt="">`
        : `<div class="media media-empty" aria-hidden="true"></div>`;
      const copy = subtitle || list(points.slice(0, 4), "points") || "";
      const body = `<div class="copy">${eyebrow(content)}<h1 class="title">${title}</h1>${copy}</div>${media}`;
      return shell(
        "layout-image",
        (px, portrait) => `
  .layout-image{display:grid;gap:${px(56)};align-items:center;${
          portrait ? "grid-template-rows:auto 1fr;" : "grid-template-columns:1.05fr 1fr;"
        }}
  .layout-image .copy{display:grid;gap:${px(22)};}
  .layout-image .title{font-size:${px(56)};}
  .layout-image .media{width:100%;height:${portrait ? "100%" : px(480)};object-fit:cover;border-radius:${px(22)};}
  .layout-image .media-empty{background:linear-gradient(135deg,var(--slyde-accent),transparent 120%);opacity:.55;}
  .layout-image .points{margin:0;padding-left:1.1em;font-size:${px(26)};line-height:1.5;color:var(--slyde-muted);}`,
        body,
        size
      );
    }
    case "two_columns": {
      const half = Math.ceil(points.length / 2);
      const left = points.slice(0, half);
      const right = points.slice(half);
      const column = (items) =>
        items.length ? `<div class="col">${items.map((item) => `<p>${esc(item)}</p>`).join("")}</div>` : `<div class="col"></div>`;
      const body = `${eyebrow(content)}<h1 class="title">${title}</h1>${subtitle}<div class="cols">${column(left.length ? left : [content.subtitle].filter(Boolean))}${column(right)}</div>`;
      return shell(
        "layout-columns",
        (px, portrait) => `
  .layout-columns{gap:${px(22)};}
  .layout-columns .title{font-size:${px(54)};}
  .layout-columns .cols{display:grid;gap:${px(48)};margin-top:${px(18)};${
          portrait ? "" : "grid-template-columns:1fr 1fr;"
        }}
  .layout-columns .col{display:grid;gap:${px(18)};align-content:start;border-top:${px(3)} solid var(--slyde-accent);padding-top:${px(20)};}
  .layout-columns .col p{margin:0;font-size:${px(26)};line-height:1.5;}`,
        body,
        size
      );
    }
    case "quote": {
      const quote = content.paragraphs[0] || content.subtitle || content.title;
      const by = content.paragraphs[0] ? content.title : content.subtitle ? "" : "";
      const body = `<span class="mark" aria-hidden="true">“</span><blockquote class="title quote">${esc(quote)}</blockquote>${
        by ? `<p class="sub by">${esc(by)}</p>` : ""
      }`;
      return shell(
        "layout-quote",
        (px) => `
  .layout-quote{justify-content:center;gap:${px(18)};}
  .layout-quote .mark{font-size:${px(200)};line-height:.6;height:${px(90)};color:var(--slyde-accent);font-family:Georgia,serif;}
  .layout-quote .quote{margin:0;font-size:${px(54)};line-height:1.2;font-weight:500;letter-spacing:-.02em;max-width:22ch;}
  .layout-quote .by{margin-top:${px(10)};font-size:${px(22)};}
  .layout-quote .by::before{content:"— ";}`,
        body,
        size
      );
    }
    case "big_number": {
      const number = leadNumber(content) || content.title;
      const label = number === content.title ? content.subtitle : content.title;
      const body = `${eyebrow(content)}<div class="figure title">${esc(number)}</div>${
        label ? `<p class="sub label">${esc(label)}</p>` : ""
      }${content.subtitle && label !== content.subtitle ? `<p class="sub">${esc(content.subtitle)}</p>` : ""}`;
      return shell(
        "layout-number",
        (px) => `
  .layout-number{justify-content:center;gap:${px(14)};}
  .layout-number .figure{margin:0;font-size:${px(230)};line-height:.9;letter-spacing:-.05em;font-weight:650;color:var(--slyde-accent);}
  .layout-number .label{font-size:${px(34)};color:var(--slyde-fg);max-width:24ch;}`,
        body,
        size
      );
    }
    case "section": {
      const body = `${eyebrow(content) || '<span class="eyebrow"></span>'}<h1 class="title">${title}</h1><span class="rule" aria-hidden="true"></span>${subtitle}`;
      return shell(
        "layout-section",
        (px) => `
  .layout-section{justify-content:center;align-items:flex-start;gap:${px(26)};}
  .layout-section .title{font-size:${px(96)};max-width:14ch;}
  .layout-section .rule{display:block;width:${px(120)};height:${px(8)};border-radius:${px(4)};background:var(--slyde-accent);}`,
        body,
        size
      );
    }
    default: {
      // hero
      const body = `${eyebrow(content)}<h1 class="title">${title}</h1>${subtitle}`;
      return shell(
        "layout-hero",
        (px) => `
  .layout-hero{justify-content:center;gap:${px(24)};}
  .layout-hero .title{font-size:${px(84)};}`,
        body,
        size
      );
    }
  }
}
