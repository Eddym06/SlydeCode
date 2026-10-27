// Palettes: a theme change reaches slides that hard-coded the old colors, the
// copilot can design its own palette, and the retired `terminal` id still works.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";
import { sleep } from "../../scripts/lib/cdp.mjs";

let studio;
let page;
let origin;

before(async () => {
  studio = await startStudio();
  page = studio.page;
  origin = new URL(studio.appUrl).origin;
});

after(async () => {
  await studio?.stop();
});

async function agent(payload) {
  const response = await fetch(`${origin}/api/agent?t=${encodeURIComponent(studio.token)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  assert.equal(response.status, 200, `agent call failed: ${await response.clone().text()}`);
  return response.json();
}

const current = async () => (await studio.state()).view.presentation;

test("a theme change reaches slides that hard-coded the previous palette", async () => {
  const slideId = (await current()).slides[0].id;
  // Studio's own background and accent, written as literals the way a model does.
  await agent({
    action: "upsert_slide",
    index: 0,
    slide_id: slideId,
    html_code:
      '<div style="position:absolute;inset:0;background:#17161A;color:#F7F4EF">' +
      '<h1 style="color:#FF6B4A">Hello</h1></div>',
  });
  assert.equal((await current()).theme.id, "studio");

  await studio.command("set_theme", { theme_id: "paper" });
  await page.waitFor(
    `fetch("/api/state?t=${studio.token}").then((r) => r.json()).then((s) => s.view.presentation.theme.id === "paper")`,
    { timeout: 8000 }
  );
  const code = (await current()).slides[0].code;
  assert.match(code, /background:var\(--slyde-bg\)/, `the background follows the theme: ${code}`);
  assert.match(code, /color:var\(--slyde-fg\)/, `the text follows the theme: ${code}`);
  assert.match(code, /color:var\(--slyde-accent\)/, `the accent follows the theme: ${code}`);
  assert.doesNotMatch(code, /#17161A/i, "no stale studio color is left behind");
});

test("the copilot's own palette is named and shown in the Deck tab", async () => {
  await agent({
    action: "set_theme",
    background: "#2B1B12",
    foreground: "#F6E9DC",
    muted: "rgba(246,233,220,.6)",
    accent: "#E0A458",
    name: "Espresso",
  });
  const theme = (await current()).theme;
  assert.equal(theme.id, "custom");
  assert.equal(theme.name, "Espresso");
  assert.equal(theme.background, "#2B1B12");

  await page.evaluate(`document.getElementById("tab-inspector")?.click(); document.getElementById("subtab-deck").click(); true`);
  await page.waitFor(
    `[...document.querySelectorAll(".theme-swatch[aria-checked='true'] .swatch-name")].some((n) => n.textContent === "Espresso")`,
    { timeout: 8000 }
  );
  const presets = await page.evaluate(
    `[...document.querySelectorAll(".theme-swatch:not([disabled])")].map((b) => b.dataset.theme)`
  );
  assert.deepEqual(presets, ["studio", "paper", "night", "original", "academic", "pitch"]);
});

test("`original` replaces `terminal`, which is still accepted as an alias", async () => {
  await studio.command("set_theme", { theme_id: "original" });
  await page.waitFor(
    `fetch("/api/state?t=${studio.token}").then((r) => r.json()).then((s) => s.view.presentation.theme.id === "original")`,
    { timeout: 8000 }
  );
  assert.equal((await current()).theme.name, "Original");

  await agent({ action: "set_theme", theme_id: "studio" });
  await agent({ action: "set_theme", theme_id: "terminal" });
  await sleep(100);
  assert.equal((await current()).theme.id, "original", "an old client naming `terminal` gets `original`");
});
