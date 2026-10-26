// Ready-made blocks for the elements library: shapes, icons, charts and tables.
//
// Everything is inline SVG or plain HTML painted with the deck theme variables
// (`--slyde-accent`, `--slyde-fg`, …), so a block follows the theme the moment
// the theme changes. No scripts, no external files, nothing the sandbox would
// refuse. Each item carries its default size on the 1280x720 canvas.

const ACCENT = "var(--slyde-accent)";
const FG = "var(--slyde-fg)";
const MUTED = "var(--slyde-muted)";

const shape = (path, extra = "") =>
  `<svg viewBox="0 0 100 100" preserveAspectRatio="none" width="100%" height="100%" fill="${ACCENT}" ${extra} aria-hidden="true">${path}</svg>`;

const icon = (body) =>
  `<svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="${ACCENT}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

const chart = (body) =>
  `<svg viewBox="0 0 200 120" preserveAspectRatio="xMidYMid meet" width="100%" height="100%" aria-hidden="true" style="font-family:var(--slyde-font)">${body}</svg>`;

const cell = (tag, text, header) =>
  `<${tag} style="padding:10px 14px;border:1px solid ${MUTED};text-align:left;${header ? `background:${ACCENT};color:var(--slyde-bg);font-weight:650` : `color:${FG}`}">${text}</${tag}>`;

function table(columns, rows) {
  const head = `<tr>${Array.from({ length: columns }, (_, i) => cell("th", `Header ${i + 1}`, true)).join("")}</tr>`;
  const body = Array.from(
    { length: rows },
    () => `<tr>${Array.from({ length: columns }, () => cell("td", "Text", false)).join("")}</tr>`
  ).join("");
  return `<table style="width:100%;height:100%;border-collapse:collapse;font-size:20px;font-family:var(--slyde-font)">${head}${body}</table>`;
}

const SHAPES = [
  ["rect", 220, 140, shape('<rect width="100" height="100"/>')],
  ["rounded", 220, 140, shape('<rect width="100" height="100" rx="18"/>')],
  ["circle", 160, 160, shape('<ellipse cx="50" cy="50" rx="50" ry="50"/>')],
  ["ring", 160, 160, shape('<ellipse cx="50" cy="50" rx="44" ry="44" fill="none" stroke="var(--slyde-accent)" stroke-width="10"/>')],
  ["triangle", 180, 160, shape('<polygon points="50,2 98,98 2,98"/>')],
  ["diamond", 160, 160, shape('<polygon points="50,2 98,50 50,98 2,50"/>')],
  ["hexagon", 180, 160, shape('<polygon points="25,4 75,4 100,50 75,96 25,96 0,50"/>')],
  ["star", 170, 170, shape('<polygon points="50,3 62,36 97,38 69,59 79,94 50,74 21,94 31,59 3,38 38,36"/>')],
  ["arrow", 220, 120, shape('<polygon points="0,32 62,32 62,6 100,50 62,94 62,68 0,68"/>')],
  ["chevron", 120, 160, shape('<polygon points="16,0 100,50 16,100 0,86 68,50 0,14"/>')],
  ["bubble", 220, 150, shape('<path d="M8 4h84a6 6 0 0 1 6 6v58a6 6 0 0 1-6 6H44L22 96V74H8a6 6 0 0 1-6-6V10a6 6 0 0 1 6-6z"/>')],
  ["line", 260, 12, shape('<rect y="35" width="100" height="30" rx="15"/>')],
].map(([id, w, h, html]) => ({ id, category: "shapes", w, h, html }));

const ICONS = [
  ["check", '<path d="m5 12.5 4.5 4.5L19 7"/>'],
  ["close", '<path d="M6 6l12 12M18 6 6 18"/>'],
  ["plus", '<path d="M12 5v14M5 12h14"/>'],
  ["star", '<path d="m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z"/>'],
  ["heart", '<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.5A4 4 0 0 1 19 10c0 5.6-7 10-7 10z"/>'],
  ["bolt", '<path d="M13 3 5 13h6l-1 8 8-10h-6z"/>'],
  ["rocket", '<path d="M5 15c-1 1-1.5 4-1.5 4s3-.5 4-1.5M9 15l-3-3 6-6a8 8 0 0 1 8-2 8 8 0 0 1-2 8l-6 6-3-3"/><circle cx="15" cy="9" r="1"/>'],
  ["target", '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1"/>'],
  ["idea", '<path d="M9 18h6M10 21h4M12 3a6 6 0 0 0-4 10.5c.8.8 1 1.5 1 2.5h6c0-1 .2-1.7 1-2.5A6 6 0 0 0 12 3z"/>'],
  ["growth", '<path d="M4 20V4M4 20h16M8 16v-4M12 16V9M16 16V6"/>'],
  ["team", '<circle cx="9" cy="8" r="3.2"/><path d="M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6"/><circle cx="17" cy="9" r="2.5"/><path d="M17 14c2.8 0 4 2 4 5"/>'],
  ["globe", '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3.2 3 14.8 0 18M12 3c-3 3.2-3 14.8 0 18"/>'],
  ["shield", '<path d="m12 3 8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/><path d="m9 12 2 2 4-4"/>'],
  ["lock", '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>'],
  ["clock", '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>'],
  ["calendar", '<rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 10h16M8 3v4M16 3v4"/>'],
  ["mail", '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/>'],
  ["phone", '<path d="M5 4h4l2 5-2.5 1.5a11 11 0 0 0 5 5L15 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2z"/>'],
  ["pin", '<path d="M12 21s-7-6-7-11a7 7 0 0 1 14 0c0 5-7 11-7 11z"/><circle cx="12" cy="10" r="2.5"/>'],
  ["search", '<circle cx="11" cy="11" r="6"/><path d="m16 16 4 4"/>'],
  ["settings", '<path d="M4 7h10M18 7h2M4 17h2M10 17h10"/><circle cx="16" cy="7" r="2"/><circle cx="8" cy="17" r="2"/>'],
  ["flag", '<path d="M5 21V4M5 4h11l-2 4 2 4H5"/>'],
  ["trophy", '<path d="M8 4h8v5a4 4 0 0 1-8 0zM8 6H4v1a3 3 0 0 0 4 3M16 6h4v1a3 3 0 0 1-4 3M12 13v4M8 21h8M10 17h4"/>'],
  ["like", '<path d="M7 11v9H4v-9zM7 11l4-7c1.5 0 2.5 1 2.5 2.5V9H19a2 2 0 0 1 2 2.4l-1.2 6A2 2 0 0 1 18 19H7"/>'],
  ["leaf", '<path d="M5 19c0-8 5-14 15-14 0 10-6 15-14 15M5 19l8-8"/>'],
].map(([id, body]) => ({ id, category: "icons", w: 96, h: 96, html: icon(body) }));

const CHARTS = [
  [
    "bars",
    360,
    220,
    chart(
      `<path d="M10 110h180" stroke="${MUTED}" stroke-width="1"/>` +
        [40, 70, 55, 90, 65]
          .map(
            (height, index) =>
              `<rect x="${18 + index * 35}" y="${110 - height}" width="22" height="${height}" rx="4" fill="${index === 3 ? ACCENT : FG}" opacity="${index === 3 ? 1 : 0.5}"/>`
          )
          .join("")
    ),
  ],
  [
    "line",
    360,
    220,
    chart(
      `<path d="M10 110h180M10 80h180M10 50h180" stroke="${MUTED}" stroke-width="0.6" opacity="0.5"/>` +
        `<polyline points="14,95 52,70 90,78 128,40 166,52 190,22" fill="none" stroke="${ACCENT}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>` +
        [[14, 95], [52, 70], [90, 78], [128, 40], [166, 52], [190, 22]]
          .map(([x, y]) => `<circle cx="${x}" cy="${y}" r="3.5" fill="var(--slyde-bg)" stroke="${ACCENT}" stroke-width="2"/>`)
          .join("")
    ),
  ],
  [
    "donut",
    260,
    260,
    chart(
      `<g transform="rotate(-90 100 60)" fill="none" stroke-width="22">` +
        `<circle cx="100" cy="60" r="40" stroke="${FG}" opacity="0.18"/>` +
        `<circle cx="100" cy="60" r="40" stroke="${ACCENT}" stroke-dasharray="125 251"/>` +
        `<circle cx="100" cy="60" r="40" stroke="${FG}" opacity="0.55" stroke-dasharray="62 251" stroke-dashoffset="-128"/></g>`
    ),
  ],
  [
    "progress",
    220,
    220,
    chart(
      `<g transform="rotate(-90 100 60)" fill="none" stroke-width="14" stroke-linecap="round">` +
        `<circle cx="100" cy="60" r="44" stroke="${FG}" opacity="0.15"/>` +
        `<circle cx="100" cy="60" r="44" stroke="${ACCENT}" stroke-dasharray="199 277"/></g>` +
        `<text x="100" y="68" text-anchor="middle" font-size="26" font-weight="700" fill="${FG}">72%</text>`
    ),
  ],
  [
    "meters",
    380,
    200,
    chart(
      [["Label 1", 82], ["Label 2", 58], ["Label 3", 36]]
        .map(
          ([label, value], index) =>
            `<text x="8" y="${24 + index * 36}" font-size="11" fill="${FG}">${label}</text>` +
            `<rect x="8" y="${30 + index * 36}" width="184" height="8" rx="4" fill="${FG}" opacity="0.15"/>` +
            `<rect x="8" y="${30 + index * 36}" width="${value * 1.84}" height="8" rx="4" fill="${index === 0 ? ACCENT : FG}" opacity="${index === 0 ? 1 : 0.6}"/>`
        )
        .join("")
    ),
  ],
].map(([id, w, h, html]) => ({ id, category: "charts", w, h, html }));

const TABLES = [
  ["2x2", 2, 2, 520, 150],
  ["3x3", 3, 3, 640, 200],
  ["4x4", 4, 4, 760, 250],
].map(([id, columns, rows, w, h]) => ({ id, category: "tables", w, h, html: table(columns, rows) }));

export const CATEGORIES = ["shapes", "icons", "charts", "tables"];
export const CATALOG = [...SHAPES, ...ICONS, ...CHARTS, ...TABLES];

/** Centered position for a block of `w`x`h` on a canvas, clamped inside it. */
export function centeredPlacement(item, canvas) {
  const scale = Math.min(1, (canvas.w * 0.8) / item.w, (canvas.h * 0.8) / item.h);
  const w = Math.round(item.w * scale);
  const h = Math.round(item.h * scale);
  return { x: Math.round((canvas.w - w) / 2), y: Math.round((canvas.h - h) / 2), w, h };
}
