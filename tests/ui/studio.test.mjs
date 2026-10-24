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

import { launchChrome, sleep } from "../../scripts/lib/cdp.mjs";

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
      sandbox: document.getElementById("present-slide").getAttribute("sandbox")
    })`);
    assert.equal(start.current, "1");
    assert.equal(start.total, "2");
    assert.equal(start.progress, "50");
    assert.equal(start.sandbox, "allow-scripts");

    await present.evaluate(`document.body.focus(); true`);
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
  assert.equal(focus.isThumb, true, "Tab lands on the selected storyboard thumb");
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
