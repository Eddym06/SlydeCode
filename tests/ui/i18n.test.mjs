// Dictionary parity tests for the i18n layer.
//
// The locale modules are imported directly, so this runs without a browser,
// a server or any dependency: node --test "tests/ui/i18n.test.mjs".
//
// It proves that both dictionaries hold the same keys with the same
// placeholders, that no value is empty, that every key referenced from the
// HTML resolves in both locales, and that the dictionaries have not crept back
// into ui/static/i18n.js.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import en from "../../ui/static/locales/en.js";
import es from "../../ui/static/locales/es.js";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const htmlFiles = ["ui/index.html", "ui/present.html"];
const i18nPath = join(repoRoot, "ui", "static", "i18n.js");

// Matches data-i18n, data-i18n-title, data-i18n-aria and data-i18n-placeholder
// with either quote style. The longer names are tried first, otherwise
// data-i18n-title would be read as data-i18n.
const attributePattern =
  /\bdata-i18n(?:-(?:title|aria|placeholder))?\s*=\s*(?:"([^"]*)"|'([^']*)')/g;
const placeholderPattern = /\{([A-Za-z0-9_]+)\}/g;

const locales = { en, es };

function referencedKeys() {
  const keys = new Set();
  for (const file of htmlFiles) {
    const html = readFileSync(join(repoRoot, file), "utf8");
    for (const match of html.matchAll(attributePattern)) {
      keys.add(match[1] ?? match[2]);
    }
  }
  return [...keys].sort();
}

function placeholders(value) {
  return [...new Set([...value.matchAll(placeholderPattern)].map((match) => match[1]))].sort();
}

test("both locales have exactly the same keys", () => {
  const onlyInEn = Object.keys(en).filter((key) => !Object.hasOwn(es, key));
  const onlyInEs = Object.keys(es).filter((key) => !Object.hasOwn(en, key));

  assert.deepEqual(
    { onlyInEn, onlyInEs },
    { onlyInEn: [], onlyInEs: [] },
    `locale drift: ${onlyInEn.length} key(s) missing from es, ` +
      `${onlyInEs.length} key(s) missing from en`
  );
});

test("no locale value is empty or whitespace-only", () => {
  const offenders = [];
  for (const [code, dict] of Object.entries(locales)) {
    for (const [key, value] of Object.entries(dict)) {
      if (typeof value !== "string") {
        offenders.push(`${code}.${key} is ${typeof value}, not a string`);
      } else if (value.trim() === "") {
        offenders.push(`${code}.${key} is blank`);
      }
    }
  }

  assert.deepEqual(offenders, [], `${offenders.length} unusable locale value(s)`);
});

test("every key referenced from the HTML resolves in both locales", () => {
  const keys = referencedKeys();
  // Guard against a silent scan failure: the two pages reference ~90 keys.
  assert.ok(keys.length > 50, `only ${keys.length} data-i18n key(s) found in the HTML`);

  const missing = [];
  for (const key of keys) {
    for (const [code, dict] of Object.entries(locales)) {
      if (!Object.hasOwn(dict, key)) missing.push(`${code} is missing "${key}"`);
    }
  }

  assert.deepEqual(missing, [], `${missing.length} HTML reference(s) do not resolve`);
});

test("placeholders match between locales for every key", () => {
  const drift = [];
  for (const key of Object.keys(en)) {
    const expected = placeholders(en[key] ?? "");
    const actual = placeholders(es[key] ?? "");
    if (expected.join(",") !== actual.join(",")) {
      drift.push({ key, en: expected, es: actual });
    }
  }

  assert.deepEqual(drift, [], `${drift.length} key(s) use different placeholders`);
});

test("both dictionaries are non-trivial", () => {
  const enCount = Object.keys(en).length;
  const esCount = Object.keys(es).length;

  // A truncated locale file would otherwise pass every check above.
  assert.ok(enCount > 200, `en has only ${enCount} keys`);
  assert.ok(esCount > 200, `es has only ${esCount} keys`);
  assert.equal(enCount, esCount, "the two locales must stay the same size");
});

test("i18n.js delegates to the locale modules instead of holding a monolith", () => {
  const source = readFileSync(i18nPath, "utf8");
  const literalEntries = source.match(/^\s*"[^"\n]+"\s*:\s*"/gm) ?? [];

  assert.ok(
    literalEntries.length <= 5,
    `ui/static/i18n.js still holds ${literalEntries.length} literal dictionary entries`
  );
  for (const specifier of ["./locales/en.js", "./locales/es.js"]) {
    assert.ok(
      source.includes(specifier),
      `ui/static/i18n.js no longer imports ${specifier}`
    );
  }
});
