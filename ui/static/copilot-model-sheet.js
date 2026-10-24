// Provider and model sheet for the copilot.
//
// Owns key entry, model search and the "Free" filter. It never displays a saved
// key: the server only reports whether a provider is configured.

import { t } from "./i18n.js";
import { providerIcon, providerTint } from "./provider-icons.js";

const MAX_RENDERED_MODELS = 300;

export function createModelSheet({
  root,
  client,
  getConfig,
  getSelection,
  onConfigChange,
  onProviderFocus,
  onSelect,
  onClose,
}) {
  const state = {
    open: false,
    provider: null,
    view: "models",
    models: [],
    loading: false,
    error: null,
    query: "",
    free: false,
    keyValue: "",
    showKey: false,
    saving: false,
    keyError: null,
    generation: 0,
  };

  function config() {
    return getConfig();
  }

  function providerById(id) {
    const cfg = config();
    if (!cfg || !cfg.providers) return null;
    return cfg.providers.find((provider) => provider.id === id) || null;
  }

  function configured(provider) {
    return Boolean(provider && provider.configured);
  }

  function open(providerHint) {
    const selection = getSelection();
    const cfg = config();
    const fallback =
      (cfg && cfg.providers.find((provider) => provider.configured)) ||
      (cfg && cfg.providers[0]);
    const wanted =
      providerHint ||
      (selection && selection.provider) ||
      (cfg && cfg.last_provider) ||
      (fallback && fallback.id);
    state.provider = wanted || null;
    state.open = true;
    state.query = "";
    state.free = false;
    state.keyValue = "";
    state.keyError = null;
    state.showKey = false;
    state.models = [];
    state.error = null;
    state.loading = false;
    const provider = providerById(state.provider);
    state.view = configured(provider) ? "models" : "key";
    root.hidden = false;
    render();
    if (state.view === "models") {
      loadModels();
      focusLater("#sheet-search");
    } else {
      focusLater("#sheet-key");
    }
  }

  function close() {
    if (!state.open) return;
    state.open = false;
    root.hidden = true;
    root.innerHTML = "";
    if (onClose) onClose();
  }

  function selectProvider(id) {
    state.provider = id;
    if (onProviderFocus) onProviderFocus(id);
    state.query = "";
    state.free = false;
    state.keyValue = "";
    state.keyError = null;
    state.models = [];
    state.error = null;
    state.loading = false;
    const provider = providerById(id);
    state.view = configured(provider) ? "models" : "key";
    render();
    if (state.view === "models") {
      loadModels();
      focusLater("#sheet-search");
    } else {
      focusLater("#sheet-key");
    }
  }

  function render() {
    if (!state.open) return;
    const provider = providerById(state.provider);
    const providers = (config() && config().providers) || [];
    root.innerHTML = `
      <div class="sheet-head">
        <div>
          <p class="sheet-eyebrow">${escapeHtml(t("copilot.title"))}</p>
          <h2 id="copilot-sheet-title">${escapeHtml(t("copilot.sheetTitle"))}</h2>
        </div>
        <button type="button" class="icon-btn icon-btn-sm" id="sheet-close"
          aria-label="${escapeHtml(t("copilot.close"))}" title="${escapeHtml(t("copilot.close"))}">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
            stroke-linecap="round"><path d="M6 6l12 12M18 6 6 18"/></svg>
        </button>
      </div>
      <div class="provider-strip" role="group" aria-label="${escapeHtml(t("copilot.provider"))}">
        ${providers
          .map((item) => {
            const selected = item.id === state.provider;
            return `<button type="button" class="provider-card${selected ? " is-selected" : ""}"
              data-provider="${escapeHtml(item.id)}" aria-pressed="${selected}"
              aria-label="${escapeHtml(item.name)}${item.configured ? ` — ${escapeHtml(t("copilot.connected"))}` : ""}">
              <span class="provider-mark" style="color:${providerTint(item.id)}">${providerIcon(item.id)}</span>
              <span class="provider-card-name">${escapeHtml(item.name)}</span>
              ${item.configured ? `<span class="provider-dot" title="${escapeHtml(t("copilot.connected"))}"></span>` : ""}
            </button>`;
          })
          .join("")}
      </div>
      <div class="sheet-content" id="sheet-content"></div>`;

    root.querySelector("#sheet-close").addEventListener("click", close);
    root.querySelectorAll("[data-provider]").forEach((button) => {
      button.addEventListener("click", () => selectProvider(button.dataset.provider));
    });
    renderContent(provider);
  }

  function renderContent(provider = providerById(state.provider)) {
    const content = root.querySelector("#sheet-content");
    if (!content) return;
    if (!provider) {
      content.innerHTML = `<p class="sheet-empty">${escapeHtml(t("copilot.modelsError"))}</p>`;
      return;
    }
    state.view === "key" ? renderKeyForm(content, provider) : renderModels(content, provider);
  }

  function renderKeyForm(content, provider) {
    const keyUrl = /^https:\/\//.test(provider.key_url) ? provider.key_url : "";
    content.innerHTML = `
      <div class="sheet-provider-head">
        <div>
          <h3>${escapeHtml(provider.name)}</h3>
          <p>${escapeHtml(provider.company)}</p>
        </div>
        ${provider.configured ? `<span class="connected-pill">${escapeHtml(t("copilot.configured"))}</span>` : ""}
      </div>
      <p class="sheet-hint">${escapeHtml(t("copilot.connectBody"))}</p>
      <form class="key-form" id="sheet-key-form">
        <label class="sheet-label" for="sheet-key">${escapeHtml(t("copilot.apiKey"))}</label>
        <div class="key-row">
          <input id="sheet-key" class="sheet-input" type="${state.showKey ? "text" : "password"}"
            value="${escapeHtml(state.keyValue)}" placeholder="${escapeHtml(t("copilot.keyPlaceholder"))}"
            autocomplete="off" spellcheck="false" maxlength="512"
            aria-label="${escapeHtml(t("copilot.apiKey"))}">
          <button type="button" class="key-toggle" id="sheet-key-toggle"
            aria-label="${escapeHtml(t(state.showKey ? "copilot.hideKey" : "copilot.showKey"))}">
            ${escapeHtml(t(state.showKey ? "copilot.hideKey" : "copilot.showKey"))}
          </button>
        </div>
        ${state.keyError ? `<p class="sheet-error" role="alert">${escapeHtml(errorText(state.keyError))}</p>` : ""}
        <button type="submit" class="btn btn-primary sheet-connect" ${state.saving ? "disabled" : ""}>
          ${escapeHtml(state.saving ? t("copilot.checking") : t("copilot.connectAction"))}
        </button>
      </form>
      <div class="key-meta">
        ${keyUrl ? `<a href="${escapeHtml(keyUrl)}" target="_blank" rel="noopener noreferrer" class="sheet-link">${escapeHtml(t("copilot.getKey"))} ↗</a>` : ""}
        <p class="sheet-fineprint">${escapeHtml(t("copilot.keyPrivacy"))}</p>
      </div>`;

    const input = content.querySelector("#sheet-key");
    input.addEventListener("input", () => {
      state.keyValue = input.value;
    });
    content.querySelector("#sheet-key-toggle").addEventListener("click", () => {
      state.showKey = !state.showKey;
      renderContent();
      focusLater("#sheet-key", true);
    });
    content.querySelector("#sheet-key-form").addEventListener("submit", async (event) => {
      event.preventDefault();
      await saveKey(provider);
    });
  }

  async function saveKey(provider) {
    const key = state.keyValue.trim();
    if (!key) {
      state.keyError = "key_required";
      renderContent();
      focusLater("#sheet-key");
      return;
    }
    state.saving = true;
    state.keyError = null;
    renderContent();
    try {
      const models = await client.copilotSaveKey(provider.id, key);
      const current = config();
      if (current) {
        onConfigChange({
          ...current,
          providers: current.providers.map((item) =>
            item.id === provider.id ? { ...item, configured: true } : item
          ),
        });
      }
      state.saving = false;
      state.keyValue = "";
      state.showKey = false;
      state.models = models;
      state.view = "models";
      state.error = null;
      render();
      focusLater("#sheet-search");
    } catch (error) {
      state.saving = false;
      state.keyError = error.code || "generic";
      renderContent();
      focusLater("#sheet-key");
    }
  }

  function renderModels(content, provider) {
    content.innerHTML = `
      <div class="model-tools">
        <div class="model-search">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"
            stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="6.2"/><path d="m16 16 4 4"/></svg>
          <input id="sheet-search" class="sheet-input" type="search"
            value="${escapeHtml(state.query)}" placeholder="${escapeHtml(t("copilot.searchModels"))}"
            aria-label="${escapeHtml(t("copilot.searchModels"))}">
        </div>
        <button type="button" class="free-toggle" id="sheet-free"
          aria-pressed="${state.free}">${escapeHtml(t("copilot.freeOnly"))}</button>
      </div>
      <div class="model-area" id="sheet-model-area"></div>
      <div class="sheet-footer">
        <button type="button" class="sheet-link" id="sheet-manage-key">${escapeHtml(t("copilot.manageKey"))}</button>
      </div>`;

    const search = content.querySelector("#sheet-search");
    search.addEventListener("input", () => {
      state.query = search.value;
      renderModelArea();
    });
    content.querySelector("#sheet-free").addEventListener("click", () => {
      state.free = !state.free;
      const button = content.querySelector("#sheet-free");
      button.setAttribute("aria-pressed", String(state.free));
      renderModelArea();
    });
    content.querySelector("#sheet-manage-key").addEventListener("click", () => {
      state.view = "key";
      state.keyValue = "";
      state.keyError = null;
      renderContent();
      focusLater("#sheet-key");
    });
    renderModelArea();
  }

  function filteredModels() {
    const query = state.query.trim().toLowerCase();
    return state.models.filter((model) => {
      if (state.free && !model.free) return false;
      if (!query) return true;
      return (
        model.name.toLowerCase().includes(query) || model.id.toLowerCase().includes(query)
      );
    });
  }

  function renderModelArea() {
    const area = root.querySelector("#sheet-model-area");
    if (!area) return;
    if (state.loading) {
      area.innerHTML = `
        <p class="sheet-loading">${escapeHtml(t("copilot.loadingModels"))}</p>
        <div class="model-skeleton" aria-hidden="true"><span></span><span></span><span></span></div>`;
      return;
    }
    if (state.error) {
      area.innerHTML = `
        <p class="sheet-error" role="alert">${escapeHtml(t("copilot.modelsError"))}</p>
        <button type="button" class="sheet-link" id="sheet-retry">${escapeHtml(t("copilot.retry"))}</button>`;
      const retry = area.querySelector("#sheet-retry");
      if (retry) retry.addEventListener("click", loadModels);
      return;
    }

    const models = filteredModels();
    if (!models.length) {
      const message =
        state.models.length === 0
          ? t("copilot.noModels")
          : state.free
            ? t("copilot.noFree")
            : t("copilot.noMatches");
      area.innerHTML = `<p class="sheet-empty">${escapeHtml(message)}</p>`;
      return;
    }

    const selection = getSelection();
    area.innerHTML = `
      <p class="model-count">${escapeHtml(t("copilot.modelCount", { n: models.length }))}</p>
      <div class="model-list">
        ${models
          .slice(0, MAX_RENDERED_MODELS)
          .map((model) => {
            const selected =
              selection && selection.provider === state.provider && selection.model === model.id;
            return `<button type="button" class="model-row${selected ? " is-selected" : ""}"
              data-model="${escapeHtml(model.id)}" aria-pressed="${selected}">
              <span class="model-main">
                <span class="model-name">${escapeHtml(model.name)}</span>
                ${model.name !== model.id ? `<span class="model-id">${escapeHtml(model.id)}</span>` : ""}
              </span>
              <span class="model-badges">
                ${model.free ? `<span class="model-badge is-free">${escapeHtml(t("copilot.freeBadge"))}</span>` : ""}
                ${model.context_window ? `<span class="model-badge">${escapeHtml(contextLabel(model.context_window))}</span>` : ""}
                ${selected ? checkMark() : ""}
              </span>
            </button>`;
          })
          .join("")}
      </div>`;

    area.querySelectorAll("[data-model]").forEach((button) => {
      button.addEventListener("click", () => {
        const model = state.models.find((item) => item.id === button.dataset.model);
        if (!model) return;
        onSelect(state.provider, model);
        close();
      });
    });
  }

  async function loadModels() {
    const provider = providerById(state.provider);
    if (!configured(provider)) return;
    const generation = ++state.generation;
    state.loading = true;
    state.error = null;
    state.models = [];
    renderModelArea();
    try {
      const models = await client.copilotModels(provider.id);
      if (generation !== state.generation) return;
      state.models = models;
      state.loading = false;
      renderModelArea();
    } catch (error) {
      if (generation !== state.generation) return;
      state.loading = false;
      state.error = error.code || "generic";
      renderModelArea();
    }
  }

  function focusLater(selector, select = false) {
    window.requestAnimationFrame(() => {
      const el = root.querySelector(selector);
      if (!el) return;
      el.focus();
      if (select && el.select) el.select();
    });
  }

  return {
    open,
    close,
    render,
    isOpen: () => state.open,
  };
}

function errorText(code) {
  switch (code) {
    case "key_required":
      return t("copilot.keyRequired");
    case "invalid_key":
      return t("copilot.error.invalid_key");
    case "rate_limited":
      return t("copilot.error.rate_limited");
    case "network":
      return t("copilot.error.network");
    case "upstream":
      return t("copilot.error.upstream");
    case "invalid_request":
      return t("copilot.error.invalid_request");
    default:
      return t("copilot.error.generic");
  }
}

function contextLabel(tokens) {
  if (tokens >= 1_000_000) {
    const value = Math.round((tokens / 1_000_000) * 10) / 10;
    return `${value}M`;
  }
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}K`;
  return String(tokens);
}

function checkMark() {
  return `<svg class="model-check" viewBox="0 0 24 24" fill="none" stroke="currentColor"
    stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m5 12.5 4.5 4.5L19 7"/></svg>`;
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
