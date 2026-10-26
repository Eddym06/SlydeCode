// Module smoke test.
//
// The UI has no bundler and no build step, so a syntax error in any module is
// only discovered when the browser refuses to load it — and because `app.js`
// imports the whole graph, one bad file leaves a blank studio with no console
// message in the product. This test parses (and, where it can, initialises) every
// UI module in Node and fails only on *parse* errors.
//
// Runtime errors are expected and ignored: `app.js` reads `window.location` at
// import time and several modules touch `document`. A missing DOM is not the bug
// this test is looking for; a `SyntaxError` (duplicate `let`, unbalanced brace,
// reserved word) is.

import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = dirname(fileURLToPath(import.meta.url));
const staticDir = join(here, "..", "..", "ui", "static");
const localeDir = join(staticDir, "locales");

function modulesIn(dir) {
  return readdirSync(dir)
    .filter((name) => name.endsWith(".js"))
    .map((name) => join(dir, name))
    .sort();
}

test("every UI module parses as an ES module", async () => {
  const files = [...modulesIn(staticDir), ...modulesIn(localeDir)];
  assert.ok(files.length >= 12, `expected the UI modules, found ${files.length}`);

  const parseFailures = [];
  for (const file of files) {
    try {
      await import(pathToFileURL(file).href);
    } catch (error) {
      if (error instanceof SyntaxError) {
        parseFailures.push(`${file.split(/[\\/]/).pop()}: ${error.message}`);
      }
      // Anything else is a missing browser global, not a broken module.
    }
  }

  assert.deepEqual(parseFailures, [], `modules with syntax errors:\n${parseFailures.join("\n")}`);
});

test("the locale modules export dictionaries of the same shape", async () => {
  const en = (await import(pathToFileURL(join(localeDir, "en.js")).href)).default;
  const es = (await import(pathToFileURL(join(localeDir, "es.js")).href)).default;
  for (const [code, table] of [
    ["en", en],
    ["es", es],
  ]) {
    assert.equal(typeof table, "object", `${code} exports a dictionary`);
    const keys = Object.keys(table);
    assert.ok(keys.length > 200, `${code} has ${keys.length} keys`);
  }
  assert.deepEqual(Object.keys(en).sort(), Object.keys(es).sort(), "locales have the same keys");
});

test("i18n exports the public API the app relies on", async () => {
  const i18n = await import(pathToFileURL(join(staticDir, "i18n.js")).href);
  for (const name of ["t", "apply", "lang", "setLang", "onLangChange"]) {
    assert.equal(typeof i18n[name], "function", `i18n.${name} is exported`);
  }
});

test("api.js exports the client factory without touching the DOM", async () => {
  const api = await import(pathToFileURL(join(staticDir, "api.js")).href);
  assert.equal(typeof api.createClient, "function");
  assert.equal(typeof api.readToken, "function");
  const client = api.createClient("token-under-test");
  for (const name of [
    "getState",
    "command",
    "decks",
    "deckNew",
    "deckOpen",
    "deckRename",
    "deckDuplicate",
    "deckDelete",
    "exportDeck",
    "streamChat",
    "connect",
  ]) {
    assert.equal(typeof client[name], "function", `client.${name} exists`);
  }
});

test("every stylesheet and script the HTML pages link exists", () => {
  // The UI has no bundler: a stylesheet that was split or renamed but still
  // linked (or the reverse) is a silently unstyled studio.
  const uiDir = join(here, "..", "..", "ui");
  for (const page of ["index.html", "present.html"]) {
    const html = readFileSync(join(uiDir, page), "utf8");
    const linked = [...html.matchAll(/(?:href|src)="\/static\/([^"?#]+)"/g)].map((m) => m[1]);
    assert.ok(linked.length > 0, `${page} links nothing`);
    for (const file of linked) {
      assert.ok(existsSync(join(staticDir, file)), `${page} links a missing file: /static/${file}`);
    }
  }
});

test("the studio styles are split by responsibility and no sheet creeps back past 800 lines", () => {
  assert.ok(!existsSync(join(staticDir, "app.css")), "app.css was split; do not recreate the monolith");
  for (const name of readdirSync(staticDir).filter((n) => n.endsWith(".css"))) {
    const lines = readFileSync(join(staticDir, name), "utf8").split("\n").length;
    assert.ok(lines <= 800, `${name} has ${lines} lines; split it by responsibility`);
  }
});
