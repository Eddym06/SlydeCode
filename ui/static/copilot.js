// The in-app copilot: visible conversation, attachments and streamed tool
// status. The server owns provider keys and the tool loop; this module never
// sees a key and never renders provider-authored free text.

import { recommendedModel } from "./onboarding-models.js";
import { expectedSlides, formatElapsed, lastThought, recentThinking, trackModel, workPhase } from "./copilot-work.js";
import { lang, onLangChange, t } from "./i18n.js";
import { createModelSheet } from "./copilot-model-sheet.js";
import { providerIcon, providerTint } from "./provider-icons.js";
import { mountComposerMenus } from "./composer-menus.js";
import { mountReport } from "./copilot-report.js";

const SELECTION_KEY = "slyde.copilot.selection";
const MAX_ATTACHMENTS = 4;
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const MAX_DOCUMENT_BYTES = 12 * 1024 * 1024;
const MAX_HISTORY_MESSAGES = 24;
const MAX_HISTORY_ATTACHMENTS = 6;
const MAX_HISTORY_USER_ATTACHMENT_MESSAGES = 2;

const IMAGE_MIMES = new Set(["image/png", "image/jpeg", "image/webp"]);
const IMAGE_BY_EXT = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
};
const TEXT_EXT = new Set([
  "txt",
  "md",
  "csv",
  "json",
  "html",
  "css",
  "js",
  "ts",
  "yaml",
  "yml",
]);

const $ = (id) => document.getElementById(id);

export function mountCopilot({ client, getSelectedSlideId, getSelectedElementId, notify, getDeckId }) {
  const els = {
    panel: $("copilot-panel"),
    thread: $("copilot-thread"),
    announcer: $("copilot-announcer"),
    form: $("copilot-form"),
    input: $("copilot-input"),
    send: $("copilot-send"),
    attach: $("copilot-attach"),
    file: $("copilot-file"),
    attachments: $("copilot-attachments"),
    provider: $("copilot-provider"),
    providerIcon: $("copilot-provider-icon"),
    providerName: $("copilot-provider-name"),
    modelLabel: $("copilot-model-label"),
    modelChip: $("copilot-model-chip"),
    modelChipText: $("copilot-model-chip-text"),
    modelChipIcon: $("copilot-model-chip-icon"),
    newChat: $("copilot-new"),
    live: $("copilot-live"),
    sheet: $("copilot-sheet"),
  };

  let config = null;
  let configError = null;
  let configLoading = true;
  let configRequest = null;
  let providerId = null;
  let modelId = "";
  let messages = [];
  let pending = [];
  let streaming = false;
  let controller = null;
  let currentRequestId = null;
  let epoch = 0;

  const sheet = createModelSheet({
    root: els.sheet,
    client,
    getConfig: () => config,
    getSelection: () => ({ provider: providerId, model: modelId }),
    onConfigChange(next) {
      config = next;
      configError = null;
      syncHeader();
      if (!messages.length) renderThread();
    },
    onProviderFocus(provider) {
      if (providerId === provider) return;
      providerId = provider;
      modelId = "";
      persistSelection();
      syncHeader();
    },
    onSelect(provider, model) {
      providerId = provider;
      modelId = model.id;
      persistSelection();
      syncHeader();
      window.requestAnimationFrame(() => els.input.focus());
    },
    onClose() {
      els.provider.focus();
    },
  });

  // ---------- configuration & header ----------

  function activeProvider() {
    if (!config || !config.providers || !providerId) return null;
    return config.providers.find((provider) => provider.id === providerId) || null;
  }

  function readSelection() {
    try {
      const saved = JSON.parse(localStorage.getItem(SELECTION_KEY) || "null");
      if (saved && typeof saved.provider === "string" && typeof saved.model === "string") {
        return saved;
      }
    } catch {
      // A corrupted preference should not block the copilot.
    }
    return null;
  }

  function persistSelection() {
    try {
      localStorage.setItem(
        SELECTION_KEY,
        JSON.stringify({ provider: providerId, model: modelId })
      );
    } catch {
      // Private-mode storage failures are non-fatal.
    }
  }

  function resolveSelection() {
    if (!config || !config.providers || !config.providers.length) {
      providerId = null;
      modelId = "";
      return;
    }
    const saved = readSelection();
    const known = (id) => Boolean(id && config.providers.some((p) => p.id === id));
    const firstConfigured = config.providers.find((p) => p.configured);
    const nextProvider = known(providerId)
      ? providerId
      : known(saved && saved.provider)
        ? saved.provider
        : known(config.last_provider)
          ? config.last_provider
          : (firstConfigured && firstConfigured.id) || config.providers[0].id;

    providerId = nextProvider;
    const provider = activeProvider();
    if (!provider || !provider.configured) {
      modelId = "";
      return;
    }
    const candidates = [
      modelId,
      saved && saved.provider === providerId ? saved.model : "",
      config.last_provider === providerId ? config.last_model : "",
    ];
    modelId = candidates.find((candidate) => typeof candidate === "string" && candidate) || "";
  }

  function loadConfig() {
    if (configRequest) return configRequest;
    configLoading = true;
    if (!messages.length) renderThread();
    configRequest = (async () => {
      try {
        config = await client.copilotConfig();
        configError = null;
        resolveSelection();
      } catch (error) {
        configError = error;
      } finally {
        configLoading = false;
        configRequest = null;
        syncHeader();
        if (!messages.length) renderThread();
      }
    })();
    return configRequest;
  }

  function syncHeader() {
    const provider = activeProvider();
    const anyConfigured = Boolean(
      config && config.providers && config.providers.some((p) => p.configured)
    );
    const iconId = providerId || "openai";
    els.providerIcon.innerHTML = providerIcon(iconId);
    els.providerIcon.style.color = providerTint(iconId);

    let name;
    let model;
    if (configError) {
      name = t("copilot.connect");
      model = t("copilot.configError");
    } else if (configLoading) {
      name = t("copilot.connect");
      model = t("copilot.loading");
    } else if (!config || !provider || !anyConfigured) {
      name = t("copilot.connect");
      model = t("copilot.chooseModel");
    } else {
      name = provider.name;
      model = modelId || t("copilot.chooseModel");
    }

    els.providerName.textContent = name;
    els.modelLabel.textContent = model;
    els.provider.setAttribute("aria-label", `${name} — ${model}`);
    els.provider.title = `${name} — ${model}`;
    els.modelChipText.textContent = modelId || t("copilot.model");
    if (els.modelChipIcon) {
      els.modelChipIcon.innerHTML = providerIcon(iconId);
      els.modelChipIcon.style.color = providerTint(iconId);
    }
    els.modelChip.title = model;
    els.modelChip.setAttribute("aria-label", model);
    updateSendEnabled();
  }

  function openSheet() {
    if (sheet.isOpen()) return;
    if (configError) {
      loadConfig().then(() => {
        if (config) sheet.open(providerId);
      });
      return;
    }
    if (!config) {
      loadConfig().then(() => {
        if (config) sheet.open(providerId);
      });
      return;
    }
    sheet.open(providerId);
  }

  // ---------- thread rendering ----------

  // ---------- reasoning drawer ----------

  const drawerSlots = new WeakMap();

  // The expanded drawer lives in a layer attached to <body>: inside the thread
  // it is boxed in by the panel width, and a transformed ancestor would break
  // `position: fixed` anyway.
  function ensureThinkingLayer() {
    let layer = document.getElementById("thinking-layer");
    if (!layer) {
      layer = document.createElement("div");
      layer.id = "thinking-layer";
      layer.className = "thinking-layer";
      layer.hidden = true;
      layer.addEventListener("click", (event) => {
        if (event.target === layer) closeExpandedThinking();
      });
      document.body.append(layer);
    }
    return layer;
  }

  function wireThinkingDrawer(drawer, slot) {
    drawerSlots.set(drawer, slot);
    const toggle = drawer.querySelector("[data-thinking-expand]");
    if (toggle) {
      toggle.addEventListener("click", (event) => {
        // A click inside <summary> would also open/close the drawer.
        event.preventDefault();
        event.stopPropagation();
        if (drawer.classList.contains("is-expanded")) collapseThinking(drawer);
        else expandThinking(drawer);
      });
    }
    const header = drawer.querySelector(".thinking-header");
    if (header) {
      header.addEventListener("click", (event) => {
        // The second half of a double click would undo the first toggle.
        if (event.detail > 1) event.preventDefault();
      });
      header.addEventListener("dblclick", (event) => {
        event.preventDefault();
        expandThinking(drawer);
      });
    }
    syncThinkingToggle(drawer);
  }

  function syncThinkingToggle(drawer) {
    const toggle = drawer.querySelector("[data-thinking-expand]");
    if (!toggle) return;
    const expanded = drawer.classList.contains("is-expanded");
    const label = t(expanded ? "copilot.thinking.collapse" : "copilot.thinking.expand");
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
    toggle.title = label;
  }

  function expandThinking(drawer) {
    if (!drawer || drawer.classList.contains("is-expanded")) return;
    const layer = ensureThinkingLayer();
    layer.hidden = false;
    layer.classList.add("is-active");
    // Moving the node keeps its listeners, text and scroll offset.
    layer.append(drawer);
    drawer.open = true;
    drawer.classList.add("is-expanded");
    document.body.classList.add("has-thinking-expanded");
    syncThinkingToggle(drawer);
    const stream = drawer.querySelector(".thinking-stream");
    if (stream && stream.dataset.pinned !== "false") stream.scrollTop = stream.scrollHeight;
    drawer.querySelector("[data-thinking-expand]")?.focus();
  }

  function collapseThinking(drawer) {
    const layer = document.getElementById("thinking-layer");
    if (!drawer || !layer || !layer.contains(drawer)) return;
    drawer.classList.remove("is-expanded");
    layer.classList.remove("is-active");
    layer.hidden = true;
    document.body.classList.remove("has-thinking-expanded");
    const slot = drawerSlots.get(drawer);
    if (slot?.isConnected) slot.append(drawer);
    else drawer.remove();
    syncThinkingToggle(drawer);
    drawer.querySelector("[data-thinking-expand]")?.focus();
  }

  function closeExpandedThinking() {
    const drawer = document.getElementById("thinking-layer")?.querySelector(".thinking-drawer");
    if (drawer) collapseThinking(drawer);
  }

  function renderThread({ forceScroll = false } = {}) {
    const stick = forceScroll || nearBottom();
    // Re-rendering the thread destroys the drawer's slot, so an expanded one
    // would be left floating over a thread that no longer owns it.
    closeExpandedThinking();
    els.thread.innerHTML = "";
    if (!messages.length) {
      renderEmptyState();
      return;
    }
    for (const message of messages) {
      const node = renderMessage(message);
      els.thread.append(node);
      message.el = node;
      if (message.role === "assistant") syncAssistant(message, false);
    }
    if (stick) scrollToBottom();
  }

  function renderEmptyState() {
    const wrap = document.createElement("div");
    if (configLoading) {
      wrap.className = "copilot-empty is-loading";
      wrap.innerHTML = `<div class="copilot-empty-mark" aria-hidden="true">${sparkSvg()}</div>
        <p class="copilot-empty-loading">${escapeHtml(t("copilot.loading"))}</p>`;
      els.thread.append(wrap);
      return;
    }

    if (configError) {
      wrap.className = "copilot-empty";
      wrap.innerHTML = `
        <div class="copilot-empty-mark" aria-hidden="true">${sparkSvg()}</div>
        <h2>${escapeHtml(t("copilot.connectTitle"))}</h2>
        <p>${escapeHtml(t("copilot.configError"))}</p>
        <button type="button" class="btn btn-primary" data-config-retry>${escapeHtml(t("copilot.retry"))}</button>`;
      wrap
        .querySelector("[data-config-retry]")
        .addEventListener("click", () => loadConfig());
      els.thread.append(wrap);
      return;
    }

    const anyConfigured = Boolean(config && config.providers.some((p) => p.configured));
    if (!anyConfigured) {
      wrap.className = "copilot-empty copilot-connect";
      wrap.innerHTML = `
        <div class="copilot-empty-mark" aria-hidden="true">${sparkSvg()}</div>
        <h2>${escapeHtml(t("copilot.connectTitle"))}</h2>
        <p>${escapeHtml(t("copilot.connectBody"))}</p>
        <button type="button" class="btn btn-primary" data-connect>${escapeHtml(t("copilot.connect"))}</button>`;
      wrap.querySelector("[data-connect]").addEventListener("click", openSheet);
      els.thread.append(wrap);
      return;
    }

    wrap.className = "copilot-empty";
    wrap.innerHTML = `
      <div class="copilot-empty-mark" aria-hidden="true">${sparkSvg()}</div>
      <h2>${escapeHtml(t("copilot.emptyTitle"))}</h2>
      <p>${escapeHtml(t("copilot.emptyBody"))}</p>
      <div class="copilot-suggestions">
        ${["copilot.suggestion1", "copilot.suggestion2", "copilot.suggestion3"]
          .map(
            (key, index) => `<button type="button" class="suggestion" data-suggestion="${index}">
              <span class="suggestion-arrow" aria-hidden="true">↗</span>
              ${escapeHtml(t(key))}
            </button>`
          )
          .join("")}
      </div>`;
    wrap.querySelectorAll("[data-suggestion]").forEach((button) => {
      button.addEventListener("click", () => {
        els.input.value = t(`copilot.suggestion${Number(button.dataset.suggestion) + 1}`);
        els.input.dispatchEvent(new Event("input", { bubbles: true }));
        autosize();
        updateSendEnabled();
        els.input.focus();
      });
    });
    els.thread.append(wrap);
  }

  function renderMessage(message) {
    const article = document.createElement("article");
    article.className = `msg msg-${message.role}`;
    article.dataset.state = message.state || "done";
    if (message.role === "user") {
      article.innerHTML = `
        <div class="msg-body">
          <div class="msg-attachments"></div>
          <div class="msg-row">
            <div class="msg-tools" role="group" aria-label="${escapeHtml(t("copilot.messageActions"))}">
              <button type="button" class="msg-tool" data-rerun title="${escapeHtml(t("copilot.rerun"))}" aria-label="${escapeHtml(t("copilot.rerun"))}">${rerunSvg()}</button>
              <button type="button" class="msg-tool" data-edit title="${escapeHtml(t("copilot.edit"))}" aria-label="${escapeHtml(t("copilot.edit"))}">${editSvg()}</button>
            </div>
            <div class="msg-text"></div>
          </div>
        </div>`;
      const text = article.querySelector(".msg-text");
      text.textContent = message.text;
      text.hidden = !message.text;
      const files = article.querySelector(".msg-attachments");
      if (message.attachments.length) {
        files.innerHTML = message.attachments.map(userAttachment).join("");
      } else {
        files.hidden = true;
      }
      return article;
    }

    article.innerHTML = `
      <div class="msg-body">
        <div class="msg-meta">
          <span class="msg-mark" aria-hidden="true">${sparkSvg()}</span>
          <span class="msg-who">${escapeHtml(t("copilot.messageAssistant"))}</span>
          <span class="msg-state"></span>
        </div>
        <div class="msg-thinking" hidden></div>
        <div class="msg-text"></div>
        <div class="work-card" hidden></div>
        <div class="tool-list" hidden></div>
        <div class="msg-typing" hidden aria-hidden="true"><span></span><span></span><span></span></div>
        <div class="msg-error" hidden></div>
        <div class="msg-actions" hidden></div>
        <div class="msg-usage" hidden></div>
      </div>`;
    return article;
  }

  function syncAssistant(message, forceScroll = false) {
    if (!message.el) return;
    const stick = forceScroll || nearBottom();
    message.el.dataset.state = message.state;
    const text = message.el.querySelector(".msg-text");
    text.textContent = message.text;
    text.hidden = !message.text;
    text.classList.toggle("is-streaming", message.state === "streaming");

    const thinkingEl = message.el.querySelector(".msg-thinking");
    if (thinkingEl) {
      const hasThinking = Boolean(message.thinking && message.thinking.trim());
      thinkingEl.hidden = !hasThinking;
      if (hasThinking) {
        const isStreamingThinking = message.state === "streaming" && !message.text;
        // The drawer is built once and then only its text changes: rebuilding
        // it on every token destroyed the user's scroll and selection.
        let drawer = thinkingEl.querySelector(".thinking-drawer");
        if (!drawer) {
          thinkingEl.innerHTML = `
            <details class="thinking-drawer">
              <summary class="thinking-header">
                <span class="thinking-spark" aria-hidden="true">${sparkSvg()}</span>
                <span class="thinking-title">${escapeHtml(t("copilot.thinking"))}</span>
                <span class="thinking-badge"></span>
                <button type="button" class="thinking-expand" data-thinking-expand>
                  <svg class="thinking-expand-open" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 4H5a1 1 0 0 0-1 1v4M15 20h4a1 1 0 0 0 1-1v-4M4 15v4a1 1 0 0 0 1 1h4M20 9V5a1 1 0 0 0-1-1h-4"/></svg>
                  <svg class="thinking-expand-close" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 9H5V5M15 9h4V5M9 15H5v4M15 15h4v4"/></svg>
                </button>
                <svg class="thinking-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m6 9 6 6 6-6"/></svg>
              </summary>
              <div class="thinking-body">
                <div class="thinking-stream" tabindex="0" role="group" aria-label="${escapeHtml(t("copilot.thinking"))}"></div>
              </div>
            </details>`;
          drawer = thinkingEl.querySelector(".thinking-drawer");
          wireThinkingDrawer(drawer, thinkingEl);
        }
        const stream = drawer.querySelector(".thinking-stream");
        if (stream && stream.textContent !== message.thinking) {
          stream.textContent = message.thinking;
        }
        // The reasoning grows downward past its own max-height, so it has to
        // follow the tail itself; the thread only ever scrolled as a whole.
        if (stream && isStreamingThinking && stream.dataset.pinned !== "false") {
          stream.scrollTop = stream.scrollHeight;
        }
        const badge = drawer.querySelector(".thinking-badge");
        if (badge) {
          const label = t(isStreamingThinking ? "copilot.thinking.live" : "copilot.thinking.done");
          if (badge.textContent !== label) badge.textContent = label;
          badge.classList.toggle("is-live", isStreamingThinking);
        }
      }
    }

    const state = message.el.querySelector(".msg-state");
    state.textContent = message.state === "stopped" ? t("copilot.stopped") : "";

    const usageEl = message.el.querySelector(".msg-usage");
    if (usageEl) {
      const showUsage = message.state === "done" && Boolean(message.usage);
      usageEl.hidden = !showUsage;
      if (showUsage) usageEl.innerHTML = usageMarkup(message.usage);
    }

    const actionsEl = message.el.querySelector(".msg-actions");
    if (actionsEl) {
      const canRevert =
        message.reversible && message.state !== "streaming" && messages[messages.length - 1] === message;
      actionsEl.hidden = !canRevert;
      if (canRevert && !actionsEl.firstElementChild) {
        actionsEl.innerHTML = `<button type="button" class="msg-revert" data-revert>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 7H5v4"/><path d="M5.5 11a7 7 0 1 1 1.6 7.3"/></svg>
          <span>${escapeHtml(t("copilot.revert"))}</span></button>`;
      }
      if (!canRevert) actionsEl.innerHTML = "";
    }

    // The old three-dot indicator stays in the markup for compatibility but is
    // never shown: the work card below says what is really happening.
    message.el.querySelector(".msg-typing").hidden = true;
    renderWorkCard(message);

    const list = message.el.querySelector(".tool-list");
    // While the turn runs, the work card is the one place that says what is
    // happening; the list appears when it settles, as a summary of what was done.
    list.hidden = message.tools.length === 0 || message.state === "streaming";
    if (message.tools.length) {
      list.innerHTML = message.tools.map(toolRow).join("");
    }

    const error = message.el.querySelector(".msg-error");
    const failed = message.state === "error" || message.state === "stopped";
    error.hidden = !failed;
    if (failed) {
      const canRetry = messages[messages.length - 1] === message;
      const generic = message.state === "stopped" ? t("copilot.stopped") : errorText(message.errorCode);
      // Provider payloads are untrusted and usually English: they never reach
      // the thread. Our own validation messages are shown because they say what
      // to fix; everything else goes to the console for debugging.
      const showDetail =
        message.errorCode === "invalid_request" && Boolean(message.errorMessage);
      if (message.errorMessage && !showDetail) {
        console.warn("[copilot]", message.errorCode, message.errorMessage);
      }
      const detail = showDetail
        ? `<div class="msg-error-detail">${escapeHtml(message.errorMessage)}</div>`
        : "";
      const retryButton = canRetry
        ? `<button type="button" class="msg-retry" data-retry>${rerunSvg()}<span>${escapeHtml(
            t("copilot.retryAction")
          )}</span></button>`
        : "";
      // The report is only offered when the provider said something to report.
      const reportButton = message.errorDetail
        ? `<button type="button" class="msg-report" data-report>${reportSvg()}<span>${escapeHtml(
            t("copilot.report")
          )}</span></button>`
        : "";
      error.innerHTML = `
        <p>${escapeHtml(generic)}</p>
        ${detail}
        ${
          retryButton || reportButton
            ? `<div class="msg-error-actions">${retryButton}${reportButton}</div>`
            : ""
        }`;
    }
    if (stick) scrollToBottom();
  }

  function toolRow(tool) {
    const label =
      t(`copilot.tool.${tool.name}`) === `copilot.tool.${tool.name}`
        ? t("copilot.tool.generic")
        : t(`copilot.tool.${tool.name}`);
    const glyph =
      tool.status === "running" || tool.status === "writing"
        ? `<span class="tool-spinner" aria-hidden="true"></span>`
        : tool.status === "done"
          ? `<svg class="tool-check" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
               <path d="m5 12.5 4.5 4.5L19 7"/></svg>`
          : `<svg class="tool-check" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2" stroke-linecap="round" aria-hidden="true">
               <path d="M12 7v6M12 17h.01"/></svg>`;
    let detail = "";
    if ((tool.status === "running" || tool.status === "writing") && tool.progress) {
      detail = tool.progress.items > 0
        ? t("copilot.progress.slides", { n: tool.progress.items })
        : t("copilot.progress.chars", { n: formatTokens(tool.progress.chars) });
    }
    return `<div class="tool-row" data-status="${escapeHtml(tool.status === "writing" ? "running" : tool.status)}">
      ${glyph}<span>${escapeHtml(label)}</span>${
        detail ? `<span class="tool-detail">${escapeHtml(detail)}</span>` : ""
      }</div>`;
  }

  function userAttachment(file) {
    if (file.preview) {
      return `<span class="file-thumb"><img src="${escapeHtml(file.preview)}" alt="${escapeHtml(file.name)}"></span>`;
    }
    return `<span class="file-pill">${file.link ? linkSvg() : docSvg()}<span>${escapeHtml(file.label || file.name)}</span></span>`;
  }

  /** Reads a web page or an image from a link and queues it like any other file. */
  async function addLink(url) {
    if (pending.length >= MAX_ATTACHMENTS) throw new Error(t("copilot.attachLimit"));
    let result;
    try {
      result = await client.copilotFetchUrl(url);
    } catch (error) {
      throw new Error(linkErrorText(error.code));
    }
    if (result.kind === "image") {
      pending.push({
        name: result.name || "image",
        mime: result.mime,
        data: result.data,
        size: Math.floor((result.data.length * 3) / 4),
        preview: `data:${result.mime};base64,${result.data}`,
        link: url,
      });
    } else {
      const body = `Source: ${result.url}\nTitle: ${result.title}\n\n${result.text}`;
      const bytes = new TextEncoder().encode(body);
      const label = String(result.title || new URL(result.url).hostname).replace(/[\\/:*?"<>|]+/g, " ").trim().slice(0, 56);
      pending.push({
        name: `${label || "page"}.txt`,
        label: label || "page",
        mime: "text/plain",
        data: bytesToBase64(bytes),
        size: bytes.length,
        preview: null,
        link: result.url,
      });
    }
    renderPending();
    updateSendEnabled();
  }

  function linkErrorText(code) {
    const key = `copilot.link.${code}`;
    const text = t(key);
    return text === key ? t("copilot.attach.linkFailed") : text;
  }

  // ---------- attachments ----------

  function renderPending() {
    els.attachments.hidden = pending.length === 0;
    els.attachments.innerHTML = pending
      .map(
        (file, index) => `
        <span class="pending-file${file.preview ? " is-image" : ""}">
          ${file.preview ? `<img src="${escapeHtml(file.preview)}" alt="">` : file.link ? linkSvg() : docSvg()}
          <span class="pending-copy">
            <span class="pending-name">${escapeHtml(file.label || file.name)}</span>
            <span class="pending-size">${escapeHtml(formatBytes(file.size))}</span>
          </span>
          <button type="button" class="pending-remove" data-remove="${index}"
            aria-label="${escapeHtml(t("copilot.removeAttachment", { name: file.name }))}">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9"
              stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>
          </button>
        </span>`
      )
      .join("");
    els.attachments.querySelectorAll("[data-remove]").forEach((button) => {
      button.addEventListener("click", () => {
        pending.splice(Number(button.dataset.remove), 1);
        renderPending();
        updateSendEnabled();
      });
    });
    updateSendEnabled();
  }

  async function addFiles(fileList) {
    const chosen = [...fileList];
    if (!chosen.length) return;
    if (pending.length + chosen.length > MAX_ATTACHMENTS) {
      notify(t("copilot.attachLimit"));
    }
    for (const file of chosen) {
      if (pending.length >= MAX_ATTACHMENTS) break;
      try {
        pending.push(await prepareFile(file));
        renderPending();
      } catch (error) {
        notify(error.message || t("copilot.error.invalid_request"));
      }
    }
    els.file.value = "";
  }

  async function prepareFile(file) {
    const extension = (file.name.split(".").pop() || "").toLowerCase();
    let mime = (file.type || "").split(";")[0].trim().toLowerCase();
    let isImage = IMAGE_MIMES.has(mime);

    if (!isImage && IMAGE_BY_EXT[extension]) {
      mime = IMAGE_BY_EXT[extension];
      isImage = true;
    }
    if (!isImage && mime.startsWith("image/")) {
      throw new Error(t("copilot.fileType"));
    }

    if (!isImage) {
      if (mime === "application/pdf" || extension === "pdf") {
        mime = "application/pdf";
      } else if (extension === "docx") {
        mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
      } else if (extension === "pptx") {
        mime = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
      } else if (
        TEXT_EXT.has(extension) ||
        mime.startsWith("text/") ||
        mime === "application/json"
      ) {
        if (!mime) mime = "text/plain";
      } else {
        throw new Error(t("copilot.fileType"));
      }
    }

    const limit = isImage ? MAX_IMAGE_BYTES : MAX_DOCUMENT_BYTES;
    if (file.size > limit) throw new Error(t("copilot.fileTooLarge"));

    const dataUrl = await readDataUrl(file);
    const comma = dataUrl.indexOf(",");
    if (comma < 0) throw new Error(t("copilot.fileType"));
    return {
      name: file.name || "attachment",
      mime,
      data: dataUrl.slice(comma + 1),
      size: file.size,
      preview: isImage ? dataUrl : null,
    };
  }

  // ---------- composer & streaming ----------

  function updateSendEnabled() {
    if (streaming) {
      els.send.disabled = false;
      return;
    }
    els.send.disabled = !els.input.value.trim() && pending.length === 0;
  }

  function setComposerBusy(busy) {
    streaming = busy;
    els.form.setAttribute("aria-busy", String(busy));
    els.send.classList.toggle("is-stopping", busy);
    els.live.hidden = !busy;
    els.panel.dataset.busy = String(busy);
    const label = t(busy ? "copilot.stop" : "copilot.send");
    els.send.title = label;
    els.send.setAttribute("aria-label", label);
    updateSendEnabled();
  }

  function stopTurn() {
    const requestId = currentRequestId;
    currentRequestId = null;
    if (requestId) {
      // Best effort: aborting the fetch is the local half of the cancel.
      client.copilotCancel(requestId).catch(() => {});
    }
    if (controller) controller.abort();
    document.dispatchEvent(new CustomEvent("slyde:draft-end"));
  }

  function send() {
    if (streaming) return;
    const text = els.input.value.trim();
    if (!text && !pending.length) {
      els.input.focus();
      return;
    }
    if (!config) {
      notify(configError ? t("copilot.configError") : t("copilot.loading"));
      loadConfig();
      return;
    }
    const provider = activeProvider();
    if (!provider || !provider.configured) {
      openSheet();
      return;
    }
    if (!modelId) {
      openSheet();
      return;
    }

    recallIndex = -1;
    const attachments = pending;
    pending = [];
    els.input.value = "";
    els.attachments.innerHTML = "";
    els.attachments.hidden = true;
    autosize();
    updateSendEnabled();

    messages.forEach((message) => {
      if (message.reversible) message.reversible = false;
    });
    messages.push({
      role: "user",
      text,
      attachments,
      state: "done",
      el: null,
    });
    messages.push({
      role: "assistant",
      text: "",
      thinking: "",
      tools: [],
      state: "streaming",
      errorCode: null,
      el: null,
    });
    renderThread({ forceScroll: true });
    startStream(buildHistory());
    els.input.focus();
  }

  async function startStream(history) {
    const assistant = messages[messages.length - 1];
    if (!assistant || assistant.role !== "assistant") return;
    const runEpoch = ++epoch;
    controller = new AbortController();
    assistant.startedAt = Date.now();
    assistant.writtenSlides = 0;
    const lastUser = [...messages].reverse().find((message) => message.role === "user");
    assistant.expected = expectedSlides(lastUser && lastUser.text);
    startWorkClock();
    setComposerBusy(true);
    syncAssistant(assistant, true);

    // The server keeps this id so Stop can cancel the turn server-side, not just
    // abort the local stream (otherwise the tool rounds keep running).
    const requestId =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `turn-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    currentRequestId = requestId;

    const payload = {
      provider: providerId,
      model: modelId,
      messages: history,
      selected_slide_id: getSelectedSlideId() || null,
      selected_element_id:
        typeof getSelectedElementId === "function" ? getSelectedElementId() || null : null,
      language: lang(),
      request_id: requestId,
    };

    try {
      await client.streamChat(payload, {
        signal: controller.signal,
        onEvent(event) {
          if (runEpoch !== epoch) return;
          handleStreamEvent(assistant, event);
        },
      });
      if (runEpoch === epoch && assistant.state === "streaming") {
        assistant.state = "error";
        assistant.errorCode = "network";
        assistant.errorMessage = t("copilot.error.network");
        syncAssistant(assistant);
        announce(t("copilot.errorTitle"));
      }
    } catch (error) {
      if (runEpoch !== epoch) return;
      if (controller && controller.signal.aborted) {
        if (assistant.state === "streaming") assistant.state = "stopped";
      } else {
        if (assistant.state === "streaming") {
          assistant.state = "error";
          assistant.errorCode = error.code || "network";
          assistant.errorMessage = error.message || "";
        }
        announce(t("copilot.errorTitle"));
      }
    } finally {
      if (runEpoch === epoch) {
        controller = null;
        stopWorkClock();
        publishPending(0);
        setComposerBusy(false);
        syncAssistant(assistant);
        persistThread();
      }
    }
  }

  /** Calls and tokens as three quiet figures under the reply. */
  function usageMarkup(usage) {
    const mark = usage.estimated ? "~" : "";
    const item = (glyph, value, label) =>
      `<span class="usage-item" title="${escapeHtml(label)}" aria-label="${escapeHtml(label)}">${glyph}<span>${escapeHtml(value)}</span></span>`;
    const parts = [
      item(usageSvg("calls"), String(usage.calls), t("copilot.usage.calls", { n: usage.calls })),
      item(usageSvg("in"), `${mark}${formatTokens(usage.input)}`, t("copilot.usage.input", { n: usage.input })),
    ];
    if (usage.output && !usage.estimated) {
      parts.push(item(usageSvg("out"), formatTokens(usage.output), t("copilot.usage.output", { n: usage.output })));
    }
    return parts.join("");
  }

  function usageLabel(usage) {
    const mark = usage.estimated ? "~" : "";
    if (usage.estimated || !usage.output) {
      return t("copilot.usage", { calls: usage.calls, tokens: `${mark}${formatTokens(usage.input)}` });
    }
    return t("copilot.usageFull", {
      calls: usage.calls,
      input: formatTokens(usage.input),
      output: formatTokens(usage.output),
    });
  }

  // ---------- the work card ----------

  let workClock = null;
  let lastPending = 0;

  function startWorkClock() {
    stopWorkClock();
    workClock = window.setInterval(() => {
      const current = messages[messages.length - 1];
      if (current && current.role === "assistant" && current.state === "streaming") {
        renderWorkCard(current, { tickOnly: true });
      }
    }, 1000);
  }

  function stopWorkClock() {
    if (workClock) window.clearInterval(workClock);
    workClock = null;
  }

  /** Tells the storyboard how many slides are still to come, for its placeholders. */
  function publishPending(count) {
    if (count === lastPending) return;
    lastPending = count;
    document.dispatchEvent(new CustomEvent("slyde:pending", { detail: { count } }));
  }

  function renderWorkCard(message, { tickOnly = false } = {}) {
    const card = message.el && message.el.querySelector(".work-card");
    if (!card) return;
    const info = workPhase(message);
    const active = message.state === "streaming" && info.phase !== "replying";
    card.hidden = !active;
    if (!active) {
      if (message.state !== "streaming") publishPending(0);
      return;
    }
    if (!card.firstElementChild) {
      card.innerHTML = `
        <div class="work-head">
          <span class="work-orb" aria-hidden="true">${sparkSvg()}</span>
          <span class="work-title"></span>
          <span class="work-time" aria-hidden="true"></span>
        </div>
        <div class="work-track" role="progressbar" aria-valuemin="0" aria-valuemax="100"></div>
        <p class="work-line"></p>
        <p class="work-hint" hidden></p>`;
      card.querySelector(".work-track").setAttribute("aria-label", t("copilot.work.label"));
    }
    const elapsed = Date.now() - (message.startedAt || Date.now());
    card.dataset.phase = info.phase;
    card.querySelector(".work-time").textContent = formatElapsed(elapsed);
    const hint = card.querySelector(".work-hint");
    const slow = elapsed > 25000 && (info.phase === "waiting" || info.phase === "thinking");
    if (hint.hidden === slow) {
      hint.hidden = !slow;
      if (slow) hint.textContent = t("copilot.work.hint");
    }
    if (tickOnly) return;

    const written = message.writtenSlides || 0;
    const title = card.querySelector(".work-title");
    const titleText = t(`copilot.work.${info.phase}`, { n: info.slide || 1 });
    if (title.textContent !== titleText) {
      title.textContent = titleText;
      title.classList.remove("is-swapping");
      void title.offsetWidth;
      title.classList.add("is-swapping");
    }

    const line = card.querySelector(".work-line");
    let lineText = "";
    // While the model reasons, the line is a small viewport onto its latest words:
    // new text arrives at the bottom and the older lines roll up and out.
    const thinkingNow = info.phase === "thinking" && Boolean(recentThinking(message.thinking));
    line.classList.toggle("is-thought", thinkingNow);
    if (info.phase === "thinking") lineText = recentThinking(message.thinking) || t("copilot.work.words", { n: info.words });
    else if (info.phase === "writing") lineText = t("copilot.work.chars", { n: formatTokens(info.chars) });
    else if (info.phase === "applying") lineText = t("copilot.work.applyingLine");
    else if (info.phase === "reviewing") lineText = t("copilot.work.reviewingLine");
    else lineText = t("copilot.work.waitingLine");
    if (line.textContent !== lineText) {
      line.textContent = lineText;
      // A thought changes with every few words; animating each change would flicker.
      if (!thinkingNow) {
        line.classList.remove("is-swapping");
        void line.offsetWidth;
        line.classList.add("is-swapping");
      }
    }

    const model = trackModel({ expected: message.expected, written, current: info.phase === "writing" });
    const track = card.querySelector(".work-track");
    const trackKey = `${model.segments}:${model.done}:${model.current}:${info.phase}`;
    if (track.dataset.key !== trackKey) {
      track.dataset.key = trackKey;
      if (model.segments) {
        track.innerHTML = Array.from({ length: model.segments }, (_, index) => {
          const state = index < model.done ? "done" : index === model.done && model.current ? "current" : "todo";
          return `<span class="work-seg" data-state="${state}"></span>`;
        }).join("");
        track.dataset.mode = "segments";
        track.setAttribute("aria-valuenow", String(Math.round((model.done / model.segments) * 100)));
        track.setAttribute(
          "aria-valuetext",
          t("copilot.work.slidesDone", { done: written, total: message.expected })
        );
      } else {
        track.innerHTML = `<span class="work-bar"></span>`;
        track.dataset.mode = "bar";
        track.removeAttribute("aria-valuenow");
        track.setAttribute("aria-valuetext", titleText);
      }
    }

    // Placeholders in the storyboard for the slides still to come.
    let pending = 0;
    if (message.expected) pending = Math.max(0, Math.min(6, message.expected - written));
    else if (info.phase === "writing") pending = 1;
    publishPending(pending);
  }

  function formatTokens(n) {
    return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
  }

  function handleStreamEvent(assistant, event) {
    if (event.type === "thinking") {
      if (assistant.state !== "streaming") return;
      assistant.thinking = (assistant.thinking || "") + (event.text || "");
      syncAssistant(assistant);
      return;
    }
    if (event.type === "delta") {
      if (assistant.state !== "streaming") return;
      assistant.text += event.text || "";
      syncAssistant(assistant);
      return;
    }
    if (event.type === "tool") {
      const name = typeof event.name === "string" ? event.name : "";
      if (!name) return;
      if (event.status === "running") {
        // A call that was being written becomes the running one: same row.
        const writing = [...assistant.tools].reverse().find((tool) => tool.name === name && tool.status === "writing");
        if (writing) writing.status = "running";
        else assistant.tools.push({ name, status: "running" });
        announce(toolLabel(name));
      } else {
        const index = assistant.tools
          .map((tool) => tool.name)
          .lastIndexOf(name);
        if (index >= 0) {
          assistant.tools[index].status = event.status;
          const items = assistant.tools[index].progress && assistant.tools[index].progress.items;
          if (items && event.status === "done") {
            assistant.writtenSlides = Math.max(assistant.writtenSlides || 0, items);
          }
        }
        else assistant.tools.push({ name, status: event.status });
      }
      syncAssistant(assistant);
      return;
    }
    if (event.type === "usage") {
      assistant.usage = {
        calls: event.calls,
        input: event.input_tokens || 0,
        output: event.output_tokens || 0,
        estimated: Boolean(event.estimated),
      };
      return;
    }
    if (event.type === "progress") {
      // The call is still being written: it already has a row, with a live count.
      let row = [...assistant.tools].reverse().find((tool) => tool.name === event.name && tool.status === "writing");
      if (!row) {
        row = { name: event.name, status: "writing" };
        assistant.tools.push(row);
      }
      row.progress = { chars: event.chars, items: event.items };
      if (event.draft) {
        document.dispatchEvent(new CustomEvent("slyde:draft", { detail: { html: event.draft } }));
      }
      assistant.writtenSlides = Math.max(assistant.writtenSlides || 0, Math.max(0, event.items - 1));
      syncAssistant(assistant);
      return;
    }
    if (event.type === "reversible") {
      assistant.reversible = true;
      document.dispatchEvent(new CustomEvent("slyde:versions-changed"));
      return;
    }
    if (event.type === "done") {
      document.dispatchEvent(new CustomEvent("slyde:draft-end"));
      assistant.state = "done";
      syncAssistant(assistant);
      announce(t("copilot.announceDone"));
      return;
    }
    if (event.type === "error") {
      document.dispatchEvent(new CustomEvent("slyde:draft-end"));
      assistant.state = "error";
      assistant.errorCode = event.code || "generic";
      assistant.errorMessage = event.message || "";
      // What the provider actually replied, for the report. It is never shown in
      // the thread itself.
      assistant.errorDetail = event.detail || "";
      assistant.errorStatus = event.status || null;
      assistant.errorProvider = (activeProvider() || {}).name || providerId || "";
      assistant.errorModel = modelId;
      assistant.errorAt = new Date().toISOString();
      syncAssistant(assistant);
      announce(t("copilot.errorTitle"));
    }
  }

  function stopStream() {
    if (!streaming) return;
    stopTurn();
  }

  function retry(message) {
    if (streaming || messages[messages.length - 1] !== message) return;
    const provider = activeProvider();
    if (!provider || !provider.configured || !modelId) {
      openSheet();
      return;
    }
    message.text = "";
    message.thinking = "";
    message.tools = [];
    message.errorCode = null;
    message.errorDetail = "";
    message.errorStatus = null;
    message.state = "streaming";
    syncAssistant(message, true);
    startStream(buildHistory());
  }

  const report = mountReport({ host: els.panel, t, escapeHtml });

  function openReport(message, trigger) {
    const code = message.errorCode || "generic";
    const hintKey = `copilot.report.hint.${code}`;
    const hint = t(hintKey);
    report.open(
      {
        provider: message.errorProvider,
        model: message.errorModel,
        status: message.errorStatus,
        code,
        summary: errorText(code),
        hint: hint === hintKey ? "" : hint,
        detail: message.errorDetail,
        when: message.errorAt ? new Date(message.errorAt).toLocaleString() : "",
      },
      trigger
    );
  }

  /** Sends the same prompt again: everything after it is dropped and answered anew. */
  function rerunMessage(userMessage) {
    if (streaming) return;
    const index = messages.indexOf(userMessage);
    if (index < 0) return;
    const provider = activeProvider();
    if (!provider || !provider.configured || !modelId) {
      openSheet();
      return;
    }
    messages.slice(index + 1).forEach((message) => message.el && message.el.remove());
    messages = messages.slice(0, index + 1);
    messages.forEach((message) => {
      if (message.reversible) message.reversible = false;
    });
    messages.push({
      role: "assistant",
      text: "",
      thinking: "",
      tools: [],
      state: "streaming",
      errorCode: null,
      el: null,
    });
    renderThread({ forceScroll: true });
    startStream(buildHistory());
  }

  /** Puts a sent prompt back in the composer, with its files, and forgets it and what followed. */
  function editMessage(userMessage) {
    if (streaming) return;
    const index = messages.indexOf(userMessage);
    if (index < 0) return;
    messages = messages.slice(0, index);
    pending = (userMessage.attachments || []).slice();
    els.input.value = userMessage.text || "";
    persistThread();
    renderThread();
    renderPending();
    autosize();
    updateSendEnabled();
    els.input.focus();
    els.input.setSelectionRange(els.input.value.length, els.input.value.length);
  }

  function newChat() {
    if (streaming) {
      stopTurn();
      epoch += 1;
      controller = null;
      setComposerBusy(false);
    }
    messages = [];
    persistThread();
    pending = [];
    els.input.value = "";
    els.file.value = "";
    els.attachments.innerHTML = "";
    els.attachments.hidden = true;
    autosize();
    renderThread();
    updateSendEnabled();
    els.input.focus();
  }

  /// Conversation persistence: one thread per deck, so switching decks does not
  /// mix two conversations and a reload does not lose the argument you were in.
  function currentDeckId() {
    if (typeof getDeckId !== "function") return null;
    const deck = getDeckId();
    return typeof deck === "string" && deck ? deck : null;
  }

  async function restoreThread() {
    const deck = currentDeckId();
    if (!deck || streaming) return;
    try {
      const stored = await client.copilotThread(deck);
      if (!stored.length) {
        messages = [];
        renderThread();
        return;
      }
      messages = stored.map((message) => ({
        role: message.role === "assistant" ? "assistant" : "user",
        text: message.text || "",
        attachments: [],
        thinking: "",
        tools: [],
        state: "done",
        errorCode: null,
        el: null,
      }));
      renderThread({ forceScroll: true });
    } catch {
      /* The thread is a convenience; the panel still works without it. */
    }
  }

  function persistThread() {
    const deck = currentDeckId();
    if (!deck || !messages.length) return;
    const payload = messages
      .filter((message) => message.state !== "streaming")
      .map((message) => ({ role: message.role, text: message.text || "" }))
      .filter((message) => message.text.trim());
    client.copilotSaveThread(deck, payload).catch(() => {});
  }

  function buildHistory() {
    const recent = messages.slice(-MAX_HISTORY_MESSAGES);
    const userIndexes = new Set();
    for (let index = recent.length - 1; index >= 0; index -= 1) {
      if (recent[index].role === "user") {
        userIndexes.add(index);
        if (userIndexes.size >= MAX_HISTORY_USER_ATTACHMENT_MESSAGES) break;
      }
    }
    let attachmentBudget = MAX_HISTORY_ATTACHMENTS;

    return recent
      .map((message, index) => {
        if (message.role === "user") {
          const attachments = [];
          if (userIndexes.has(index) && attachmentBudget > 0) {
            for (const file of message.attachments) {
              if (attachmentBudget <= 0) break;
              attachments.push({ name: file.name, mime: file.mime, data: file.data });
              attachmentBudget -= 1;
            }
          }
          if (!message.text.trim() && !attachments.length) return null;
          return { role: "user", text: message.text, attachments };
        }
        if (message.role === "assistant") {
          if (message.state !== "done" || !message.text.trim()) return null;
          return { role: "assistant", text: message.text };
        }
        return null;
      })
      .filter(Boolean);
  }

  // ---------- small utilities ----------

  function nearBottom() {
    return els.thread.scrollHeight - els.thread.scrollTop - els.thread.clientHeight < 140;
  }

  function scrollToBottom() {
    window.requestAnimationFrame(() => {
      els.thread.scrollTop = els.thread.scrollHeight;
    });
  }

  function autosize() {
    els.input.style.height = "0px";
    // Two extra pixels keep the glyphs from triggering a scrollbar on the
    // exact-fit line count; the CSS max-height still caps the composer.
    els.input.style.height = `${Math.min(152, els.input.scrollHeight + 2)}px`;
  }

  function announce(text) {
    if (!text) return;
    els.announcer.textContent = "";
    window.requestAnimationFrame(() => {
      els.announcer.textContent = text;
    });
  }

  function toolLabel(name) {
    const key = `copilot.tool.${name}`;
    const label = t(key);
    return label === key ? t("copilot.tool.generic") : label;
  }

  // ---------- events ----------

  els.input.addEventListener("input", () => {
    autosize();
    updateSendEnabled();
  });
  // ArrowUp in an empty composer (or while browsing) recalls earlier prompts, like
  // a shell: a quick way to re-run or tweak the last request.
  let recallIndex = -1;
  els.input.addEventListener("keydown", (event) => {
    if ((event.key === "ArrowUp" || event.key === "ArrowDown") && !event.shiftKey && !event.isComposing) {
      const sent = messages.filter((message) => message.role === "user" && message.text).map((message) => message.text);
      const browsing = recallIndex >= 0;
      if (!sent.length || (!browsing && els.input.value.trim())) return;
      const caretAtEdge =
        event.key === "ArrowUp" ? els.input.selectionStart === 0 : els.input.selectionEnd === els.input.value.length;
      if (!caretAtEdge && !browsing) return;
      event.preventDefault();
      recallIndex =
        event.key === "ArrowUp"
          ? Math.min(recallIndex + 1, sent.length - 1)
          : recallIndex - 1;
      els.input.value = recallIndex < 0 ? "" : sent[sent.length - 1 - recallIndex];
      autosize();
      updateSendEnabled();
      return;
    }
    if (event.key.length === 1) recallIndex = -1;
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    if (streaming) return;
    event.preventDefault();
    send();
  });
  els.form.addEventListener("submit", (event) => {
    event.preventDefault();
    if (streaming) stopStream();
    else send();
  });
  els.send.addEventListener("click", (event) => {
    if (!streaming) return;
    event.preventDefault();
    stopStream();
  });
  const menus = mountComposerMenus({
    host: els.form,
    attachButton: els.attach,
    modelButton: els.modelChip,
    fileInput: els.file,
    t,
    escapeHtml,
    loadModels: (id) => client.copilotModels(id),
    getSelection: () => ({ provider: providerId, model: modelId }),
    onLink: addLink,
    onPickModel(model) {
      modelId = model.id;
      persistSelection();
      syncHeader();
      els.input.focus();
    },
    onConfigure: openSheet,
  });
  els.file.addEventListener("change", () => addFiles(els.file.files || []));
  els.provider.addEventListener("click", () => {
    menus.close({ restoreFocus: false });
    openSheet();
  });

  // Pasting a screenshot or a file into the composer attaches it, as in any chat.
  els.input.addEventListener("paste", (event) => {
    const files = [...((event.clipboardData && event.clipboardData.files) || [])];
    if (!files.length) return;
    event.preventDefault();
    addFiles(files);
  });
  // Dropping files anywhere on the copilot panel does the same.
  let dragDepth = 0;
  const hasFiles = (event) => [...((event.dataTransfer && event.dataTransfer.types) || [])].includes("Files");
  els.panel.addEventListener("dragenter", (event) => {
    if (!hasFiles(event)) return;
    dragDepth += 1;
    els.panel.classList.add("is-dropping");
  });
  els.panel.addEventListener("dragover", (event) => {
    if (hasFiles(event)) event.preventDefault();
  });
  els.panel.addEventListener("dragleave", (event) => {
    if (!hasFiles(event)) return;
    dragDepth = Math.max(0, dragDepth - 1);
    if (!dragDepth) els.panel.classList.remove("is-dropping");
  });
  els.panel.addEventListener("drop", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragDepth = 0;
    els.panel.classList.remove("is-dropping");
    addFiles(event.dataTransfer.files);
  });
  els.newChat.addEventListener("click", newChat);
  els.thread.addEventListener("click", async (event) => {
    const reportButton = event.target.closest("[data-report]");
    if (reportButton) {
      const message = messages.find((item) => item.el === reportButton.closest(".msg"));
      if (message) openReport(message, reportButton);
      return;
    }
    const toolButton = event.target.closest("[data-rerun], [data-edit]");
    if (toolButton) {
      const message = messages.find((item) => item.el === toolButton.closest(".msg"));
      if (message) {
        if (toolButton.hasAttribute("data-edit")) editMessage(message);
        else rerunMessage(message);
      }
      return;
    }
    const revertButton = event.target.closest("[data-revert]");
    if (revertButton) {
      revertButton.disabled = true;
      try {
        await client.copilotRevert();
        const article = revertButton.closest(".msg");
        const message = messages.find((item) => item.el === article);
        if (message) {
          message.reversible = false;
          syncAssistant(message);
        }
        notify(t("copilot.reverted"));
      } catch (error) {
        revertButton.disabled = false;
        notify(error.message || t("copilot.revertFailed"));
      }
      return;
    }
    const retryButton = event.target.closest("[data-retry]");
    if (!retryButton) return;
    const article = retryButton.closest(".msg");
    const message = messages.find((item) => item.el === article);
    if (message) retry(message);
  });

  // The reasoning stream scrolls on its own, so it must remember whether the
  // reader parked it away from the tail; otherwise every token yanks it back.
  document.addEventListener(
    "scroll",
    (event) => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      if (!target.classList.contains("thinking-stream")) return;
      const gap = target.scrollHeight - target.scrollTop - target.clientHeight;
      target.dataset.pinned = gap < 24 ? "true" : "false";
    },
    true
  );

  document.addEventListener(
    "keydown",
    (event) => {
      if (event.key !== "Escape") return;
      const layer = document.getElementById("thinking-layer");
      if (!layer || layer.hidden) return;
      event.preventDefault();
      event.stopPropagation();
      closeExpandedThinking();
    },
    true
  );

  document.addEventListener(
    "keydown",
    (event) => {
      if (!sheet.isOpen()) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        sheet.close();
        els.provider.focus();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = [...els.sheet.querySelectorAll(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      )].filter((el) => el.offsetParent !== null);
      if (!focusable.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    },
    true
  );

  onLangChange(() => {
    syncHeader();
    renderPending();
    renderThread();
    sheet.render();
  });

  syncHeader();
  renderThread();
  autosize();
  loadConfig();

  // Restore the conversation for the deck that is already open when the panel
  // mounts (a reload, or coming back to a deck).
  restoreThread();

  return {
    focus() {
      els.input.focus();
    },
    /** Re-reads the conversation for the current deck (after a deck switch). */
    refreshThread() {
      restoreThread();
    },
    open() {
      openSheet();
    },
    /** Runs an action pill from the canvas or the command palette. */
    ask(text) {
      if (!text) return;
      els.input.value = text;
      updateSendEnabled();
      send();
    },
    isStreaming() {
      return streaming;
    },
    /** True when a provider with a saved key and a chosen model is ready. */
    isConnected() {
      const provider = activeProvider();
      return Boolean(provider && provider.configured && modelId);
    },
    /**
     * One-step setup for the welcome wizard: saves the key, picks a model that
     * writes slides well and makes it the active selection.
     */
    async connect(provider, key) {
      const models = await client.copilotSaveKey(provider, key);
      const model = recommendedModel(provider, models);
      if (!model) {
        const error = new Error("no_model");
        error.code = "no_model";
        throw error;
      }
      if (config && config.providers) {
        config = {
          ...config,
          providers: config.providers.map((item) =>
            item.id === provider ? { ...item, configured: true } : item
          ),
        };
      }
      providerId = provider;
      modelId = model;
      persistSelection();
      syncHeader();
      return { model };
    },
    /** Provider list for the wizard (loads the configuration when needed). */
    async providers() {
      if (!config) await loadConfig();
      return config && config.providers ? config.providers : [];
    },
  };
}

function bytesToBase64(bytes) {
  let binary = "";
  for (let at = 0; at < bytes.length; at += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  }
  return btoa(binary);
}

function svg(paths, size = 15) {
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
}

function linkSvg() {
  return svg('<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1"/>');
}

function rerunSvg() {
  return svg('<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v5h-5"/>', 16);
}

function editSvg() {
  return svg('<path d="M4 20h4L19.5 8.5a2.1 2.1 0 0 0-3-3L5 17v3z"/><path d="m14.5 7.5 3 3"/>', 16);
}

function reportSvg() {
  return svg('<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h4"/>', 15);
}

function usageSvg(kind) {
  const paths = {
    calls: '<path d="M4 12a8 8 0 0 1 13.7-5.6L20 8.5"/><path d="M20 4v4.5h-4.5"/><path d="M20 12a8 8 0 0 1-13.7 5.6L4 15.5"/><path d="M4 20v-4.5h4.5"/>',
    in: '<path d="M12 4v13M6.5 12 12 17.5 17.5 12"/><path d="M5 20.5h14"/>',
    out: '<path d="M12 20V7M6.5 12 12 6.5l5.5 5.5"/><path d="M5 3.5h14"/>',
  };
  return svg(paths[kind], 13);
}

function readDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ""));
    reader.onerror = () => reject(new Error(t("copilot.fileType")));
    reader.readAsDataURL(file);
  });
}

function formatBytes(bytes) {
  if (bytes >= 1024 * 1024) {
    return `${Math.round((bytes / (1024 * 1024)) * 10) / 10} MB`;
  }
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function errorText(code) {
  const key = `copilot.error.${code}`;
  const text = t(key);
  return text === key ? t("copilot.error.generic") : text;
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function sparkSvg() {
  // The copilot mark: a slide with a prompt, a blinking cursor and a spark.
  return `<svg class="spark" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path class="spark-main" d="M14.2 5.2H7A3 3 0 0 0 4 8.2v8a3 3 0 0 0 3 3h10a3 3 0 0 0 3-3V11"/><path class="spark-chev" d="M9.4 10.2 12.6 13.2 9.4 16.2"/><path class="spark-dot" d="M14.6 16.4h2.6"/><path class="spark-small" fill="currentColor" stroke-width="1" d="M18.3 1.9c.3 2.1 1 2.9 3.1 3.2-2.1.3-2.8 1.1-3.1 3.2-.3-2.1-1-2.9-3.1-3.2 2.1-.3 2.8-1.1 3.1-3.2Z"/></svg>`;
}

function userSvg() {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
    stroke-linecap="round"><circle cx="12" cy="8" r="3.2"/><path d="M5.5 19c.8-3.1 3.3-4.7 6.5-4.7s5.7 1.6 6.5 4.7"/></svg>`;
}

function docSvg() {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M7 3.8h7l4 4V20a1.2 1.2 0 0 1-1.2 1.2H7A1.2 1.2 0 0 1 5.8 20V5A1.2 1.2 0 0 1 7 3.8Z"/>
    <path d="M14 3.9V8h4"/></svg>`;
}
