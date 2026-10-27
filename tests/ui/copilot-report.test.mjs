// A refused request: the thread says it in one sentence, offers Retry and Report,
// and the report shows what the provider actually said, ready to copy.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";
import { sleep } from "../../scripts/lib/cdp.mjs";

let studio;
let page;
let chatRequests = 0;

const config = {
  providers: [
    { id: "groq", name: "Groq", company: "Groq", icon: "groq", key_url: "https://example.com", configured: true },
  ],
  last_provider: "groq",
  last_model: "llama-3.1-8b-instant",
};
const reply = JSON.stringify({
  error: {
    message: "Request too large for model on tokens per minute (TPM): Limit 6000, Requested 9000",
    type: "tokens",
    code: "rate_limit_exceeded",
  },
});
const events = [
  { type: "meta", provider: "groq", model: "llama-3.1-8b-instant" },
  { type: "error", code: "too_large", message: "the request is larger than the model or plan allows", status: 413, detail: reply },
];
const sse = `${events.map((event) => `data: ${JSON.stringify(event)}`).join("\n\n")}\n\n`;

before(async () => {
  studio = await startStudio();
  page = studio.page;
  const fulfill = (requestId, payload, type = "application/json") =>
    page.send("Fetch.fulfillRequest", {
      requestId,
      responseCode: 200,
      responseHeaders: [{ name: "content-type", value: type }],
      body: Buffer.from(typeof payload === "string" ? payload : JSON.stringify(payload)).toString("base64"),
    });
  page.client.on("Fetch.requestPaused", async ({ requestId, request }) => {
    try {
      if (request.url.includes("/api/copilot/config")) await fulfill(requestId, { ok: true, config });
      else if (request.url.includes("/api/copilot/chat")) {
        chatRequests += 1;
        await fulfill(requestId, sse, "text/event-stream");
      } else await page.send("Fetch.continueRequest", { requestId });
    } catch {
      /* page gone */
    }
  });
  await page.send("Fetch.enable", {
    patterns: ["config", "chat"].map((name) => ({ urlPattern: `*api/copilot/${name}*`, requestStage: "Request" })),
  });
  const origin = await page.evaluate("performance.timeOrigin");
  await page.evaluate(
    `localStorage.setItem("slyde.copilot.selection", JSON.stringify({provider:"groq",model:"llama-3.1-8b-instant"})); true`
  );
  await page.send("Page.reload", { ignoreCache: true });
  await page.waitFor(
    `performance.timeOrigin !== ${JSON.stringify(origin)} && document.getElementById("app") && !document.getElementById("app").hidden`,
    { timeout: 15000 }
  );
  await page.evaluate(`document.getElementById("tab-copilot").click(); true`);
  await page.waitFor(`document.getElementById("copilot-provider-name").textContent === "Groq"`);
});

after(async () => {
  await studio?.stop();
});

test("a refusal offers Retry and Report side by side", async () => {
  await page.evaluate(`(() => {
    const input = document.getElementById("copilot-input");
    input.value = "Build a deck";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.getElementById("copilot-form").requestSubmit();
    return true;
  })()`);
  await page.waitFor(`Boolean(document.querySelector(".msg-error .msg-report"))`);
  const actions = await page.evaluate(
    `[...document.querySelectorAll(".msg-error-actions button")].map((button) => button.className)`
  );
  assert.deepEqual(actions, ["msg-retry", "msg-report"]);
  const sentence = await page.evaluate(`document.querySelector(".msg-error p").textContent`);
  assert.match(sentence, /bigger than this model or plan/i);
  assert.doesNotMatch(sentence, /6000|rate_limit/, "the provider's own words stay out of the thread");
});

test("the report shows the provider's reply, formatted, with a copy button in its corner", async () => {
  await page.evaluate(`document.querySelector(".msg-report").click(); true`);
  await page.waitFor(`Boolean(document.querySelector(".report"))`);

  const meta = await page.evaluate(
    `[...document.querySelectorAll(".report-meta > div")].map((cell) => cell.textContent.replace(/\\s+/g, " ").trim())`
  );
  assert.ok(meta.some((cell) => cell.includes("Groq")), `provider is shown: ${meta}`);
  assert.ok(meta.some((cell) => cell.includes("llama-3.1-8b-instant")), `model is shown: ${meta}`);
  assert.ok(meta.some((cell) => cell.includes("HTTP 413")), `status is shown: ${meta}`);

  const code = await page.evaluate(`document.querySelector(".report-code pre").textContent`);
  assert.match(code, /Limit 6000/, "the provider's numbers are visible");
  assert.match(code, /\n\s+"error": \{/, "a JSON reply is pretty-printed");
  assert.ok(
    await page.evaluate(`document.querySelectorAll(".report-code .rp-key").length > 0`),
    "keys are highlighted"
  );

  // The copy button sits in the corner of the code block.
  const placement = await page.evaluate(`(() => {
    const block = document.querySelector(".report-code").getBoundingClientRect();
    const button = document.querySelector("[data-copy]").getBoundingClientRect();
    return { right: block.right - button.right, top: button.top - block.top };
  })()`);
  assert.ok(placement.right < 16 && placement.top < 16, `copy sits in the corner: ${JSON.stringify(placement)}`);

  assert.equal(
    await page.evaluate(`document.activeElement.className`),
    "report-close",
    "focus moves into the dialog"
  );
  await page.evaluate(`document.querySelector("[data-copy]").click(); true`);
  await page.waitFor(`Boolean(document.querySelector("[data-copy]").dataset.state)`);
  const state = await page.evaluate(`document.querySelector("[data-copy]").dataset.state`);
  assert.ok(["done", "failed"].includes(state), "the button always says what happened");

  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`!document.querySelector(".report")`);
  assert.equal(
    await page.evaluate(`document.activeElement.className.includes("msg-report")`),
    true,
    "closing the report hands focus back to its button"
  );
});

test("Retry sends the same request again", async () => {
  const before = chatRequests;
  await page.evaluate(`document.querySelector(".msg-retry").click(); true`);
  await page.waitFor(`Boolean(document.querySelector(".msg-error .msg-report"))`);
  assert.equal(chatRequests, before + 1);
  await sleep(50);
});
