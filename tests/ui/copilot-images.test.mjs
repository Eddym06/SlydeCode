// Background pictures: the chat reports each one, then keeps updating it after
// the text turn has finished. Failures stay visible instead of disappearing with
// the stream.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";

let studio;
let page;
let retryImageRequests = 0;

const config = {
  providers: [
    { id: "groq", name: "Groq", company: "Groq", icon: "groq", key_url: "https://example.com", configured: true },
  ],
  last_provider: "groq",
  last_model: "llama-3.1-8b-instant",
};

const events = [
  { type: "meta", provider: "groq", model: "llama-3.1-8b-instant" },
  {
    type: "images",
    jobs: [{ id: "img_cover", status: "generating", slide_index: 0 }],
    errors: [],
  },
  {
    type: "images",
    jobs: [
      {
        id: "img_cover",
        status: "failed",
        prompt: "editorial photo of a red apple",
        alt: "A red apple",
        slide_index: 0,
        error_code: "invalid_key",
        error: "the image model rejected this key",
      },
    ],
    errors: [
      {
        code: "invalid_key",
        message: "the image model rejected this key",
      },
    ],
  },
  { type: "done" },
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
      else if (request.url.includes("/api/copilot/image-retry")) {
        retryImageRequests += 1;
        const input = JSON.parse(request.postData || "{}");
        await fulfill(requestId, {
          ok: true,
          job: {
            id: input.image_id,
            status: "queued",
            prompt: input.prompt,
            alt: input.alt,
            slide_index: input.slide_index,
          },
        });
      } else if (request.url.includes("/api/copilot/chat")) await fulfill(requestId, sse, "text/event-stream");
      else await page.send("Fetch.continueRequest", { requestId });
    } catch {
      /* page gone */
    }
  });
  await page.send("Fetch.enable", {
    patterns: ["config", "image-retry", "chat"].map((name) => ({
      urlPattern: `*api/copilot/${name}*`,
      requestStage: "Request",
    })),
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

test("a failed picture stays in the thread with its reason", async () => {
  await page.evaluate(`(() => {
    const input = document.getElementById("copilot-input");
    input.value = "Build a deck with a cover";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.getElementById("copilot-form").requestSubmit();
    return true;
  })()`);
  await page.waitFor(`Boolean(document.querySelector('.image-row[data-status="failed"]'))`);
  const row = await page.evaluate(`document.querySelector(".image-row[data-status='failed']").textContent.replace(/\\s+/g, " ").trim()`);
  assert.match(row, /failed/i);
  assert.match(row, /rejected the API key/i);
  const notice = await page.evaluate(`document.querySelector(".image-errors").textContent.replace(/\\s+/g, " ").trim()`);
  assert.match(notice, /could not be generated/i);
  assert.match(notice, /rejected the API key/i);
  const detail = await page.evaluate(`document.querySelector(".image-errors li").getAttribute("title")`);
  assert.match(detail, /rejected this key/);
});

test("a failed picture offers a retry that sends the same prompt", async () => {
  assert.equal(
    await page.evaluate(`Boolean(document.querySelector("[data-retry-image]"))`),
    true,
    "the failed row has a retry button"
  );
  await page.evaluate(`document.querySelector("[data-retry-image]").click(); true`);
  await page.waitFor(`Boolean(document.querySelector('.image-row[data-status="queued"]'))`);
  assert.equal(retryImageRequests, 1, "the retry reaches the server");
  assert.equal(
    await page.evaluate(`document.querySelector(".image-errors").hidden`),
    true,
    "the stale failure clears when the retry starts"
  );
});

test("a late WebSocket update moves the picture to placed", async () => {
  const turnId = await page.evaluate(`document.querySelector(".msg-assistant").dataset.turnId`);
  assert.ok(turnId, "the assistant turn carries the id the server reports");
  await page.evaluate(`(() => {
    document.dispatchEvent(new CustomEvent("slyde:image", { detail: {
      type: "image_progress",
      turn_id: ${JSON.stringify(turnId)},
      job: { id: "img_cover", status: "placed", slide_index: 0, url: "/assets/abc123.png" }
    }}));
    return true;
  })()`);
  await page.waitFor(`Boolean(document.querySelector('.image-row[data-status="placed"]'))`);
  const thumb = await page.evaluate(`document.querySelector(".image-row .image-thumb").getAttribute("src")`);
  assert.equal(thumb, "/assets/abc123.png");
});
