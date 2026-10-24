import { createClient, readToken } from "./api.js";
import { apply as applyI18n, lang, onLangChange, setLang, t } from "./i18n.js";

const $ = (id) => document.getElementById(id);

const token = readToken();
if (!token) {
  $("fatal").hidden = false;
  applyI18n();
} else {
  boot(token);
}

function showFatal(message) {
  $("app").hidden = true;
  $("fatal").hidden = false;
  if (message) {
    const p = $("fatal").querySelector("p");
    if (p) p.textContent = message;
  }
}

function boot(sessionToken) {
  const client = createClient(sessionToken);

  let view = null;
  let sandboxOrigin = "";
  let selectedId = null;
  let stageKey = "";
  let thumbsKey = "";
  let pendingAnimations = [];
  let notesTimer = null;
  let titleTimer = null;
  let resizeTimer = null;

  $("app").hidden = false;

  const els = {
    title: $("deck-title"),
    storyboard: $("storyboard"),
    slideCount: $("slide-count"),
    frame: $("slide-frame"),
    frameWrap: $("stage-frame"),
    empty: $("empty"),
    chips: $("chips"),
    inspector: $("inspector"),
    revision: $("revision"),
    statusDot: $("status-dot"),
    statusText: $("status-text"),
    toast: $("toast"),
    undo: $("undo"),
    redo: $("redo"),
  };

  // ---------- helpers ----------

  const deck = () => (view ? view.presentation : null);
  const slide = () => {
    const d = deck();
    if (!d) return null;
    return d.slides.find((s) => s.id === selectedId) || null;
  };

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  function hash(text) {
    let h = 5381;
    for (let i = 0; i < text.length; i += 1) {
      h = ((h << 5) + h + text.charCodeAt(i)) | 0;
    }
    return h;
  }

  let toastTimer = null;
  function toast(message) {
    els.toast.textContent = message;
    els.toast.dataset.show = "true";
    if (toastTimer) window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      els.toast.dataset.show = "false";
    }, 1800);
  }

  async function run(action, extra, { silent = false } = {}) {
    try {
      const payload = await client.command(action, extra);
      applyView(payload.view);
      return true;
    } catch (error) {
      if (!silent) toast(`${t("toast.error")} — ${error.message}`);
      return false;
    }
  }

  function setStatus(state) {
    els.statusDot.dataset.state = state;
    const key = state === "online" ? "stage.ready" : state === "offline" ? "stage.offline" : "stage.connecting";
    els.statusText.textContent = t(key);
  }

  // ---------- rendering ----------

  function applyView(next) {
    view = next;
    const d = deck();
    if (!d) return;

    if (!selectedId || !d.slides.some((s) => s.id === selectedId)) {
      selectedId = d.slides.length ? d.slides[0].id : null;
    }

    if (document.activeElement !== els.title) {
      els.title.value = d.title;
      els.title.placeholder = t("deck.untitled");
    }

    els.slideCount.textContent = String(d.slides.length);
    els.revision.textContent = `rev ${view.revision}`;
    els.undo.disabled = !view.can_undo;
    els.redo.disabled = !view.can_redo;

    renderStoryboard(d);
    renderStage(d);
    renderChips(d);
    renderInspector(d);
  }

  function renderStoryboard(d) {
    const key = d.slides.map((s) => `${s.id}:${hash(s.code)}:${s.template}`).join("|") + `:${d.aspect_ratio}`;
    if (key === thumbsKey) {
      els.storyboard.querySelectorAll(".thumb").forEach((el) => {
        el.setAttribute("aria-current", String(el.dataset.id === selectedId));
      });
      return;
    }
    thumbsKey = key;

    const [w, h] = d.aspect_ratio.split(":").map(Number);

    els.storyboard.innerHTML = "";
    if (!d.slides.length) {
      const hint = document.createElement("p");
      hint.className = "hint";
      hint.style.padding = "8px 4px";
      hint.textContent = t("empty.body");
      els.storyboard.append(hint);
      return;
    }

    d.slides.forEach((s, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "thumb";
      button.dataset.id = s.id;
      button.setAttribute("role", "listitem");
      button.setAttribute("aria-current", String(s.id === selectedId));
      button.innerHTML = `
        <span class="thumb-media" style="aspect-ratio:${w}/${h}">
          <iframe sandbox="allow-scripts" loading="lazy" tabindex="-1"
            src="${sandboxOrigin}/s/${encodeURIComponent(s.id)}?static=1"></iframe>
        </span>
        <span class="thumb-foot">
          <span class="thumb-index">${String(index + 1).padStart(2, "0")}</span>
          <span class="thumb-template">${escapeHtml(t(`template.${s.template}`))}</span>
        </span>`;
      button.addEventListener("click", () => {
        selectedId = s.id;
        stageKey = "";
        applyView(view);
      });
      els.storyboard.append(button);
    });
  }

  function renderStage(d) {
    const current = slide();
    if (!current) {
      els.frameWrap.hidden = true;
      els.empty.hidden = false;
      return;
    }

    els.empty.hidden = true;
    els.frameWrap.hidden = false;

    const key = `${current.id}:${hash(current.code)}:${hash(JSON.stringify(current.animation))}:${d.aspect_ratio}`;
    if (key !== stageKey) {
      stageKey = key;
      els.frame.src = `${sandboxOrigin}/s/${encodeURIComponent(current.id)}?static=1`;
    }
    fitStage(d);
  }

  function fitStage(d) {
    if (!d || els.frameWrap.hidden) return;
    const [w, h] = d.aspect_ratio.split(":").map(Number);
    const pad = 64;
    const availableW = Math.max(160, $("stage").clientWidth - pad);
    const availableH = Math.max(90, $("stage").clientHeight - pad);
    let width = availableW;
    let height = (width * h) / w;
    if (height > availableH) {
      height = availableH;
      width = (height * w) / h;
    }
    els.frameWrap.style.width = `${Math.round(width)}px`;
    els.frameWrap.style.height = `${Math.round(height)}px`;
  }

  function renderChips(d) {
    const current = slide();
    els.chips.innerHTML = "";
    if (!current) return;
    if (!current.animation.length) {
      const hint = document.createElement("span");
      hint.className = "hint";
      hint.textContent = t("inspector.noAnimation");
      els.chips.append(hint);
      return;
    }
    current.animation.forEach((step, index) => {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "chip";
      chip.innerHTML = `<span class="chip-dot"></span>${escapeHtml(step.kind)} · ${escapeHtml(step.target)}`;
      chip.addEventListener("click", () => run("trigger_animation", { slide_id: current.id, step_index: index }));
      els.chips.append(chip);
    });
  }

  function renderInspector(d) {
    const active = document.activeElement;
    const notesFocused = active && active.classList && active.classList.contains("notes");

    const current = slide();
    const themeOptions = ["studio", "paper", "night"]
      .map((id) => `<option value="${id}" ${d.theme.id === id ? "selected" : ""}>${escapeHtml(t(`theme.${id}`))}</option>`)
      .join("");
    const ratioOptions = ["16:9", "4:3", "9:16"]
      .map((ratio) => `<option value="${ratio}" ${d.aspect_ratio === ratio ? "selected" : ""}>${ratio}</option>`)
      .join("");

    const deckSection = `
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.deck"))}</span>
        <label class="field">
          <span class="hint">${escapeHtml(t("inspector.theme"))}</span>
          <select class="select" id="theme-select">${themeOptions}</select>
        </label>
        <label class="field">
          <span class="hint">${escapeHtml(t("inspector.aspect"))}</span>
          <select class="select" id="ratio-select">${ratioOptions}</select>
        </label>
      </section>
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.file"))}</span>
        <div class="file-actions">
          <button type="button" class="file-btn" id="save-deck">${escapeHtml(t("inspector.saveCopy"))}</button>
          <button type="button" class="file-btn" id="open-deck">${escapeHtml(t("inspector.open"))}</button>
        </div>
        <p class="hint">${escapeHtml(t("inspector.autosave"))}</p>
        <input type="file" id="deck-file" accept=".slyde,application/json" hidden>
      </section>`;

    if (!current) {
      els.inspector.innerHTML = deckSection;
      bindDeckControls();
      applyI18n(els.inspector);
      return;
    }

    const steps = current.animation.length
      ? `<div class="steps">${current.animation
          .map(
            (step, index) => `
          <div class="step-row">
            <span class="step-index">${index + 1}</span>
            <span class="step-name">${escapeHtml(step.kind)}
              <span class="step-target">${escapeHtml(step.target)}</span>
            </span>
            <button type="button" class="step-play" data-play="${index}">${escapeHtml(t("action.play"))}</button>
          </div>`
          )
          .join("")}</div>`
      : `<p class="hint">${escapeHtml(t("inspector.noAnimation"))}</p>`;

    els.inspector.innerHTML = `
      ${deckSection}
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.slide"))}</span>
        <div class="field-value">${escapeHtml(t(`template.${current.template}`))}</div>
        <div class="mono" title="${escapeHtml(current.id)}">${escapeHtml(current.id)}</div>
      </section>
      <section class="field">
        <label class="field-label" for="notes">${escapeHtml(t("inspector.notes"))}</label>
        <textarea class="notes" id="notes" placeholder="${escapeHtml(t("inspector.notesPlaceholder"))}">${escapeHtml(current.notes || "")}</textarea>
      </section>
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.animation"))}</span>
        ${steps}
      </section>
      <button type="button" class="danger-btn" id="delete-slide">${escapeHtml(t("action.delete"))}</button>
      <details class="dev">
        <summary>${escapeHtml(t("inspector.developer"))}</summary>
        <p class="hint">${escapeHtml(t("inspector.developerHint"))}</p>
        <pre class="dev-code">${escapeHtml(current.code)}</pre>
      </details>`;

    bindDeckControls();

    const notes = $("notes");
    if (notes) {
      notes.addEventListener("input", () => {
        if (notesTimer) window.clearTimeout(notesTimer);
        notesTimer = window.setTimeout(() => {
          run("set_notes", { slide_id: current.id, notes: notes.value || null }, { silent: true });
        }, 450);
      });
    }

    els.inspector.querySelectorAll("[data-play]").forEach((button) => {
      button.addEventListener("click", () => {
        const stepIndex = Number(button.dataset.play);
        run("trigger_animation", { slide_id: current.id, step_index: stepIndex });
      });
    });

    const remove = $("delete-slide");
    if (remove) {
      remove.addEventListener("click", async () => {
        const ok = await run("remove_slide", { slide_id: current.id }, { silent: true });
        if (ok) {
          selectedId = null;
          stageKey = "";
          applyView(view);
        }
      });
    }

    if (notesFocused && notes) {
      notes.focus();
    }
    applyI18n(els.inspector);
  }

  function bindDeckControls() {
    const theme = $("theme-select");
    if (theme) theme.addEventListener("change", () => run("set_theme", { theme_id: theme.value }));

    const ratio = $("ratio-select");
    if (ratio) {
      ratio.addEventListener("change", () => {
        stageKey = "";
        thumbsKey = "";
        run("set_aspect_ratio", { ratio: ratio.value });
      });
    }

    const save = $("save-deck");
    if (save) {
      save.addEventListener("click", () => {
        // A download link is the browser-native "save as": it lands in the
        // user's downloads folder with the deck name, no extension prompt.
        const link = document.createElement("a");
        link.href = `/api/deck?t=${encodeURIComponent(sessionToken)}`;
        link.download = "";
        document.body.append(link);
        link.click();
        link.remove();
        toast(t("toast.downloaded"));
      });
    }

    const open = $("open-deck");
    const file = $("deck-file");
    if (open && file) {
      open.addEventListener("click", () => file.click());
      file.addEventListener("change", async () => {
        const chosen = file.files && file.files[0];
        if (!chosen) return;
        try {
          const payload = await client.openDeck(await chosen.text());
          selectedId = null;
          stageKey = "";
          thumbsKey = "";
          applyView(payload.view);
          toast(t("toast.opened"));
        } catch (error) {
          toast(`${t("toast.error")} — ${error.message}`);
        }
        file.value = "";
      });
    }
  }

  // ---------- events ----------

  function handleEvent(event) {
    if (event.type === "state_changed") {
      applyView(event.view);
      return;
    }
    if (event.type === "animation") {
      const payload = { __slyde: true, type: "animate", step: event.step };
      if (!els.frame.contentWindow) {
        pendingAnimations.push(payload);
        return;
      }
      els.frame.contentWindow.postMessage(payload, "*");
    }
  }

  els.frame.addEventListener("load", () => {
    while (pendingAnimations.length) {
      const payload = pendingAnimations.shift();
      if (els.frame.contentWindow) els.frame.contentWindow.postMessage(payload, "*");
    }
  });

  $("present").addEventListener("click", () => {
    window.open(`/present?t=${encodeURIComponent(sessionToken)}`, "slyde-present", "width=1440,height=900");
  });

  $("add-slide").addEventListener("click", () => run("add_slide", { template: "hero" }));
  $("empty-cta").addEventListener("click", () => run("add_slide", { template: "hero" }));

  els.undo.addEventListener("click", () => run("undo"));
  els.redo.addEventListener("click", () => run("redo"));

  els.title.addEventListener("input", () => {
    if (titleTimer) window.clearTimeout(titleTimer);
    const value = els.title.value;
    titleTimer = window.setTimeout(() => run("set_title", { title: value }, { silent: true }), 400);
  });

  $("theme-toggle").addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("slyde.theme", next);
  });

  $("lang-switch").querySelectorAll("[data-lang]").forEach((button) => {
    button.addEventListener("click", () => setLang(button.dataset.lang));
  });

  function syncLangButtons() {
    $("lang-switch").querySelectorAll("[data-lang]").forEach((button) => {
      button.setAttribute("aria-pressed", String(button.dataset.lang === lang()));
    });
  }

  onLangChange(() => {
    syncLangButtons();
    applyI18n();
    if (view) {
      thumbsKey = "";
      applyView(view);
    }
    if (view) setStatus(els.statusDot.dataset.state || "connecting");
  });

  document.addEventListener("keydown", (event) => {
    const tag = (event.target.tagName || "").toLowerCase();
    const typing = tag === "input" || tag === "textarea" || tag === "select";
    const mod = event.metaKey || event.ctrlKey;

    if (mod && event.key.toLowerCase() === "z" && !typing) {
      event.preventDefault();
      run(event.shiftKey ? "redo" : "undo");
      return;
    }
    if (typing) return;

    if (event.key === "ArrowDown" || event.key === "ArrowRight") {
      event.preventDefault();
      moveSelection(1);
    } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
      event.preventDefault();
      moveSelection(-1);
    }
  });

  function moveSelection(delta) {
    const d = deck();
    if (!d || !d.slides.length) return;
    const index = d.slides.findIndex((s) => s.id === selectedId);
    const next = Math.min(d.slides.length - 1, Math.max(0, (index < 0 ? 0 : index) + delta));
    if (d.slides[next].id !== selectedId) {
      selectedId = d.slides[next].id;
      stageKey = "";
      applyView(view);
    }
  }

  window.addEventListener("resize", () => {
    if (resizeTimer) window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => {
      thumbsKey = "";
      if (view) applyView(view);
    }, 120);
  });

  // ---------- start ----------

  (async () => {
    try {
      const payload = await client.getState();
      sandboxOrigin = payload.sandbox_origin;
      setStatus("connecting");
      applyView(payload.view);
      syncLangButtons();
      applyI18n();
      client.connect({ onEvent: handleEvent, onStatus: setStatus });
    } catch (error) {
      showFatal(error.message);
    }
  })();
}
