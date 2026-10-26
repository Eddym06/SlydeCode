// Starts the real studio binary against an isolated data directory and opens it
// in headless Chrome. Shared by the UI suites that do not need the large
// fixture set `studio.test.mjs` builds for itself.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { launchChrome } from "../../../scripts/lib/cdp.mjs";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/** The newest build of the binary, so a stale one is never tested by accident. */
export function findBinary() {
  if (process.env.SLYDE_BIN) return process.env.SLYDE_BIN;
  const name = process.platform === "win32" ? "slyde.exe" : "slyde";
  const profiles = [join("x86_64-pc-windows-gnu", "debug"), "debug", "release"];
  const found = profiles
    .map((profile) => join(repoRoot, "target", profile, name))
    .filter((path) => existsSync(path))
    .map((path) => ({ path, mtime: statSync(path).mtimeMs }))
    .sort((left, right) => right.mtime - left.mtime);
  return found[0]?.path;
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

export async function startStudio({ viewport = { width: 1280, height: 800 }, welcomed = true } = {}) {
  const binary = findBinary();
  if (!binary) throw new Error("slyde binary not found. Run `cargo build` or set SLYDE_BIN.");
  const dataDir = mkdtempSync(join(tmpdir(), "slyde-ui-panels-"));
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
  const page = await browser.newPage(appUrl, { welcomed });
  await page.waitFor("document.querySelectorAll('.thumb').length >= 1");

  /** POSTs a studio command and waits for the UI to see the new state. */
  async function command(action, payload = {}) {
    return page.evaluate(`fetch("/api/command?t=${token}", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(${JSON.stringify({ action, ...payload })})
    }).then((response) => response.json())`);
  }

  const state = () => page.evaluate(`fetch("/api/state?t=${token}").then((r) => r.json())`);

  return {
    page,
    token,
    appUrl,
    command,
    state,
    setViewport: (width, height = 800) =>
      page.send("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: 1,
        mobile: false,
      }),
    async stop() {
      await page.close();
      browser.close();
      server.kill();
      rmSync(dataDir, { recursive: true, force: true });
    },
  };
}
