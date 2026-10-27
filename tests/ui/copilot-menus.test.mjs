// The copilot composer: the attach menu (documents, images, web link), pasting
// files, the model menu, the actions on a sent message, and the usage footer.
// The provider is mocked with CDP `Fetch.fulfillRequest`, so no key or network is needed.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { startStudio } from "./support/harness.mjs";
import { sleep } from "../../scripts/lib/cdp.mjs";

let studio;
let page;
let chatRequests = 0;

const providers = [
  ["openai", "ChatGPT", "OpenAI"],
  ["anthropic", "Claude", "Anthropic"],
  ["deepseek", "DeepSeek", "DeepSeek"],
].map(([id, name, company]) => ({
  id,
  name,
  company,
  icon: id,
  key_url: "https://example.com",
  configured: id === "openai",
}));
const config = { providers, last_provider: "openai", last_model: "gpt-4.1-mini" };
const models = Array.from({ length: 30 }, (_, index) => ({
  id: index === 0 ? "gpt-4.1-mini" : `gpt-model-${index}`,
  name: index === 0 ? "gpt-4.1-mini" : `gpt-model-${index}`,
  free: index % 5 === 0,
  context_window: 128000,
}));
const events = [
  { type: "meta", provider: "openai", model: "gpt-4.1-mini" },
  { type: "delta", text: "Done." },
  { type: "usage", calls: 3, input_tokens: 4200, output_tokens: 900, estimated: false },
  { type: "done" },
];
const sse = `${events.map((event) => `data: ${JSON.stringify(event)}`).join("\n\n")}\n\n`;

const body = (payload) => Buffer.from(JSON.stringify(payload)).toString("base64");

before(async () => {
  studio = await startStudio();
  page = studio.page;

  const fulfill = (requestId, payload, { type = "application/json", code = 200 } = {}) =>
    page.send("Fetch.fulfillRequest", {
      requestId,
      responseCode: code,
      responseHeaders: [{ name: "content-type", value: type }],
      body: typeof payload === "string" ? Buffer.from(payload).toString("base64") : body(payload),
    });

  page.client.on("Fetch.requestPaused", async ({ requestId, request }) => {
    try {
      if (request.url.includes("/api/copilot/config")) {
        await fulfill(requestId, { ok: true, config });
      } else if (request.url.includes("/api/copilot/models")) {
        await fulfill(requestId, { ok: true, models });
      } else if (request.url.includes("/api/copilot/fetch-url")) {
        const { url } = JSON.parse(request.postData || "{}");
        if (url.includes("127.0.0.1")) {
          await fulfill(
            requestId,
            { ok: false, code: "link_blocked", error: "only public pages" },
            { code: 422 }
          );
        } else {
          await fulfill(requestId, {
            ok: true,
            result: { kind: "page", title: "Example article", text: "Hello from the page.", url },
          });
        }
      } else if (request.url.includes("/api/copilot/chat")) {
        chatRequests += 1;
        await fulfill(requestId, sse, { type: "text/event-stream" });
      } else {
        await page.send("Fetch.continueRequest", { requestId });
      }
    } catch {
      /* the page may have been torn down */
    }
  });
  await page.send("Fetch.enable", {
    patterns: ["config", "models", "fetch-url", "chat"].map((name) => ({
      urlPattern: `*api/copilot/${name}*`,
      requestStage: "Request",
    })),
  });

  const origin = await page.evaluate("performance.timeOrigin");
  await page.evaluate(
    `localStorage.setItem("slyde.copilot.selection", JSON.stringify({provider:"openai",model:"gpt-4.1-mini"})); true`
  );
  await page.send("Page.reload", { ignoreCache: true });
  await page.waitFor(
    `performance.timeOrigin !== ${JSON.stringify(origin)} && document.getElementById("app") && !document.getElementById("app").hidden`,
    { timeout: 15000 }
  );
  await page.evaluate(`document.getElementById("tab-copilot").click(); true`);
  await page.waitFor(`document.getElementById("copilot-provider-name").textContent === "ChatGPT"`);
});

after(async () => {
  await studio?.stop();
});

const menuOpen = () => page.evaluate(`Boolean(document.querySelector(".cmenu") && !document.querySelector(".cmenu").hidden)`);

test("the attach button opens a menu with documents, images and a web link", async () => {
  await page.evaluate(`document.getElementById("copilot-attach").click(); true`);
  await page.waitFor(`!document.querySelector(".cmenu").hidden`);
  const items = await page.evaluate(
    `[...document.querySelectorAll(".cmenu .cmenu-item")].map((item) => item.dataset.act)`
  );
  assert.deepEqual(items, ["documents", "images", "link"]);
  assert.equal(
    await page.evaluate(`document.getElementById("copilot-attach").getAttribute("aria-expanded")`),
    "true"
  );

  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.querySelector(".cmenu").hidden`);
  assert.equal(
    await page.evaluate(`document.activeElement.id`),
    "copilot-attach",
    "closing the menu hands focus back to its button"
  );
});

test("a web link becomes an attachment, and a refused link explains why", async () => {
  await page.evaluate(`document.getElementById("copilot-attach").click(); true`);
  await page.waitFor(`!document.querySelector(".cmenu").hidden`);
  await page.evaluate(`document.querySelector('.cmenu [data-act="link"]').click(); true`);
  await page.waitFor(`Boolean(document.querySelector(".cmenu .cmenu-input"))`);

  // A link the server refuses: the reason shows up next to the field.
  await page.evaluate(`(() => {
    const input = document.querySelector(".cmenu .cmenu-input");
    input.value = "http://127.0.0.1/";
    document.querySelector(".cmenu-add").click();
    return true;
  })()`);
  await page.waitFor(`!document.querySelector(".cmenu-error").hidden`);
  assert.match(
    await page.evaluate(`document.querySelector(".cmenu-error").textContent`),
    /public/i,
    "the refusal is explained in the user's language"
  );

  await page.evaluate(`(() => {
    const input = document.querySelector(".cmenu .cmenu-input");
    input.value = "example.com/article";
    return true;
  })()`);
  // Enter in the field adds the link; it must not send the chat message.
  await page.press("Enter", { keyCode: 13 });
  await page.waitFor(`document.querySelectorAll("#copilot-attachments .pending-file").length === 1`);
  const chip = await page.evaluate(`document.querySelector("#copilot-attachments .pending-name").textContent`);
  assert.equal(chip, "Example article");
  assert.equal(await menuOpen(), false, "the menu closes once the link is added");

  await page.evaluate(`document.querySelector("#copilot-attachments .pending-remove").click(); true`);
  await page.waitFor(`document.querySelectorAll("#copilot-attachments .pending-file").length === 0`);
});

test("pasting an image into the composer attaches it", async () => {
  await page.evaluate(`(() => {
    const bytes = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=="), (c) => c.charCodeAt(0));
    const data = new DataTransfer();
    data.items.add(new File([bytes], "screenshot.png", { type: "image/png" }));
    const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
    document.getElementById("copilot-input").dispatchEvent(event);
    return true;
  })()`);
  await page.waitFor(`document.querySelectorAll("#copilot-attachments .pending-file.is-image").length === 1`);
  assert.equal(
    await page.evaluate(`document.getElementById("copilot-send").disabled`),
    false,
    "an attached picture is enough to send"
  );
  await page.evaluate(`document.querySelector("#copilot-attachments .pending-remove").click(); true`);
  await page.waitFor(`document.querySelectorAll("#copilot-attachments .pending-file").length === 0`);
});

test("the model menu searches, scrolls and picks, with a way to configure", async () => {
  await page.evaluate(`document.getElementById("copilot-model-chip").click(); true`);
  await page.waitFor(`document.querySelectorAll(".cmenu .cmenu-model").length > 10`);
  const scrolls = await page.evaluate(`(() => {
    const list = document.querySelector(".cmenu-models");
    return list.scrollHeight > list.clientHeight;
  })()`);
  assert.equal(scrolls, true, "a long list scrolls inside the menu");

  await page.evaluate(`(() => {
    const input = document.querySelector(".cmenu-search-input");
    input.value = "model-2";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  })()`);
  await page.waitFor(
    `document.querySelectorAll(".cmenu .cmenu-model").length > 0 &&
     [...document.querySelectorAll(".cmenu .cmenu-model")].every((row) => row.dataset.model.includes("model-2"))`
  );

  await page.evaluate(`document.querySelector('.cmenu-model[data-model="gpt-model-2"]').click(); true`);
  await page.waitFor(`document.getElementById("copilot-model-chip-text").textContent === "gpt-model-2"`);
  assert.equal(
    await page.evaluate(`JSON.parse(localStorage.getItem("slyde.copilot.selection")).model`),
    "gpt-model-2",
    "the choice is remembered"
  );

  await page.evaluate(`document.getElementById("copilot-model-chip").click(); true`);
  await page.waitFor(`Boolean(document.querySelector(".cmenu [data-configure]"))`);
  await page.evaluate(`document.querySelector(".cmenu [data-configure]").click(); true`);
  await page.waitFor(`!document.getElementById("copilot-sheet").hidden`);
  await page.press("Escape", { keyCode: 27 });
  await page.waitFor(`document.getElementById("copilot-sheet").hidden`);
});

test("a sent message offers run again and edit, and the reply shows its usage", async () => {
  await page.evaluate(`(() => {
    const input = document.getElementById("copilot-input");
    input.value = "Make a cover";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.getElementById("copilot-form").requestSubmit();
    return true;
  })()`);
  await page.waitFor(`document.querySelectorAll(".msg-assistant .msg-text").length === 1 &&
    document.querySelector(".msg-assistant").dataset.state === "done"`);

  const usage = await page.evaluate(`[...document.querySelectorAll(".msg-usage .usage-item")].map((item) => item.getAttribute("aria-label"))`);
  assert.equal(usage.length, 3, `calls, input and output are listed: ${usage}`);
  assert.match(usage[0], /3/);
  assert.equal(
    await page.evaluate(`Boolean(document.querySelector(".msg-assistant .msg-meta .msg-usage, .msg-assistant .msg-meta .usage-item"))`),
    false,
    "usage no longer sits beside the name"
  );

  // Run again: the same prompt is sent once more and the thread keeps one pair.
  const before = chatRequests;
  await page.evaluate(`document.querySelector(".msg-user [data-rerun]").click(); true`);
  await page.waitFor(`document.querySelector(".msg-assistant")?.dataset.state === "done"`);
  assert.equal(chatRequests, before + 1, "run again asks the provider once more");
  assert.equal(await page.evaluate(`document.querySelectorAll(".msg-user").length`), 1);
  assert.equal(await page.evaluate(`document.querySelectorAll(".msg-assistant").length`), 1);

  // Edit: the prompt returns to the composer and the exchange is forgotten.
  await page.evaluate(`document.querySelector(".msg-user [data-edit]").click(); true`);
  await page.waitFor(`document.getElementById("copilot-input").value === "Make a cover"`);
  assert.equal(await page.evaluate(`document.querySelectorAll(".msg").length`), 0);
  await sleep(100);
});
