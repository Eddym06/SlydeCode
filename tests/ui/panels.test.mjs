// The right-hand column and the topbar: responsive shell, the Design | Copilot
// switch, menus, shortcut sheet, storyboard multi-select, the code editor and
// library covers. Runs the real binary like `studio.test.mjs`.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";
import { sleep } from "../../scripts/lib/cdp.mjs";

let studio;
let page;

before(async () => {
  studio = await startStudio();
  page = studio.page;
});

after(async () => {
  await studio?.stop();
});

test("the shell never overflows and the right panel becomes a drawer on narrow windows", async () => {
  for (const width of [1440, 1000, 820, 560]) {
    await studio.setViewport(width);
    // The stage resizes on a 120 ms debounce, so wait for the layout to settle
    // instead of sleeping: a loaded CI machine needs longer than a fast one.
    await page.waitFor(
      `document.documentElement.scrollWidth <= window.innerWidth &&
       document.getElementById("present").getBoundingClientRect().right <= window.innerWidth`,
      { timeout: 10000 }
    );
    const layout = await page.evaluate(`(() => {
      const present = document.getElementById("present").getBoundingClientRect();
      return {
        overflow: document.documentElement.scrollWidth - window.innerWidth,
        presentRight: present.right,
        width: window.innerWidth,
      };
    })()`);
    assert.ok(layout.overflow <= 0, `no horizontal overflow at ${width}px (${layout.overflow})`);
    assert.ok(layout.presentRight <= layout.width, `Present stays on screen at ${width}px`);
  }

  await studio.setViewport(820);
  await page.waitFor(
    `document.getElementById("right-panel").getBoundingClientRect().left >= window.innerWidth - 1`,
    { timeout: 10000 }
  );

  await page.evaluate(`document.getElementById("panel-toggle").click(); true`);
  await page.waitFor(
    `document.getElementById("app").dataset.panel === "open" &&
     document.getElementById("right-panel").getBoundingClientRect().left < window.innerWidth - 100`
  );
  assert.equal(
    await page.evaluate(`document.getElementById("panel-toggle").getAttribute("aria-expanded")`),
    "true"
  );

  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.getElementById("app").dataset.panel === "closed"`);
  await studio.setViewport(1280);
  await page.waitFor(`document.getElementById("app").dataset.panel === "closed"`);
});

test("the Design | Copilot switch moves its thumb and swaps the panels", async () => {
  const before = await page.evaluate(`({
    active: document.querySelector(".panel-switch").dataset.active,
    design: !document.getElementById("inspector-panel").hidden,
  })`);
  assert.deepEqual(before, { active: "inspector", design: true });

  await page.evaluate(`document.getElementById("tab-copilot").click(); true`);
  await page.waitFor(`document.querySelector(".panel-switch").dataset.active === "copilot"`);
  const copilot = await page.evaluate(`({
    copilotShown: !document.getElementById("copilot-panel").hidden,
    designHidden: document.getElementById("inspector-panel").hidden,
    selected: document.getElementById("tab-copilot").getAttribute("aria-selected"),
    label: document.getElementById("tab-inspector").textContent.trim(),
  })`);
  assert.equal(copilot.copilotShown, true);
  assert.equal(copilot.designHidden, true);
  assert.equal(copilot.selected, "true");
  assert.equal(copilot.label, "Design", "the panel is called Design, not Inspector");

  await page.evaluate(`document.getElementById("tab-inspector").click(); true`);
  await page.waitFor(`document.querySelector(".panel-switch").dataset.active === "inspector"`);
});

test("the File menu opens, walks with the arrow keys and returns focus on Escape", async () => {
  await page.evaluate(`document.getElementById("file-menu-trigger").focus(); true`);
  await page.press("ArrowDown", { keyCode: 40 });
  await page.waitFor(`!document.getElementById("file-menu").hidden`);
  assert.equal(
    await page.evaluate(`document.getElementById("file-menu-trigger").getAttribute("aria-expanded")`),
    "true"
  );
  await page.waitFor(`document.activeElement && document.activeElement.id === "topbar-library"`);
  await page.press("ArrowDown", { keyCode: 40 });
  await page.waitFor(`document.activeElement.id === "topbar-new-deck"`);

  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.getElementById("file-menu").hidden`);
  assert.equal(
    await page.evaluate(`document.activeElement.id`),
    "file-menu-trigger",
    "focus returns to the trigger"
  );
});

test("the shortcut sheet opens with ? and keeps Tab inside it", async () => {
  await page.evaluate(`document.body.focus(); true`);
  await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "?", text: "?" });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "?" });
  await page.waitFor(`!document.querySelector(".shortcuts").hidden`);

  for (let step = 0; step < 4; step += 1) await page.press("Tab", { keyCode: 9 });
  assert.ok(
    await page.evaluate(`document.querySelector(".shortcuts").contains(document.activeElement)`),
    "focus never leaves the dialog"
  );

  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.querySelector(".shortcuts").hidden`);
});

test("multi-select removes several slides as one undoable step", async () => {
  for (let index = 0; index < 3; index += 1) await studio.command("add_slide", { template: "bullet" });
  await page.waitFor(`document.querySelectorAll(".thumb").length >= 4`);
  const total = await page.evaluate(`document.querySelectorAll(".thumb").length`);

  await page.evaluate(`(() => {
    const thumbs = [...document.querySelectorAll(".thumb")];
    thumbs[0].click();
    for (const thumb of thumbs.slice(1, 3)) {
      thumb.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
    }
    return true;
  })()`);
  await page.waitFor(`document.querySelectorAll(".thumb.is-multi").length === 3`);
  assert.equal(await page.evaluate(`document.getElementById("bulk-bar").hidden`), false);

  await page.evaluate(`document.querySelector('[data-bulk="delete"]').click(); true`);
  await page.waitFor(`document.querySelectorAll(".thumb").length === ${total - 3}`);

  await studio.command("undo");
  await page.waitFor(`document.querySelectorAll(".thumb").length === ${total}`);
});

test("the grid view toggle persists and lays thumbnails out in two columns", async () => {
  await page.evaluate(`document.getElementById("storyboard-view").click(); true`);
  assert.equal(await page.evaluate(`document.getElementById("storyboard").dataset.view`), "grid");
  const columns = await page.evaluate(
    `getComputedStyle(document.getElementById("storyboard")).gridTemplateColumns.split(" ").length`
  );
  assert.equal(columns, 2);
  await page.evaluate(`document.getElementById("storyboard-view").click(); true`);
});

test("the source editor saves while typing and is not overwritten by the re-render", async () => {
  await page.evaluate(`(() => {
    document.querySelectorAll(".thumb")[0].click();
    return true;
  })()`);
  await page.waitFor(`document.querySelector("details.dev")`);
  await page.evaluate(`document.querySelector("details.dev").open = true; true`);
  await page.waitFor(`document.querySelector(".code-input")`);

  await page.evaluate(`(() => {
    const input = document.querySelector(".code-input");
    input.focus();
    input.value = input.value + "\\n<!-- typed-by-test -->";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  // Wait past the debounce, then look at the server and at the editor itself.
  await sleep(1200);
  const saved = await studio.state();
  const code = JSON.stringify(saved.view.presentation.slides[0].code);
  assert.ok(code.includes("typed-by-test"), "the edit reached the deck");
  assert.ok(
    await page.evaluate(`document.querySelector(".code-input").value.includes("typed-by-test")`),
    "the editor still shows what was typed"
  );
  assert.ok(
    await page.evaluate(`document.querySelector(".code-highlight .tok-comment")`),
    "comments are highlighted"
  );
});

test("the library shows a cover thumbnail for every deck", async () => {
  await studio.command("new_deck", { title: "Second deck" });
  await page.evaluate(`document.getElementById("topbar-library").click(); true`);
  await page.waitFor(`document.querySelectorAll(".deck-cover iframe").length >= 1`);
  const covers = await page.evaluate(`[...document.querySelectorAll(".deck-cover iframe")].map((f) => f.src)`);
  assert.ok(covers.every((src) => src.includes("/d/")), "covers come from the sandbox deck route");
  await page.evaluate(`document.querySelector('[data-library-action="close"]')?.click(); true`);
});
