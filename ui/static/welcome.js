// First-run wizard: describe an idea, pick a template or open a ready example.
//
// A newcomer should reach a finished-looking deck in under a minute without
// meeting a settings screen. The AI path connects a provider in the same step
// (key in, model chosen automatically), then hands the topic to the copilot.

import { lang, t } from "./i18n.js";
import { trapFocus } from "./focus-trap.js";
import { FEATURED_PROVIDERS } from "./onboarding-models.js";
import { providerIcon, providerTint } from "./provider-icons.js";
import { THEME_COLORS } from "./layout-previews.js";
import { TEMPLATES, buildTemplate } from "./deck-templates.js";

export const WELCOMED_KEY = "slyde.welcomed";

const escapeHtml = (value) =>
  String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

export function hasBeenWelcomed() {
  try {
    return localStorage.getItem(WELCOMED_KEY) === "1";
  } catch {
    return true; // Without storage we cannot remember, so never nag.
  }
}

function markWelcomed() {
  try {
    localStorage.setItem(WELCOMED_KEY, "1");
  } catch {
    // Non-fatal: the wizard may simply show again.
  }
}

/** True when the open deck is still the untouched starter, so the wizard is welcome. */
export function isPristineDeck(deck) {
  return Boolean(deck) && deck.slides.length <= 1;
}

export function mountWelcome({ copilot, applyTemplate, getSize }) {
  let root = null;
  let release = null;
  let opener = null;
  let state = null;

  // Document-level so Escape works even while focus is between two renders.
  function onKeydown(event) {
    if (event.key === "Escape") close();
  }

  function close({ remember = true } = {}) {
    if (!root) return;
    document.removeEventListener("keydown", onKeydown);
    if (remember) markWelcomed();
    release?.();
    root.classList.add("is-leaving");
    const node = root;
    root = null;
    window.setTimeout(() => node.remove(), 180);
    opener?.focus?.();
  }

  function open(step = "home") {
    if (root) return;
    opener = document.activeElement;
    state = { step, topic: "", provider: FEATURED_PROVIDERS[0], key: "", showKey: false, busy: false, error: "", providers: [] };
    root = document.createElement("div");
    root.className = "welcome";
    root.innerHTML = `<div class="welcome-card" role="dialog" aria-modal="true" aria-labelledby="welcome-title" tabindex="-1"></div>`;
    document.body.append(root);
    const card = root.querySelector(".welcome-card");
    release = trapFocus(card);
    document.addEventListener("keydown", onKeydown);
    root.addEventListener("mousedown", (event) => {
      if (event.target === root) close();
    });
    render();
    copilot.providers().then((list) => {
      state.providers = list;
      if (state.step === "connect") render();
    });
  }

  function go(step) {
    state.step = step;
    state.error = "";
    render();
  }

  function render() {
    const card = root.querySelector(".welcome-card");
    const view = { home: renderHome, topic: renderTopic, connect: renderConnect, templates: renderTemplates }[state.step];
    card.innerHTML = `<button class="welcome-close" type="button" data-act="close" aria-label="${escapeHtml(t("welcome.skip"))}">×</button>${view()}`;
    card.dataset.step = state.step;
    card.querySelectorAll("[data-act]").forEach((element) =>
      element.addEventListener("click", () => handle(element.dataset.act, element))
    );
    const form = card.querySelector("form");
    form?.addEventListener("submit", (event) => {
      event.preventDefault();
      handle(form.dataset.submit);
    });
    card.querySelector("textarea")?.addEventListener("input", (event) => {
      state.topic = event.target.value;
    });
    card.querySelector("#welcome-key")?.addEventListener("input", (event) => {
      state.key = event.target.value;
    });
    const focusTarget = card.querySelector("[data-autofocus]") || card.querySelector("button:not(.welcome-close)");
    window.requestAnimationFrame(() => focusTarget?.focus());
  }

  function renderHome() {
    return `
      <div class="welcome-hero">
        <p class="welcome-eyebrow">${escapeHtml(t("welcome.eyebrow"))}</p>
        <h2 id="welcome-title">${escapeHtml(t("welcome.title"))}</h2>
        <p class="welcome-sub">${escapeHtml(t("welcome.subtitle"))}</p>
      </div>
      <div class="welcome-demo" aria-hidden="true">
        <div class="demo-slide">
          <span class="demo-line demo-eyebrow"></span>
          <span class="demo-line demo-title"></span>
          <span class="demo-line demo-title short"></span>
          <span class="demo-line demo-text"></span>
          <span class="demo-line demo-text mid"></span>
          <span class="demo-cursor"></span>
        </div>
      </div>
      <div class="welcome-options">
        <button class="welcome-option primary" type="button" data-act="idea" data-autofocus>
          <strong>${escapeHtml(t("welcome.optIdea"))}</strong>
          <span>${escapeHtml(t("welcome.optIdeaHint"))}</span>
        </button>
        <button class="welcome-option" type="button" data-act="templates">
          <strong>${escapeHtml(t("welcome.optTemplate"))}</strong>
          <span>${escapeHtml(t("welcome.optTemplateHint"))}</span>
        </button>
        <button class="welcome-option" type="button" data-act="example">
          <strong>${escapeHtml(t("welcome.optExample"))}</strong>
          <span>${escapeHtml(t("welcome.optExampleHint"))}</span>
        </button>
      </div>
      <button class="welcome-skip" type="button" data-act="close">${escapeHtml(t("welcome.skip"))}</button>`;
  }

  function renderTopic() {
    return `
      <button class="welcome-back" type="button" data-act="home">← ${escapeHtml(t("welcome.back"))}</button>
      <h2 id="welcome-title">${escapeHtml(t("welcome.topicTitle"))}</h2>
      <p class="welcome-sub">${escapeHtml(t("welcome.topicHint"))}</p>
      <form data-submit="send-topic">
        <label class="sr-only" for="welcome-topic">${escapeHtml(t("welcome.topicTitle"))}</label>
        <textarea id="welcome-topic" rows="4" maxlength="800" data-autofocus
          placeholder="${escapeHtml(t("welcome.topicPlaceholder"))}">${escapeHtml(state.topic)}</textarea>
        <div class="welcome-chips">
          ${["suggest1", "suggest2", "suggest3"]
            .map((key) => `<button type="button" class="welcome-chip" data-act="suggest" data-text="${escapeHtml(t(`welcome.${key}`))}">${escapeHtml(t(`welcome.${key}`))}</button>`)
            .join("")}
        </div>
        <button class="btn btn-primary welcome-go" type="submit">${escapeHtml(t("welcome.create"))}</button>
      </form>`;
  }

  function renderConnect() {
    const known = new Map(state.providers.map((provider) => [provider.id, provider]));
    const choices = FEATURED_PROVIDERS.filter((id) => known.has(id) || !state.providers.length);
    const provider = known.get(state.provider);
    const keyUrl = provider && /^https:\/\//.test(provider.key_url) ? provider.key_url : "";
    return `
      <button class="welcome-back" type="button" data-act="topic">← ${escapeHtml(t("welcome.back"))}</button>
      <h2 id="welcome-title">${escapeHtml(t("welcome.connectTitle"))}</h2>
      <p class="welcome-sub">${escapeHtml(t("welcome.connectHint"))}</p>
      <div class="welcome-providers" role="radiogroup" aria-label="${escapeHtml(t("welcome.provider"))}">
        ${choices
          .map((id) => {
            const info = known.get(id);
            const name = info ? info.name : id;
            const icon = info && info.icon ? info.icon : id;
            return `<button type="button" role="radio" aria-checked="${id === state.provider}" class="welcome-provider ${id === state.provider ? "is-on" : ""}"
              data-act="provider" data-id="${escapeHtml(id)}" style="--tint:${providerTint(icon)}">
              <span class="welcome-provider-icon" aria-hidden="true">${providerIcon(icon)}</span>${escapeHtml(name)}</button>`;
          })
          .join("")}
      </div>
      <form data-submit="send-key">
        <label class="sheet-label" for="welcome-key">${escapeHtml(t("copilot.apiKey"))}</label>
        <div class="key-row">
          <input id="welcome-key" class="sheet-input" type="${state.showKey ? "text" : "password"}" data-autofocus
            value="${escapeHtml(state.key)}" placeholder="${escapeHtml(t("copilot.keyPlaceholder"))}"
            autocomplete="off" spellcheck="false" maxlength="512">
          <button type="button" class="key-toggle" data-act="toggle-key">${escapeHtml(t(state.showKey ? "copilot.hideKey" : "copilot.showKey"))}</button>
        </div>
        ${state.error ? `<p class="sheet-error" role="alert">${escapeHtml(state.error)}</p>` : ""}
        <button class="btn btn-primary welcome-go" type="submit" ${state.busy ? "disabled" : ""}>
          ${escapeHtml(state.busy ? t("copilot.checking") : t("welcome.connectGo"))}
        </button>
      </form>
      <div class="key-meta">
        ${keyUrl ? `<a href="${escapeHtml(keyUrl)}" target="_blank" rel="noopener noreferrer" class="sheet-link">${escapeHtml(t("copilot.getKey"))} ↗</a>` : ""}
        <p class="sheet-fineprint">${escapeHtml(t("copilot.keyPrivacy"))}</p>
      </div>`;
  }

  function renderTemplates() {
    const code = lang();
    return `
      <button class="welcome-back" type="button" data-act="home">← ${escapeHtml(t("welcome.back"))}</button>
      <h2 id="welcome-title">${escapeHtml(t("welcome.templatesTitle"))}</h2>
      <p class="welcome-sub">${escapeHtml(t("welcome.templatesHint"))}</p>
      <div class="welcome-templates">
        ${TEMPLATES.map((template, index) => {
          const colors = THEME_COLORS[template.theme] || THEME_COLORS.studio;
          return `<button type="button" class="welcome-template" data-act="template" data-id="${template.id}" ${index === 0 ? "data-autofocus" : ""}>
            <span class="welcome-template-art" aria-hidden="true" style="background:${colors.bg}">
              <i style="background:${colors.accent}"></i><i style="background:${colors.fg}"></i><i style="background:${colors.fg};opacity:.45"></i>
            </span>
            <strong>${escapeHtml(template.name[code] || template.name.en)}</strong>
            <span>${escapeHtml(template.description[code] || template.description.en)}</span>
            <small>${escapeHtml(t("welcome.slidesCount", { n: template.slides.length }))}</small>
          </button>`;
        }).join("")}
      </div>`;
  }

  async function handle(action, element) {
    switch (action) {
      case "close":
        close();
        break;
      case "home":
      case "templates":
      case "topic":
        go(action);
        break;
      case "idea":
        go("topic");
        break;
      case "suggest":
        state.topic = element.dataset.text;
        render();
        break;
      case "provider":
        state.provider = element.dataset.id;
        state.key = "";
        render();
        break;
      case "toggle-key":
        state.showKey = !state.showKey;
        render();
        break;
      case "example":
        await useTemplate(TEMPLATES[0]);
        break;
      case "template":
        await useTemplate(TEMPLATES.find((template) => template.id === element.dataset.id));
        break;
      case "send-topic":
        submitTopic();
        break;
      case "send-key":
        await submitConnect();
        break;
      default:
        break;
    }
  }

  async function useTemplate(template) {
    if (!template) return;
    const ok = await applyTemplate(buildTemplate(template, lang(), getSize()));
    if (ok) close();
  }

  function submitTopic() {
    const topic = state.topic.trim();
    if (!topic) {
      root.querySelector("#welcome-topic")?.focus();
      return;
    }
    if (!copilot.isConnected()) {
      go("connect");
      return;
    }
    startCopilot(topic);
  }

  function startCopilot(topic) {
    close();
    copilot.ask(t("welcome.prompt", { topic }));
  }

  async function submitConnect() {
    const key = state.key.trim();
    if (!key) {
      state.error = t("copilot.keyRequired");
      render();
      return;
    }
    state.busy = true;
    state.error = "";
    render();
    try {
      await copilot.connect(state.provider, key);
      state.busy = false;
      startCopilot(state.topic.trim());
    } catch (error) {
      state.busy = false;
      const code = error.code || "generic";
      const known = ["invalid_key", "rate_limited", "network", "upstream"];
      state.error = known.includes(code) ? t(`copilot.error.${code}`) : t(code === "no_model" ? "welcome.noModel" : "copilot.error.generic");
      render();
    }
  }

  return {
    open,
    close,
    isOpen: () => Boolean(root),
  };
}
