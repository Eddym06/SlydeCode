// Wireframe previews for the slide layouts, painted with the active theme so the
// layout picker shows what a new slide will look like in this deck.

/** Mirrors the built-in themes in `slyde-core` (background, foreground, accent). */
export const THEME_COLORS = {
  studio: { bg: "#17161A", fg: "#F6F2EA", accent: "#7C3AED" },
  paper: { bg: "#FBF8F3", fg: "#17161A", accent: "#7C3AED" },
  night: { bg: "#1E1B4B", fg: "#F5F3FF", accent: "#FF8265" },
  terminal: { bg: "#0B0C0E", fg: "#00FF66", accent: "#00E5FF" },
  academic: { bg: "#FFFFFF", fg: "#111111", accent: "#0055D4" },
  pitch: { bg: "#0F172A", fg: "#F8FAFC", accent: "#38BDF8" },
};

const SHAPES = {
  hero: (c) => `
    <rect x="6" y="7" width="12" height="1.6" rx=".8" fill="${c.accent}"/>
    <rect x="6" y="17" width="40" height="5" rx="1.4" fill="${c.fg}"/>
    <rect x="6" y="25" width="26" height="5" rx="1.4" fill="${c.accent}"/>
    <rect x="6" y="34" width="34" height="1.8" rx=".9" fill="${c.fg}" opacity=".45"/>`,
  bullet: (c) => `
    <rect x="6" y="6" width="26" height="4" rx="1.2" fill="${c.fg}"/>
    ${[0, 1, 2]
      .map(
        (i) => `<circle cx="8" cy="${18 + i * 8}" r="1.5" fill="${c.accent}"/>
    <rect x="12" y="${17 + i * 8}" width="${40 - i * 7}" height="2.2" rx="1.1" fill="${c.fg}" opacity=".6"/>`
      )
      .join("")}`,
  diagram: (c) => `
    <rect x="5" y="14" width="14" height="12" rx="2.5" fill="${c.accent}"/>
    <rect x="25" y="8" width="14" height="10" rx="2.5" fill="none" stroke="${c.fg}" stroke-width="1.2"/>
    <rect x="25" y="23" width="14" height="10" rx="2.5" fill="none" stroke="${c.fg}" stroke-width="1.2"/>
    <rect x="45" y="14" width="14" height="12" rx="2.5" fill="${c.fg}" opacity=".85"/>
    <path d="M19 20h3M22 20v-7h3M22 20v8h3M39 13h3v7h3M39 28h3v-8h3" stroke="${c.fg}" stroke-width="1" fill="none" opacity=".7"/>`,
  chart: (c) => `
    <rect x="6" y="5" width="22" height="3.4" rx="1.2" fill="${c.fg}"/>
    ${[12, 20, 15, 26]
      .map(
        (h, i) =>
          `<rect x="${9 + i * 12}" y="${36 - h}" width="8" height="${h}" rx="1.6" fill="${
            i === 3 ? c.accent : c.fg
          }" opacity="${i === 3 ? 1 : 0.5}"/>`
      )
      .join("")}`,
  image_layout: (c) => `
    <rect x="5" y="5" width="28" height="30" rx="3" fill="${c.accent}" opacity=".85"/>
    <circle cx="14" cy="14" r="3" fill="${c.bg}" opacity=".7"/>
    <path d="M5 30l9-8 7 6 5-4 7 6v5H8a3 3 0 0 1-3-3z" fill="${c.bg}" opacity=".45"/>
    <rect x="38" y="9" width="20" height="3.6" rx="1.2" fill="${c.fg}"/>
    <rect x="38" y="17" width="22" height="2" rx="1" fill="${c.fg}" opacity=".5"/>
    <rect x="38" y="22" width="18" height="2" rx="1" fill="${c.fg}" opacity=".5"/>`,
  code: (c) => `
    <rect x="5" y="6" width="54" height="28" rx="3" fill="${c.fg}" opacity=".1"/>
    <circle cx="9" cy="10" r="1" fill="${c.accent}"/><circle cx="12.5" cy="10" r="1" fill="${c.fg}" opacity=".5"/>
    <rect x="9" y="16" width="14" height="1.8" rx=".9" fill="${c.accent}"/>
    <rect x="13" y="21" width="26" height="1.8" rx=".9" fill="${c.fg}" opacity=".7"/>
    <rect x="13" y="26" width="18" height="1.8" rx=".9" fill="${c.fg}" opacity=".7"/>`,
};

SHAPES.custom = (c) => `
    <rect x="6" y="6" width="24" height="3.4" rx="1.2" fill="${c.fg}"/>
    <rect x="6" y="14" width="22" height="20" rx="2.5" fill="${c.accent}" opacity=".85"/>
    <rect x="34" y="14" width="24" height="9" rx="2.5" fill="${c.fg}" opacity=".55"/>
    <rect x="34" y="26" width="24" height="8" rx="2.5" fill="${c.fg}" opacity=".3"/>`;

export function layoutPreview(layout, themeId = "studio") {
  const colors = THEME_COLORS[themeId] || THEME_COLORS.studio;
  const shapes = (SHAPES[layout] || SHAPES.hero)(colors);
  return `<svg class="layout-preview" viewBox="0 0 64 40" role="img" aria-hidden="true">
    <rect width="64" height="40" rx="3.5" fill="${colors.bg}"/>${shapes}</svg>`;
}
