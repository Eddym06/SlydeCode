// First-run wizard against the real binary: it opens once for an untouched
// deck, builds a complete deck from a template in one undoable step, and
// leaves the copilot ready for the AI path.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";

let studio;
let page;

before(async () => {
  studio = await startStudio({ welcomed: false });
  page = studio.page;
});

after(async () => {
  await studio?.stop();
});

const slideCount = () => page.evaluate(`document.querySelectorAll(".thumb").length`);

test("the wizard greets a new user, is a labelled dialog and traps focus", async () => {
  await page.waitFor(`document.querySelector(".welcome-card") !== null`, { timeout: 10000 });
  const info = await page.evaluate(`(() => {
    const card = document.querySelector(".welcome-card");
    return {
      role: card.getAttribute("role"),
      modal: card.getAttribute("aria-modal"),
      label: document.getElementById(card.getAttribute("aria-labelledby"))?.textContent,
      options: document.querySelectorAll(".welcome-option").length,
      focusInside: card.contains(document.activeElement),
    };
  })()`);
  assert.equal(info.role, "dialog");
  assert.equal(info.modal, "true");
  assert.ok(info.label && info.label.length > 5, "the dialog has a visible title");
  assert.equal(info.options, 3);
  assert.ok(info.focusInside, "focus moves into the wizard");
});

test("choosing a template builds a complete deck in one step that undo reverts", async () => {
  const before = await slideCount();
  await page.evaluate(`document.querySelector('[data-act="templates"]').click()`);
  await page.waitFor(`document.querySelectorAll(".welcome-template").length >= 5`);
  await page.evaluate(`document.querySelector('.welcome-template[data-id="class_lesson"]').click()`);
  await page.waitFor(`document.querySelector(".welcome") === null`, { timeout: 10000 });
  await page.waitFor(`document.querySelectorAll(".thumb").length === 6`, { timeout: 10000 });
  const titleInput = await page.evaluate(`document.getElementById("deck-title")?.value ?? ""`);
  assert.match(titleInput, /Lesson|Lección/, "the template title is applied");
  assert.ok(before <= 1, "the wizard only appears for an untouched deck");
  const reverted = await studio.command("undo");
  assert.ok(reverted, "undo is accepted");
  await page.waitFor(`document.querySelectorAll(".thumb").length === ${before}`, { timeout: 10000 });
});

test("the wizard does not return after it was dismissed, and the menu reopens it", async () => {
  const remembered = await page.evaluate(`localStorage.getItem("slyde.welcomed")`);
  assert.equal(remembered, "1");
  await page.evaluate(`document.getElementById("topbar-welcome").click()`);
  await page.waitFor(`document.querySelector(".welcome-card") !== null`);
  await page.evaluate(`document.querySelector('[data-act="idea"]').click()`);
  await page.waitFor(`document.getElementById("welcome-topic") !== null`);
  // Without a connected provider the topic step leads to the one-step connect form.
  await page.evaluate(`(() => {
    const area = document.getElementById("welcome-topic");
    area.value = "Pitch for a meal planning app";
    area.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector(".welcome form").requestSubmit();
  })()`);
  await page.waitFor(`document.getElementById("welcome-key") !== null`, { timeout: 8000 });
  const providers = await page.evaluate(`document.querySelectorAll(".welcome-provider").length`);
  assert.ok(providers >= 2, "featured providers are offered");
  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.querySelector(".welcome") === null`);
});
