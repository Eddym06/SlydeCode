// Minimal i18n layer. Every user-facing string goes through here; adding a
// string means adding it to both locales in the same change.

// Dictionaries live in ./locales/<code>.js, one module per locale, so both
// files can be reviewed side by side. Keep them in lockstep:
// tests/ui/i18n.test.mjs fails when keys, placeholders or values drift.

import en from "./locales/en.js";
import es from "./locales/es.js";

const DICT = { en, es };

const listeners = new Set();
let current = null;

function detect() {
  const stored = localStorage.getItem("slyde.lang");
  if (stored === "en" || stored === "es") return stored;
  return (navigator.language || "en").toLowerCase().startsWith("es") ? "es" : "en";
}

export function lang() {
  if (!current) current = detect();
  return current;
}

export function setLang(next) {
  if (next !== "en" && next !== "es") return;
  current = next;
  localStorage.setItem("slyde.lang", next);
  document.documentElement.lang = next;
  apply();
  listeners.forEach((fn) => fn(next));
}

export function onLangChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function t(key, vars) {
  const table = DICT[lang()] || DICT.en;
  let value = table[key] ?? DICT.en[key] ?? key;
  if (vars) {
    for (const [name, replacement] of Object.entries(vars)) {
      value = value.replace(`{${name}}`, String(replacement));
    }
  }
  return value;
}

export function apply(root = document) {
  document.documentElement.lang = lang();
  root.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  root.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    el.placeholder = t(el.dataset.i18nPlaceholder);
  });
  root.querySelectorAll("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle);
  });
  root.querySelectorAll("[data-i18n-aria]").forEach((el) => {
    el.setAttribute("aria-label", t(el.dataset.i18nAria));
  });
}
