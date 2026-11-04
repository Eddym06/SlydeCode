import { trapFocus } from "./focus-trap.js";
// Provider and model sheet for the copilot.
//
// Owns key entry, model search and the "Free" filter. It never displays a saved
// key: the server only reports whether a provider is configured.

import { t } from "./i18n.js";
import { providerIcon, providerTint } from "./provider-icons.js";

const MAX_RENDERED_MODELS = 300;
const KIND_KEY = "slyde.copilot.modelFor";

function savedKind() {
  try {
    return localStorage.getItem(KIND_KEY) === "image" ? "image" : "text";
  } catch {
    return "text";
  }
}

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
    kind: savedKind(),
    view: "models",
    models: [],
    loading: false,
    error: null,
    imageError: null,
    imageSaving: false,
    stockError: null,
    stockSaving: false,
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

  /** The providers the current switch can configure. Text lists every provider;
   *  Image only the ones whose key unlocks a picture model. */
  function providersForKind() {
    const all = (config() && config().providers) || [];
    return state.kind === "image" ? all.filter((provider) => provider.image_model) : all;
  }

  function preferredProviderId() {
    const cfg = config();
    if (!cfg) return null;
    return state.kind === "image" ? cfg.last_image_provider : cfg.last_provider;
  }

  function configured(provider) {
    return Boolean(provider && provider.configured);
  }

  let releaseTrap = null;

  function open(providerHint) {
    const selection = getSelection();
    const cfg = config();
    const pool = providersForKind();
    const has = (id) => Boolean(id && pool.some((provider) => provider.id === id));
    const fallback = pool.find((provider) => provider.configured) || pool[0];
    const remembered = preferredProviderId();
    const wanted = has(providerHint)
      ? providerHint
      : has(remembered)
        ? remembered
        : has(selection && selection.provider)
          ? selection.provider
          : (fallback && fallback.id);
    state.provider = wanted || null;
    state.open = true;
    state.query = "";
    state.free = false;
    state.keyValue = "";
    state.keyError = null;
    state.showKey = false;
    state.models = [];
    state.error = null;
    state.imageError = null;
    state.imageSaving = false;
    state.stockError = null;
    state.stockSaving = false;
    state.loading = false;
    const provider = providerById(state.provider);
    state.view = configured(provider) ? "models" : "key";
    root.hidden = false;
    releaseTrap = trapFocus(root);
    render();
    if (state.view === "models") loadModels();
    focusCurrentView();
  }

  function focusCurrentView() {
    if (state.view === "key") {
      focusLater("#sheet-key");
    } else {
      focusLater("#sheet-search");
    }
  }

  /** Switches the sheet between chat models and picture models. The chat
   *  selection is untouched; each role remembers its own provider. */
  function setKind(kind) {
    if (kind !== "text" && kind !== "image") return;
    if (state.kind === kind) return;
    state.kind = kind;
    try {
      localStorage.setItem(KIND_KEY, kind);
    } catch {
      // Private-mode storage failures are non-fatal.
    }
    const pool = providersForKind();
    const current = pool.find((provider) => provider.id === state.provider);
    const next = current || pool.find((provider) => provider.configured) || pool[0];
    if (next) state.provider = next.id;
    state.query = "";
    state.free = false;
    state.models = [];
    state.error = null;
    state.imageError = null;
    state.imageSaving = false;
    state.stockError = null;
    state.stockSaving = false;
    state.loading = false;
    const provider = providerById(state.provider);
    state.view = configured(provider) ? "models" : "key";
    render();
    if (state.view === "models") loadModels();
    focusCurrentView();
  }

  function close() {
    if (!state.open) return;
    state.open = false;
    root.hidden = true;
    if (releaseTrap) releaseTrap();
    releaseTrap = null;
    root.innerHTML = "";
    if (onClose) onClose();
  }

  function selectProvider(id) {
    state.provider = id;
    if (state.kind === "text" && onProviderFocus) onProviderFocus(id);
    state.query = "";
    state.free = false;
    state.keyValue = "";
    state.keyError = null;
    state.models = [];
    state.error = null;
    state.imageError = null;
    state.imageSaving = false;
    state.stockError = null;
    state.stockSaving = false;
    state.loading = false;
    const provider = providerById(id);
    state.view = configured(provider) ? "models" : "key";
    render();
    if (state.view === "models") loadModels();
    focusCurrentView();
  }

  function render() {
    if (!state.open) return;
    const provider = providerById(state.provider);
    const providers = providersForKind();
    root.innerHTML = `
      <div class="sheet-head">
        <div class="sheet-head-copy">
          <p class="sheet-eyebrow">${escapeHtml(t("copilot.title"))}</p>
          <h2 id="copilot-sheet-title">${escapeHtml(t("copilot.sheetTitle"))}</h2>
        </div>
        <div class="sheet-head-tools">
          <div class="model-for">
            <span class="model-for-label" id="sheet-model-for-label">${escapeHtml(t("copilot.modelFor"))}</span>
            <div class="model-for-switch" role="group" aria-labelledby="sheet-model-for-label"
              data-active="${state.kind}">
              <span class="model-for-thumb" aria-hidden="true"></span>
              <button type="button" class="model-for-option${state.kind === "text" ? " is-selected" : ""}"
                data-kind="text" aria-pressed="${state.kind === "text"}">${escapeHtml(t("copilot.modelForText"))}</button>
              <button type="button" class="model-for-option${state.kind === "image" ? " is-selected" : ""}"
                data-kind="image" aria-pressed="${state.kind === "image"}">${escapeHtml(t("copilot.modelForImage"))}</button>
            </div>
          </div>
          <button type="button" class="sheet-close-btn" id="sheet-close"
            aria-label="${escapeHtml(t("copilot.close"))}" title="${escapeHtml(t("copilot.close"))}">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
              stroke-linecap="round"><path d="M6 6l12 12M18 6 6 18"/></svg>
          </button>
        </div>
      </div>
      <div class="provider-strip${state.kind === "image" ? " is-image" : ""}" role="group" aria-label="${escapeHtml(t("copilot.provider"))}">
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
    root.querySelectorAll("[data-kind]").forEach((button) => {
      button.addEventListener("click", () => setKind(button.dataset.kind));
    });
    root.querySelectorAll("[data-provider]").forEach((button) => {
      button.addEventListener("click", () => selectProvider(button.dataset.provider));
    });
    renderContent(provider);
  }

  function renderContent(provider = providerById(state.provider)) {
    const content = root.querySelector("#sheet-content");
    if (!content) return;
    if (state.view === "stock") {
      renderStock(content);
      return;
    }
    if (!provider) {
      const message =
        state.kind === "image" ? t("copilot.noImageModel") : t("copilot.modelsError");
      content.innerHTML = `<p class="sheet-empty">${escapeHtml(message)}</p>`;
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
        <button type="button" class="sheet-link" id="sheet-stock-keys">${escapeHtml(t("copilot.stockKeys"))}</button>
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
    bindStockKeysButton(content);
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
      state.models = state.kind === "text" ? models : [];
      state.view = "models";
      state.error = null;
      render();
      if (state.kind === "image") loadModels();
      focusCurrentView();
    } catch (error) {
      state.saving = false;
      state.keyError = error.code || "generic";
      renderContent();
      focusLater("#sheet-key");
    }
  }

  function bindStockKeysButton(content) {
    const button = content.querySelector("#sheet-stock-keys");
    if (!button) return;
    button.addEventListener("click", () => {
      state.view = "stock";
      state.stockError = null;
      state.stockSaving = false;
      renderContent();
      focusLater("[data-stock-key]");
    });
  }

  /** API keys for the stock-image fallback. They live beside the provider
   *  keys, never in the deck, and are saved without a network validation. */
  function renderStock(content) {
    const sources =
      (config() && Array.isArray(config().stock) && config().stock.length
        ? config().stock
        : [
            { id: "unsplash", name: "Unsplash", configured: false },
            { id: "pexels", name: "Pexels", configured: false },
            { id: "vecteezy", name: "Vecteezy", configured: false },
          ]);
    content.innerHTML = `
      <div class="sheet-provider-head">
        <div>
          <h3>${escapeHtml(t("copilot.stockTitle"))}</h3>
          <p>${escapeHtml(t("copilot.stockBody"))}</p>
        </div>
      </div>
      <p class="sheet-hint">${escapeHtml(t("copilot.stockHint"))}</p>
      ${sources
        .map(
          (source) => `
        <form class="key-form stock-source" data-stock-form="${escapeHtml(source.id)}">
          <div class="sheet-provider-head">
            <h4>${escapeHtml(source.name)}</h4>
            ${source.configured ? `<span class="connected-pill">${escapeHtml(t("copilot.configured"))}</span>` : ""}
          </div>
          <label class="sheet-label" for="stock-key-${escapeHtml(source.id)}">${escapeHtml(t("copilot.apiKey"))}</label>
          <div class="key-row">
            <input id="stock-key-${escapeHtml(source.id)}" class="sheet-input" type="password"
              data-stock-key autocomplete="off" spellcheck="false" maxlength="1024"
              placeholder="${escapeHtml(t("copilot.stockKeyPlaceholder"))}">
          </div>
          ${
            source.id === "vecteezy"
              ? `<label class="sheet-label" for="stock-account-vecteezy">${escapeHtml(t("copilot.stockAccount"))}</label>
                 <div class="key-row">
                   <input id="stock-account-vecteezy" class="sheet-input" type="text"
                     data-stock-account autocomplete="off" spellcheck="false" maxlength="64"
                     placeholder="${escapeHtml(t("copilot.stockAccountPlaceholder"))}">
                 </div>`
              : ""
          }
          <button type="submit" class="btn btn-primary sheet-connect">
            ${escapeHtml(t("copilot.stockSave"))}
          </button>
        </form>`
        )
        .join("")}
      ${state.stockError ? `<p class="sheet-error" role="alert">${escapeHtml(errorText(state.stockError))}</p>` : ""}
      <div class="key-meta">
        <button type="button" class="sheet-link" id="sheet-stock-back">${escapeHtml(t("copilot.stockBack"))}</button>
      </div>`;

    content.querySelectorAll("[data-stock-form]").forEach((form) => {
      form.addEventListener("submit", async (event) => {
        event.preventDefault();
        await saveStock(form);
      });
    });
    content.querySelector("#sheet-stock-back").addEventListener("click", () => {
      state.view = "models";
      state.stockError = null;
      renderContent();
      focusCurrentView();
    });
  }

  async function saveStock(form) {
    const provider = form.dataset.stockForm;
    const key = (form.querySelector("[data-stock-key]")?.value || "").trim();
    const account = (form.querySelector("[data-stock-account]")?.value || "").trim();
    if (provider === "vecteezy" && !key && account) {
      state.stockError = "key_required";
      renderContent();
      focusLater("[data-stock-key]");
      return;
    }
    state.stockSaving = true;
    state.stockError = null;
    try {
      const next = await client.copilotSaveStockKey(provider, key, account);
      state.stockSaving = false;
      if (next) onConfigChange(next);
      renderContent();
      focusLater("[data-stock-key]");
    } catch (error) {
      state.stockSaving = false;
      state.stockError = error.code || "generic";
      renderContent();
      focusLater("[data-stock-key]");
    }
  }

  async function chooseImageModel(provider, model) {
    if (state.imageSaving) return;
    state.imageSaving = true;
    state.imageError = null;
    renderModelArea();
    try {
      const next = await client.copilotImageModel(provider.id, model);
      state.imageSaving = false;
      if (next) onConfigChange(next);
      close();
    } catch (error) {
      state.imageSaving = false;
      state.imageError = error.code || "generic";
      renderModelArea();
      focusLater("[data-model]");
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
      ${state.kind === "image" ? `<p class="sheet-hint">${escapeHtml(t("copilot.imageModelBody"))}</p>` : ""}
      <div class="model-area" id="sheet-model-area"></div>
      <div class="sheet-footer">
        <button type="button" class="sheet-link" id="sheet-manage-key">${escapeHtml(t("copilot.manageKey"))}</button>
        <button type="button" class="sheet-link" id="sheet-stock-keys">${escapeHtml(t("copilot.stockKeys"))}</button>
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
    bindStockKeysButton(content);
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
          ? state.kind === "image"
            ? t("copilot.noImageModels")
            : t("copilot.noModels")
          : state.free
            ? t("copilot.noFree")
            : t("copilot.noMatches");
      area.innerHTML = `<p class="sheet-empty">${escapeHtml(message)}</p>`;
      return;
    }

    const selection = getSelection();
    const cfg = config();
    const imageMode = state.kind === "image";
    const selectedImage = (model) =>
      Boolean(
        cfg &&
          cfg.last_image_provider === state.provider &&
          cfg.last_image_model === model.id
      );
    area.innerHTML = `
      <div class="model-legend" role="note" aria-label="${escapeHtml(t("copilot.priceLegend"))}">
        <span class="model-legend-item is-input">${priceArrow("down")}${escapeHtml(t("copilot.priceInput"))}</span>
        <span class="model-legend-item is-output">${priceArrow("up")}${escapeHtml(t("copilot.priceOutput"))}</span>
        <span class="model-legend-note">${escapeHtml(t("copilot.priceUnit"))}</span>
      </div>
      <p class="model-count">${escapeHtml(t("copilot.modelCount", { n: models.length }))}</p>
      ${state.imageError ? `<p class="sheet-error" role="alert">${escapeHtml(errorText(state.imageError))}</p>` : ""}
      <div class="model-list">
        ${models
          .slice(0, MAX_RENDERED_MODELS)
          .map((model) => {
            const selected = imageMode
              ? selectedImage(model)
              : selection &&
                selection.provider === state.provider &&
                selection.model === model.id;
            return `<button type="button" class="model-row${selected ? " is-selected" : ""}"
              data-model="${escapeHtml(model.id)}" aria-pressed="${selected}"
              ${state.imageSaving ? "disabled" : ""}>
              <span class="model-main">
                <span class="model-name">${escapeHtml(model.name)}</span>
                <span class="model-sub">
                  ${model.name !== model.id ? `<span class="model-id">${escapeHtml(model.id)}</span>` : ""}
                  ${priceGroup(model, imageMode)}
                </span>
              </span>
              <span class="model-badges">
                ${model.free ? `<span class="model-badge is-free">${escapeHtml(t("copilot.freeBadge"))}</span>` : ""}
                ${imageMode ? `<span class="model-badge is-image">${escapeHtml(t("copilot.modelForImage"))}</span>` : ""}
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
        if (imageMode) {
          const provider = providerById(state.provider);
          if (provider) chooseImageModel(provider, model.id);
        } else {
          onSelect(state.provider, model);
          close();
        }
      });
    });
  }

  async function loadModels() {
    const provider = providerById(state.provider);
    if (!configured(provider)) return;
    const generation = ++state.generation;
    state.loading = true;
    state.error = null;
    state.imageError = null;
    state.models = [];
    renderModelArea();
    try {
      const models = await client.copilotModels(provider.id, state.kind);
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

function priceGroup(model, imageMode) {
  const input = Number.isFinite(model.input_price) ? model.input_price : null;
  const output = Number.isFinite(model.output_price) ? model.output_price : null;
  const image = Number.isFinite(model.image_price) ? model.image_price : null;
  if (input === null && output === null) {
    if (image !== null) {
      return `<span class="model-prices"><span class="model-price is-image">${escapeHtml(
        t("copilot.perImage", { price: formatPrice(image) })
      )}</span></span>`;
    }
    return `<span class="model-prices is-unknown">${escapeHtml(
      imageMode ? t("copilot.pricePerImageOnly") : t("copilot.priceUnknown")
    )}</span>`;
  }
  return `<span class="model-prices">
    <span class="model-price is-input" title="${escapeHtml(t("copilot.priceInput"))}">
      ${priceArrow("down")}${formatPrice(input)}
    </span>
    <span class="model-price is-output" title="${escapeHtml(t("copilot.priceOutput"))}">
      ${priceArrow("up")}${formatPrice(output)}
    </span>
    ${image !== null ? `<span class="model-price is-image">${escapeHtml(
      t("copilot.perImage", { price: formatPrice(image) })
    )}</span>` : ""}
  </span>`;
}

function formatPrice(value) {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  if (value === 0) return "$0";
  if (value >= 100) return `$${value.toFixed(0)}`;
  if (value >= 10) return `$${value.toFixed(1)}`;
  if (value >= 1) return `$${value.toFixed(2)}`;
  if (value >= 0.1) return `$${value.toFixed(3)}`;
  if (value >= 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toPrecision(2)}`;
}

function priceArrow(direction) {
  const path =
    direction === "down" ? "M12 5v14M6 13l6 6 6-6" : "M12 19V5M6 11l6-6 6 6";
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${path}"/></svg>`;
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
