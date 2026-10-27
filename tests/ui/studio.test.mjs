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
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
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
  // Several target directories can exist at once (a cross build and a host
  // build). Taking the first hit silently tested a binary from an older build,
  // so pick the newest one and say which it is.
  const candidates = profiles
    .map((profile) => join(repoRoot, "target", profile, name))
    .filter((path) => existsSync(path))
    .map((path) => ({ path, mtime: statSync(path).mtimeMs }))
    .sort((left, right) => right.mtime - left.mtime);
  if (candidates.length === 0) return undefined;
  return candidates[0].path;
}

const binary = findBinary();

let server;
let browser;
let page;
let dataDir;
let appUrl;
let token;
let sandboxOrigin;
let sandboxToken = "";
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

  // Slide documents are authenticated with their own token; tests that open a
  // slide directly need it, and it must not be the session token.
  const boot = await page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`);
  sandboxOrigin = boot.sandbox_origin;
  sandboxToken = boot.sandbox_token;
  assert.ok(sandboxToken, "the server publishes a sandbox token");
  assert.notEqual(sandboxToken, token, "slide documents use a different secret");
});

/** URL for a slide document, carrying the sandbox token. */
function slideUrl(slideId, extra = "") {
  const params = new URLSearchParams(extra);
  params.set("t", sandboxToken);
  return `${sandboxOrigin}/s/${encodeURIComponent(slideId)}?${params}`;
}

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

  // Decks live in the library now: one file per deck, named after the active id.
  // The save is debounced, so poll instead of racing the 300 ms window.
  const library = await page.evaluate(`fetch("/api/decks?t=${token}").then((r) => r.json())`);
  assert.ok(library.active, "a deck is active in the library");
  const deckPath = join(dataDir, "decks", `${library.active}.slyde`);
  let stored = null;
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      const candidate = JSON.parse(readFileSync(deckPath, "utf8"));
      if (candidate.presentation.slides.length === 2) {
        stored = candidate;
        break;
      }
    } catch (error) {
      // The file may not exist yet on the very first pass.
    }
    await sleep(100);
  }
  assert.ok(stored, `the active deck file was written: ${deckPath}`);
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

    // The PNGs are rendered in one Chrome session at 2x; reading the IHDR of the
    // first stored PNG proves both (a per-slide run would be 1x, and the old
    // sequential path would report 1280). PNGs are stored in the ZIP, so the
    // bytes appear verbatim in the archive.
    const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const at = zip.indexOf(signature);
    assert.ok(at >= 0, "the bundle contains a PNG");
    const width = zip.readUInt32BE(at + 16);
    const height = zip.readUInt32BE(at + 20);
    assert.equal(width, 2560, "slides are rendered at 2x (2560px wide)");
    assert.equal(height, 1440, "the 16:9 ratio is preserved at 2x");
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
  const slidePage = await browser.newPage(slideUrl(firstSlideId, "static=1"));
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

/** The live CDP target of the studio stage document for `slideId`. */
async function stageTargetFor(slideId) {
  const needle = new RegExp(`/s/${slideId}[^"]*edit=1`);
  const targets = await fetch(`${browser.origin}/json/list`).then((r) => r.json());
  return targets.find((entry) => entry.type === "iframe" && needle.test(entry.url)) || null;
}

/**
 * Waits until the stage iframe has run its own script, which is what installs
 * the selection handlers (`data-slyde-ready`). The stage document lives on the
 * sandbox origin as an out-of-process target, so it is polled through its own
 * CDP session; on a cold runner a slide document can take seconds to load, and
 * a click that lands earlier selects nothing.
 */
async function waitForStageReady(slideId, { timeout = 20000 } = {}) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    const target = await stageTargetFor(slideId);
    if (target) {
      const cdp = connectCdp(target.webSocketDebuggerUrl);
      try {
        await cdp.ready;
        await cdp.send("Runtime.enable");
        const ready = await cdp.evaluate(
          `document.documentElement.getAttribute("data-slyde-ready") === "1"`
        );
        if (ready === true) return true;
      } catch {
        /* the target may be mid-navigation; try again on the next tick */
      } finally {
        cdp.close();
      }
    }
    await sleep(150);
  }
  throw new Error(`the stage never became ready for slide ${slideId}`);
}

/**
 * Selects the first block of the stage document and waits for the studio HUD.
 *
 * The pointer path is tried first. Clicking through the parent page relies on
 * Chrome routing input into the out-of-process stage frame, which is not
 * reliable on every machine, so the fallback clicks the block through the
 * stage's own CDP session: the sandbox handler, the `select` message, the HUD
 * and the scoped command are the same either way, and `via` records which path
 * worked.
 *
 * The HUD node is created lazily by the first selection, so every read below
 * tolerates a missing node; a failure throws with a snapshot instead of a bare
 * `TypeError`.
 */
async function selectBlockOnStage(t, page, slideId, { pointerAttempts = 3 } = {}) {
  // `SLYDE_UI_FORCE_DOCUMENT_SELECT=1` exercises the fallback branch without
  // waiting for a machine where parent-page input misses the stage frame.
  const attempts =
    process.env.SLYDE_UI_FORCE_DOCUMENT_SELECT === "1" ? 0 : pointerAttempts;
  const probe = () =>
    page.evaluate(`(() => {
      const frame = document.getElementById("slide-frame");
      const rect = frame.getBoundingClientRect();
      const hud = document.getElementById("element-hud");
      const tag = document.getElementById("element-hud-tag");
      return {
        x: rect.x + rect.width / 2,
        y: rect.y + rect.height / 2,
        w: Math.round(rect.width),
        h: Math.round(rect.height),
        hud: !!hud && hud.hidden === false,
        tag: tag ? tag.textContent : "",
        src: frame.src,
        viewport: { w: window.innerWidth, h: window.innerHeight },
        thumbs: [...document.querySelectorAll(".thumb")].map((node) => node.dataset.id),
        tree: [...document.querySelectorAll("[data-tree-id]")].map(
          (node) => node.dataset.treeId
        ),
        messages: Array.isArray(window.__sandboxMsgs)
          ? window.__sandboxMsgs.slice(-12)
          : null
      };
    })()`);
  const selected = (value) => Boolean(value) && value.hud && value.tag.length > 0 && !value.tag.startsWith("#");

  let last = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    last = await probe();
    if (selected(last)) return { tag: last.tag, via: "pointer" };
    // A frame that has not been laid out yet reports a tiny rect; its centre
    // would land on the shell instead of the slide.
    if (last.w > 100 && last.h > 60) {
      for (const type of ["mousePressed", "mouseReleased"]) {
        await page.send("Input.dispatchMouseEvent", {
          type,
          x: last.x,
          y: last.y,
          button: "left",
          clickCount: 1,
        });
      }
    }
    await sleep(400);
    last = await probe();
    if (selected(last)) return { tag: last.tag, via: "pointer" };
  }

  const target = await stageTargetFor(slideId);
  if (target) {
    const cdp = connectCdp(target.webSocketDebuggerUrl);
    try {
      await cdp.ready;
      await cdp.send("Runtime.enable");
      await cdp.evaluate(`(() => {
        const block = document.querySelector("[data-slyde-id]");
        if (block) block.click();
        return !!block;
      })()`);
    } finally {
      cdp.close();
    }
    const started = Date.now();
    while (Date.now() - started < 5000) {
      last = await probe();
      if (selected(last)) {
        t.diagnostic(
          "the studio selected the block through the stage document, not through a parent click"
        );
        return { tag: last.tag, via: "document" };
      }
      await sleep(150);
    }
  }

  throw new Error(`the stage selected nothing: ${JSON.stringify(last)}`);
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

    // The conversation is stored per deck...
    const decks = await page.evaluate(
      `fetch("/api/decks?t=${token}").then((r) => r.json())`
    );
    assert.ok(decks.active, "a deck is active");
    const saved = await page.evaluate(
      `fetch("/api/copilot/thread?deck=${decks.active}&t=${token}").then((r) => r.json())`
    );
    assert.equal(saved.ok, true);
    assert.ok(saved.messages.length >= 2, "the turn was stored");
    assert.equal(saved.messages[0].role, "user");
    assert.equal(saved.messages[0].text, "Crea una portada");

    // ...and reloading the studio brings it back instead of an empty panel.
    const reloadedAt = await page.evaluate("performance.timeOrigin");
    await page.send("Page.reload", { ignoreCache: true });
    await page.waitFor(
      `performance.timeOrigin !== ${JSON.stringify(reloadedAt)} &&
       document.querySelectorAll(".thumb").length >= 1`,
      { timeout: 15000 }
    );
    await page.waitFor(
      `document.getElementById("copilot-thread").textContent.includes("Crea una portada")`,
      { timeout: 10000 }
    );
    const restored = await page.evaluate(`(() => ({
      user: document.querySelector(".msg-user .msg-text")?.textContent || "",
      assistant: document.querySelector(".msg-assistant .msg-text")?.textContent || ""
    }))()`);
    assert.equal(restored.user, "Crea una portada", "the question survived the reload");
    assert.equal(
      restored.assistant,
      "I'll build a cover slide.",
      "the answer survived the reload"
    );

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

test("a failed turn explains itself without echoing provider text", async () => {
  const fakeConfig = {
    providers: [
      { id: "openai", name: "ChatGPT", company: "OpenAI", icon: "openai", key_url: "https://platform.openai.com/api-keys", configured: true },
    ],
    last_provider: "openai",
    last_model: "free-model",
  };
  const rawLimit =
    "the copilot reached its limit of 20 steps and stopped; the changes it already made are saved";
  const rawInvalid = "provider is required";
  const responses = [
    { type: "error", code: "tool_limit", message: rawLimit },
    { type: "error", code: "invalid_request", message: rawInvalid },
  ];
  let chatCalls = 0;

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
        const payload = responses[Math.min(chatCalls, responses.length - 1)];
        chatCalls += 1;
        await fulfill(requestId, `data: ${JSON.stringify(payload)}\n\n`, "text/event-stream");
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

  const readError = `(() => {
    const boxes = [...document.querySelectorAll(".msg-assistant .msg-error")];
    const box = boxes[boxes.length - 1];
    return {
      visible: Boolean(box && !box.hidden),
      text: box ? box.querySelector("p")?.textContent ?? "" : "",
      detail: box?.querySelector(".msg-error-detail")?.textContent ?? null,
      retry: Boolean(box?.querySelector("[data-retry]")),
      thread: document.getElementById("copilot-thread").textContent
    };
  })()`;

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
      `document.getElementById("copilot-provider-name").textContent === "ChatGPT"`
    );

    // Conversations persist per deck now, so this test starts from a clean one
    // (which also exercises the new-chat path and its stored-thread clearing).
    await page.evaluate(`document.getElementById("copilot-new").click(); true`);
    await sleep(300);
    assert.equal(
      await page.evaluate(`document.querySelectorAll("#copilot-thread .msg").length`),
      0,
      "a new chat clears the restored thread"
    );

    await page.evaluate(`(() => {
      const input = document.getElementById("copilot-input");
      input.value = "Hazme una portada";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      document.getElementById("copilot-form").requestSubmit();
      return true;
    })()`);

    await page.waitFor(
      `(() => {
        const box = document.querySelector(".msg-assistant .msg-error");
        return box && !box.hidden && Boolean(box.querySelector("[data-retry]"));
      })()`,
      { timeout: 5000 }
    );

    const stopped = await page.evaluate(readError);
    assert.equal(stopped.visible, true);
    assert.match(stopped.text, /El copiloto se detuvo tras muchos pasos/);
    assert.equal(stopped.detail, null, "provider payloads never reach the thread");
    assert.ok(!stopped.thread.includes(rawLimit), "the raw loop message stays out of the DOM");
    assert.equal(stopped.retry, true, "a failed turn offers a retry");

    await page.evaluate(`document.querySelector(".msg-assistant [data-retry]").click(); true`);
    await page.waitFor(
      `document.getElementById("copilot-thread").textContent.includes("provider is required") ||
       document.querySelector(".msg-assistant").dataset.state === "error"`,
      { timeout: 5000 }
    );
    await page.waitFor(
      `(() => {
        const box = document.querySelector(".msg-assistant .msg-error");
        return box && !box.hidden && box.querySelector(".msg-error-detail");
      })()`,
      { timeout: 5000 }
    );

    const invalid = await page.evaluate(readError);
    assert.equal(invalid.detail, rawInvalid, "our own validation message is actionable");
    assert.match(invalid.text, /petición no era válida/);
    assert.ok(!invalid.thread.includes(rawLimit));
    assert.equal(chatCalls, 2);
  } finally {
    off();
    await page.send("Fetch.disable").catch(() => {});
  }
});

test("the new deck button starts a deck without destroying the previous one", async () => {
  // The library changed the semantics of this button on purpose: it used to
  // replace the open deck (recoverable only by undo), now it creates another
  // deck file and leaves the previous one in the library.
  const start = await page.evaluate(`document.querySelectorAll(".thumb").length`);
  for (let index = start; index < 2; index += 1) {
    const expected = index + 1;
    await page.evaluate(`document.getElementById("add-slide").click(); true`);
    await page.waitFor(`document.querySelectorAll(".thumb").length === ${expected}`);
  }
  await page.evaluate(`(() => {
    const title = document.getElementById("deck-title");
    title.value = "E2E original deck";
    title.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await sleep(700);

  const before = await page.evaluate(
    `fetch("/api/decks?t=${token}").then((r) => r.json())`
  );
  const previous = before.active;
  const previousCount = before.decks.length;

  await page.evaluate(`document.getElementById("topbar-new-deck").click(); true`);
  await page.waitFor(`document.querySelectorAll(".thumb").length === 1`, { timeout: 8000 });
  await sleep(400);

  const after = await page.evaluate(`(() => ({
    slides: document.querySelectorAll(".thumb").length,
    title: document.getElementById("deck-title").value,
    toast: document.getElementById("toast")?.textContent ?? ""
  }))()`);
  assert.equal(after.slides, 1, "the new deck starts empty");
  assert.match(after.title, /sin título|untitled/i);
  assert.match(after.toast, /lista|ready/i, "the toast confirms the new deck");

  const list = await page.evaluate(`fetch("/api/decks?t=${token}").then((r) => r.json())`);
  assert.equal(list.decks.length, previousCount + 1, "one more deck in the library");
  assert.notEqual(list.active, previous, "a different deck is open now");
  const survivor = list.decks.find((deck) => deck.id === previous);
  assert.ok(survivor, "the previous deck survived");
  assert.equal(survivor.slides, 2, "with all of its slides");
  assert.equal(survivor.title, "E2E original deck");
});

test("the deck library sheet lists, renames and reopens decks", async () => {
  // The previous test guarantees an "E2E original deck" with two slides plus the
  // open one, so this test never depends on absolute deck counts.
  await page.evaluate(`document.getElementById("topbar-library").click(); true`);
  await page.waitFor(`!document.getElementById("deck-library").hidden`, { timeout: 5000 });
  await sleep(300);

  const opened = await page.evaluate(`(() => ({
    rows: document.querySelectorAll(".deck-row").length,
    badge: document.querySelector(".deck-badge")?.textContent ?? "",
    title: document.getElementById("library-title").textContent
  }))()`);
  assert.ok(opened.rows >= 2, "every deck in the library is listed");
  assert.match(opened.badge, /abierto|open/i, "the open deck is marked");
  assert.ok(opened.title.length > 2, "the sheet is localized");

  // Search narrows the list without touching the decks.
  await page.evaluate(`(() => {
    const search = document.getElementById("library-search");
    search.value = "original deck";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await sleep(150);
  assert.ok(
    (await page.evaluate(`document.querySelectorAll(".deck-row").length`)) >= 1,
    "the search keeps the matching deck"
  );
  assert.ok(
    (await page.evaluate(`document.querySelectorAll(".deck-row").length`)) <
      (await page.evaluate(`fetch("/api/decks?t=${token}").then((r) => r.json())`)).decks.length,
    "the search hides the rest"
  );
  await page.evaluate(`(() => {
    const search = document.getElementById("library-search");
    search.value = "";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await sleep(150);

  // Rename the open deck inline and check both the state and the file list.
  await page.evaluate(
    `document.querySelector('.deck-row.is-active [data-library-action="rename"]').click(); true`
  );
  await sleep(200);
  await page.evaluate(`(() => {
    const input = document.getElementById("deck-rename-input");
    input.value = "Renamed from the sheet";
    input.closest("form").requestSubmit();
    return true;
  })()`);
  await sleep(600);
  const renamed = await page.evaluate(
    `fetch("/api/state?t=${token}").then((r) => r.json())`
  );
  assert.equal(renamed.view.presentation.title, "Renamed from the sheet");
  assert.match(
    await page.evaluate(`document.getElementById("deck-library").textContent`),
    /Renamed from the sheet/,
    "the list shows the new name"
  );


  // Reopen the old deck: its slides are still there and the sheet closes.
  await page.evaluate(`(() => {
    const row = [...document.querySelectorAll(".deck-row")].find((item) =>
      item.textContent.includes("E2E original deck")
    );
    row.querySelector(".deck-open").click();
    return true;
  })()`);
  await page.waitFor(`document.getElementById("deck-library").hidden`, { timeout: 5000 });
  await page.waitFor(`document.querySelectorAll(".thumb").length === 2`, { timeout: 8000 });
  const reopened = await page.evaluate(
    `fetch("/api/state?t=${token}").then((r) => r.json())`
  );
  assert.equal(reopened.view.presentation.title, "E2E original deck");
  assert.equal(reopened.view.presentation.slides.length, 2);
});


test("opening a .slyde file from disk restores the deck and its title", async () => {
  const deckJson = await page.evaluate(`fetch("/api/deck?t=${token}").then((r) => r.text())`);
  const parsed = JSON.parse(deckJson);
  const expectedTitle = (parsed.presentation || parsed).title;
  const expectedSlides = await page.evaluate(`document.querySelectorAll(".thumb").length`);
  const filePath = join(dataDir, "roundtrip.slyde");
  writeFileSync(filePath, deckJson, "utf8");

  // Rename first, so a silent no-op open cannot pass the assertion below.
  await page.evaluate(`(() => {
    const input = document.getElementById("deck-title");
    input.value = "Temporary title";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await page.waitFor(`document.getElementById("deck-title").value === "Temporary title"`);

  const doc = await page.send("DOM.getDocument", { depth: 1 });
  const node = await page.send("DOM.querySelector", {
    nodeId: doc.root.nodeId,
    selector: "#topbar-file-picker",
  });
  await page.send("DOM.setFileInputFiles", { nodeId: node.nodeId, files: [filePath] });

  await page.waitFor(
    `["Presentación abierta", "Presentation opened"].includes(document.getElementById("toast").textContent)`,
    { timeout: 10000 }
  );
  const opened = await page.evaluate(`(() => ({
    title: document.getElementById("deck-title").value,
    slides: document.querySelectorAll(".thumb").length
  }))()`);
  assert.equal(opened.title, expectedTitle, "the file title wins over the edited one");
  assert.equal(opened.slides, expectedSlides);
  rmSync(filePath, { force: true });
});

test("inserts an image, moves it and keeps it inside the .slyde bundle", async () => {
  // A 1x1 PNG is enough: the store sniffs the format, and the canvas places the
  // picture by aspect ratio rather than by its own assumptions.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64"
  );
  const fixture = join(mkdtempSync(join(tmpdir(), "slyde-image-")), "pixel.png");
  writeFileSync(fixture, png);

  // Start from a known deck so the assertions are not at the mercy of prior tests.
  await page.evaluate(`fetch("/api/command?t=${token}", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ action: "new_deck", title: "Images E2E", template: "hero" })
  }).then((r) => r.json())`);
  await sleep(500);
  await page.evaluate(`document.querySelectorAll(".thumb")[0].click(); true`);
  await sleep(400);

  // Drive the inspector's real file input through CDP, so the change event takes
  // the same path a user's file picker would.
  const { root } = await page.send("DOM.getDocument", { depth: -1 });
  const { nodeId } = await page.send("DOM.querySelector", {
    nodeId: root.nodeId,
    selector: "#inspector-image-file",
  });
  assert.ok(nodeId, "the inspector exposes an image input");
  await page.send("DOM.setFileInputFiles", { files: [fixture], nodeId });

  // Upload + insert are two round trips; wait for the block to appear.
  let state = null;
  for (let attempt = 0; attempt < 40; attempt += 1) {
    state = await page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`);
    const code = state.view.presentation.slides[0].code;
    if (code.includes("/assets/") && code.includes('data-id="image-1"')) break;
    await sleep(150);
  }
  const code = state.view.presentation.slides[0].code;
  assert.match(code, /data-id="image-1"/, "the image becomes a block with a handle");
  assert.match(code, /src="\/assets\/[0-9a-f]+\.png"/, "the image lives in the asset store");
  assert.match(code, /position: absolute/, "it is positioned over the slide");
  assert.match(code, /width: \d+px/, "it has a size");

  // The slide document renders it from the sandbox origin.
  const slideId = state.view.presentation.slides[0].id;
  const slidePage = await browser.newPage(slideUrl(slideId, "static=1&edit=1"));
  let rendered;
  try {
    await slidePage.waitFor(
      `document.querySelectorAll("img[data-id='image-1']").length === 1`,
      { timeout: 10000 }
    );
    rendered = await slidePage.evaluate(`(() => {
      const img = document.querySelector("img[data-id='image-1']");
      return {
        src: img.getAttribute("src"),
        complete: img.complete,
        naturalWidth: img.naturalWidth,
        left: img.style.left,
        width: img.style.width
      };
    })()`);
  } finally {
    await slidePage.close();
  }
  assert.match(rendered.src, /^\/assets\/[0-9a-f]+\.png$/, "served from the sandbox origin");
  assert.equal(rendered.complete, true, "the image actually loaded");
  assert.equal(rendered.naturalWidth, 1, "the stored bytes are the uploaded pixels");

  // Select it on the canvas and drag it: the geometry the canvas reports must
  // reach the deck state (and survive a reload of the slide).
  const frame = await page.evaluate(`(() => {
    const rect = document.getElementById("slide-frame").getBoundingClientRect();
    return { x: rect.x, y: rect.y, w: rect.width, h: rect.height };
  })()`);
  const before = Number(rendered.left.replace("px", ""));
  const beforeWidth = Number(rendered.width.replace("px", ""));
  const cx = frame.x + (before + beforeWidth / 2) * (frame.w / 1280);
  const cy = frame.y + frame.h / 2;
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", {
      type,
      x: cx,
      y: cy,
      button: "left",
      clickCount: 1,
    });
  }
  await sleep(300);

  // Drag it 40 screen pixels to the right. The canvas is scaled to the stage, so
  // the expected offset is that distance divided by the render scale.
  const dragBy = 40;
  await page.send("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: cx,
    y: cy,
    button: "left",
    buttons: 1,
    clickCount: 1,
  });
  for (const step of [10, 20, 30, 40]) {
    await page.send("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: cx + step,
      y: cy,
      button: "left",
      buttons: 1,
    });
  }
  await page.send("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: cx + dragBy,
    y: cy,
    button: "left",
    buttons: 0,
    clickCount: 1,
  });
  await sleep(800);

  state = await page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`);
  const moved = state.view.presentation.slides[0].code.match(/left: (\d+)px/);
  assert.ok(moved, "the block still has a left offset");
  const expected = before + Math.round(dragBy / (frame.w / 1280));
  assert.ok(
    Math.abs(Number(moved[1]) - expected) <= 2,
    `dragging moved the image to ${moved[1]}px (expected ~${expected}px)`
  );

  // The saved deck is a bundle with the picture inside it.
  const response = await fetch(`http://127.0.0.1:${new URL(appUrl).port}/api/deck?t=${token}`);
  assert.equal(response.status, 200);
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.equal(
    bytes.subarray(0, 2).toString("latin1"),
    "PK",
    "a deck with images downloads as a ZIP"
  );
  assert.ok(bytes.includes(Buffer.from("assets/")), "the bundle carries the asset folder");
});

test("the reasoning drawer follows the stream, scrolls and expands", async () => {
  const fakeConfig = {
    providers: [
      { id: "openai", name: "ChatGPT", company: "OpenAI", icon: "openai", key_url: "https://platform.openai.com/api-keys", configured: true },
    ],
    last_provider: "openai",
    last_model: "free-model",
  };
  // Enough reasoning to overflow the 180px box several times over.
  const chunk = "Let me weigh the layout options for this cover slide. ";
  const thinking = Array.from({ length: 90 }, (_, index) => `${index} ${chunk}`).join("");
  const events = [
    { type: "thinking", text: thinking },
    { type: "delta", text: "Done." },
    { type: "done" },
  ];
  const sse = `${events.map((event) => `data: ${JSON.stringify(event)}`).join("\n\n")}\n\n`;

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
      `document.getElementById("copilot-provider-name").textContent === "ChatGPT"`
    );
    await page.evaluate(`(() => {
      const input = document.getElementById("copilot-input");
      input.value = "Piensa en voz alta";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      document.getElementById("copilot-form").requestSubmit();
      return true;
    })()`);

    // The reasoning drawer starts collapsed (the work card summarizes it); a user
    // who wants the raw text opens it.
    await page.waitFor(`Boolean(document.querySelector(".thinking-drawer"))`);
    assert.equal(
      await page.evaluate(`document.querySelector(".thinking-drawer").open`),
      false,
      "reasoning is collapsed by default while the copilot works"
    );
    await page.evaluate(`document.querySelector(".thinking-drawer").open = true; true`);

    await page.waitFor(
      `(() => {
        const stream = document.querySelector(".thinking-stream");
        return stream && stream.scrollHeight > stream.clientHeight + 200;
      })()`,
      { timeout: 8000 }
    );

    // `<details>` fires its toggle event asynchronously, and the stream pins itself
    // to the tail from that handler: wait for the state instead of racing it.
    await page.waitFor(
      `(() => {
        const stream = document.querySelector(".thinking-stream");
        return stream.scrollHeight - stream.scrollTop - stream.clientHeight < 8;
      })()`,
      { timeout: 4000 }
    );

    const stuck = await page.evaluate(`(() => {
      const stream = document.querySelector(".thinking-stream");
      const box = stream.getBoundingClientRect();
      return {
        overflow: stream.scrollHeight > stream.clientHeight,
        atTail: stream.scrollHeight - stream.scrollTop - stream.clientHeight < 8,
        capped: box.height <= 200,
        scrolled: stream.scrollTop > 0
      };
    })()`);
    assert.equal(stuck.overflow, true, "the reasoning overflows its box");
    assert.equal(stuck.capped, true, "the inline box keeps its height cap");
    assert.equal(stuck.atTail, true, "the stream follows the tail while it grows");

    // Double click on the header promotes the drawer to a large overlay.
    await page.evaluate(`(() => {
      const header = document.querySelector(".thinking-header");
      header.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
      return true;
    })()`);
    await page.waitFor(`Boolean(document.querySelector("#thinking-layer .thinking-drawer.is-expanded"))`);

    const expanded = await page.evaluate(`(() => {
      const layer = document.getElementById("thinking-layer");
      const drawer = layer.querySelector(".thinking-drawer");
      const stream = drawer.querySelector(".thinking-stream");
      const box = drawer.getBoundingClientRect();
      return {
        layerVisible: !layer.hidden,
        inLayer: layer.contains(drawer),
        stacked: document.querySelectorAll(".msg-thinking .thinking-drawer").length,
        width: Math.round(box.width),
        height: Math.round(box.height),
        roomier: stream.clientHeight > 260,
        toggleExpanded: drawer.querySelector("[data-thinking-expand]").getAttribute("aria-expanded")
      };
    })()`);
    assert.equal(expanded.layerVisible, true);
    assert.equal(expanded.inLayer, true);
    assert.equal(expanded.stacked, 0, "the drawer leaves the thread while expanded");
    assert.ok(expanded.width >= 600, `expanded width is ${expanded.width}`);
    assert.ok(expanded.height >= 400, `expanded height is ${expanded.height}`);
    assert.equal(expanded.roomier, true, "expanded mode gives the text more room");
    assert.equal(expanded.toggleExpanded, "true");

    await page.evaluate(`document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
    await page.waitFor(`document.getElementById("thinking-layer").hidden === true`);
    const restored = await page.evaluate(`document.querySelectorAll(".msg-thinking .thinking-drawer").length`);
    assert.equal(restored, 1, "the drawer returns to its message");
  } finally {
    off();
    await page.send("Fetch.disable").catch(() => {});
  }
});

test("selects a block on the canvas and lists the slide structure", async (t) => {
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
  const addedState = await page.evaluate(
    `fetch("/api/state?t=${token}").then((r) => r.json())`
  );
  const addedId = addedState.view.presentation.slides[0].id;
  await waitForStageReady(addedId);

  const { tag: hudTag } = await selectBlockOnStage(t, page, addedId);
  assert.ok(hudTag.length > 0 && !hudTag.startsWith("#"), "the HUD names the block in plain words");

  const selection = await page.evaluate(`({
    hud: !document.getElementById("element-hud").hidden,
    pill: document.getElementById("copilot-context-pill").textContent,
    tree: document.querySelectorAll(".element-tree-item").length
  })`);
  assert.equal(selection.hud, true, "the element HUD is visible");
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

test("editing a block only touches the slide that owns it", async (t) => {
  // Regression: generated handles are unique per slide document, not per deck, so
  // both slides below carry `data-id="slyde-1"`. The canvas used to send only the
  // handle and the server edited the first slide that matched.
  const block = (label) =>
    `<h1 style="position:absolute;inset:0;display:grid;place-items:center">${label}</h1>`;

  await page.evaluate(`fetch("/api/agent?t=${token}", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ action: "create_presentation", title: "Scoped edits" })
  }).then((r) => r.json())`);
  for (const [index, label] of [[0, "UNO"], [1, "DOS"]]) {
    await page.evaluate(`fetch("/api/agent?t=${token}", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        action: "upsert_slide",
        index: ${index},
        html_code: ${JSON.stringify(block(label))},
        template: "hero"
      })
    }).then((r) => r.json())`);
  }
  const stateUrl = `/api/state?t=${token}`;
  let state = await page.evaluate(`fetch("${stateUrl}").then((r) => r.json())`);
  const [firstId, secondId] = state.view.presentation.slides.map((slide) => slide.id);
  for (const slide of state.view.presentation.slides) {
    assert.ok(
      slide.code.includes('data-id="slyde-1"'),
      `both slides share the generated handle: ${slide.code}`
    );
  }

  // Record the commands the studio sends while we drive a real canvas edit.
  await page.evaluate(`(() => {
    window.__originalFetch = window.fetch;
    window.__sentCommands = [];
    window.fetch = (input, init) => {
      const url = typeof input === "string" ? input : input.url;
      if (url.includes("/api/command") && init && init.body) {
        try { window.__sentCommands.push(JSON.parse(init.body)); } catch (error) {}
      }
      return window.__originalFetch(input, init);
    };
    return true;
  })()`);

  try {
    // Select the second slide and wait until the stage document is live. The
    // iframe reloads whenever state lands, and its click handlers only exist
    // once the sandbox script has run, so a fixed delay is a coin flip on a
    // cold runner.
    await page.evaluate(`document.querySelectorAll(".thumb")[1].click(); true`);
    await waitForStageReady(secondId);

    // Record what the sandbox reports so a failure explains itself: a missing
    // `select` means the click never reached the stage document, while a
    // `fromStage: false` entry means the studio dropped a stale frame's message.
    await page.evaluate(`(() => {
      window.__sandboxMsgs = [];
      window.__sandboxListener = (event) => {
        const data = event.data;
        if (!data || data.__slyde !== true) return;
        const frame = document.getElementById("slide-frame");
        window.__sandboxMsgs.push({
          type: data.type,
          id: data.id || "",
          fromStage: !!frame && event.source === frame.contentWindow
        });
        if (window.__sandboxMsgs.length > 40) window.__sandboxMsgs.shift();
      };
      window.addEventListener("message", window.__sandboxListener);
      return true;
    })()`);

    const { tag: hudTag } = await selectBlockOnStage(t, page, secondId);
    assert.ok(hudTag.length > 0 && !hudTag.startsWith("#"), "the stage selected the block on the second slide");

    await page.evaluate(
      `document.querySelector('[data-element-action="align-left"]').click(); true`
    );

    // The command travels over fetch; poll for it rather than racing a delay.
    let mutation = null;
    const mutationStarted = Date.now();
    while (!mutation && Date.now() - mutationStarted < 5000) {
      const sent = await page.evaluate(`window.__sentCommands`);
      mutation = sent.filter((command) => command.action === "mutate_element").pop() ?? null;
      if (!mutation) await sleep(150);
    }
    assert.ok(mutation, "the HUD sent a mutate_element command");
    assert.equal(mutation.element_id, "slyde-1");
    assert.equal(
      mutation.slide_id,
      secondId,
      "the command names the slide the user is looking at"
    );

    state = await page.evaluate(`fetch("${stateUrl}").then((r) => r.json())`);
    const [first, second] = state.view.presentation.slides;
    assert.equal(first.id, firstId);
    assert.ok(
      !first.code.includes("text-align: left"),
      `the first slide must stay untouched: ${first.code}`
    );
    assert.ok(
      second.code.includes("text-align: left"),
      `the edited slide carries the change: ${second.code}`
    );
  } finally {
    await page.evaluate(`(() => {
      if (window.__originalFetch) window.fetch = window.__originalFetch;
      delete window.__originalFetch;
      delete window.__sentCommands;
      if (window.__sandboxListener) {
        window.removeEventListener("message", window.__sandboxListener);
      }
      delete window.__sandboxListener;
      delete window.__sandboxMsgs;
      return true;
    })()`);
  }
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

  const slidePage = await browser.newPage(slideUrl(slideId, "static=1"));
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

test("edits the animation timeline from the inspector", async () => {
  await page.evaluate(`document.querySelectorAll(".thumb")[0].click(); true`);
  await sleep(600);

  const readSteps = async () => {
    const state = await page.evaluate(
      `fetch("/api/state?t=${token}").then((r) => r.json())`
    );
    return state.view.presentation.slides[0].animation;
  };

  const before = await readSteps();

  // Add two steps so there is something to reorder.
  for (let index = 0; index < 2; index += 1) {
    await page.evaluate(`document.getElementById("anim-add").click(); true`);
    await sleep(500);
  }
  const added = await readSteps();
  assert.equal(added.length, before.length + 2, "add step appends one each time");

  // Change the kind of the last step.
  await page.evaluate(`(() => {
    const rows = document.querySelectorAll(".anim-step");
    const row = rows[rows.length - 1];
    const select = row.querySelector('[data-field="kind"]');
    select.value = "pulse";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  })()`);
  await sleep(600);
  let steps = await readSteps();
  assert.equal(steps[steps.length - 1].kind, "pulse", "the kind is editable");

  // Move it up one place.
  await page.evaluate(`(() => {
    const rows = document.querySelectorAll(".anim-step");
    const row = rows[rows.length - 1];
    row.querySelector('[data-move="-1"]').click();
    return true;
  })()`);
  await sleep(600);
  steps = await readSteps();
  assert.equal(steps.length, added.length, "reordering keeps the count");
  assert.equal(
    steps[steps.length - 2].kind,
    "pulse",
    "the moved step is no longer last"
  );

  // Remove the last step.
  await page.evaluate(`(() => {
    const rows = document.querySelectorAll(".anim-step");
    rows[rows.length - 1].querySelector("[data-remove]").click();
    return true;
  })()`);
  await sleep(600);
  steps = await readSteps();
  assert.equal(steps.length, added.length - 1, "remove drops one step");
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
      inRail: Boolean(el.closest(".rail")),
      focusVisible: el.matches(":focus-visible"),
      outlineWidth: ring.outlineWidth,
      outlineStyle: ring.outlineStyle
    };
  })()`);
  // Tab leaves the top bar and lands inside the storyboard column: the grid/list
  // toggle in its header comes first, then the selected thumb.
  assert.ok(focus.inRail, `Tab lands inside the storyboard: ${JSON.stringify(focus)}`);
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

    // Only what is on screen: a hidden dialog's controls have a zero box and are
    // not targets anyone can hit.
    const iconTargets = [...document.querySelectorAll(".icon-btn")]
      .filter((button) => button.getClientRects().length > 0)
      .map((button) => {
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
