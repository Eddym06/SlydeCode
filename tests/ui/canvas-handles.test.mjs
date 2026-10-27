// Direct manipulation on the canvas: the resize handles around the selected block
// and the magnetic guides. Runs the real binary like the other UI suites.
//
// The slide lives in a cross-origin iframe, so the test drives it the way a
// person does: real mouse events at screen coordinates, then reads what reached
// the deck through the API.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";
import { sleep } from "../../scripts/lib/cdp.mjs";

let studio;
let page;

// A block that is already absolutely positioned: 300..700 x 200..400 on the
// 1280x720 stage, so its east edge is 640 stage pixels away from nothing in
// particular and 640 is the stage centre.
const BLOCK_LEFT = 300;
const BLOCK_TOP = 200;
const BLOCK_WIDTH = 400;
const BLOCK_HEIGHT = 200;

before(async () => {
  studio = await startStudio();
  page = studio.page;
});

after(async () => {
  await studio?.stop();
});

async function mouse(type, x, y, buttons = 0) {
  await page.send("Input.dispatchMouseEvent", {
    type,
    x,
    y,
    button: "left",
    buttons,
    clickCount: 1,
  });
}

async function blockCode() {
  const state = await studio.state();
  return state.view.presentation.slides[0].code;
}

function declared(code, property) {
  const match = code.match(new RegExp(`${property}: (-?\\d+)px`));
  return match ? Number(match[1]) : null;
}

async function frameBox() {
  return page.evaluate(`(() => {
    const rect = document.getElementById("slide-frame").getBoundingClientRect();
    return { x: rect.x, y: rect.y, w: rect.width, h: rect.height };
  })()`);
}

test("a handle resizes any block, and the edge snaps to the stage centre", async () => {
  const origin = new URL(studio.appUrl).origin;
  const slideId = (await studio.state()).view.presentation.slides[0].id;
  const response = await fetch(`${origin}/api/agent?t=${encodeURIComponent(studio.token)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      action: "upsert_slide",
      index: 0,
      slide_id: slideId,
      html_code:
        `<div data-id="card" style="position:absolute;left:${BLOCK_LEFT}px;top:${BLOCK_TOP}px;` +
        `width:${BLOCK_WIDTH}px;height:${BLOCK_HEIGHT}px;background:#7C3AED"></div>`,
    }),
  });
  assert.equal(response.status, 200);
  await sleep(1200);

  const frame = await frameBox();
  const scale = frame.w / 1280;
  const at = (stageX, stageY) => ({ x: frame.x + stageX * scale, y: frame.y + stageY * scale });

  // Select the block with a plain click.
  const centre = at(BLOCK_LEFT + BLOCK_WIDTH / 2, BLOCK_TOP + BLOCK_HEIGHT / 2);
  await mouse("mousePressed", centre.x, centre.y, 1);
  await mouse("mouseReleased", centre.x, centre.y);
  await sleep(500);

  // Grab the south-east handle and pull the east edge to 3 stage pixels short of
  // the stage centre: inside the snap distance, so it must land exactly on 640.
  const grip = at(BLOCK_LEFT + BLOCK_WIDTH, BLOCK_TOP + BLOCK_HEIGHT);
  const target = at(637, BLOCK_TOP + BLOCK_HEIGHT);
  await mouse("mousePressed", grip.x, grip.y, 1);
  for (const step of [0.25, 0.5, 0.75, 1]) {
    await mouse(
      "mouseMoved",
      grip.x + (target.x - grip.x) * step,
      grip.y + (target.y - grip.y) * step,
      1
    );
  }
  await mouse("mouseReleased", target.x, target.y);

  let code = "";
  for (let attempt = 0; attempt < 30; attempt += 1) {
    code = await blockCode();
    if (declared(code, "width") !== BLOCK_WIDTH) break;
    await sleep(150);
  }
  assert.equal(declared(code, "left"), BLOCK_LEFT, `the west edge did not move: ${code}`);
  assert.equal(declared(code, "top"), BLOCK_TOP, `the north edge did not move: ${code}`);
  assert.equal(
    declared(code, "width"),
    640 - BLOCK_LEFT,
    `the east edge snapped to the stage centre (x = 640): ${code}`
  );
  assert.equal(declared(code, "height"), BLOCK_HEIGHT, `the height is untouched: ${code}`);
});

test("a west handle moves only the west edge and a block never shrinks below 16px", async () => {
  const code = await blockCode();
  const left = declared(code, "left");
  const width = declared(code, "width");
  const frame = await frameBox();
  const scale = frame.w / 1280;
  const at = (stageX, stageY) => ({ x: frame.x + stageX * scale, y: frame.y + stageY * scale });

  // The slide reloaded after the first commit, so select the block again.
  const east = left + width;
  const midY = declared(code, "top") + declared(code, "height") / 2;
  const inside = at(left + width / 2, midY);
  await mouse("mousePressed", inside.x, inside.y, 1);
  await mouse("mouseReleased", inside.x, inside.y);
  await sleep(500);

  // West handle sits at the middle of the left edge. Drag it far past the east
  // edge: the block collapses to the minimum size and keeps its east edge.
  const grip = at(left, midY);
  const target = at(east + 200, midY);
  await mouse("mousePressed", grip.x, grip.y, 1);
  for (const step of [0.25, 0.5, 0.75, 1]) {
    await mouse(
      "mouseMoved",
      grip.x + (target.x - grip.x) * step,
      grip.y + (target.y - grip.y) * step,
      1
    );
  }
  await mouse("mouseReleased", target.x, target.y);

  let next = "";
  for (let attempt = 0; attempt < 30; attempt += 1) {
    next = await blockCode();
    if (declared(next, "width") !== width) break;
    await sleep(150);
  }
  assert.equal(declared(next, "width"), 16, `the block stops at the minimum size: ${next}`);
  assert.equal(
    declared(next, "left") + declared(next, "width"),
    east,
    `the east edge stayed where it was: ${next}`
  );
});
