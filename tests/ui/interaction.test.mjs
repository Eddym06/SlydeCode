// Interaction and stability suite.
//
// Everything here uses *real* input: CDP mouse and key events into headless
// Chrome, the same path a person takes. It covers what the studio suite only
// checked structurally: the transform gizmo (drag, resize, snap, nudge), canvas
// pan and zoom, the slide shortcuts, the presenter laser and pen, and then
// hammers all of it to prove nothing leaks, throws or corrupts the deck.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { sleep, startStudio } from "./harness.mjs";

let studio;
let page;

// A 1x1 PNG, enough for the image-paste path.
const PNG_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

before(async () => {
  studio = await startStudio();
  page = studio.page;
});

after(async () => {
  await studio?.stop();
});

// ---------- helpers ----------

/** Polls until `read()` returns a truthy value. */
async function until(read, { timeout = 6000, interval = 80, label = "condition" } = {}) {
  const started = Date.now();
  let last;
  while (Date.now() - started < timeout) {
    last = await read();
    if (last) return last;
    await sleep(interval);
  }
  throw new Error(`timed out waiting for ${label} (last: ${JSON.stringify(last)})`);
}

/** Box of a stage element in stage pixels, read inside the slide document. */
function stageBox(slideId, selector) {
  return studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, w: r.width, h: r.height };
    })()`)
  );
}

/** Inline style of the stage element, as the live DOM has it. */
function liveStyle(slideId, selector) {
  return studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`(() => {
      const s = document.querySelector(${JSON.stringify(selector)}).style;
      return {
        position: s.position, left: s.left, top: s.top,
        width: s.width, height: s.height, maxWidth: s.maxWidth
      };
    })()`)
  );
}

/** A declaration the deck stores for the first tag carrying `class="<cls>"`. */
function storedPx(code, cls, property) {
  const tag = code.match(new RegExp(`<[a-z0-9]+[^>]*class="${cls}"[^>]*>`, "i"));
  assert.ok(tag, `no <… class="${cls}"> in the slide code`);
  const style = tag[0].match(/style="([^"]*)"/);
  if (!style) return null;
  const decl = style[1].match(new RegExp(`(?:^|;|\\s)${property}:\\s*(-?[\\d.]+)px`));
  return decl ? Number(decl[1]) : null;
}

const px = (value) => Number(String(value).replace("px", ""));

/** The deck code for a slide, after the debounce has let the commit land. */
async function committedCode(slideId, settle = 900) {
  await sleep(settle);
  const slide = (await studio.slides()).find((s) => s.id === slideId);
  return slide.code;
}

/** Screen position of the centre of an element box in the stage iframe. */
function screenCentre(frame, box) {
  return {
    x: frame.x + box.x + box.w / 2,
    y: frame.y + box.y + box.h / 2,
  };
}

async function slideCount() {
  return (await studio.slides()).length;
}

async function reloadStudio() {
  studio.resetStageClients?.();
  await page.send("Page.reload");
  await page.waitFor("document.querySelectorAll('.thumb').length >= 1", { timeout: 15000 });
  await sleep(300);
}

/** Back to a single clean slide so every test starts from the same place. */
async function resetDeck() {
  const slides = await studio.slides();
  for (const slide of slides.slice(1)) {
    await studio.command({ action: "remove_slide", slide_id: slide.id });
  }
  await until(async () => (await slideCount()) === 1, { label: "single slide" });
  await reloadStudio();
  return studio.openSlide(0);
}

// =====================================================================
// Transform gizmo: real drag, resize, snap and nudge
// =====================================================================

test("gizmo: a click selects without unflowing, a drag moves and commits", async () => {
  const slideId = await resetDeck();
  const frame = await studio.frameMetrics();
  const before = await stageBox(slideId, ".sub");

  // Select with a plain click; the block must stay in the document flow.
  const centre = screenCentre(frame, before);
  await studio.click(centre.x, centre.y);
  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub")?.click(); true`)
  );
  await until(
    async () =>
      studio.inStage(slideId, (cdp) =>
        cdp.evaluate(`document.querySelectorAll("[data-slyde-handle]").length === 8`)
      ),
    { label: "handles visible" }
  );
  const selected = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`(() => {
      const h = document.querySelector("[data-slyde-handle]");
      return {
        handles: document.querySelectorAll("[data-slyde-handle]").length,
        layer: h ? h.parentElement.style.display : "none",
        position: document.querySelector(".sub").style.position
      };
    })()`)
  );
  assert.equal(selected.handles, 8, "eight handles");
  assert.equal(selected.layer, "block", "handles are visible");
  assert.equal(selected.position, "", "selecting does not unflow the block");

  // Pressing again without moving is still just a click.
  await studio.click(centre.x, centre.y);
  await sleep(300);
  assert.equal((await liveStyle(slideId, ".sub")).position, "", "a bare click never unflows");

  // Drag by (+60, +30) screen pixels with Alt held: snapping is off, so the
  // result is exactly the pointer travel divided by the render scale.
  const dx = 60;
  const dy = 30;
  await studio.drag(centre, { x: centre.x + dx, y: centre.y + dy }, { modifiers: 1 });
  await sleep(250);

  const live = await liveStyle(slideId, ".sub");
  assert.equal(live.position, "absolute", "a real drag pins the block");
  assert.equal(live.maxWidth, "none", "authored caps no longer clamp it");
  const expectedLeft = Math.round((before.x + dx) / frame.scale);
  const expectedTop = Math.round((before.y + dy) / frame.scale);
  assert.ok(Math.abs(px(live.left) - expectedLeft) <= 4, `left ${live.left} ≈ ${expectedLeft}`);
  assert.ok(Math.abs(px(live.top) - expectedTop) <= 4, `top ${live.top} ≈ ${expectedTop}`);

  // The same numbers reach the deck.
  const code = await committedCode(slideId);
  assert.ok(Math.abs(storedPx(code, "sub", "left") - expectedLeft) <= 4, "left was committed");
  assert.ok(Math.abs(storedPx(code, "sub", "top") - expectedTop) <= 4, "top was committed");
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("gizmo: the corner handle resizes, and the box never collapses", async () => {
  const slideId = await studio.openSlide(0);
  const frame = await studio.frameMetrics();

  // Select .sub so the handles appear
  const subBox = await stageBox(slideId, ".sub");
  const centre = screenCentre(frame, subBox);
  await studio.click(centre.x, centre.y);
  await sleep(350);

  const before = await stageBox(slideId, ".sub");
  const beforeW = Math.round(before.w / frame.scale);
  const beforeH = Math.round(before.h / frame.scale);

  const handle = await stageBox(slideId, '[data-slyde-handle="se"]');
  const from = screenCentre(frame, handle);
  await studio.drag(from, { x: from.x + 50, y: from.y + 20 }, { modifiers: 1 });
  await sleep(250);

  const grown = await liveStyle(slideId, ".sub");
  assert.ok(
    Math.abs(px(grown.width) - (beforeW + Math.round(50 / frame.scale))) <= 6,
    `width ${beforeW} → ${grown.width}`
  );
  assert.ok(
    Math.abs(px(grown.height) - (beforeH + Math.round(20 / frame.scale))) <= 6,
    `height ${beforeH} → ${grown.height}`
  );
  assert.ok(px(grown.left) > 0, "the opposite corner stays put");
  assert.ok(px(grown.top) > 0, "the opposite corner stays put");

  // Drag the handle far past the opposite corner: the box clamps at 16px.
  const handleAgain = await stageBox(slideId, '[data-slyde-handle="se"]');
  const start = screenCentre(frame, handleAgain);
  await studio.drag(start, { x: frame.x + 2, y: frame.y + 2 }, { modifiers: 1, steps: 10 });
  await sleep(250);
  const clamped = await liveStyle(slideId, ".sub");
  assert.ok(px(clamped.width) >= 16 && px(clamped.width) <= 22, `min width, got ${clamped.width}`);
  assert.ok(px(clamped.height) >= 16 && px(clamped.height) <= 22, `min height, got ${clamped.height}`);

  const code = await committedCode(slideId);
  assert.ok(storedPx(code, "sub", "width") >= 16, "width stored");
  assert.ok(storedPx(code, "sub", "height") >= 16, "height stored");
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("gizmo: edges snap to the safe margin and a guide is drawn", async () => {
  const slideId = await studio.openSlide(0);
  const frame = await studio.frameMetrics();
  const box = await stageBox(slideId, ".sub");
  const centre = screenCentre(frame, box);

  // Click to select
  await studio.click(centre.x, centre.y);
  await sleep(350);

  // Aim the left edge towards the 64px safe margin
  const targetX = 66 * frame.scale;
  const dx = targetX - box.x;
  await studio.mouse("mouseMoved", centre.x, centre.y);
  await studio.mouse("mousePressed", centre.x, centre.y, { buttons: 1 });
  for (let step = 1; step <= 6; step += 1) {
    await studio.mouse("mouseMoved", centre.x + (dx * step) / 6, centre.y, { buttons: 1 });
  }
  const guide = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`(() => {
      const layers = [...document.querySelectorAll('[data-slyde-ui]')];
      const guides = layers.find((n) => n.style.pointerEvents === "none" && n.style.zIndex === "2147483645");
      return { shown: !!guides && guides.style.display === "block", lines: guides ? guides.children.length : 0 };
    })()`)
  );
  await studio.mouse("mouseReleased", centre.x + dx, centre.y);
  await sleep(250);

  assert.equal(guide.shown, true, "a snap guide is visible mid-drag");
  assert.ok(guide.lines >= 1, "at least one guide line");
  assert.equal(px((await liveStyle(slideId, ".sub")).left), 64, "snapped onto the safe margin");

  // Without Alt the guide goes away on release.
  const hidden = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`(() => {
      const g = [...document.querySelectorAll('[data-slyde-ui]')].find((n) => n.style.zIndex === "2147483645");
      return !g || g.style.display === "none";
    })()`)
  );
  assert.equal(hidden, true, "guides clear after the drag");
});

test("gizmo: arrow keys nudge by 1px, 10px with Shift, and commit", async () => {
  const slideId = await studio.openSlide(0);
  const frame = await studio.frameMetrics();
  const box = await stageBox(slideId, ".sub");
  const centre = screenCentre(frame, box);

  // Select .sub
  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub").click()`)
  );
  await sleep(350);
  const initialLeft = px((await liveStyle(slideId, ".sub")).left) || 0;

  // Dispatch arrow key into the stage document
  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }))`)
  );
  await until(
    async () => {
      const s = await liveStyle(slideId, ".sub");
      return s.position === "absolute" && px(s.left) > 0 && px(s.left) !== initialLeft;
    },
    { label: "1px nudge" }
  );
  const startLeft = px((await liveStyle(slideId, ".sub")).left);

  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub").click()`)
  );
  await sleep(200);

  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(
      `document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", shiftKey: true, bubbles: true }))`
    )
  );
  await until(async () => px((await liveStyle(slideId, ".sub")).left) === startLeft + 10, {
    label: "10px nudge",
  });

  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub").click()`)
  );
  await sleep(200);

  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }))`)
  );
  await until(
    async () => {
      const s = await liveStyle(slideId, ".sub");
      return px(s.left) === startLeft + 9;
    },
    { label: "nudge back" }
  );

  const code = await committedCode(slideId);
  assert.equal(storedPx(code, "sub", "left"), startLeft + 9, "nudges reached the deck");

  // The slide did not change underneath the nudge.
  assert.equal(await slideCount(), 1);
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("gizmo: a moved block undoes and redoes through the history", async () => {
  const slideId = await studio.openSlide(0);
  const code = await committedCode(slideId, 300);
  const left = storedPx(code, "sub", "left");

  const undo = await studio.command({ action: "undo" });
  assert.equal(undo.ok, true, undo.body);
  const afterUndo = await committedCode(slideId, 300);
  assert.notEqual(storedPx(afterUndo, "sub", "left"), left, "undo reverts the last geometry");

  const redo = await studio.command({ action: "redo" });
  assert.equal(redo.ok, true, redo.body);
  const afterRedo = await committedCode(slideId, 300);
  assert.equal(storedPx(afterRedo, "sub", "left"), left, "redo restores it");
});

// =====================================================================
// Canvas pan and zoom
// =====================================================================

test("canvas: Space+drag pans, Ctrl+wheel zooms, the pill resets both", async () => {
  await resetDeck();
  const stage = await page.evaluate(`(() => {
    const r = document.getElementById("stage").getBoundingClientRect();
    const f = document.getElementById("slide-frame").getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height, fx: f.x, fy: f.y, fw: f.width, fh: f.height };
  })()`);

  const transform = () =>
    page.evaluate(`document.getElementById("stage-frame").style.transform`);
  const zoomLabel = () => page.evaluate(`document.getElementById("zoom-value").textContent`);
  const stageClasses = () => page.evaluate(`document.getElementById("stage").className`);
  const parseTranslate = (value) => {
    const m = String(value).match(/translate\((-?\d+)px,\s*(-?\d+)px\)/);
    return m ? { x: Number(m[1]), y: Number(m[2]) } : null;
  };

  // A point on the stage that is not covered by the slide iframe, so the
  // pointer events reach the shell.
  const margin = { x: stage.x + 12, y: stage.y + 12 };
  assert.ok(
    margin.x < stage.fx || margin.y < stage.fy,
    "the margin point is outside the slide frame"
  );

  await page.evaluate(`document.body.tabIndex = -1; document.body.focus(); true`);
  assert.deepEqual(parseTranslate(await transform()), { x: 0, y: 0 }, "starts centred");

  // Without Space a drag on the stage must not pan.
  await studio.drag(margin, { x: margin.x + 40, y: margin.y + 25 });
  assert.deepEqual(parseTranslate(await transform()), { x: 0, y: 0 }, "no pan without Space");

  // Hold Space: grab cursor class appears; dragging pans by the pointer travel.
  await page.send("Input.dispatchKeyEvent", {
    type: "keyDown", key: " ", code: "Space", windowsVirtualKeyCode: 32,
  });
  assert.match(await stageClasses(), /is-space-pan/, "Space arms the pan mode");
  await studio.mouse("mouseMoved", margin.x, margin.y);
  await studio.mouse("mousePressed", margin.x, margin.y, { buttons: 1 });
  assert.match(await stageClasses(), /is-panning/, "pressing starts the pan");
  for (let step = 1; step <= 5; step += 1) {
    await studio.mouse("mouseMoved", margin.x + 8 * step, margin.y + 5 * step, { buttons: 1 });
  }
  await studio.mouse("mouseReleased", margin.x + 40, margin.y + 25);
  const panned = parseTranslate(await transform());
  assert.deepEqual(panned, { x: 40, y: 25 }, "the frame followed the pointer");
  assert.doesNotMatch(await stageClasses(), /is-panning/, "release ends the pan");

  await page.send("Input.dispatchKeyEvent", {
    type: "keyUp", key: " ", code: "Space", windowsVirtualKeyCode: 32,
  });
  assert.doesNotMatch(await stageClasses(), /is-space-pan/, "releasing Space disarms it");

  // The offset is kept once Space is up, and plain drags still do nothing.
  await studio.drag(margin, { x: margin.x + 90, y: margin.y + 90 });
  assert.deepEqual(parseTranslate(await transform()), { x: 40, y: 25 }, "pan offset is stable");

  // A wheel without Ctrl leaves the zoom alone; with Ctrl it steps.
  const fit = await zoomLabel();
  await page.send("Input.dispatchMouseEvent", {
    type: "mouseWheel", x: margin.x, y: margin.y, deltaX: 0, deltaY: -120,
  });
  await sleep(150);
  assert.equal(await zoomLabel(), fit, "a plain wheel does not zoom");

  await page.send("Input.dispatchMouseEvent", {
    type: "mouseWheel", x: margin.x, y: margin.y, deltaX: 0, deltaY: -120, modifiers: 2,
  });
  await until(async () => (await zoomLabel()) !== fit, { label: "zoom in" });
  const zoomedIn = await zoomLabel();
  assert.match(zoomedIn, /%$/, "a percentage replaces Fit");

  await page.send("Input.dispatchMouseEvent", {
    type: "mouseWheel", x: margin.x, y: margin.y, deltaX: 0, deltaY: 120, modifiers: 2,
  });
  await until(async () => (await zoomLabel()) === fit, { label: "zoom back to fit" });

  // The pill is a full reset: zoom and pan.
  await page.send("Input.dispatchMouseEvent", {
    type: "mouseWheel", x: margin.x, y: margin.y, deltaX: 0, deltaY: -120, modifiers: 2,
  });
  await until(async () => (await zoomLabel()) !== fit, { label: "zoom in again" });
  await page.evaluate(`document.getElementById("zoom-value").click(); true`);
  await until(async () => (await zoomLabel()) === fit, { label: "reset" });
  assert.deepEqual(parseTranslate(await transform()), { x: 0, y: 0 }, "reset recentres");
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

// =====================================================================
// Slide shortcuts
// =====================================================================

test("shortcuts: duplicate, copy, paste and delete behave, and typing is safe", async () => {
  const firstId = await resetDeck();
  const focusThumb = async () => {
    await page.waitFor(`document.querySelector('.thumb[aria-current="true"]')`);
    await page.evaluate(`document.querySelector('.thumb[aria-current="true"]').focus(); true`);
  };

  // Ctrl+D duplicates exactly once.
  await focusThumb();
  await studio.key("d", { modifiers: 2 });
  await until(async () => (await slideCount()) === 2, { label: "duplicate" });
  await sleep(300);
  assert.equal(await slideCount(), 2, "exactly one copy, not two");

  // Ctrl+C then a paste event inserts a faithful copy right after the selection.
  await page.evaluate(`document.querySelectorAll('.thumb')[0].click(); true`);
  await until(
    async () =>
      page.evaluate(
        `document.querySelectorAll('.thumb')[0].getAttribute("aria-current") === "true"`
      ),
    { label: "first slide selected" }
  );
  await focusThumb();
  await studio.key("c", { modifiers: 2 });
  const before = await studio.slides();
  await studio.key("v", { modifiers: 2 });
  await until(async () => (await slideCount()) === 3, { label: "paste" });
  const afterPaste = await studio.slides();
  assert.equal(afterPaste[1].code, before[0].code, "the pasted slide matches the source");
  assert.equal(afterPaste[0].id, before[0].id, "the source did not move");

  // A clipboard that carries an image belongs to the canvas, not to slide paste.
  const countBefore = await slideCount();
  await page.evaluate(`(async () => {
    const bytes = Uint8Array.from(atob(${JSON.stringify(PNG_BASE64)}), (c) => c.charCodeAt(0));
    const data = new DataTransfer();
    data.items.add(new File([bytes], "dot.png", { type: "image/png" }));
    document.body.dispatchEvent(new ClipboardEvent("paste", {
      bubbles: true, cancelable: true, clipboardData: data
    }));
    return true;
  })()`);
  await until(
    async () => (await studio.slides()).some((s) => /<img[^>]+\/assets\//.test(s.code)),
    { label: "image inserted by paste" }
  );
  assert.equal(await slideCount(), countBefore, "pasting an image does not paste a slide");

  // Delete removes exactly the selected slide...
  await focusThumb();
  const preDelete = await slideCount();
  await studio.key("Delete");
  await until(async () => (await slideCount()) === preDelete - 1, { label: "delete" });
  await sleep(300);
  assert.equal(await slideCount(), preDelete - 1, "one slide, not two");

  // ...but never while typing in a field.
  const stable = await slideCount();
  await page.evaluate(`document.getElementById("tab-copilot")?.click(); document.getElementById("copilot-input").focus(); true`);
  await studio.key("Backspace");
  await studio.key("Delete");
  await studio.key("d", { modifiers: 2 });
  await sleep(400);
  assert.equal(await slideCount(), stable, "typing keys never touch the deck");

  // And never the last remaining slide.
  await resetDeck();
  await focusThumb();
  await studio.key("Delete");
  await sleep(400);
  assert.equal(await slideCount(), 1, "the last slide cannot be deleted");

  // A selected canvas block owns Delete: the slide stays.
  const currentSlides = await studio.slides();
  await studio.command({ action: "duplicate_slide", slide_id: currentSlides[0].id });
  await until(async () => (await slideCount()) === 2, { label: "second slide" });
  const slideId = await studio.openSlide(0);
  const frame = await studio.frameMetrics();
  const box = await stageBox(slideId, ".sub");
  const centre = screenCentre(frame, box);
  await studio.click(centre.x, centre.y);
  await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub")?.click(); true`)
  );
  await until(
    async () => page.evaluate(`!document.getElementById("element-hud").hidden`),
    { label: "element hud visible", timeout: 2000 }
  ).catch(() => {});
  await sleep(200);
  await studio.key("Delete");
  await sleep(400);
  assert.equal(await slideCount(), 2, "Delete with a block selected keeps the slide");
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

// =====================================================================
// Presenter laser and pen
// =====================================================================

const presenter = {
  open: async () => {
    await page.evaluate(`document.body.tabIndex = -1; document.body.focus(); true`);
    await studio.key("F5");
    await until(
      () => page.evaluate(`!document.getElementById("present-overlay").hidden`),
      { label: "presenter overlay" }
    );
    await until(
      () =>
        page.evaluate(`(() => {
          const r = document.getElementById("present-frame").getBoundingClientRect();
          return r.width > 200 && !!document.querySelector(".present-draw-canvas");
        })()`),
      { label: "presenter frame" }
    );
    await sleep(500);
  },
  close: async () => {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      if (await page.evaluate(`document.getElementById("present-overlay").hidden`)) return;
      await studio.key("Escape");
      await sleep(250);
    }
    assert.equal(await page.evaluate(`document.getElementById("present-overlay").hidden`), true);
  },
  rect: () =>
    page.evaluate(`(() => {
      const r = document.getElementById("present-frame").getBoundingClientRect();
      return { x: r.x, y: r.y, w: r.width, h: r.height };
    })()`),
  /** Count of painted (non-transparent) pixels on the annotation canvas. */
  ink: () =>
    page.evaluate(`(() => {
      const c = document.querySelector(".present-draw-canvas");
      const data = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      let n = 0;
      for (let i = 3; i < data.length; i += 4) if (data[i] > 0) n += 1;
      return n;
    })()`),
  /** Alpha at a screen point, in canvas pixels. */
  alphaAt: (x, y) =>
    page.evaluate(`(() => {
      const c = document.querySelector(".present-draw-canvas");
      const r = c.getBoundingClientRect();
      const k = c.width / r.width;
      const px = Math.round((${x} - r.left) * k);
      const py = Math.round((${y} - r.top) * k);
      return c.getContext("2d").getImageData(px, py, 1, 1).data[3];
    })()`),
  currentIndex: () => page.evaluate(`Number(document.getElementById("hud-current").textContent)`),
};

test("presenter: the laser follows the cursor and leaves no trail", async () => {
  await resetDeck();
  await studio.command({ action: "duplicate_slide", slide_id: (await studio.slides())[0].id });
  await until(async () => (await slideCount()) === 2, { label: "two slides" });
  await reloadStudio();
  await studio.openSlide(0);

  await presenter.open();
  assert.equal(await presenter.ink(), 0, "a fresh canvas is empty");

  await studio.key("l");
  const state = await page.evaluate(`(() => {
    const wrap = document.getElementById("present-frame");
    const c = document.querySelector(".present-draw-canvas");
    return { cursor: wrap.style.cursor, events: c.style.pointerEvents };
  })()`);
  assert.equal(state.cursor, "none", "the system cursor hides under the laser");
  assert.equal(state.events, "auto", "the overlay receives the pointer over the slide iframe");

  const frame = await presenter.rect();
  const a = { x: frame.x + frame.w * 0.3, y: frame.y + frame.h * 0.4 };
  const b = { x: frame.x + frame.w * 0.7, y: frame.y + frame.h * 0.6 };

  await studio.mouse("mouseMoved", a.x - 10, a.y - 10);
  await studio.mouse("mouseMoved", a.x, a.y);
  await until(async () => (await presenter.alphaAt(a.x, a.y)) > 0, { label: "dot at A" });

  await studio.mouse("mouseMoved", b.x, b.y);
  await until(async () => (await presenter.alphaAt(b.x, b.y)) > 0, { label: "dot at B" });
  await until(async () => (await presenter.alphaAt(a.x, a.y)) === 0, { label: "no trail at A" });

  // Toggling the laser off wipes it.
  await studio.key("l");
  await until(async () => (await presenter.ink()) === 0, { label: "laser cleared" });
  assert.equal(
    await page.evaluate(`document.getElementById("present-frame").style.cursor`),
    "",
    "the cursor comes back"
  );
  await presenter.close();
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("presenter: the pen draws only while pressed, C clears, strokes never turn the page", async () => {
  await presenter.open();
  const startIndex = await presenter.currentIndex();
  await studio.key("p");

  const frame = await presenter.rect();
  const p0 = { x: frame.x + frame.w * 0.25, y: frame.y + frame.h * 0.5 };
  const p1 = { x: frame.x + frame.w * 0.75, y: frame.y + frame.h * 0.5 };

  // Hovering without the button leaves nothing.
  await studio.mouse("mouseMoved", p0.x, p0.y);
  await studio.mouse("mouseMoved", p1.x, p1.y);
  await sleep(150);
  assert.equal(await presenter.ink(), 0, "moving without pressing draws nothing");

  // A press-drag-release paints a continuous line.
  await studio.drag(p0, p1, { steps: 12 });
  await sleep(200);
  const mid = { x: (p0.x + p1.x) / 2, y: p0.y };
  assert.ok((await presenter.alphaAt(mid.x, mid.y)) > 0, "the stroke covers its midpoint");
  assert.ok((await presenter.ink()) > 200, "a real stroke was painted");
  assert.equal(await presenter.currentIndex(), startIndex, "finishing a stroke does not turn the page");

  // C clears everything.
  await studio.key("c");
  await until(async () => (await presenter.ink()) === 0, { label: "cleared" });

  // Ink belongs to the slide: moving to the next one starts clean.
  await studio.drag(p0, p1, { steps: 8 });
  await sleep(150);
  assert.ok((await presenter.ink()) > 0, "drawing again works after a clear");
  for (let i = 0; i < 4; i++) {
    await studio.key("ArrowRight");
    await sleep(150);
    if ((await presenter.currentIndex()) === startIndex + 1) break;
  }
  await until(async () => (await presenter.currentIndex()) === startIndex + 1, { label: "next slide" });
  await until(async () => (await presenter.ink()) === 0, { label: "ink cleared on slide change" });

  // Toggling the pen off disables painting entirely.
  await studio.key("p");
  await studio.drag(p0, p1, { steps: 6 });
  await sleep(150);
  assert.equal(await presenter.ink(), 0, "no drawing once the pen is off");

  await presenter.close();
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("presenter: leaving resets the tools so the next run starts clean", async () => {
  await presenter.open();
  await studio.key("p");
  const frame = await presenter.rect();
  await studio.drag(
    { x: frame.x + frame.w * 0.3, y: frame.y + frame.h * 0.3 },
    { x: frame.x + frame.w * 0.6, y: frame.y + frame.h * 0.6 },
    { steps: 8 }
  );
  assert.ok((await presenter.ink()) > 0, "ink exists before leaving");
  await presenter.close();

  await presenter.open();
  assert.equal(await presenter.ink(), 0, "no ink survives a close and reopen");
  const mode = await page.evaluate(`(() => {
    const c = document.querySelector(".present-draw-canvas");
    return { events: c.style.pointerEvents, cursor: document.getElementById("present-frame").style.cursor };
  })()`);
  assert.equal(mode.events, "none", "the overlay is inert until a tool is chosen");
  assert.equal(mode.cursor, "", "no leftover cursor");
  await presenter.close();
});

// =====================================================================
// Stability
// =====================================================================

test("stability: drag round-trips keep the block exact and the page error-free", async () => {
  const slideId = await resetDeck();
  const frame = await studio.frameMetrics();
  const box = await stageBox(slideId, ".sub");
  const centre = screenCentre(frame, box);
  await studio.click(centre.x, centre.y);
  await sleep(350);

  // First gesture pins the block; capture its settled left as the baseline.
  await studio.drag(centre, { x: centre.x + 30, y: centre.y }, { modifiers: 1, steps: 3 });
  await sleep(200);
  const baseline = px((await liveStyle(slideId, ".sub")).left);
  const elementsBefore = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelectorAll("*").length`)
  );

  for (let round = 0; round < 3; round += 1) {
    const sub = await stageBox(slideId, ".sub");
    const here = screenCentre(frame, sub);
    await studio.drag(here, { x: here.x + 40, y: here.y + 12 }, { modifiers: 1, steps: 3 });
    await sleep(250);
    const moved = await stageBox(slideId, ".sub");
    const back = screenCentre(frame, moved);
    await studio.drag(back, { x: back.x - 40, y: back.y - 12 }, { modifiers: 1, steps: 3 });
    await sleep(250);
  }
  await sleep(300);

  const settled = px((await liveStyle(slideId, ".sub")).left);
  assert.ok(Math.abs(settled - baseline) <= 3, `no drift after 25 round-trips: ${baseline} → ${settled}`);

  const elementsAfter = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelectorAll("*").length`)
  );
  assert.equal(elementsAfter, elementsBefore, "the gizmo does not leak DOM nodes");
  const handles = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelectorAll("[data-slyde-handle]").length`)
  );
  assert.equal(handles, 8, "still exactly one set of handles");

  const code = await committedCode(slideId, 1200);
  assert.equal((code.match(/<div\b/g) || []).length, (code.match(/<\/div>/g) || []).length, "tags stay balanced");
  assert.equal(
    (code.match(/class="sub"[^>]*style="[^"]*"/g) || []).length,
    1,
    "one style attribute, never duplicated"
  );
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("stability: 40 concurrent geometry commands leave one coherent declaration", async () => {
  const slideId = await studio.openSlide(0);
  const id = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub").getAttribute("data-slyde-id")`)
  );

  const results = await page.evaluate(`Promise.all(Array.from({ length: 40 }, (_, i) =>
    fetch("/api/command?t=${studio.token}", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        action: "set_geometry", slide_id: ${JSON.stringify(slideId)}, element_id: ${JSON.stringify(id)},
        x: 10 + i * 7, y: 20 + i * 3, w: 200 + i, h: 80 + i
      })
    }).then((r) => r.status)
  ))`);
  assert.ok(results.every((status) => status === 200), `all accepted: ${results.join(",")}`);

  const code = await committedCode(slideId, 300);
  const tag = (code.match(/<[a-z0-9]+[^>]*class="sub"[^>]*>/i) || [])[0];
  for (const property of ["left", "top", "width", "height", "position"]) {
    const count = (tag.match(new RegExp(`(?:^|[;"\\s])${property}:`, "g")) || []).length;
    assert.equal(count, 1, `exactly one ${property} in ${tag}`);
  }
  const left = storedPx(code, "sub", "left");
  assert.ok((left - 10) % 7 === 0 && left >= 10 && left <= 283, `left ${left} came from one whole command`);
  const w = storedPx(code, "sub", "width");
  const top = storedPx(code, "sub", "top");
  const i = (left - 10) / 7;
  assert.equal(w, 200 + i, "width belongs to the same command as left");
  assert.equal(top, 20 + i * 3, "top belongs to the same command as left");
});

test("stability: hostile geometry is refused with a readable error, never a crash", async () => {
  const slideId = await studio.openSlide(0);
  const id = await studio.inStage(slideId, (cdp) =>
    cdp.evaluate(`document.querySelector(".sub").getAttribute("data-slyde-id")`)
  );
  const before = await committedCode(slideId, 200);

  const bad = [
    { x: 1e15, y: 0, w: 100, h: 100 },
    { x: 0, y: 0, w: -50, h: 100 },
    { x: 0, y: 0, w: 0, h: 0 },
    { x: "10", y: 0, w: 100, h: 100 },
    { x: null, y: 0, w: 100, h: 100 },
    { x: 0, y: 0, w: 100 },
  ];
  for (const rect of bad) {
    const response = await studio.command({
      action: "set_geometry", slide_id: slideId, element_id: id, ...rect,
    });
    assert.ok(response.status >= 400 && response.status < 500, `${JSON.stringify(rect)} → ${response.status}`);
    assert.ok(response.body.length > 0, "the error explains itself");
  }

  const unknown = await studio.command({
    action: "set_geometry", slide_id: slideId, element_id: "does-not-exist", x: 0, y: 0, w: 10, h: 10,
  });
  assert.ok(unknown.status >= 400 && unknown.status < 500, "unknown element is a client error");
  const missingSlide = await studio.command({
    action: "set_geometry", slide_id: "nope", element_id: id, x: 0, y: 0, w: 10, h: 10,
  });
  assert.ok(missingSlide.status >= 400 && missingSlide.status < 500, "unknown slide is a client error");

  assert.equal(await committedCode(slideId, 200), before, "refused commands leave the deck untouched");
  const health = await fetch(`${studio.appUrl.split("/?")[0]}/health`);
  assert.equal(health.status, 200, "the server is still healthy");
});

test("stability: shortcut spam round-trips through undo and redo", async () => {
  await resetDeck();
  const focusThumb = () =>
    page.evaluate(`document.querySelector('.thumb[aria-current="true"]')?.focus(); true`);
  await focusThumb();

  for (let i = 0; i < 12; i += 1) {
    await studio.key("d", { modifiers: 2 });
  }
  await until(async () => (await slideCount()) === 13, { label: "twelve duplicates", timeout: 12000 });
  await sleep(500);
  assert.equal(await slideCount(), 13, "no duplicate was swallowed or doubled");

  const ids = (await studio.slides()).map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length, "every slide id is unique");

  for (let i = 0; i < 12; i += 1) await studio.command({ action: "undo" });
  assert.equal(await slideCount(), 1, "twelve undos return to the start");
  for (let i = 0; i < 12; i += 1) await studio.command({ action: "redo" });
  assert.equal(await slideCount(), 13, "twelve redos return to the end");

  await resetDeck();
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("stability: rapid slide navigation keeps the stage alive", async () => {
  await resetDeck();
  const first = (await studio.slides())[0].id;
  for (let i = 0; i < 4; i += 1) await studio.command({ action: "duplicate_slide", slide_id: first });
  await until(async () => (await slideCount()) === 5, { label: "five slides" });
  await reloadStudio();
  await studio.openSlide(0);
  await page.evaluate(`document.querySelector('.thumb[aria-current="true"]').focus(); true`);

  for (let i = 0; i < 40; i += 1) {
    await studio.key(i % 8 < 4 ? "ArrowDown" : "ArrowUp");
  }
  await sleep(600);

  const current = await page.evaluate(
    `document.querySelector('.thumb[aria-current="true"]').dataset.id`
  );
  const slides = await studio.slides();
  assert.ok(slides.some((s) => s.id === current), "the selection is a real slide");
  await studio.waitForStageReady(current);
  const frameSrc = await page.evaluate(`document.getElementById("slide-frame").src`);
  assert.ok(frameSrc.includes(encodeURIComponent(current)), "the stage shows the selected slide");
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("stability: opening and closing the presenter 12 times does not leak", async () => {
  await studio.openSlide(0);
  const metrics = () =>
    page.evaluate(`({
      nodes: document.querySelectorAll("*").length,
      canvases: document.querySelectorAll(".present-draw-canvas").length,
      listeners: typeof window.__slydeListeners === "undefined" ? 0 : window.__slydeListeners
    })`);

  await presenter.open();
  await presenter.close();
  await sleep(300);
  const baseline = await metrics();

  for (let i = 0; i < 12; i += 1) {
    await presenter.open();
    await studio.key("p");
    const frame = await presenter.rect();
    await studio.drag(
      { x: frame.x + 80, y: frame.y + 80 },
      { x: frame.x + 200, y: frame.y + 140 },
      { steps: 4 }
    );
    await presenter.close();
  }
  await sleep(400);

  const after = await metrics();
  assert.equal(after.canvases, 1, "one annotation canvas, never one per run");
  assert.ok(
    after.nodes <= baseline.nodes + Math.ceil(baseline.nodes * 0.05),
    `DOM grew from ${baseline.nodes} to ${after.nodes} nodes`
  );
  assert.equal(
    await page.evaluate(`document.getElementById("present-overlay").hidden`),
    true,
    "the studio is back"
  );
  assert.equal(studio.errors.length, 0, `page errors: ${studio.errors.join(" | ")}`);
});

test("stability: the whole run produced no page errors", () => {
  assert.deepEqual(studio.errors, [], `uncaught errors:\n${studio.errors.join("\n")}`);
});
