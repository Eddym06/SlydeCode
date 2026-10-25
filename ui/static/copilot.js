// The in-app copilot: visible conversation, attachments and streamed tool
// status. The server owns provider keys and the tool loop; this module never
// sees a key and never renders provider-authored free text.

import { lang, onLangChange, t } from "./i18n.js";
import { createModelSheet } from "./copilot-model-sheet.js";
import { providerIcon, providerTint } from "./provider-icons.js";

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

export function mountCopilot({ client, getSelectedSlideId, notify }) {
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

  function renderThread({ forceScroll = false } = {}) {
    const stick = forceScroll || nearBottom();
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
        <div class="msg-avatar msg-avatar-you" aria-hidden="true">${userSvg()}</div>
        <div class="msg-body">
          <div class="msg-meta"><span>${escapeHtml(t("copilot.messageYou"))}</span></div>
          <div class="msg-text"></div>
          <div class="msg-attachments"></div>
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
      <div class="msg-avatar msg-avatar-ai" aria-hidden="true">${sparkSvg()}</div>
      <div class="msg-body">
        <div class="msg-meta">
          <span>${escapeHtml(t("copilot.messageAssistant"))}</span>
          <span class="msg-state"></span>
        </div>
        <div class="msg-thinking" hidden></div>
        <div class="msg-text"></div>
        <div class="tool-list" hidden></div>
        <div class="msg-typing" hidden aria-hidden="true"><span></span><span></span><span></span></div>
        <div class="msg-error" hidden></div>
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
        const wasOpen = thinkingEl.querySelector(".thinking-drawer")?.open ?? isStreamingThinking;
        thinkingEl.innerHTML = `
          <details class="thinking-drawer" ${wasOpen ? "open" : ""}>
            <summary class="thinking-header">
              <span class="thinking-spark" aria-hidden="true">${sparkSvg()}</span>
              <span class="thinking-title">${escapeHtml(t("copilot.thinking"))}</span>
              <span class="thinking-badge ${isStreamingThinking ? "is-live" : ""}">${isStreamingThinking ? "pensando…" : "razonamiento"}</span>
              <svg class="thinking-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m6 9 6 6 6-6"/></svg>
            </summary>
            <div class="thinking-body">
              <div class="thinking-stream">${escapeHtml(message.thinking)}</div>
            </div>
          </details>`;
      }
    }

    const state = message.el.querySelector(".msg-state");
    state.textContent = message.state === "stopped" ? t("copilot.stopped") : "";

    const typing = message.el.querySelector(".msg-typing");
    const running = message.tools.some((tool) => tool.status === "running");
    const showTyping = message.state === "streaming" && (!message.text || running);
    typing.hidden = !showTyping;
    typing.classList.toggle("has-tools", running);

    const list = message.el.querySelector(".tool-list");
    list.hidden = message.tools.length === 0;
    if (message.tools.length) {
      list.innerHTML = message.tools.map(toolRow).join("");
    }

    const error = message.el.querySelector(".msg-error");
    const failed = message.state === "error" || message.state === "stopped";
    error.hidden = !failed;
    if (failed) {
      const canRetry = messages[messages.length - 1] === message;
      const generic = message.state === "stopped" ? t("copilot.stopped") : errorText(message.errorCode);
      const detail = message.errorMessage ? `<div class="msg-error-detail">${escapeHtml(message.errorMessage)}</div>` : "";
      error.innerHTML = `
        <p>${escapeHtml(generic)}</p>
        ${detail}
        ${
          canRetry
            ? `<button type="button" class="msg-retry" data-retry>${escapeHtml(
                t("copilot.retryAction")
              )}</button>`
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
      tool.status === "running"
        ? `<span class="tool-spinner" aria-hidden="true"></span>`
        : tool.status === "done"
          ? `<svg class="tool-check" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
               <path d="m5 12.5 4.5 4.5L19 7"/></svg>`
          : `<svg class="tool-check" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2" stroke-linecap="round" aria-hidden="true">
               <path d="M12 7v6M12 17h.01"/></svg>`;
    return `<div class="tool-row" data-status="${escapeHtml(tool.status)}">
      ${glyph}<span>${escapeHtml(label)}</span></div>`;
  }

  function userAttachment(file) {
    if (file.preview) {
      return `<span class="file-thumb"><img src="${escapeHtml(file.preview)}" alt=""></span>`;
    }
    return `<span class="file-pill">${docSvg()}<span>${escapeHtml(file.name)}</span></span>`;
  }

  // ---------- attachments ----------

  function renderPending() {
    els.attachments.hidden = pending.length === 0;
    els.attachments.innerHTML = pending
      .map(
        (file, index) => `
        <span class="pending-file${file.preview ? " is-image" : ""}">
          ${file.preview ? `<img src="${escapeHtml(file.preview)}" alt="">` : docSvg()}
          <span class="pending-copy">
            <span class="pending-name">${escapeHtml(file.name)}</span>
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

    const attachments = pending;
    pending = [];
    els.input.value = "";
    els.attachments.innerHTML = "";
    els.attachments.hidden = true;
    autosize();
    updateSendEnabled();

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
    setComposerBusy(true);
    syncAssistant(assistant, true);

    const payload = {
      provider: providerId,
      model: modelId,
      messages: history,
      selected_slide_id: getSelectedSlideId() || null,
      language: lang(),
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
        setComposerBusy(false);
        syncAssistant(assistant);
      }
    }
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
        assistant.tools.push({ name, status: "running" });
        announce(toolLabel(name));
      } else {
        const index = assistant.tools
          .map((tool) => tool.name)
          .lastIndexOf(name);
        if (index >= 0) assistant.tools[index].status = event.status;
        else assistant.tools.push({ name, status: event.status });
      }
      syncAssistant(assistant);
      return;
    }
    if (event.type === "done") {
      assistant.state = "done";
      syncAssistant(assistant);
      announce(t("copilot.announceDone"));
      return;
    }
    if (event.type === "error") {
      assistant.state = "error";
      assistant.errorCode = event.code || "generic";
      assistant.errorMessage = event.message || "";
      syncAssistant(assistant);
      announce(t("copilot.errorTitle"));
    }
  }

  function stopStream() {
    if (!streaming) return;
    if (controller) controller.abort();
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
    message.state = "streaming";
    syncAssistant(message, true);
    startStream(buildHistory());
  }

  function newChat() {
    if (streaming) {
      epoch += 1;
      if (controller) controller.abort();
      controller = null;
      setComposerBusy(false);
    }
    messages = [];
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
  els.input.addEventListener("keydown", (event) => {
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
  els.attach.addEventListener("click", () => els.file.click());
  els.file.addEventListener("change", () => addFiles(els.file.files || []));
  els.provider.addEventListener("click", openSheet);
  els.modelChip.addEventListener("click", openSheet);
  els.newChat.addEventListener("click", newChat);
  els.thread.addEventListener("click", (event) => {
    const retryButton = event.target.closest("[data-retry]");
    if (!retryButton) return;
    const article = retryButton.closest(".msg");
    const message = messages.find((item) => item.el === article);
    if (message) retry(message);
  });

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

  return {
    focus() {
      els.input.focus();
    },
    open() {
      openSheet();
    },
    isStreaming() {
      return streaming;
    },
  };
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
  return `<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 2.6c.7 5.3 1.9 6.5 7.2 7.2-5.3.7-6.5 1.9-7.2 7.2-.7-5.3-1.9-6.5-7.2-7.2 5.3-.7 6.5-1.9 7.2-7.2Zm6.7 11.1c.3 2.4.9 3 3.3 3.3-2.4.3-3 .9-3.3 3.3-.3-2.4-.9-3-3.3-3.3 2.4-.3 3-.9 3.3-3.3Z"/></svg>`;
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
