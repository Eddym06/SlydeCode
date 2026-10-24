// Minimal Chrome DevTools Protocol client and headless launcher.
//
// No npm dependencies: Node's built-in WebSocket and fetch are enough. Shared
// by `scripts/inspect.mjs` (manual visual checks) and `tests/ui/*.test.mjs`
// (the end-to-end suite).

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const WINDOWS_CANDIDATES = [
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
];

const UNIX_CANDIDATES = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
];

export function resolveChrome() {
  if (process.env.SLYDE_BROWSER) return process.env.SLYDE_BROWSER;

  const candidates = [...WINDOWS_CANDIDATES, ...UNIX_CANDIDATES];
  if (process.env.LOCALAPPDATA) {
    candidates.unshift(
      join(process.env.LOCALAPPDATA, "Google", "Chrome", "Application", "chrome.exe")
    );
  }
  const found = candidates.find((path) => existsSync(path));
  if (!found) {
    throw new Error(
      "Chrome/Chromium not found. Set SLYDE_BROWSER to the executable path."
    );
  }
  return found;
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function connectCdp(wsUrl) {
  const socket = new WebSocket(wsUrl);
  let nextId = 1;
  const pending = new Map();
  const handlers = new Map();

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
      return;
    }
    if (message.method) {
      for (const handler of handlers.get(message.method) || []) {
        handler(message.params);
      }
    }
  });

  const ready = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  function on(method, handler) {
    if (!handlers.has(method)) handlers.set(method, new Set());
    handlers.get(method).add(handler);
    return () => handlers.get(method)?.delete(handler);
  }

  return {
    ready,
    on,
    send(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        socket.send(JSON.stringify({ id, method, params }));
      });
    },
    async evaluate(expression) {
      const result = await this.send("Runtime.evaluate", {
        expression,
        returnByValue: true,
        awaitPromise: true,
      });
      if (result.exceptionDetails) {
        const detail =
          result.exceptionDetails.exception?.description ||
          result.exceptionDetails.text ||
          "evaluation failed";
        throw new Error(detail);
      }
      return result.result.value;
    },
    close() {
      socket.close();
    },
  };
}

async function waitForPort(profile, child) {
  const file = join(profile, "DevToolsActivePort");
  for (let attempt = 0; attempt < 120; attempt += 1) {
    if (child.exitCode !== null) {
      throw new Error(`Chrome exited before exposing a debugging port (${child.exitCode})`);
    }
    if (existsSync(file)) {
      const [port] = readFileSync(file, "utf8").split("\n");
      if (port) return Number(port);
    }
    await sleep(100);
  }
  throw new Error("Chrome did not expose a debugging port");
}

/**
 * Launches headless Chrome with an ephemeral profile and port.
 *
 * `newPage(url)` returns a page with `send`, `evaluate`, `waitFor`, `press`
 * and `close`. `close()` terminates Chrome and removes the profile.
 */
export async function launchChrome({
  chrome = resolveChrome(),
  viewport,
  headless = true,
  args = [],
} = {}) {
  const profile = mkdtempSync(join(tmpdir(), "slyde-cdp-"));
  const child = spawn(
    chrome,
    [
      ...(headless ? ["--headless=new", "--disable-gpu"] : []),
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-extensions",
      "--mute-audio",
      ...(viewport
        ? [`--window-size=${viewport.width},${viewport.height + 80}`]
        : []),
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      ...args,
      "about:blank",
    ],
    { stdio: "ignore" }
  );

  const port = await waitForPort(profile, child);
  const origin = `http://127.0.0.1:${port}`;

  return {
    origin,
    async newPage(url) {
      const target = await fetch(
        `${origin}/json/new?${encodeURIComponent(url)}`,
        { method: "PUT" }
      ).then((response) => response.json());
      const cdp = connectCdp(target.webSocketDebuggerUrl);
      await cdp.ready;
      await cdp.send("Runtime.enable");
      await cdp.send("Page.enable");

      if (viewport && viewport.width > 0) {
        await cdp.send("Emulation.setDeviceMetricsOverride", {
          width: viewport.width,
          height: viewport.height,
          deviceScaleFactor: viewport.deviceScaleFactor || 1,
          mobile: false,
        });
      }

      return {
        client: cdp,
        send: (method, params) => cdp.send(method, params),
        evaluate: (expression) => cdp.evaluate(expression),
        async waitFor(expression, { timeout = 8000, interval = 100 } = {}) {
          const started = Date.now();
          let lastError = null;
          while (Date.now() - started < timeout) {
            try {
              if (await cdp.evaluate(expression)) return true;
            } catch (error) {
              lastError = error;
            }
            await sleep(interval);
          }
          throw new Error(
            `Timed out waiting for: ${expression}` +
              (lastError ? ` (last error: ${lastError.message})` : "")
          );
        },
        async press(key, { keyCode } = {}) {
          const code = key.length === 1 ? `Key${key.toUpperCase()}` : key;
          const virtualKeyCode = keyCode ?? (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
          await cdp.send("Input.dispatchKeyEvent", {
            type: "keyDown",
            key,
            code,
            windowsVirtualKeyCode: virtualKeyCode,
            nativeVirtualKeyCode: virtualKeyCode,
          });
          await cdp.send("Input.dispatchKeyEvent", {
            type: "keyUp",
            key,
            code,
            windowsVirtualKeyCode: virtualKeyCode,
            nativeVirtualKeyCode: virtualKeyCode,
          });
        },
        async close() {
          cdp.close();
          await fetch(`${origin}/json/close/${target.id}`).catch(() => {});
        },
      };
    },
    close() {
      try {
        child.kill();
      } catch {
        /* already gone */
      }
      try {
        rmSync(profile, { recursive: true, force: true });
      } catch {
        /* Chrome may still hold a lock; the OS temp cleaner will get it. */
      }
    },
  };
}
