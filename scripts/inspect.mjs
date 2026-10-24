// Dev helper: evaluate an expression in a page through the Chrome DevTools
// Protocol and print the result as JSON.
//
// Usage:
//   node scripts/inspect.mjs <url> "<js expression>"
//
// Environment:
//   SLYDE_BROWSER=<path>         Chrome/Chromium executable.
//   SLYDE_VIEWPORT=1440x900      Emulate a viewport before evaluating.
//   SLYDE_WAIT=2500              Milliseconds to wait before evaluating.
//   SLYDE_BEFORE="<js>"          Run before the wait and screenshot (set up state).
//   SLYDE_SCREENSHOT=shot.png    Capture the page after the wait, then evaluate.
//
// Example:
//   node scripts/inspect.mjs "http://127.0.0.1:1234/" "JSON.stringify(document.body.getBoundingClientRect())"

import { writeFileSync } from "node:fs";
import { launchChrome, sleep } from "./lib/cdp.mjs";

const [url, expression] = process.argv.slice(2);
if (!url || !expression) {
  console.error('usage: node scripts/inspect.mjs <url> "<expression>"');
  process.exit(2);
}

const raw = (process.env.SLYDE_VIEWPORT || "").split("x").map(Number);
const viewport =
  raw.length === 2 && raw[0] > 0 ? { width: raw[0], height: raw[1] } : undefined;

const browser = await launchChrome({ viewport });
try {
  const page = await browser.newPage(url);
  const before = process.env.SLYDE_BEFORE;
  if (before) await page.evaluate(before);
  await sleep(Number(process.env.SLYDE_WAIT) || 1800);

  const shot = process.env.SLYDE_SCREENSHOT;
  if (shot) {
    const capture = await page.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(shot, Buffer.from(capture.data, "base64"));
  }

  console.log(JSON.stringify(await page.evaluate(expression), null, 2));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  browser.close();
}
