// Provider marks for the copilot picker.
//
// These are deliberately small, geometric interpretations rather than copied
// brand assets: they stay legible at 16px, inherit the theme, and never pull
// remote images into the app. `providerIcon` returns trusted static SVG only.

const ICONS = {
  openai: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M12 3.1 19.7 7.6v8.8L12 20.9 4.3 16.4V7.6L12 3.1Z" stroke="currentColor" stroke-width="1.35"/>
    <path d="M8.4 9.7 12 7.6l3.6 2.1v4.6L12 16.4l-3.6-2.1V9.7Z" fill="currentColor" opacity=".5"/>
  </svg>`,
  claude: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M12 3.2v17.6M3.2 12h17.6M5.8 5.8l12.4 12.4M18.2 5.8 5.8 18.2"
      stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/>
    <circle cx="12" cy="12" r="2.1" fill="currentColor"/>
  </svg>`,
  gemini: `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
    <path d="M12 2.4c.7 5.4 2 6.7 7.4 7.4-5.4.7-6.7 2-7.4 7.4-.7-5.4-2-6.7-7.4-7.4 5.4-.7 6.7-2 7.4-7.4Z"/>
    <path d="M19.1 14.2c.3 2.3.9 2.9 3.2 3.2-2.3.3-2.9.9-3.2 3.2-.3-2.3-.9-2.9-3.2-3.2 2.3-.3 2.9-.9 3.2-3.2Z" opacity=".72"/>
  </svg>`,
  xai: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M5 5.2 19 18.8M19 5.2 5 18.8" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
  </svg>`,
  groq: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M13.4 2.8 5.9 13h5.2l-1.6 8.1L18 10.2h-5.4l.8-7.4Z" fill="currentColor"/>
  </svg>`,
  openrouter: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <circle cx="5.3" cy="6.5" r="1.8" fill="currentColor"/>
    <circle cx="18.7" cy="17.5" r="1.8" fill="currentColor"/>
    <path d="M7.1 7.6 10 10.2c1.2 1.1 1.9 1.5 3.1 1.5h2.8M16.9 16.4 14 13.8c-1.2-1.1-1.9-1.5-3.1-1.5H8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
    <path d="m14 9.6 2.4 2.1-2.4 2.1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>`,
  deepseek: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M3.4 14.4c2.5-1.3 4.1-3.2 5.7-5.8 1.7-2.7 3.6-3.7 5.7-3.1 1.9.5 3.3 2 3.9 4.2.7 2.6-.2 5.5-2.2 7.2-1.9 1.6-4.7 1.8-6.6.4-1.1-.8-1.5-2.2-1-3.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
    <circle cx="13.8" cy="10.4" r="1.1" fill="currentColor"/>
    <path d="M16.7 7.1c.8-.8 1.8-1.2 3.1-1.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>
  </svg>`,
  nvidia: `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <rect x="3.6" y="5" width="16.8" height="14" rx="3.2" stroke="currentColor" stroke-width="1.4"/>
    <path d="M7 15.4V9.3c2.8-.2 5.4.5 7.2 2.6-1.7 2.2-4.2 3.6-7.2 3.5Z" fill="currentColor" opacity=".55"/>
    <circle cx="10.6" cy="12.1" r="1.2" fill="currentColor"/>
  </svg>`,
};

const TINT = {
  openai: "var(--ink)",
  claude: "#d97757",
  gemini: "#4285f4",
  xai: "var(--ink)",
  groq: "#f55036",
  openrouter: "#8b5cf6",
  deepseek: "#4d6bfe",
  nvidia: "#76b900",
};

export function providerIcon(id) {
  return ICONS[id] || ICONS.openai;
}

export function providerTint(id) {
  return TINT[id] || "var(--ink)";
}
