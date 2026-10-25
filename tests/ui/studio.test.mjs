// End-to-end smoke flow for the studio UI.
//
// Runs the real binary against an isolated data directory and drives the real
// UI in headless Chrome over CDP. No npm dependencies: `node --test tests/ui`.
//
// Covered surfaces: bundled fonts, storyboard + keyboard navigation, runtime
// i18n switch, theme toggle, speaker notes persistence, `.slyde` export,
// sandbox slide document, presenter chrome and the accessibility baseline.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

import { connectCdp, launchChrome, sleep } from "../../scripts/lib/cdp.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const uiDir = join(repoRoot, "ui");
const assetsDir = join(repoRoot, "assets");

function findBinary() {
  if (process.env.SLYDE_BIN) return process.env.SLYDE_BIN;
  const name = process.platform === "win32" ? "slyde.exe" : "slyde";
  const profiles = [
    join("x86_64-pc-windows-gnu", "debug"),
    "debug",
    "release",
    join("x86_64-pc-windows-gnu", "release"),
  ];
  return profiles
    .map((profile) => join(repoRoot, "target", profile, name))
    .find((path) => existsSync(path));
}

const binary = findBinary();

let server;
let browser;
let page;
let dataDir;
let appUrl;
let token;
let sandboxOrigin;
let firstSlideId;

function waitForAppUrl(child) {
  return new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(() => {
      reject(new Error(`slyde did not print a URL within 20s:\n${output}`));
    }, 20000);

    const onData = (chunk) => {
      output += chunk.toString();
      const match = output.match(/http:\/\/127\.0\.0\.1:\d+\/\?t=[a-f0-9]+/);
      if (match) {
        clearTimeout(timer);
        child.stdout.off("data", onData);
        resolve(match[0]);
      }
    };
    child.stdout.on("data", onData);
    child.stderr.on("data", (chunk) => {
      output += chunk.toString();
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`slyde exited with ${code} before listening:\n${output}`));
    });
  });
}

async function waitForFile(path, timeout) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    if (existsSync(path)) {
      const bytes = readFileSync(path);
      // A finished download is stable; wait a beat so Chrome is done writing.
      if (bytes.length > 0) {
        await sleep(250);
        return readFileSync(path);
      }
    }
    await sleep(200);
  }
  throw new Error(`download did not appear: ${path}`);
}

before(async () => {
  assert.ok(
    binary,
    "slyde binary not found. Run `cargo build` or set SLYDE_BIN."
  );

  dataDir = mkdtempSync(join(tmpdir(), "slyde-ui-test-"));
  server = spawn(binary, ["app", "--no-open", "--port", "0"], {
    cwd: repoRoot,
    env: {
      ...process.env,
      SLYDE_DATA_DIR: dataDir,
      SLYDE_UI_DIR: uiDir,
      SLYDE_ASSETS_DIR: assetsDir,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  appUrl = await waitForAppUrl(server);
  token = new URL(appUrl).searchParams.get("t");

  browser = await launchChrome({ viewport: { width: 1280, height: 800 } });
  page = await browser.newPage(appUrl);
  await page.waitFor("document.querySelectorAll('.thumb').length === 1");
});

after(async () => {
  await page?.close().catch(() => {});
  browser?.close();
  server?.kill();
  if (dataDir) {
    await sleep(200);
    try {
      rmSync(dataDir, { recursive: true, force: true });
    } catch {
      /* Windows may still hold a file; temp cleaner will get it. */
    }
  }
});

test("serves the UI with the bundled Geist faces and an isolated slide origin", async () => {
  const info = await page.evaluate(`(async () => {
    await document.fonts.ready;
    const frame = document.getElementById("slide-frame");
    return {
      geist: document.fonts.check("16px Geist"),
      mono: document.fonts.check("16px Geist Mono"),
      family: getComputedStyle(document.body).fontFamily,
      sandbox: frame.getAttribute("sandbox"),
      crossOrigin: new URL(frame.src).origin !== location.origin,
      lang: document.documentElement.lang
    };
  })()`);

  assert.equal(info.geist, true, "Geist sans should load from /fonts");
  assert.equal(info.mono, true, "Geist Mono should load from /fonts");
  assert.match(info.family, /^Geist,/);
  assert.equal(info.sandbox, "allow-scripts", "no allow-same-origin on slide iframes");
  assert.equal(info.crossOrigin, true, "slides must live on the sandbox origin");
});

test("storyboard adds slides and moves selection with the arrow keys", async () => {
  await page.evaluate(`document.getElementById("add-slide").click(); true`);
  await page.waitFor("document.querySelectorAll('.thumb').length === 2");

  const initial = await page.evaluate(`(() => {
    const thumbs = [...document.querySelectorAll(".thumb")];
    thumbs[0].focus();
    return {
      tabIndex: thumbs.map((thumb) => thumb.tabIndex),
      labels: thumbs.map((thumb) => thumb.getAttribute("aria-label")),
      current: thumbs.map((thumb) => thumb.getAttribute("aria-current"))
    };
  })()`);

  assert.deepEqual(initial.tabIndex, [0, -1], "only the selected thumb is tabbable");
  assert.equal(initial.current[0], "true");
  assert.ok(initial.labels.every((label) => label && label.length > 0));

  await page.press("ArrowRight", { keyCode: 39 });
  await page.waitFor(
    `document.querySelectorAll('.thumb')[1].getAttribute("aria-current") === "true"`
  );

  const moved = await page.evaluate(`(() => {
    const thumbs = [...document.querySelectorAll(".thumb")];
    return {
      tabIndex: thumbs.map((thumb) => thumb.tabIndex),
      focused: document.activeElement.dataset.id === thumbs[1].dataset.id
    };
  })()`);

  assert.deepEqual(moved.tabIndex, [-1, 0]);
  assert.equal(moved.focused, true, "keyboard focus follows the selection");
});

test("switches language at runtime and toggles the theme", async () => {
  await page.evaluate(`document.querySelector('[data-lang="en"]').click(); true`);
  await page.waitFor(`document.documentElement.lang === "en"`);

  const english = await page.evaluate(`({
    storyboard: document.querySelector(".panel-head span").textContent,
    deckLabel: document.getElementById("deck-title").getAttribute("aria-label"),
    note: document.querySelectorAll(".thumb")[1].getAttribute("aria-label")
  })`);
  assert.equal(english.storyboard, "Storyboard");
  assert.equal(english.deckLabel, "Deck title");
  assert.match(english.note, /^Slide 2,/);

  await page.evaluate(`document.querySelector('[data-lang="es"]').click(); true`);
  await page.waitFor(`document.documentElement.lang === "es"`);

  const spanish = await page.evaluate(`({
    storyboard: document.querySelector(".panel-head span").textContent,
    deckLabel: document.getElementById("deck-title").getAttribute("aria-label"),
    themeLabel: document.getElementById("theme-toggle").getAttribute("aria-label"),
    note: document.querySelectorAll(".thumb")[1].getAttribute("aria-label")
  })`);
  assert.equal(spanish.storyboard, "Guion visual");
  assert.equal(spanish.deckLabel, "Título de la presentación");
  assert.equal(spanish.themeLabel, "Cambiar entre tema claro y oscuro");
  assert.match(spanish.note, /^Diapositiva 2,/);

  const beforeTheme = await page.evaluate(`document.documentElement.dataset.theme`);
  await page.evaluate(`document.getElementById("theme-toggle").click(); true`);
  const afterTheme = await page.evaluate(`({
    theme: document.documentElement.dataset.theme,
    pressed: document.getElementById("theme-toggle").getAttribute("aria-pressed")
  })`);
  assert.notEqual(afterTheme.theme, beforeTheme);
  assert.equal(afterTheme.pressed, String(afterTheme.theme === "dark"));
});

test("autosaves speaker notes and exports a valid .slyde file", async () => {
  await page.evaluate(`(() => {
    const notes = document.getElementById("notes");
    notes.value = "Nota E2E";
    notes.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await sleep(900);

  const state = await page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`);
  sandboxOrigin = state.sandbox_origin;
  firstSlideId = state.view.presentation.slides[0].id;
  assert.equal(state.view.presentation.slides.length, 2);
  assert.equal(state.view.presentation.slides[1].notes, "Nota E2E");

  const stored = JSON.parse(
    readFileSync(join(dataDir, "decks", "current.slyde"), "utf8")
  );
  assert.equal(stored.format, "slydecore");
  assert.equal(stored.presentation.slides.length, 2);
  assert.equal(stored.presentation.slides[1].notes, "Nota E2E");

  const deck = await page.evaluate(`fetch("/api/deck?t=${token}").then((r) => r.json())`);
  assert.equal(deck.format, "slydecore");
  assert.equal(deck.presentation.slides.length, 2);
});

test("exports a PDF and a PNG bundle from the inspector with the deck name", async () => {
  await page.evaluate(`(() => {
    const input = document.getElementById("deck-title");
    input.value = "Q3 Review";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await sleep(900); // title debounce + autosave

  const downloads = mkdtempSync(join(tmpdir(), "slyde-downloads-"));
  try {
    await page.send("Page.setDownloadBehavior", {
      behavior: "allow",
      downloadPath: downloads,
    });

    const labels = await page.evaluate(`({
      busy: document.getElementById("export-section").getAttribute("aria-busy"),
      pdf: document.getElementById("export-pdf").textContent,
      png: document.getElementById("export-png").textContent,
      hint: document.getElementById("export-status").textContent
    })`);
    assert.equal(labels.busy, "false");
    assert.equal(labels.pdf, "PDF");
    assert.equal(labels.png, "Imágenes PNG");
    assert.match(labels.hint, /ZIP/);

    const busy = await page.evaluate(`(() => {
      document.getElementById("export-pdf").click();
      return {
        busy: document.getElementById("export-section").getAttribute("aria-busy"),
        disabled: document.getElementById("export-png").disabled,
        status: document.getElementById("export-status").textContent
      };
    })()`);
    assert.equal(busy.busy, "true", "the section exposes its busy state");
    assert.equal(busy.disabled, true, "both buttons are disabled while exporting");
    assert.ok(busy.status.length > 0, "progress is announced");

    const pdf = await waitForFile(join(downloads, "q3-review.pdf"), 45000);
    assert.equal(pdf.subarray(0, 5).toString("ascii"), "%PDF-");
    await page.waitFor(
      `document.getElementById("export-section").getAttribute("aria-busy") === "false"`
    );

    const pngBusy = await page.evaluate(`(() => {
      document.getElementById("export-png").click();
      return document.getElementById("export-status").textContent;
    })()`);
    assert.match(pngBusy, /Exportando 1 de 2/);
    // The second label change can only come from a WebSocket progress event.
    await page.waitFor(
      `/Exportando 2 de 2/.test(document.getElementById("export-status").textContent)`,
      { timeout: 45000 }
    );

    const zip = await waitForFile(join(downloads, "q3-review.zip"), 90000);
    assert.equal(zip.subarray(0, 4).toString("latin1"), "PK\x03\x04");

    const names = [
      ...new Set(
        [...zip.toString("latin1").matchAll(/\d{2}-sld_[a-z0-9]+\.png/g)].map(
          (match) => match[0]
        )
      ),
    ];
    assert.equal(names.length, 2, "one PNG per slide");
  } finally {
    await page.send("Page.setDownloadBehavior", { behavior: "default" }).catch(() => {});
    try {
      rmSync(downloads, { recursive: true, force: true });
    } catch {
      /* Chrome may still hold a file; the temp cleaner will get it. */
    }
  }
});

test("shows agent activity and follows the slide it writes", async () => {
  // Start on the second slide, then let go of the UI so the activity signal
  // is allowed to move the stage to the slide the agent writes.
  await page.evaluate(`(() => {
    document.querySelectorAll(".thumb")[1].click();
    document.activeElement.blur();
    return true;
  })()`);
  await sleep(2100);

  const origin = new URL(appUrl).origin;
  const pending = fetch(`${origin}/api/agent?t=${encodeURIComponent(token)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      action: "upsert_slide",
      index: 0,
      slide_id: firstSlideId,
      html_code:
        '<section class="slide"><h1>Written by the agent</h1></section>',
    }),
  });

  await page.waitFor(
    `(() => {
      const chip = document.getElementById("agent-chip");
      return chip && !chip.hidden && chip.dataset.phase === "writing";
    })()`,
    { timeout: 5000 }
  );
  const writing = await page.evaluate(`(() => {
    const chip = document.getElementById("agent-chip");
    return {
      phase: chip.dataset.phase,
      text: chip.textContent.trim(),
      announced: document.getElementById("agent-announcer").textContent
    };
  })()`);
  assert.equal(writing.phase, "writing");
  assert.match(writing.text, /diapositiva 1|slide 1/i);
  assert.match(writing.announced, /diapositiva 1|slide 1/i);

  const response = await pending;
  assert.equal(response.status, 200);

  // The done event follows the written slide while the chip still holds the
  // active phase, so the sweep is anchored to the slide that just landed.
  await page.waitFor(
    `(() => {
      const current = document.querySelector('.thumb[aria-current="true"]');
      const frame = document.getElementById("stage-frame");
      return current && current.dataset.id === ${JSON.stringify(firstSlideId)}
        && frame.dataset.agentPhase === "writing";
    })()`,
    { timeout: 5000 }
  );

  await page.waitFor(
    `document.getElementById("agent-chip").dataset.phase === "done"`,
    { timeout: 5000 }
  );
  const followed = await page.evaluate(`(() => ({
    current: document.querySelector('.thumb[aria-current="true"]').dataset.id,
    src: document.getElementById("slide-frame").src,
    label: document.getElementById("agent-label").textContent,
    announced: document.getElementById("agent-announcer").textContent
  }))()`);
  assert.equal(followed.current, firstSlideId);
  assert.match(followed.src, new RegExp(firstSlideId));
  assert.match(followed.label, /lista|ready/i);
  assert.match(followed.announced, /lista|ready/i);
});

test("renders slides from the sandbox origin with bundled fonts", async () => {
  const slidePage = await browser.newPage(
    `${sandboxOrigin}/s/${encodeURIComponent(firstSlideId)}?static=1`
  );
  try {
    const doc = await slidePage.evaluate(`(async () => {
      await document.fonts.ready;
      const stage = document.getElementById("slyde-stage");
      return {
        geist: document.fonts.check("16px Geist"),
        family: getComputedStyle(document.body).fontFamily,
        stage: !!stage,
        csp: document.querySelector('meta[http-equiv="Content-Security-Policy"]').content
      };
    })()`);
    assert.equal(doc.stage, true);
    assert.equal(doc.geist, true, "sandbox documents load Geist from /fonts");
    assert.match(doc.family, /Geist/);
    assert.match(doc.csp, /font-src 'self' data:/);
  } finally {
    await slidePage.close();
  }
});

test("presenter shows progress and advances with the keyboard", async () => {
  const presentUrl = `${new URL(appUrl).origin}/present?t=${token}`;
  const present = await browser.newPage(presentUrl);
  try {
    await present.waitFor(`document.getElementById("total").textContent === "2"`);

    const start = await present.evaluate(`({
      current: document.getElementById("current").textContent,
      total: document.getElementById("total").textContent,
      progress: document.getElementById("progress").getAttribute("aria-valuenow"),
      sandbox: document.getElementById("present-slide").getAttribute("sandbox"),
      speaker: getComputedStyle(document.getElementById("speaker-console")).display
    })`);
    assert.equal(start.current, "1");
    assert.equal(start.total, "2");
    assert.equal(start.progress, "50");
    assert.equal(start.sandbox, "allow-scripts");
    // `.speaker-console` is `display: flex`; only an explicit `[hidden]` rule
    // keeps this panel — and its clock — from covering the slide.
    assert.equal(start.speaker, "none", "the speaker console must stay hidden");

    // Clicking the slide must not steal focus: the slide frame is not an
    // interactive surface, so the overlay keeps both clicks and keystrokes.
    await present.send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: 960,
      y: 400,
      button: "left",
      clickCount: 1,
    });
    await present.send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: 960,
      y: 400,
      button: "left",
      clickCount: 1,
    });
    await sleep(250);
    assert.equal(
      await present.evaluate(`document.getElementById("current").textContent`),
      "2",
      "clicking the slide advances the deck"
    );
    assert.notEqual(
      await present.evaluate(
        `document.activeElement ? document.activeElement.tagName : ""`
      ),
      "IFRAME",
      "the slide frame never takes keyboard focus"
    );

    await present.evaluate(`document.body.focus(); true`);
    await present.press("ArrowLeft", { keyCode: 37 });
    await present.waitFor(`document.getElementById("current").textContent === "1"`);
    await present.press("ArrowRight", { keyCode: 39 });
    await present.waitFor(`document.getElementById("current").textContent === "2"`);

    const end = await present.evaluate(
      `document.getElementById("progress").getAttribute("aria-valuenow")`
    );
    assert.equal(end, "100");
  } finally {
    await present.close();
  }
});

/**
 * The slide iframe lives on the sandbox origin, so Chrome renders it as an
 * out-of-process target: it is reachable through its own CDP session and never
 * through the parent page's frame tree.
 */
async function waitForSlideFrame(needle, { timeout = 5000 } = {}) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    const targets = await fetch(`${browser.origin}/json/list`).then((r) => r.json());
    // Stage and storyboard frames use `?static=1`; only the presenter asks for
    // the animated document (`?p=N`).
    const target = targets.find(
      (entry) => entry.type === "iframe" && needle.test(entry.url)
    );
    if (target) {
      const cdp = connectCdp(target.webSocketDebuggerUrl);
      await cdp.ready;
      await cdp.send("Runtime.enable");
      return cdp;
    }
    await sleep(100);
  }
  throw new Error(`no slide iframe matching ${needle}`);
}

test("in-studio presenter paints the slide instead of a black stage", async () => {
  // Regression: the speaker console was `display: flex` even while `hidden`, so
  // an opaque panel with a ticking clock covered the whole stage (its previews
  // looked black because the UI believed them hidden). The click that opened
  // the presenter also advanced the deck, eating the first animation step and
  // leaving every animated element stuck at `opacity: 0`.
  //
  // Earlier tests leave the deck in different shapes, so pick a slide that
  // actually animates and open the presenter on it.
  const stateUrl = `/api/state?t=${token}`;
  let state = await page.evaluate(`fetch("${stateUrl}").then((r) => r.json())`);
  let targetIndex = state.view.presentation.slides.findIndex(
    (slide) => slide.animation.length > 0
  );
  if (targetIndex < 0) {
    await page.evaluate(`fetch("/api/command?t=${token}", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ action: "add_slide", template: "hero" })
    }).then((r) => r.json())`);
    state = await page.evaluate(`fetch("${stateUrl}").then((r) => r.json())`);
    targetIndex = state.view.presentation.slides.length - 1;
  }
  assert.ok(
    state.view.presentation.slides[targetIndex].animation.length > 0,
    "the presenter test needs a slide with animation steps"
  );
  await page.evaluate(
    `document.querySelectorAll(".thumb")[${targetIndex}].click(); true`
  );

  const box = await page.evaluate(`(() => {
    const rect = document.getElementById("present").getBoundingClientRect();
    return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
  })()`);
  // A background tab freezes the Web Animations timeline, which would leave
  // every reveal animation parked at frame 0. A real presenter is on screen.
  await page.send("Page.bringToFront");
  await page.send("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: box.x,
    y: box.y,
    button: "left",
    clickCount: 1,
  });
  await page.send("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: box.x,
    y: box.y,
    button: "left",
    clickCount: 1,
  });

  try {
    await page.waitFor(`document.getElementById("present-overlay").hidden === false`);

    const chrome = await page.evaluate(`({
      overlay: getComputedStyle(document.getElementById("present-overlay")).display,
      speaker: getComputedStyle(document.getElementById("speaker-console")).display,
      current: document.getElementById("hud-current").textContent,
      src: document.getElementById("present-slide").getAttribute("src") || ""
    })`);
    assert.notEqual(chrome.overlay, "none", "the presenter overlay is on screen");
    assert.equal(chrome.speaker, "none", "the speaker console must not cover the slide");
    // A real click on the topbar must not be read as "advance the slide".
    assert.equal(chrome.current, String(targetIndex + 1));
    assert.ok(
      chrome.src.startsWith(sandboxOrigin),
      `the slide frame loads from the sandbox origin: ${chrome.src}`
    );

    const steps = state.view.presentation.slides[targetIndex].animation;
    const targets = [...new Set(steps.map((step) => step.target))];

    const slideFrame = await waitForSlideFrame(/[?&]p=\d/);
    try {
      const readOpacity = () =>
        slideFrame.evaluate(`(${JSON.stringify(targets)}).map((selector) => {
          const el = document.querySelector(selector);
          return el ? getComputedStyle(el).opacity : "missing";
        })`);
      const started = Date.now();
      let entered = [];
      while (Date.now() - started < 6000) {
        entered = await readOpacity();
        if (entered[0] === "1") break;
        await sleep(120);
      }
      assert.equal(
        entered[0],
        "1",
        `the first step plays as the slide appears: ${JSON.stringify(entered)}`
      );
      if (targets.length > 1) {
        assert.equal(
          entered[1],
          "0",
          `later steps wait for the presenter: ${JSON.stringify(entered)}`
        );
        for (let index = 1; index < targets.length; index += 1) {
          await page.press("ArrowRight", { keyCode: 39 });
          await sleep(320);
        }
        const deadline = Date.now() + 4000;
        let revealed = await readOpacity();
        while (Date.now() < deadline && revealed.some((value) => value !== "1")) {
          await sleep(120);
          revealed = await readOpacity();
        }
        assert.deepEqual(
          revealed,
          targets.map(() => "1"),
          "each key press reveals the next step"
        );
      }
    } finally {
      slideFrame.close();
    }

    // The slide must not eat input: a click on it reaches the overlay, the
    // frame never takes focus, and the keyboard keeps driving the deck.
    const total = state.view.presentation.slides.length;
    const canForward = targetIndex < total - 1;
    const viewport = await page.evaluate(
      `({ w: window.innerWidth, h: window.innerHeight })`
    );
    const clickX = Math.round(viewport.w * (canForward ? 0.75 : 0.25));
    const clickY = Math.round(viewport.h * 0.5);
    for (const type of ["mousePressed", "mouseReleased"]) {
      await page.send("Input.dispatchMouseEvent", {
        type,
        x: clickX,
        y: clickY,
        button: "left",
        clickCount: 1,
      });
    }
    await sleep(300);
    const afterClick = await page.evaluate(`({
      current: document.getElementById("hud-current").textContent,
      active: document.activeElement ? (document.activeElement.id || document.activeElement.tagName) : ""
    })`);
    assert.notEqual(afterClick.active, "present-slide", "the slide never takes focus");
    if (total > 1) {
      assert.equal(
        afterClick.current,
        String(canForward ? targetIndex + 2 : targetIndex),
        "clicking the slide advances or rewinds the deck"
      );
      await page.press(canForward ? "ArrowLeft" : "ArrowRight", {
        keyCode: canForward ? 37 : 39,
      });
      await sleep(300);
      const afterKey = await page.evaluate(
        `document.getElementById("hud-current").textContent`
      );
      assert.equal(
        afterKey,
        String(targetIndex + 1),
        "the keyboard still works after clicking the slide"
      );
    }

    // Defense in depth: if a slide script ever takes focus, its runtime forwards
    // the keystrokes so presentation controls keep working. Pick a direction
    // that is guaranteed to move away from the current slide.
    const forwardKey = targetIndex < total - 1;
    const canMove = forwardKey || targetIndex > 0;
    const focused = await waitForSlideFrame(/[?&]p=\d/);
    try {
      await focused.evaluate(
        `document.documentElement.tabIndex = -1; document.documentElement.focus(); true`
      );
      if (canMove) {
        const before = await page.evaluate(
          `document.getElementById("hud-current").textContent`
        );
        const key = forwardKey ? "ArrowRight" : "ArrowLeft";
        const keyCode = forwardKey ? 39 : 37;
        for (const type of ["keyDown", "keyUp"]) {
          await focused.send("Input.dispatchKeyEvent", {
            type,
            key,
            code: key,
            windowsVirtualKeyCode: keyCode,
            nativeVirtualKeyCode: keyCode,
          });
        }
        await sleep(400);
        const forwarded = await page.evaluate(
          `document.getElementById("hud-current").textContent`
        );
        assert.notEqual(
          forwarded,
          before,
          "a key pressed inside the slide still navigates the deck"
        );
      }
    } finally {
      focused.close();
    }
  } finally {
    // Escape leaves fullscreen first (Chrome swallows it), which the presenter
    // treats as the exit gesture.
    await page.press("Escape", { keyCode: 27 });
    await page.waitFor(`document.getElementById("present-overlay").hidden === true`, {
      timeout: 4000,
    });
  }
});

test("the present split menu opens the speaker console from a real click", async () => {
  // Regression: `.topbar { overflow: hidden }` clipped the dropdown rendered
  // below the bar, so every item (slide view, speaker console, new window) was
  // invisible and unclickable.
  await page.evaluate(`document.getElementById("present-menu-trigger").click(); true`);
  await sleep(400);

  const spot = await page.evaluate(`(() => {
    const rect = document.getElementById("present-speaker").getBoundingClientRect();
    const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
    return {
      x: rect.x + rect.width / 2,
      y: rect.y + rect.height / 2,
      hitsItem: !!hit && !!hit.closest("#present-speaker")
    };
  })()`);
  assert.equal(spot.hitsItem, true, "the dropdown is not clipped by the topbar");

  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", {
      type,
      x: spot.x,
      y: spot.y,
      button: "left",
      clickCount: 1,
    });
  }

  try {
    await page.waitFor(
      `getComputedStyle(document.getElementById("speaker-console")).display !== "none"`
    );
    const speaker = await page.evaluate(`({
      overlay: getComputedStyle(document.getElementById("present-overlay")).display,
      current: document.getElementById("sc-current-frame").getAttribute("src") || "",
      total: document.getElementById("sc-total").textContent,
      clock: document.getElementById("wall-clock").textContent,
      notes: document.getElementById("speaker-notes-body").textContent
    })`);
    assert.notEqual(speaker.overlay, "none", "the console lives inside the presenter overlay");
    assert.match(speaker.current, /\?static=1/, "the console previews the static document");
    assert.match(speaker.total, /^\d+$/);
    assert.match(speaker.clock, /^\d{2}:\d{2}:\d{2}$/);
    assert.ok(speaker.notes.length > 0, "the notes panel renders its empty state");
  } finally {
    await page.evaluate(`document.getElementById("speaker-close-btn").click(); true`);
    await page.waitFor(`document.getElementById("present-overlay").hidden === true`);
  }
});

test("copilot connects a provider, filters models and restores focus", async () => {
  // The provider transport is intentionally mocked at the HTTP boundary: this
  // test covers the panel, key form, model discovery UI and selection without
  // ever touching a real API key or a vendor network.
  const fakeConfig = {
    providers: [
      { id: "openai", name: "ChatGPT", company: "OpenAI", icon: "openai", key_url: "https://platform.openai.com/api-keys", configured: false },
      { id: "anthropic", name: "Claude", company: "Anthropic", icon: "anthropic", key_url: "https://console.anthropic.com/settings/keys", configured: false },
      { id: "gemini", name: "Gemini", company: "Google", icon: "gemini", key_url: "https://aistudio.google.com/app/apikey", configured: false },
      { id: "xai", name: "Grok", company: "xAI", icon: "xai", key_url: "https://console.x.ai", configured: false },
      { id: "groq", name: "Groq", company: "Groq", icon: "groq", key_url: "https://console.groq.com/keys", configured: false },
      { id: "openrouter", name: "OpenRouter", company: "OpenRouter", icon: "openrouter", key_url: "https://openrouter.ai/keys", configured: false },
      { id: "deepseek", name: "DeepSeek", company: "DeepSeek", icon: "deepseek", key_url: "https://platform.deepseek.com/api_keys", configured: false },
      { id: "nvidia", name: "NVIDIA", company: "NVIDIA", icon: "nvidia", key_url: "https://build.nvidia.com", configured: false },
    ],
    last_provider: null,
    last_model: null,
  };
  const fakeModels = [
    { id: "free-model", name: "Free Model", free: true, context_window: 128000 },
    { id: "paid-model", name: "Paid Model", free: false, context_window: 32000 },
    { id: "free-two", name: "Second Free", free: true, vision: true },
  ];

  const fulfill = (requestId, payload) =>
    page.send("Fetch.fulfillRequest", {
      requestId,
      responseCode: 200,
      responseHeaders: [{ name: "content-type", value: "application/json" }],
      body: Buffer.from(JSON.stringify(payload)).toString("base64"),
    });

  const off = page.client.on("Fetch.requestPaused", async ({ requestId, request }) => {
    try {
      if (request.url.includes("/api/copilot/config")) {
        await fulfill(requestId, { ok: true, config: fakeConfig });
      } else if (request.url.includes("/api/copilot/key")) {
        await fulfill(requestId, { ok: true, models: fakeModels });
      } else {
        await page.send("Fetch.continueRequest", { requestId });
      }
    } catch {
      /* The page may have been torn down; nothing to resume. */
    }
  });

  await page.send("Fetch.enable", {
    patterns: [
      { urlPattern: "*api/copilot/config*", requestStage: "Request" },
      { urlPattern: "*api/copilot/key*", requestStage: "Request" },
    ],
  });

  try {
    const timeOrigin = await page.evaluate("performance.timeOrigin");
    await page.send("Page.reload", { ignoreCache: true });
    await page.waitFor(
      `performance.timeOrigin !== ${JSON.stringify(timeOrigin)} &&
       document.getElementById("app") && !document.getElementById("app").hidden`,
      { timeout: 15000 }
    );

    await page.evaluate(`document.querySelector('[data-lang="es"]').click(); true`);
    await page.waitFor(`document.documentElement.lang === "es"`);
    await page.evaluate(`document.getElementById("tab-copilot").click(); true`);
    await page.waitFor(`!!document.querySelector("#copilot-thread [data-connect]")`);

    const spanishEmpty = await page.evaluate(
      `document.querySelector("#copilot-thread h2").textContent`
    );
    assert.equal(spanishEmpty, "Conecta tu IA");

    await page.evaluate(`document.querySelector('[data-lang="en"]').click(); true`);
    await page.waitFor(
      `document.documentElement.lang === "en" &&
       document.querySelector("#copilot-thread h2")?.textContent === "Connect your AI"`
    );

    await page.evaluate(`document.querySelector("#copilot-thread [data-connect]").click(); true`);
    await page.waitFor(`document.getElementById("copilot-sheet").hidden === false`);
    await page.waitFor(`document.activeElement && document.activeElement.id === "sheet-key"`);

    const sheet = await page.evaluate(`(() => {
      const root = document.getElementById("copilot-sheet");
      return {
        title: document.getElementById("copilot-sheet-title").textContent,
        providers: root.querySelectorAll("[data-provider]").length,
        selected: root.querySelector(".provider-card.is-selected")?.dataset.provider,
        keyVisible: !!root.querySelector("#sheet-key"),
        focused: document.activeElement && document.activeElement.id,
        marks: root.querySelectorAll(".provider-mark svg").length
      };
    })()`);
    assert.equal(sheet.title, "Provider & model");
    assert.equal(sheet.providers, 8, "all eight providers are offered");
    assert.equal(sheet.selected, "openai");
    assert.equal(sheet.keyVisible, true);
    assert.equal(sheet.focused, "sheet-key");
    assert.equal(sheet.marks, 8, "every provider has its own mark");

    await page.evaluate(`(() => {
      const key = document.getElementById("sheet-key");
      key.value = "sk-e2e-not-a-real-key";
      key.dispatchEvent(new Event("input", { bubbles: true }));
      document.getElementById("sheet-key-form").requestSubmit();
      return true;
    })()`);
    await page.waitFor(`document.getElementById("sheet-search") !== null`);

    const models = await page.evaluate(`(() => ({
      rows: document.querySelectorAll("[data-model]").length,
      free: document.querySelectorAll(".model-badge.is-free").length,
      count: document.querySelector(".model-count").textContent
    }))()`);
    assert.equal(models.rows, 3);
    assert.equal(models.free, 2);
    assert.match(models.count, /3/);

    await page.evaluate(`(() => {
      const search = document.getElementById("sheet-search");
      search.value = "paid";
      search.dispatchEvent(new Event("input", { bubbles: true }));
      return true;
    })()`);
    await page.waitFor(`document.querySelectorAll("[data-model]").length === 1`);

    await page.evaluate(`(() => {
      const search = document.getElementById("sheet-search");
      search.value = "";
      search.dispatchEvent(new Event("input", { bubbles: true }));
      document.getElementById("sheet-free").click();
      return true;
    })()`);
    await page.waitFor(`document.querySelectorAll("[data-model]").length === 2`);
    const pressed = await page.evaluate(
      `document.getElementById("sheet-free").getAttribute("aria-pressed")`
    );
    assert.equal(pressed, "true");

    await page.evaluate(`document.querySelector('[data-model="free-model"]').click(); true`);
    await page.waitFor(`document.getElementById("copilot-sheet").hidden === true`);
    await page.waitFor(`document.activeElement && document.activeElement.id === "copilot-input"`);

    const selected = await page.evaluate(`(() => ({
      provider: document.getElementById("copilot-provider-name").textContent,
      model: document.getElementById("copilot-model-label").textContent,
      chip: document.getElementById("copilot-model-chip-text").textContent,
      label: document.getElementById("copilot-provider").getAttribute("aria-label")
    }))()`);
    assert.equal(selected.provider, "ChatGPT");
    assert.equal(selected.model, "free-model");
    assert.equal(selected.chip, "free-model");
    assert.equal(selected.label, "ChatGPT — free-model");

    await page.evaluate(`(() => {
      const transfer = new DataTransfer();
      transfer.items.add(new File(["# Notes\\nA short brief."], "brief.txt", { type: "text/plain" }));
      const input = document.getElementById("copilot-file");
      input.files = transfer.files;
      input.dispatchEvent(new Event("change", { bubbles: true }));
      return true;
    })()`);
    await page.waitFor(`document.querySelectorAll("#copilot-attachments .pending-file").length === 1`);
    const attached = await page.evaluate(`(() => ({
      name: document.querySelector("#copilot-attachments .pending-name").textContent,
      sendEnabled: !document.getElementById("copilot-send").disabled,
      removeLabel: document.querySelector("#copilot-attachments .pending-remove").getAttribute("aria-label")
    }))()`);
    assert.equal(attached.name, "brief.txt");
    assert.equal(attached.sendEnabled, true);
    assert.match(attached.removeLabel, /brief\.txt/);

    await page.evaluate(
      `document.querySelector("#copilot-attachments .pending-remove").click(); true`
    );
    await page.waitFor(
      `document.querySelectorAll("#copilot-attachments .pending-file").length === 0`
    );

    await page.evaluate(`document.querySelector('[data-lang="es"]').click(); true`);
    await page.waitFor(
      `document.documentElement.lang === "es" &&
       document.querySelector("#copilot-thread h2")?.textContent === "Crea algo que merezca presentarse"`
    );
    const persisted = await page.evaluate(`(() => ({
      model: document.getElementById("copilot-model-label").textContent,
      stored: JSON.parse(localStorage.getItem("slyde.copilot.selection"))
    }))()`);
    assert.equal(persisted.model, "free-model");
    assert.deepEqual(persisted.stored, { provider: "openai", model: "free-model" });
  } finally {
    off();
    await page.send("Fetch.disable").catch(() => {});
  }
});

test("copilot streams text and localized tool status into the thread", async () => {
  const fakeConfig = {
    providers: [
      { id: "openai", name: "ChatGPT", company: "OpenAI", icon: "openai", key_url: "https://platform.openai.com/api-keys", configured: true },
      { id: "anthropic", name: "Claude", company: "Anthropic", icon: "anthropic", key_url: "https://console.anthropic.com/settings/keys", configured: false },
      { id: "gemini", name: "Gemini", company: "Google", icon: "gemini", key_url: "https://aistudio.google.com/app/apikey", configured: false },
      { id: "xai", name: "Grok", company: "xAI", icon: "xai", key_url: "https://console.x.ai", configured: false },
      { id: "groq", name: "Groq", company: "Groq", icon: "groq", key_url: "https://console.groq.com/keys", configured: false },
      { id: "openrouter", name: "OpenRouter", company: "OpenRouter", icon: "openrouter", key_url: "https://openrouter.ai/keys", configured: false },
      { id: "deepseek", name: "DeepSeek", company: "DeepSeek", icon: "deepseek", key_url: "https://platform.deepseek.com/api_keys", configured: false },
      { id: "nvidia", name: "NVIDIA", company: "NVIDIA", icon: "nvidia", key_url: "https://build.nvidia.com", configured: false },
    ],
    last_provider: "openai",
    last_model: "free-model",
  };
  const events = [
    { type: "meta", provider: "openai", model: "free-model" },
    { type: "delta", text: "I'll build " },
    { type: "delta", text: "a cover slide." },
    { type: "tool", name: "upsert_slide", status: "running" },
    { type: "tool", name: "upsert_slide", status: "done" },
    { type: "done" },
  ];
  const sse = `${events.map((event) => `data: ${JSON.stringify(event)}`).join("\n\n")}\n\n`;
  let chatPayload = null;

  const fulfill = (requestId, payload, contentType = "application/json") =>
    page.send("Fetch.fulfillRequest", {
      requestId,
      responseCode: 200,
      responseHeaders: [{ name: "content-type", value: contentType }],
      body: Buffer.from(
        typeof payload === "string" ? payload : JSON.stringify(payload)
      ).toString("base64"),
    });

  const off = page.client.on("Fetch.requestPaused", async ({ requestId, request }) => {
    try {
      if (request.url.includes("/api/copilot/config")) {
        await fulfill(requestId, { ok: true, config: fakeConfig });
      } else if (request.url.includes("/api/copilot/chat")) {
        chatPayload = JSON.parse(request.postData || "{}");
        await fulfill(requestId, sse, "text/event-stream");
      } else {
        await page.send("Fetch.continueRequest", { requestId });
      }
    } catch {
      /* The page may have been torn down; nothing to resume. */
    }
  });

  await page.send("Fetch.enable", {
    patterns: [
      { urlPattern: "*api/copilot/config*", requestStage: "Request" },
      { urlPattern: "*api/copilot/chat*", requestStage: "Request" },
    ],
  });

  try {
    const timeOrigin = await page.evaluate("performance.timeOrigin");
    await page.evaluate(`(() => {
      localStorage.setItem("slyde.copilot.selection", JSON.stringify({ provider: "openai", model: "free-model" }));
      localStorage.setItem("slyde.lang", "es");
      return true;
    })()`);
    await page.send("Page.reload", { ignoreCache: true });
    await page.waitFor(
      `performance.timeOrigin !== ${JSON.stringify(timeOrigin)} &&
       document.getElementById("app") && !document.getElementById("app").hidden`,
      { timeout: 15000 }
    );
    await page.evaluate(`document.getElementById("tab-copilot").click(); true`);
    await page.waitFor(
      `document.getElementById("copilot-provider-name").textContent === "ChatGPT" &&
       document.getElementById("copilot-model-label").textContent === "free-model"`
    );

    await page.evaluate(`(() => {
      const input = document.getElementById("copilot-input");
      input.value = "Crea una portada";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      document.getElementById("copilot-form").requestSubmit();
      return true;
    })()`);

    await page.waitFor(
      `document.querySelector(".msg-assistant .msg-text")?.textContent === "I'll build a cover slide."`,
      { timeout: 5000 }
    );
    await page.waitFor(
      `(() => {
        const button = document.getElementById("copilot-send");
        return !button.classList.contains("is-stopping") &&
          document.getElementById("copilot-live").hidden === true;
      })()`
    );
    await page.waitFor(
      `document.getElementById("copilot-announcer").textContent === "El copiloto ha terminado de responder"`
    );

    const thread = await page.evaluate(`(() => {
      const assistant = document.querySelector(".msg-assistant");
      return {
        state: assistant.dataset.state,
        text: assistant.querySelector(".msg-text").textContent,
        tools: [...assistant.querySelectorAll(".tool-row")].map((row) => ({
          status: row.dataset.status,
          label: row.textContent.trim()
        })),
        user: document.querySelector(".msg-user .msg-text").textContent,
        sendDisabled: document.getElementById("copilot-send").disabled
      };
    })()`);
    assert.equal(thread.state, "done");
    assert.equal(thread.text, "I'll build a cover slide.");
    assert.equal(thread.user, "Crea una portada");
    assert.equal(thread.tools.length, 1);
    assert.equal(thread.tools[0].status, "done");
    assert.equal(thread.tools[0].label, "Escribiendo la diapositiva…");
    assert.equal(thread.sendDisabled, true);

    assert.ok(chatPayload, "the UI posted a chat request");
    assert.equal(chatPayload.provider, "openai");
    assert.equal(chatPayload.model, "free-model");
    assert.equal(chatPayload.language, "es");
    assert.ok(chatPayload.selected_slide_id, "the selected slide rides along");
    assert.equal(chatPayload.messages.at(-1).role, "user");
    assert.equal(chatPayload.messages.at(-1).text, "Crea una portada");
  } finally {
    off();
    await page.send("Fetch.disable").catch(() => {});
  }
});

test("selects a block on the canvas and lists the slide structure", async () => {
  // Give the stage a deterministic slide with authored blocks.
  await page.evaluate(`fetch("/api/command?t=${token}", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      action: "add_slide",
      template: "hero",
      index: 0
    })
  }).then((r) => r.json())`);
  await sleep(400);
  await page.evaluate(`document.querySelectorAll(".thumb")[0].click(); true`);
  await sleep(900);

  const stage = await page.evaluate(`(() => {
    const rect = document.getElementById("slide-frame").getBoundingClientRect();
    return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
  })()`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", {
      type,
      x: stage.x,
      y: stage.y,
      button: "left",
      clickCount: 1,
    });
  }
  await sleep(500);

  const selection = await page.evaluate(`({
    hud: !document.getElementById("element-hud").hidden,
    tag: document.getElementById("element-hud-tag").textContent,
    pill: document.getElementById("copilot-context-pill").textContent,
    tree: document.querySelectorAll(".element-tree-item").length
  })`);
  assert.equal(selection.hud, true, "the element HUD is visible");
  assert.match(selection.tag, /^#\S+$/, "the HUD names the selected block");
  assert.match(selection.pill, /Destino: /, "the copilot targets the block");
  assert.ok(selection.tree > 0, "the inspector lists the slide structure");

  // The palette opens scoped to the selection.
  await page.evaluate(
    `document.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true })); true`
  );
  await sleep(250);
  const palette = await page.evaluate(`({
    open: !document.getElementById("command-palette").hidden,
    groups: [...document.querySelectorAll(".palette-group")].map((node) => node.textContent)
  })`);
  assert.equal(palette.open, true);
  assert.ok(palette.groups.includes("Elemento seleccionado"));

  await page.evaluate(
    `document.getElementById("palette-input").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); true`
  );
  await sleep(200);
  assert.equal(
    await page.evaluate(`document.getElementById("command-palette").hidden`),
    true
  );
});

test("renders code, math and diagrams inside the sandbox", async () => {
  const html = `<div class="slide tech">
    <div class="slyde-diagram">graph LR; A-->B</div>
    <p class="sub">Coste $O(n \\log n)$</p>
    <pre class="slyde-code" data-lang="rust" data-emphasis="2"><code>fn main() {
    run();
}</code></pre>
  </div>`;
  await page.evaluate(`fetch("/api/agent?t=${token}", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ action: "upsert_slide", index: 0, html_code: ${JSON.stringify(html)}, template: "code" })
  }).then((r) => r.json())`);
  const state = await page.evaluate(
    `fetch("/api/state?t=${token}").then((r) => r.json())`
  );
  const slideId = state.view.presentation.slides[0].id;

  const slidePage = await browser.newPage(
    `${sandboxOrigin}/s/${encodeURIComponent(slideId)}?static=1`
  );
  try {
    await slidePage.waitFor(
      `document.documentElement.getAttribute("data-slyde-ready") === "1"`,
      { timeout: 15000 }
    );
    const rendered = await slidePage.evaluate(`({
      lines: document.querySelectorAll(".slyde-line").length,
      emphasis: document.querySelectorAll(".slyde-line.is-emphasis").length,
      tokens: document.querySelectorAll(".hljs-keyword, .hljs-title").length,
      katex: document.querySelectorAll(".katex").length,
      diagram: document.querySelectorAll(".slyde-diagram svg").length
    })`);
    assert.equal(rendered.lines, 3, "one wrapper per code line");
    assert.equal(rendered.emphasis, 1, "the requested line is emphasised");
    assert.ok(rendered.tokens > 0, "syntax highlighting applied");
    assert.equal(rendered.katex, 1, "LaTeX rendered with KaTeX");
    assert.equal(rendered.diagram, 1, "Mermaid produced an SVG");
  } finally {
    await slidePage.close();
  }
});

test("meets the accessibility baseline: names, focus ring and AA contrast", async () => {
  // Start from a known point in the tab order, then move with the keyboard so
  // Chrome applies :focus-visible exactly as a real keyboard user would see it.
  await page.evaluate(`document.getElementById("present").focus(); true`);
  await page.press("Tab", { keyCode: 9 });

  const focus = await page.evaluate(`(() => {
    const el = document.activeElement;
    const ring = getComputedStyle(el);
    return {
      className: el.className,
      isThumb: el.classList.contains("thumb"),
      focusVisible: el.matches(":focus-visible"),
      outlineWidth: ring.outlineWidth,
      outlineStyle: ring.outlineStyle
    };
  })()`);
  // Tab leaves the top bar and lands inside the storyboard: either the selected
  // thumb itself or one of its action buttons.
  assert.match(
    focus.className,
    /thumb/,
    `Tab lands inside the storyboard: ${JSON.stringify(focus)}`
  );
  assert.equal(focus.focusVisible, true);
  assert.notEqual(focus.outlineStyle, "none");
  assert.notEqual(focus.outlineWidth, "0px");

  const audit = await page.evaluate(`(() => {
    const colors = ["--bg", "--surface", "--surface-sunken", "--ink", "--ink-muted", "--ink-subtle", "--brand", "--danger"];
    const linear = (value) => value <= 0.03928 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
    const hex = (value) => {
      let raw = value.trim().replace("#", "");
      if (raw.length === 3) raw = raw.split("").map((char) => char + char).join("");
      return [0, 2, 4].map((i) => parseInt(raw.slice(i, i + 2), 16) / 255);
    };
    const luminance = (rgb) => 0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]);
    const contrast = (a, b) => {
      const one = luminance(hex(a));
      const two = luminance(hex(b));
      return (Math.max(one, two) + 0.05) / (Math.min(one, two) + 0.05);
    };
    const readTheme = () => {
      const style = getComputedStyle(document.documentElement);
      const tokens = Object.fromEntries(colors.map((name) => [name, style.getPropertyValue(name)]));
      const pairs = [
        ["--ink", "--surface"],
        ["--ink-muted", "--surface"],
        ["--ink-subtle", "--surface"],
        ["--ink-subtle", "--surface-sunken"],
        ["--brand", "--surface"],
        ["--danger", "--surface"]
      ];
      return pairs.map(([foreground, background]) => ({
        pair: foreground + " on " + background,
        ratio: contrast(tokens[foreground], tokens[background])
      }));
    };

    const root = document.documentElement;
    const previous = root.dataset.theme;
    const light = readTheme();
    root.dataset.theme = "dark";
    const dark = readTheme();
    root.dataset.theme = previous;

    const unnamedButtons = [...document.querySelectorAll("button")].filter((button) => {
      return !(button.getAttribute("aria-label") || button.textContent.trim());
    }).length;

    const iconTargets = [...document.querySelectorAll(".icon-btn")].map((button) => {
      const box = button.getBoundingClientRect();
      return Math.min(box.width, box.height);
    });

    const untitledFrames = [...document.querySelectorAll("iframe")].filter((frame) => {
      return !frame.title && frame.getAttribute("aria-hidden") !== "true";
    }).length;

    return { light, dark, unnamedButtons, iconTargets, untitledFrames };
  })()`);

  for (const entry of [...audit.light, ...audit.dark]) {
    assert.ok(entry.ratio >= 4.5, `${entry.pair} is ${entry.ratio.toFixed(2)}:1`);
  }
  assert.equal(audit.unnamedButtons, 0, "every button needs an accessible name");
  assert.equal(audit.untitledFrames, 0, "iframes need a title or aria-hidden");
  assert.ok(audit.iconTargets.every((size) => size >= 32), "icon targets are 32px+");
});
