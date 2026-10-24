import { createClient, readToken } from "./api.js";
import {
  idleActivity,
  nextTransition,
  receiveActivity,
  tickActivity,
} from "./agent-activity.js";
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
  let exportJob = null;
  let activity = idleActivity();
  let activityTimer = null;
  let lastInteractionAt = 0;
  let lastSheenKey = "";
  let sweepPending = false;
  let lastAnnouncedLabel = "";
  let chipHideTimer = null;

  const FOLLOW_IDLE_MS = 1800;

  $("app").hidden = false;
  syncThemeToggle();

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
    agentChip: $("agent-chip"),
    agentLabel: $("agent-label"),
    agentAnnouncer: $("agent-announcer"),
    stageScan: $("stage-scan"),
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

  // ---------- AI activity ----------

  function activityLabel(state) {
    if (!state) return "";
    switch (state.phase) {
      case "writing":
        if (state.action === "set_notes") return t("agent.notes");
        if (state.action === "upsert_slide") {
          return Number.isInteger(state.slideIndex)
            ? t("agent.writing", { n: state.slideIndex + 1 })
            : t("agent.writingGeneric");
        }
        return t("agent.writingGeneric");
      case "adjusting":
        if (state.action === "create_presentation") return t("agent.preparing");
        if (state.action === "remove_slide") return t("agent.removing");
        if (state.action === "move_slide") return t("agent.moving");
        return t("agent.adjusting");
      case "animating":
        return t("agent.animating");
      case "done":
        return state.action === "upsert_slide" ? t("agent.doneSlide") : t("agent.done");
      case "thinking":
        return t("agent.thinking");
      case "error":
        return t("agent.error");
      default:
        return "";
    }
  }

  function renderAgentActivity({ announce = true } = {}) {
    const idle = activity.phase === "idle";
    const label = idle ? "" : activityLabel(activity);

    // The chip exits with a short fade instead of disappearing between two
    // frames; a new active event during the fade cancels it.
    if (idle) {
      if (!els.agentChip.hidden) {
        if (!chipHideTimer) {
          els.agentChip.classList.add("leaving");
          chipHideTimer = window.setTimeout(() => {
            chipHideTimer = null;
            els.agentChip.hidden = true;
            els.agentChip.classList.remove("leaving");
            els.agentChip.dataset.phase = "idle";
            els.agentLabel.textContent = "";
          }, 180);
        }
      } else {
        els.agentChip.classList.remove("leaving");
        els.agentChip.dataset.phase = "idle";
        els.agentLabel.textContent = "";
      }
    } else {
      if (chipHideTimer) {
        window.clearTimeout(chipHideTimer);
        chipHideTimer = null;
      }
      els.agentChip.hidden = false;
      els.agentChip.classList.remove("leaving");
      els.agentChip.dataset.phase = activity.phase;
      els.agentLabel.textContent = label;
    }

    // Live announcements follow the visual phases, not the raw events: a
    // mutation is instant, but "Slide ready" should be heard when it appears.
    if (idle) {
      if (lastAnnouncedLabel) els.agentAnnouncer.textContent = "";
      lastAnnouncedLabel = "";
    } else if (activity.phase === "thinking") {
      // Reserved for a future explicit generation channel; inferred calls never
      // emit it, so it is not announced.
    } else if (!announce) {
      lastAnnouncedLabel = label;
    } else if (label && label !== lastAnnouncedLabel) {
      lastAnnouncedLabel = label;
      els.agentAnnouncer.textContent = label;
    }

    const writingSlide =
      activity.phase === "writing" && activity.action === "upsert_slide";
    const targetVisible =
      Boolean(activity.slideId) && activity.slideId === selectedId;
    const sheen = writingSlide && targetVisible;

    if (!sheen) {
      lastSheenKey = "";
      sweepPending = false;
      delete els.frameWrap.dataset.agentPhase;
      return;
    }

    els.frameWrap.dataset.agentPhase = "writing";
    const key = `${activity.slideId}:${activity.action}`;
    if (key !== lastSheenKey) {
      lastSheenKey = key;
      sweepPending = true;
    }
    maybeSweep();
  }

  function maybeSweep() {
    if (!sweepPending || els.frameWrap.hidden) return;
    sweepPending = false;
    els.stageScan.classList.remove("run");
    void els.stageScan.offsetWidth;
    els.stageScan.classList.add("run");
  }

  function scheduleActivityTick() {
    if (activityTimer) {
      window.clearTimeout(activityTimer);
      activityTimer = null;
    }
    const now = Date.now();
    const at = nextTransition(activity, now);
    if (!at) return;
    activityTimer = window.setTimeout(() => {
      activityTimer = null;
      activity = tickActivity(activity, Date.now());
      renderAgentActivity();
      scheduleActivityTick();
    }, Math.max(0, at - now) + 20);
  }

  function userIsEngaged() {
    if (Date.now() - lastInteractionAt < FOLLOW_IDLE_MS) return true;
    const el = document.activeElement;
    if (!el) return false;
    const tag = (el.tagName || "").toLowerCase();
    if (tag === "input" || tag === "textarea" || tag === "select" || el.isContentEditable) {
      return true;
    }
    // Never move the storyboard selection while keyboard focus is in it.
    return Boolean(el.closest(".rail"));
  }

  /// Follow the slide the agent wrote only when the user is watching, not
  /// editing. Otherwise the storyboard still updates, but selection is theirs.
  function followAgentSlide(event) {
    if (!event || event.phase !== "done") return;
    if (event.action !== "upsert_slide" || !event.slide_id) return;
    if (!view || !view.presentation.slides.some((s) => s.id === event.slide_id)) {
      return;
    }
    if (selectedId === event.slide_id || userIsEngaged()) return;
    selectedId = event.slide_id;
    stageKey = "";
    applyView(view);
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
    renderAgentActivity({ announce: false });
    maybeSweep();
  }

  /// Every thumb is a button; only the selected one is in the tab order, so
  /// Tab jumps straight into the deck and the arrow keys walk the slides.
  function syncStoryboardSelection() {
    els.storyboard.querySelectorAll(".thumb").forEach((el) => {
      const current = el.dataset.id === selectedId;
      el.setAttribute("aria-current", String(current));
      el.tabIndex = current ? 0 : -1;
    });
  }

  function focusSelectedThumb() {
    const target = [...els.storyboard.querySelectorAll(".thumb")].find(
      (el) => el.dataset.id === selectedId
    );
    if (target) target.focus();
  }

  function renderStoryboard(d) {
    const key = d.slides.map((s) => `${s.id}:${hash(s.code)}:${s.template}`).join("|") + `:${d.aspect_ratio}`;
    if (key === thumbsKey) {
      syncStoryboardSelection();
      return;
    }
    thumbsKey = key;

    const hadFocus = els.storyboard.contains(document.activeElement);
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
      const label = t("storyboard.slide", {
        n: index + 1,
        template: t(`template.${s.template}`),
      });
      const button = document.createElement("button");
      button.type = "button";
      button.className = "thumb";
      button.dataset.id = s.id;
      button.tabIndex = s.id === selectedId ? 0 : -1;
      button.setAttribute("aria-current", String(s.id === selectedId));
      button.setAttribute("aria-label", label);
      button.innerHTML = `
        <span class="thumb-media" style="aspect-ratio:${w}/${h}">
          <iframe sandbox="allow-scripts" loading="lazy" tabindex="-1" aria-hidden="true" title=""
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

    if (hadFocus) focusSelectedThumb();
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
      </section>
      <section class="field" id="export-section" aria-busy="false">
        <span class="field-label">${escapeHtml(t("export.section"))}</span>
        <div class="export-actions">
          <button type="button" class="file-btn" id="export-pdf">${escapeHtml(t("export.pdf"))}</button>
          <button type="button" class="file-btn" id="export-png">${escapeHtml(t("export.png"))}</button>
        </div>
        <p class="hint export-status" id="export-status" role="status" aria-live="polite">${escapeHtml(t("export.hint"))}</p>
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

    const exportPdf = $("export-pdf");
    if (exportPdf) exportPdf.addEventListener("click", () => startExport("pdf"));
    const exportPng = $("export-png");
    if (exportPng) exportPng.addEventListener("click", () => startExport("png"));
    syncExportUi();
  }

  /// Renders and packages the deck with headless Chrome. The HTTP call only
  /// resolves when the file is ready; progress events keep the label honest.
  async function startExport(format) {
    if (exportJob) return;
    const slides = deck();
    exportJob = {
      format,
      completed: 0,
      total: format === "pdf" ? 1 : slides ? slides.slides.length : 0,
    };
    syncExportUi();

    try {
      const { blob, filename } = await client.exportDeck(format);
      saveBlob(blob, filename || (format === "pdf" ? "deck.pdf" : "deck.zip"));
      toast(t(format === "pdf" ? "toast.exportedPdf" : "toast.exportedPng"));
    } catch (error) {
      console.error("export failed", error);
      toast(exportErrorText(error));
    } finally {
      exportJob = null;
      syncExportUi();
    }
  }

  function syncExportUi() {
    const section = $("export-section");
    if (!section) return;

    const busy = Boolean(exportJob);
    section.setAttribute("aria-busy", String(busy));
    ["export-pdf", "export-png"].forEach((id) => {
      const button = $(id);
      if (button) button.disabled = busy;
    });

    const status = $("export-status");
    if (!status) return;
    if (!busy) {
      status.textContent = t("export.hint");
    } else if (exportJob.format === "pdf" || exportJob.total < 1) {
      status.textContent = t("export.preparing");
    } else {
      const done = Math.min(exportJob.completed + 1, exportJob.total);
      status.textContent = t("export.progress", { done, total: exportJob.total });
    }
  }

  function exportErrorText(error) {
    if (error.code === "browser_not_found") return t("export.noBrowser");
    if (error.code === "empty_deck") return t("export.empty");
    return t("export.failed");
  }

  function saveBlob(blob, filename) {
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.append(link);
    link.click();
    link.remove();
    // Give the browser a beat to start the download before releasing the blob.
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  // ---------- events ----------

  function handleEvent(event) {
    if (event.type === "state_changed") {
      applyView(event.view);
      return;
    }
    if (event.type === "agent_activity") {
      activity = receiveActivity(activity, event, Date.now());
      followAgentSlide(event);
      renderAgentActivity();
      scheduleActivityTick();
      return;
    }
    if (event.type === "export_progress") {
      if (exportJob && exportJob.format === event.format) {
        exportJob.completed = event.completed;
        exportJob.total = event.total;
        syncExportUi();
      }
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

  function syncThemeToggle() {
    const dark = document.documentElement.dataset.theme === "dark";
    $("theme-toggle").setAttribute("aria-pressed", String(dark));
  }

  $("theme-toggle").addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("slyde.theme", next);
    syncThemeToggle();
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
    renderAgentActivity({ announce: false });
    if (view) setStatus(els.statusDot.dataset.state || "connecting");
  });

  // Auto-follow is only for people watching, not for people editing.
  document.addEventListener(
    "pointerdown",
    () => {
      lastInteractionAt = Date.now();
    },
    { capture: true }
  );
  document.addEventListener(
    "keydown",
    () => {
      lastInteractionAt = Date.now();
    },
    { capture: true }
  );

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
      const focusInside = els.storyboard.contains(document.activeElement);
      selectedId = d.slides[next].id;
      stageKey = "";
      applyView(view);
      if (focusInside) focusSelectedThumb();
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
      syncThemeToggle();
      applyI18n();
      client.connect({ onEvent: handleEvent, onStatus: setStatus });
    } catch (error) {
      showFatal(error.message);
    }
  })();
}
