// Dev helper: evaluate an expression in a page through the Chrome DevTools
// Protocol and print the result as JSON.
//
// Usage:
//   node scripts/inspect.mjs <url> "<js expression>"
//
// Environment:
//   SLYDE_VIEWPORT=1440x900      Emulate a viewport before evaluating.
//   SLYDE_WAIT=2500              Milliseconds to wait before evaluating.
//   SLYDE_SCREENSHOT=shot.png    Capture the page after the wait, then evaluate.
//
// Example:
//   node scripts/inspect.mjs "http://127.0.0.1:1234/" "JSON.stringify(document.body.getBoundingClientRect())"

import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, existsSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url, expression] = process.argv.slice(2);
if (!url || !expression) {
  console.error("usage: node scripts/inspect.mjs <url> \"<expression>\"");
  process.exit(2);
}

const chrome =
  process.env.SLYDE_BROWSER ||
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";

const profile = mkdtempSync(join(tmpdir(), "slyde-inspect-"));
const child = spawn(
  chrome,
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { stdio: "ignore" }
);

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForPort() {
  const file = join(profile, "DevToolsActivePort");
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (existsSync(file)) {
      const [port] = readFileSync(file, "utf8").split("\n");
      if (port) return Number(port);
    }
    await sleep(100);
  }
  throw new Error("Chrome did not expose a debugging port");
}

function cdp(wsUrl) {
  const socket = new WebSocket(wsUrl);
  let nextId = 1;
  const pending = new Map();

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    }
  });

  const ready = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  return {
    ready,
    send(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        socket.send(JSON.stringify({ id, method, params }));
      });
    },
    close() {
      socket.close();
    },
  };
}

try {
  const port = await waitForPort();
  const created = await fetch(
    `http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`,
    { method: "PUT" }
  ).then((response) => response.json());

  const client = cdp(created.webSocketDebuggerUrl);
  await client.ready;
  await client.send("Runtime.enable");
  await client.send("Page.enable");

  const viewport = (process.env.SLYDE_VIEWPORT || "").split("x").map(Number);
  if (viewport.length === 2 && viewport[0] > 0) {
    await client.send("Emulation.setDeviceMetricsOverride", {
      width: viewport[0],
      height: viewport[1],
      deviceScaleFactor: 1,
      mobile: false,
    });
  }

  await sleep(Number(process.env.SLYDE_WAIT) || 1800);

  const shot = process.env.SLYDE_SCREENSHOT;
  if (shot) {
    const capture = await client.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(shot, Buffer.from(capture.data, "base64"));
  }

  const result = await client.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });

  if (result.exceptionDetails) {
    console.error(JSON.stringify(result.exceptionDetails, null, 2));
    process.exitCode = 1;
  } else {
    console.log(JSON.stringify(result.result.value, null, 2));
  }

  client.close();
} finally {
  child.kill();
  try {
    rmSync(profile, { recursive: true, force: true });
  } catch {
    /* Chrome may still hold a lock; the OS temp cleaner will get it. */
  }
}
