// Shared end-to-end harness: the real `slyde` binary against an isolated data
// directory, driven through headless Chrome over CDP. No npm dependencies.
//
// Not a test file (no `.test.mjs` suffix), so `node --test tests/ui/*.test.mjs`
// never runs it directly.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { connectCdp, launchChrome, sleep } from "../../scripts/lib/cdp.mjs";

export { sleep };

export const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** Newest `slyde` binary across the known target profiles. */
export function findBinary() {
  if (process.env.SLYDE_BIN) return process.env.SLYDE_BIN;
  const name = process.platform === "win32" ? "slyde.exe" : "slyde";
  const profiles = [
    join("x86_64-pc-windows-gnu", "debug"),
    "debug",
    "release",
    join("x86_64-pc-windows-gnu", "release"),
  ];
  const candidates = profiles
    .map((profile) => join(repoRoot, "target", profile, name))
    .filter((path) => existsSync(path))
    .map((path) => ({ path, mtime: statSync(path).mtimeMs }))
    .sort((left, right) => right.mtime - left.mtime);
  return candidates[0]?.path;
}

function waitForAppUrl(child) {
  return new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(
      () => reject(new Error(`slyde did not print a URL within 20s:\n${output}`)),
      20000
    );
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

/**
 * Boots the studio and returns helpers bound to it.
 *
 * `errors` collects uncaught page exceptions and `console.error` calls so a
 * test can assert that a whole flow ran without a single one.
 */
export async function startStudio({ viewport = { width: 1280, height: 800 } } = {}) {
  const binary = findBinary();
  assert.ok(binary, "slyde binary not found. Run `cargo build` or set SLYDE_BIN.");

  const dataDir = mkdtempSync(join(tmpdir(), "slyde-ui-int-"));
  const server = spawn(binary, ["app", "--no-open", "--port", "0"], {
    cwd: repoRoot,
    env: {
      ...process.env,
      SLYDE_DATA_DIR: dataDir,
      SLYDE_UI_DIR: join(repoRoot, "ui"),
      SLYDE_ASSETS_DIR: join(repoRoot, "assets"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  const appUrl = await waitForAppUrl(server);
  const token = new URL(appUrl).searchParams.get("t");
  const browser = await launchChrome({ viewport });
  const page = await browser.newPage(appUrl);
  await page.waitFor("document.querySelectorAll('.thumb').length >= 1");

  const errors = [];
  page.client.on("Runtime.exceptionThrown", (event) => {
    errors.push(event.exceptionDetails?.exception?.description || event.exceptionDetails?.text);
  });
  page.client.on("Runtime.consoleAPICalled", (event) => {
    console.log("[BROWSER]", event.args.map((a) => a.value ?? a.description).join(" "));
    if (event.type === "error") {
      errors.push(
        (event.args || []).map((arg) => arg.value ?? arg.description ?? "").join(" ")
      );
    }
  });

  const boot = await page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`);

  const api = {
    server,
    browser,
    page,
    token,
    appUrl,
    errors,
    sandboxOrigin: boot.sandbox_origin,
    sandboxToken: boot.sandbox_token,

    state: () => page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`),

    async slides() {
      return (await api.state()).view.presentation.slides;
    },

    command(body) {
      return page.evaluate(`fetch("/api/command?t=${token}", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: ${JSON.stringify(JSON.stringify(body))}
      }).then(async (r) => ({ ok: r.ok, status: r.status, body: await r.text() }))`);
    },

    async stageTarget(slideId) {
      const needle = new RegExp(`/s/${slideId}[^"]*edit=1`);
      const targets = await fetch(`${browser.origin}/json/list`).then((r) => r.json());
      return targets.find((e) => e.type === "iframe" && needle.test(e.url)) || null;
    },

    /** Resolves once the stage document installed its editing layer. */
    async waitForStageReady(slideId, timeout = 20000) {
      const started = Date.now();
      while (Date.now() - started < timeout) {
        const target = await api.stageTarget(slideId);
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
            /* mid-navigation: retry */
          } finally {
            cdp.close();
          }
        }
        await sleep(150);
      }
      throw new Error(`the stage never became ready for slide ${slideId}`);
    },

    /** Runs `fn(cdp)` inside the stage document of a slide. */
    async inStage(slideId, fn) {
      const target = await api.stageTarget(slideId);
      assert.ok(target, `no editable stage for ${slideId}`);
      const cdp = connectCdp(target.webSocketDebuggerUrl);
      try {
        await cdp.ready;
        await cdp.send("Runtime.enable");
        return await fn(cdp);
      } finally {
        cdp.close();
      }
    },

    /** Screen rectangle of the stage frame and its stage-pixel scale. */
    frameMetrics() {
      return page.evaluate(`(() => {
        const rect = document.getElementById("slide-frame").getBoundingClientRect();
        return { x: rect.x, y: rect.y, w: rect.width, h: rect.height, scale: rect.width / 1280 };
      })()`);
    },

    /** Selects the Nth storyboard slide and waits until its stage is live. */
    async openSlide(index = 0) {
      const slides = await api.slides();
      const slideId = slides[index].id;
      await page.evaluate(`document.querySelectorAll(".thumb")[${index}].click(); true`);
      await page.waitFor(
        `document.querySelector('.thumb[aria-current="true"]')?.dataset.id === ${JSON.stringify(slideId)}`
      );
      await api.waitForStageReady(slideId);
      return slideId;
    },

    mouse(type, x, y, extra = {}) {
      const isMoved = type === "mouseMoved";
      return page.send("Input.dispatchMouseEvent", {
        type,
        x,
        y,
        button: isMoved ? "none" : (extra.button || "left"),
        buttons: type === "mouseReleased" ? 0 : (extra.buttons ?? (isMoved ? 0 : 1)),
        clickCount: isMoved ? 0 : 1,
        ...extra,
      });
    },

    async click(x, y, extra = {}) {
      await api.mouse("mouseMoved", x, y, extra);
      await api.mouse("mousePressed", x, y, { buttons: 1, ...extra });
      await api.mouse("mouseReleased", x, y, extra);
    },

    /** Press, move through `steps` intermediate points, release. */
    async drag(from, to, { steps = 6, modifiers = 0 } = {}) {
      await api.mouse("mouseMoved", from.x, from.y, { modifiers });
      await api.mouse("mousePressed", from.x, from.y, { buttons: 1, modifiers });
      for (let step = 1; step <= steps; step += 1) {
        const k = step / steps;
        await api.mouse("mouseMoved", from.x + (to.x - from.x) * k, from.y + (to.y - from.y) * k, {
          buttons: 1,
          modifiers,
        });
      }
      await api.mouse("mouseReleased", to.x, to.y, { modifiers });
    },

    /** Real key press into the focused frame. `modifiers`: 1 alt, 2 ctrl, 4 meta, 8 shift. */
    async key(key, { modifiers = 0, code } = {}) {
      const resolvedCode = code || (key.length === 1 ? `Key${key.toUpperCase()}` : key);
      const virtual = key.length === 1 ? key.toUpperCase().charCodeAt(0) : keyCodes[key] || 0;
      const base = {
        key,
        code: resolvedCode,
        modifiers,
        windowsVirtualKeyCode: virtual,
        nativeVirtualKeyCode: virtual,
      };
      await page.send("Input.dispatchKeyEvent", { type: "keyDown", ...base });
      await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    },

    async stop() {
      try {
        await page.close();
      } catch {
        /* already gone */
      }
      try {
        await browser.close?.();
      } catch {
        /* already gone */
      }
      try {
        server.stdout?.destroy();
        server.stderr?.destroy();
        if (process.platform === "win32" && server.pid) {
          spawn("taskkill", ["/pid", String(server.pid), "/T", "/F"], { stdio: "ignore" }).unref();
        } else {
          server.kill();
        }
      } catch {
        /* already gone */
      }
      await sleep(150);
      try {
        rmSync(dataDir, { recursive: true, force: true });
      } catch {
        /* Windows may still hold a handle for a moment */
      }
    },
  };

  return api;
}

const keyCodes = {
  Delete: 46,
  Backspace: 8,
  Escape: 27,
  Enter: 13,
  F5: 116,
  ArrowLeft: 37,
  ArrowUp: 38,
  ArrowRight: 39,
  ArrowDown: 40,
  " ": 32,
};

export { connectCdp };
