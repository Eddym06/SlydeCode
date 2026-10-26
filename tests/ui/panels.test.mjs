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

// ---------- layouts, transitions, versions, elements, canvas toolbar, keyboard ----------

const firstSlide = async () => (await studio.state()).view.presentation.slides[0];
const slides = async () => (await studio.state()).view.presentation.slides;
const frameMessage = (message) =>
  page.evaluate(
    `document.querySelector("#stage-frame iframe").contentWindow.postMessage(${JSON.stringify({ __slyde: true, ...message })}, "*"); true`
  );

async function resetToOneSlide(code) {
  const current = await slides();
  for (const slide of current.slice(1)) await studio.command("remove_slide", { slide_id: slide.id });
  await studio.command("set_slide_code", { slide_id: current[0].id, code });
  await page.waitFor(`document.querySelectorAll(".thumb").length === 1`);
  await page.evaluate(`document.getElementById("tab-inspector").click(); true`);
  await page.evaluate(`document.getElementById("subtab-slide").click(); true`);
}

test("a slide written without a template is labeled from its content, not as code", async () => {
  const count = (await slides()).length;
  await page.evaluate(`fetch("/api/agent?t=${studio.token}", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ action: "upsert_slide", index: ${count}, html_code: "<h1>Plan</h1><ul><li>One</li><li>Two</li></ul>" })
  }).then((response) => response.json())`);
  const all = await slides();
  assert.equal(all[all.length - 1].template, "bullet");
  await studio.command("remove_slide", { slide_id: all[all.length - 1].id });
});

test("a layout keeps the slide's words and can be applied to the whole deck", async () => {
  await resetToOneSlide(`<h1 class="title">Growth this year</h1><p class="sub">Revenue is up 72% on last year</p>`);
  await page.waitFor(`document.querySelector('[data-layout="big_number"]')`);
  await page.evaluate(`document.querySelector('[data-layout="big_number"]').click(); true`);
  await page.waitFor(`document.querySelector('[data-layout="big_number"]').getAttribute("aria-checked") === "true"`);
  let slide = await firstSlide();
  assert.ok(slide.code.includes("layout-number"), "the layout was applied");
  assert.ok(slide.code.includes("72%"), "the number was lifted out of the sentence");
  assert.ok(slide.code.includes("Growth this year"), "the original words survive");

  await studio.command("add_slide", { template: "hero" });
  await page.waitFor(`document.querySelectorAll(".thumb").length === 2`);
  await page.evaluate(`document.querySelector('[data-layout="section"]').click(); true`);
  await page.waitFor(`!document.getElementById("layout-all").disabled`);
  await page.evaluate(`document.getElementById("layout-all").click(); true`);
  await page.waitFor(`document.querySelectorAll(".thumb").length === 2`);
  for (let attempt = 0; attempt < 30; attempt += 1) {
    if ((await slides()).every((item) => item.code.includes("layout-section"))) break;
    await sleep(100);
  }
  assert.ok((await slides()).every((item) => item.code.includes("layout-section")), "every slide uses the layout");
  assert.equal((await firstSlide()).template, "custom");
});

test("transitions are stored per slide and can be applied to every slide", async () => {
  await page.evaluate(`document.querySelector('[data-transition="zoom"]').click(); true`);
  for (let attempt = 0; attempt < 30 && (await firstSlide()).transition?.kind !== "zoom"; attempt += 1) await sleep(100);
  assert.equal((await firstSlide()).transition.kind, "zoom");
  assert.equal((await slides())[1].transition ?? null, null, "only the chosen slide changed");

  await page.evaluate(`document.getElementById("transition-all").click(); true`);
  for (let attempt = 0; attempt < 30 && !(await slides()).every((item) => item.transition?.kind === "zoom"); attempt += 1) {
    await sleep(100);
  }
  assert.ok((await slides()).every((item) => item.transition?.kind === "zoom"));
});

test("named versions can be saved and restored, and a restore is undoable", async () => {
  await page.evaluate(`document.getElementById("subtab-deck").click(); true`);
  await page.waitFor(`document.querySelector(".version-input")`);
  await page.evaluate(`(() => {
    const input = document.querySelector(".version-input");
    input.value = "Two slides";
    input.form.requestSubmit();
    return true;
  })()`);
  await page.waitFor(`[...document.querySelectorAll(".version-name")].some((node) => node.textContent === "Two slides")`);

  await studio.command("add_slide", { template: "bullet" });
  await page.waitFor(`document.querySelectorAll(".thumb").length === 3`);
  await page.evaluate(`[...document.querySelectorAll(".version-row")]
    .find((row) => row.textContent.includes("Two slides"))
    .querySelector("[data-restore]").click(); true`);
  await page.waitFor(`document.querySelectorAll(".thumb").length === 2`);
  assert.ok(
    await page.evaluate(`[...document.querySelectorAll(".version-meta")].some((node) => /automatic|automática/i.test(node.textContent))`),
    "the deck before the restore was kept as an automatic version"
  );
  await studio.command("undo");
  await page.waitFor(`document.querySelectorAll(".thumb").length === 3`);
  await studio.command("remove_slide", { slide_id: (await slides())[2].id });
  await page.evaluate(`document.getElementById("subtab-slide").click(); true`);
});

test("the elements library inserts a block without code and closes with Escape", async () => {
  const before = (await firstSlide()).code;
  await page.evaluate(`document.getElementById("insert-elements").click(); true`);
  await page.waitFor(`!document.getElementById("elements-popover").hidden`);
  assert.equal(await page.evaluate(`document.getElementById("insert-elements").getAttribute("aria-expanded")`), "true");
  await page.evaluate(`document.querySelector('.elements-tab[data-category="icons"]').click(); true`);
  await page.waitFor(`document.querySelectorAll('.elements-tile[data-category="icons"]').length > 10`);
  await page.evaluate(`document.querySelector('.elements-tile[data-category="icons"]').click(); true`);
  await page.waitFor(`document.getElementById("elements-popover").hidden`);
  for (let attempt = 0; attempt < 30 && !(await firstSlide()).code.includes("slyde-block"); attempt += 1) await sleep(100);
  const after = (await firstSlide()).code;
  assert.ok(after.includes("slyde-block") && after.length > before.length);

  await page.evaluate(`document.getElementById("insert-elements").click(); true`);
  await page.waitFor(`!document.getElementById("elements-popover").hidden`);
  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.getElementById("elements-popover").hidden`);
});

test("the canvas toolbar formats text without code and speaks plain language", async () => {
  const code = (await firstSlide()).code;
  const id = /<h1[^>]*data-id="([^"]+)"/.exec(code)?.[1];
  assert.ok(id, "the title has a handle");
  await frameMessage({ type: "select-by-id", id });
  await page.waitFor(`!document.getElementById("element-hud").hidden && document.querySelector('[data-element-action="bold"]')`);
  const label = await page.evaluate(`document.querySelector(".element-hud-tag").textContent`);
  assert.equal(label, "Title", "no tag or handle in the toolbar");
  assert.ok(!/#slyde|DIV|H1/.test(label));

  await page.evaluate(`document.querySelector('[data-element-action="italic"]').click(); true`);
  for (let attempt = 0; attempt < 30 && !(await firstSlide()).code.includes("font-style"); attempt += 1) await sleep(100);
  assert.ok((await firstSlide()).code.includes("font-style: italic"));

  await page.evaluate(`document.querySelector('[data-element-action="color"]').click(); true`);
  await page.waitFor(`document.querySelectorAll(".hud-color[data-color]").length >= 8`);
  await page.evaluate(`document.querySelector('.hud-color[data-color="#ff6b4a"]').click(); true`);
  for (let attempt = 0; attempt < 30 && !(await firstSlide()).code.includes("#ff6b4a"); attempt += 1) await sleep(100);
  assert.ok((await firstSlide()).code.includes("#ff6b4a"));
});

test("the canvas works from the keyboard and announces what it selects", async () => {
  await page.evaluate(`document.getElementById("element-hud") && document.querySelector('[data-element-action="close"]')?.click(); true`);
  await page.evaluate(`document.getElementById("stage").focus(); true`);
  assert.equal(await page.evaluate(`document.activeElement.id`), "stage");
  assert.ok(
    await page.evaluate(`document.getElementById("stage").getAttribute("aria-label").length > 20`),
    "the canvas region explains its keys"
  );

  await page.press("Enter", { keyCode: 13 });
  await page.waitFor(`!document.getElementById("element-hud").hidden`);
  await page.waitFor(`document.getElementById("canvas-announcer").textContent.includes("selected")`);

  await page.press("Tab", { keyCode: 9 });
  await sleep(300);
  assert.equal(await page.evaluate(`document.activeElement.id`), "stage", "Tab walks blocks, it does not leave the canvas");

  const before = (await firstSlide()).code;
  await page.press("ArrowRight", { keyCode: 39 });
  for (let attempt = 0; attempt < 40 && (await firstSlide()).code === before; attempt += 1) await sleep(100);
  assert.notEqual((await firstSlide()).code, before, "an arrow key moved the selected block");

  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.getElementById("element-hud").hidden`);
});

test("the Design panel never clips its text, even in Spanish, and lists blocks by name", async () => {
  await page.evaluate(`document.querySelector('[data-lang="es"]').click(); true`);
  await page.waitFor(`document.documentElement.lang === "es"`);
  await page.evaluate(`document.getElementById("tab-inspector").click(); true`);
  await sleep(500);
  const audit = await page.evaluate(`(() => {
    const scroller = document.getElementById("inspector");
    const box = scroller.getBoundingClientRect();
    const clipped = [...scroller.querySelectorAll(".card-title, .card-head, .tile-btn, .layout-tile")]
      .filter((node) => node.getClientRects().length)
      .filter((node) => node.getBoundingClientRect().left < box.left - 1)
      .map((node) => node.textContent.trim().slice(0, 20));
    const frame = document.getElementById("stage-frame").getBoundingClientRect();
    const panel = document.getElementById("right-panel").getBoundingClientRect();
    return {
      overflowX: scroller.scrollWidth - scroller.clientWidth,
      clipped,
      frameUnderPanel: frame.right > panel.left + 1,
      tree: [...document.querySelectorAll(".element-tree-item")].map((node) => node.textContent.trim()),
    };
  })()`);
  assert.ok(audit.overflowX <= 0, `no horizontal scroll (${audit.overflowX})`);
  assert.deepEqual(audit.clipped, [], "no card starts left of the panel");
  assert.equal(audit.frameUnderPanel, false, "the slide never slides under the right panel");
  assert.ok(audit.tree.length > 0);
  assert.ok(audit.tree.every((text) => !/#slyde|\bDIV\b|\bLI\b/.test(text)), `plain names only: ${audit.tree}`);
  await page.evaluate(`document.querySelector('[data-lang="en"]').click(); true`);
});
