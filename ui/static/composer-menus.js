// The two menus above the copilot composer: what to attach, and which model to use.
//
// Both open upward from their button, close on Escape or an outside click, and
// hand focus back to the button, so the composer never traps the keyboard. The
// module owns no copilot state: the copilot passes in what to show and what to do.

const DOCUMENT_ACCEPT =
  "application/pdf,.pdf,.docx,.pptx,.txt,.md,.csv,.json,.html,.css,.js,.ts,.yaml,.yml";
const IMAGE_ACCEPT = "image/png,image/jpeg,image/webp";
const MAX_ROWS = 200;

const ICONS = {
  document:
    '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h4"/>',
  image:
    '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><circle cx="9" cy="10" r="1.6"/><path d="m4 17 4.6-4.6a1.5 1.5 0 0 1 2.1 0L15 16.7l1.6-1.6a1.5 1.5 0 0 1 2.1 0L20.5 17"/>',
  link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1"/>',
  back: '<path d="M15 5l-7 7 7 7"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4-4"/>',
};

function icon(name, size = 18) {
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name]}</svg>`;
}

/** `128000` -> `128K`, `1000000` -> `1M`. */
function contextLabel(tokens) {
  if (!tokens) return "";
  if (tokens >= 1_000_000) return `${Math.round(tokens / 100_000) / 10}M`.replace(".0M", "M");
  return `${Math.round(tokens / 1000)}K`;
}

/**
 * @param {object} options
 * @param {HTMLElement} options.host            the composer; menus are positioned inside it
 * @param {HTMLElement} options.attachButton
 * @param {HTMLElement} options.modelButton
 * @param {HTMLInputElement} options.fileInput  the hidden file input the copilot already owns
 * @param {(key: string, vars?: object) => string} options.t
 * @param {(value: string) => string} options.escapeHtml
 * @param {(providerId: string) => Promise<Array>} options.loadModels
 * @param {() => {provider: string|null, model: string}} options.getSelection
 * @param {(url: string) => Promise<void>} options.onLink     resolves once the link became an attachment
 * @param {(model: object) => void} options.onPickModel
 * @param {() => void} options.onConfigure
 */
export function mountComposerMenus(options) {
  const { host, attachButton, modelButton, fileInput, t, escapeHtml } = options;

  const menu = document.createElement("div");
  menu.className = "cmenu";
  menu.hidden = true;
  host.append(menu);

  let current = null; // "attach" | "model" | null
  let view = "root"; // attach menu: "root" | "link"
  const modelCache = new Map(); // provider id -> { models } | { error }
  let modelQuery = "";

  function triggerFor(kind) {
    return kind === "attach" ? attachButton : modelButton;
  }

  function place(trigger, width) {
    const hostBox = host.getBoundingClientRect();
    const box = trigger.getBoundingClientRect();
    const left = Math.max(0, Math.min(box.left - hostBox.left - 6, hostBox.width - width));
    menu.style.setProperty("--cm-left", `${Math.round(left)}px`);
    menu.style.setProperty("--cm-w", `${width}px`);
  }

  function close({ restoreFocus = true } = {}) {
    if (!current) return;
    const trigger = triggerFor(current);
    trigger.setAttribute("aria-expanded", "false");
    menu.hidden = true;
    menu.innerHTML = "";
    current = null;
    view = "root";
    if (restoreFocus) trigger.focus();
  }

  function open(kind) {
    if (current === kind) {
      close();
      return;
    }
    close({ restoreFocus: false });
    current = kind;
    view = "root";
    modelQuery = "";
    const trigger = triggerFor(kind);
    trigger.setAttribute("aria-expanded", "true");
    menu.dataset.kind = kind;
    menu.hidden = false;
    place(trigger, kind === "attach" ? 264 : 316);
    if (kind === "attach") renderAttach();
    else renderModels();
  }

  // ---------- attachments ----------

  function renderAttach() {
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", t("copilot.attach"));
    if (view === "link") {
      renderLink();
      return;
    }
    menu.innerHTML = `
      <button type="button" class="cmenu-item" role="menuitem" data-act="documents">
        <span class="cmenu-icon">${icon("document")}</span>
        <span class="cmenu-copy"><span class="cmenu-title">${escapeHtml(t("copilot.attach.documents"))}</span>
        <span class="cmenu-hint">${escapeHtml(t("copilot.attach.documentsHint"))}</span></span>
      </button>
      <button type="button" class="cmenu-item" role="menuitem" data-act="images">
        <span class="cmenu-icon">${icon("image")}</span>
        <span class="cmenu-copy"><span class="cmenu-title">${escapeHtml(t("copilot.attach.images"))}</span>
        <span class="cmenu-hint">${escapeHtml(t("copilot.attach.imagesHint"))}</span></span>
      </button>
      <button type="button" class="cmenu-item" role="menuitem" data-act="link">
        <span class="cmenu-icon">${icon("link")}</span>
        <span class="cmenu-copy"><span class="cmenu-title">${escapeHtml(t("copilot.attach.link"))}</span>
        <span class="cmenu-hint">${escapeHtml(t("copilot.attach.linkHint"))}</span></span>
      </button>`;
    menu.querySelectorAll("[data-act]").forEach((button) => {
      button.addEventListener("click", () => {
        const act = button.dataset.act;
        if (act === "link") {
          view = "link";
          renderAttach();
          return;
        }
        fileInput.accept = act === "documents" ? DOCUMENT_ACCEPT : IMAGE_ACCEPT;
        close({ restoreFocus: false });
        fileInput.click();
      });
    });
    menu.querySelector("[data-act]")?.focus();
  }

  function renderLink() {
    menu.setAttribute("role", "group");
    menu.innerHTML = `
      <div class="cmenu-link">
        <div class="cmenu-link-head">
          <button type="button" class="cmenu-back" data-back aria-label="${escapeHtml(t("copilot.attach.back"))}">${icon("back", 16)}</button>
          <span class="cmenu-title">${escapeHtml(t("copilot.attach.link"))}</span>
        </div>
        <p class="cmenu-hint">${escapeHtml(t("copilot.attach.linkHelp"))}</p>
        <div class="cmenu-link-row">
          <input type="url" class="cmenu-input" inputmode="url" autocomplete="off" spellcheck="false"
            placeholder="https://" aria-label="${escapeHtml(t("copilot.attach.linkLabel"))}">
          <button type="button" class="cmenu-add">${escapeHtml(t("copilot.attach.add"))}</button>
        </div>
        <p class="cmenu-error" role="alert" hidden></p>
      </div>`;
    // Not a <form>: this menu sits inside the composer's own form, and HTML drops a
    // nested one (Enter would then send the chat message instead of adding the link).
    const form = menu.querySelector(".cmenu-link");
    const input = menu.querySelector("input");
    const error = menu.querySelector(".cmenu-error");
    const add = menu.querySelector(".cmenu-add");
    menu.querySelector("[data-back]").addEventListener("click", () => {
      view = "root";
      renderAttach();
    });
    const submit = async () => {
      let url = input.value.trim();
      if (!url) {
        input.focus();
        return;
      }
      if (!/^[a-z][a-z0-9+.-]*:/i.test(url)) url = `https://${url}`;
      error.hidden = true;
      add.disabled = true;
      add.dataset.loading = "true";
      form.setAttribute("aria-busy", "true");
      try {
        await options.onLink(url);
        close({ restoreFocus: false });
      } catch (failure) {
        error.textContent = failure.message || t("copilot.attach.linkFailed");
        error.hidden = false;
        add.disabled = false;
        delete add.dataset.loading;
        form.removeAttribute("aria-busy");
        input.focus();
        input.select();
      }
    };
    add.addEventListener("click", submit);
    input.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" || event.isComposing) return;
      event.preventDefault();
      event.stopPropagation();
      submit();
    });
    input.focus();
  }

  // ---------- models ----------

  async function ensureModels(providerId) {
    if (!providerId) return { error: "none" };
    if (modelCache.has(providerId)) return modelCache.get(providerId);
    try {
      const models = await options.loadModels(providerId);
      const entry = { models };
      modelCache.set(providerId, entry);
      return entry;
    } catch (failure) {
      return { error: failure.code || "generic" };
    }
  }

  function renderModels() {
    menu.setAttribute("role", "dialog");
    menu.setAttribute("aria-label", t("copilot.chooseModel"));
    menu.innerHTML = `
      <div class="cmenu-search">
        ${icon("search", 16)}
        <input type="search" class="cmenu-input cmenu-search-input" autocomplete="off" spellcheck="false"
          placeholder="${escapeHtml(t("copilot.searchModels"))}" aria-label="${escapeHtml(t("copilot.searchModels"))}">
      </div>
      <div class="cmenu-models" role="listbox" aria-label="${escapeHtml(t("copilot.chooseModel"))}"></div>
      <button type="button" class="cmenu-foot" data-configure>
        ${icon("gear", 16)}<span>${escapeHtml(t("copilot.configureModel"))}</span>
      </button>`;
    const search = menu.querySelector("input");
    search.value = modelQuery;
    search.addEventListener("input", () => {
      modelQuery = search.value;
      paintModels();
    });
    menu.querySelector("[data-configure]").addEventListener("click", () => {
      close({ restoreFocus: false });
      options.onConfigure();
    });
    search.focus();
    paintModels();
  }

  async function paintModels() {
    const list = menu.querySelector(".cmenu-models");
    if (!list) return;
    const { provider, model: selected } = options.getSelection();
    if (!modelCache.has(provider)) {
      list.innerHTML = `<div class="cmenu-skeleton" aria-hidden="true"><span></span><span></span><span></span><span></span></div>`;
      list.setAttribute("aria-busy", "true");
    }
    const entry = await ensureModels(provider);
    // The menu may have been closed or replaced while the list loaded.
    if (current !== "model" || !menu.contains(list)) return;
    list.removeAttribute("aria-busy");

    if (entry.error) {
      list.innerHTML = `<p class="cmenu-empty">${escapeHtml(t("copilot.noModels"))}</p>`;
      return;
    }
    const query = modelQuery.trim().toLowerCase();
    const rows = entry.models
      .filter((model) => !query || model.name.toLowerCase().includes(query) || model.id.toLowerCase().includes(query))
      .slice(0, MAX_ROWS);
    if (!rows.length) {
      list.innerHTML = `<p class="cmenu-empty">${escapeHtml(
        entry.models.length ? t("copilot.noMatches") : t("copilot.noModels")
      )}</p>`;
      return;
    }
    list.innerHTML = rows
      .map((model) => {
        const isSelected = model.id === selected;
        return `<button type="button" class="cmenu-model${isSelected ? " is-selected" : ""}" role="option"
          aria-selected="${isSelected}" data-model="${escapeHtml(model.id)}">
          <span class="cmenu-model-name">${escapeHtml(model.name)}</span>
          <span class="cmenu-model-meta">
            ${model.free ? `<span class="cmenu-badge is-free">${escapeHtml(t("copilot.freeBadge"))}</span>` : ""}
            ${model.context_window ? `<span class="cmenu-badge">${escapeHtml(contextLabel(model.context_window))}</span>` : ""}
            ${isSelected ? icon("check", 15) : ""}
          </span>
        </button>`;
      })
      .join("");
    list.querySelectorAll("[data-model]").forEach((button) => {
      button.addEventListener("click", () => {
        const model = entry.models.find((item) => item.id === button.dataset.model);
        if (!model) return;
        close({ restoreFocus: false });
        options.onPickModel(model);
      });
    });
    list.querySelector(".is-selected")?.scrollIntoView({ block: "nearest" });
  }

  // ---------- shared behavior ----------

  attachButton.setAttribute("aria-haspopup", "menu");
  attachButton.setAttribute("aria-expanded", "false");
  modelButton.setAttribute("aria-haspopup", "dialog");
  modelButton.setAttribute("aria-expanded", "false");
  attachButton.addEventListener("click", () => open("attach"));
  modelButton.addEventListener("click", () => open("model"));

  document.addEventListener("pointerdown", (event) => {
    if (!current) return;
    if (menu.contains(event.target) || triggerFor(current).contains(event.target)) return;
    close({ restoreFocus: false });
  });

  menu.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (current === "attach" && view === "link") {
        view = "root";
        renderAttach();
      } else {
        close();
      }
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const items = [...menu.querySelectorAll(".cmenu-item, .cmenu-model")];
    if (!items.length) return;
    event.preventDefault();
    const at = items.indexOf(document.activeElement);
    const next = event.key === "ArrowDown" ? at + 1 : at - 1;
    items[(next + items.length) % items.length].focus();
  });

  // A new selection or a new key means the cached list may be stale.
  return {
    isOpen: () => current !== null,
    close,
    forgetModels(providerId) {
      if (providerId) modelCache.delete(providerId);
      else modelCache.clear();
    },
  };
}
