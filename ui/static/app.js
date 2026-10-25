import { createClient, readToken } from "./api.js";
import {
  idleActivity,
  nextTransition,
  receiveActivity,
  tickActivity,
} from "./agent-activity.js";
import { mountCopilot } from "./copilot.js";
import { mountCommandPalette } from "./command-palette.js";
import { mountDeckLibrary } from "./deck-library.js";
import { createReveal, keyToAction, progressPercent } from "./presenter-core.js";
import { mountPresenterDraw } from "./presenter-draw.js";
import { ANIMATION_KINDS, DEFAULT_EASING, THEME_CHOICES, createInspector } from "./inspector.js";
import { createStoryboard } from "./storyboard.js";
import { createCanvas } from "./canvas.js";
import { apply as applyI18n, lang, onLangChange, setLang, t } from "./i18n.js";
import { formatClock, paceState, readingSeconds } from "./teleprompter.js";

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
  // Authorizes slide documents only. It is embedded in every iframe URL, so a
  // slide's own script can read it; the session token stays out of that document.
  let sandboxToken = "";
  // Active deck id: keys the copilot conversation and the library actions.
  let activeDeckId = null;
  /// `&t=…` suffix appended to every slide URL (`""` when the server sent none).
  let sourceQuery = "";
  let selectedId = null;
  let stageKey = "";
  let pendingAnimations = [];
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
    saveStatus: $("save-status"),
    saveDot: $("save-dot"),
    saveLabel: $("save-label"),
    zoomOut: $("zoom-out"),
    zoomIn: $("zoom-in"),
    zoomValue: $("zoom-value"),
    exportMenuTrigger: $("export-menu-trigger"),
    exportMenu: $("export-menu"),
    topbarExportPdf: $("topbar-export-pdf"),
    topbarExportPng: $("topbar-export-png"),
    topbarExportPptx: $("topbar-export-pptx"),
    topbarSaveDeck: $("topbar-save-deck"),
    topbarNewDeck: $("topbar-new-deck"),
    topbarLibrary: $("topbar-library"),
    topbarOpenDeck: $("topbar-open-deck"),
    topbarSaveDeckBtn: $("topbar-save-deck-btn"),
    topbarFilePicker: $("topbar-file-picker"),
    present: $("present"),
    presentMenuTrigger: $("present-menu-trigger"),
    presentMenu: $("present-menu"),
    presentFullscreen: $("present-fullscreen"),
    presentSpeaker: $("present-speaker"),
    presentWindow: $("present-window"),
    storyboard: $("storyboard"),
    slideCount: $("slide-count"),
    templatePopover: $("template-popover"),
    templateGrid: $("template-grid"),
    closeTemplatePopover: $("close-template-popover"),
    frame: $("slide-frame"),
    frameWrap: $("stage-frame"),
    empty: $("empty"),
    chips: $("chips"),
    subtabSlide: $("subtab-slide"),
    subtabDeck: $("subtab-deck"),
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
    copilotContextPill: $("copilot-context-pill"),
    copilotPromptPills: $("copilot-prompt-pills"),
  };

  const ZOOM_LEVELS = ["fit", 0.5, 0.75, 1.0, 1.25, 1.5];
  let currentZoomIdx = 0;
  let saveTimer = null;

  let copilot = null;
  let palette = null;
  let library = null;
  let activePanel = "inspector";

  // ---------- Command palette ----------

  // The theme list is imported from `inspector.js`: the palette and the inspector
  // must offer exactly the same decks themes, and `theme.<id>` must exist in both
  // locales.
  const PALETTE_THEMES = THEME_CHOICES.map((choice) => choice.id);
  const PALETTE_RATIOS = ["16:9", "4:3", "9:16"];

  function openPalette(scope = {}) {
    if (palette) palette.open(scope);
  }

  function askCopilotForElement(text) {
    setPanel("copilot");
    if (copilot && typeof copilot.ask === "function") {
      copilot.ask(text);
      return;
    }
    const input = $("copilot-input");
    if (input) {
      input.value = text;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.focus();
    }
  }

  function buildPaletteActions(scope = {}) {
    const element = scope.element || canvas.selectedElement;
    const d = deck();
    const actions = [];

    if (element) {
      ["concise", "bullets", "technical", "icon"].forEach((key) => {
        actions.push({
          id: `element.ai.${key}`,
          group: t("palette.group.element"),
          label: t(`element.ai.${key}`),
          hint: `#${element.id}`,
          run: () => askCopilotForElement(t(`element.ai.${key}`)),
        });
      });
      actions.push({
        id: "element.deselect",
        group: t("palette.group.element"),
        label: t("elementHud.deselect"),
        hint: "esc",
        run: deselectElement,
      });
    }

    actions.push(
      {
        id: "deck.library",
        group: t("palette.group.deck"),
        label: t("action.library"),
        hint: "L",
        run: () => openLibrary(),
      },
      {
        id: "deck.new",
        group: t("palette.group.deck"),
        label: t("library.new"),
        run: () => els.topbarNewDeck?.click(),
      },
      {
        id: "present",
        group: t("palette.group.present"),
        label: t("action.fullscreen"),
        hint: "f",
        run: () => openPresentationInStudio(false),
      },
      {
        id: "present.speaker",
        group: t("palette.group.present"),
        label: t("action.speakerConsole"),
        hint: "s",
        run: () => openPresentationInStudio(true),
      },
      {
        id: "present.window",
        group: t("palette.group.present"),
        label: t("action.presentWindow"),
        run: () => els.presentWindow.click(),
      }
    );

    if (d && d.slides.length) {
      actions.push(
        {
          id: "export.pdf",
          group: t("palette.group.export"),
          label: t("export.asPdf"),
          hint: "pdf",
          run: () => els.topbarExportPdf.click(),
        },
        {
          id: "export.png",
          group: t("palette.group.export"),
          label: t("export.asPng"),
          hint: "zip",
          run: () => els.topbarExportPng.click(),
        },
        {
          id: "export.slyde",
          group: t("palette.group.export"),
          label: t("export.asSlyde"),
          hint: ".slyde",
          run: () => els.topbarSaveDeck.click(),
        }
      );
    }

    TEMPLATES.forEach((template) => {
      actions.push({
        id: `slide.add.${template.id}`,
        group: t("palette.group.slide"),
        label: t(`template.${template.id}`),
        hint: `+ ${template.icon}`,
        run: () => run("add_slide", { template: template.id }),
      });
    });

    PALETTE_THEMES.forEach((themeId) => {
      actions.push({
        id: `theme.${themeId}`,
        group: t("palette.group.view"),
        label: t(`theme.${themeId}`),
        hint: t("palette.hint.theme"),
        run: () => run("set_theme", { theme_id: themeId }),
      });
    });

    PALETTE_RATIOS.forEach((ratio) => {
      actions.push({
        id: `ratio.${ratio}`,
        group: t("palette.group.view"),
        label: t("palette.ratio", { ratio }),
        hint: t("palette.hint.ratio"),
        run: () => run("set_aspect_ratio", { ratio }),
      });
    });

    actions.push(
      {
        id: "undo",
        group: t("palette.group.view"),
        label: t("action.undo"),
        hint: "mod+z",
        run: () => run("undo"),
      },
      {
        id: "redo",
        group: t("palette.group.view"),
        label: t("action.redo"),
        hint: "mod+shift+z",
        run: () => run("redo"),
      },
      {
        id: "theme.toggle",
        group: t("palette.group.view"),
        label: t("action.toggleTheme"),
        run: () => $("theme-toggle").click(),
      },
      {
        id: "lang.en",
        group: t("palette.group.view"),
        label: "English",
        hint: "en",
        run: () => setLang("en"),
      },
      {
        id: "lang.es",
        group: t("palette.group.view"),
        label: "Español",
        hint: "es",
        run: () => setLang("es"),
      }
    );

    return actions;
  }

  function setPanel(name, { focus = false } = {}) {
    activePanel = name === "copilot" ? "copilot" : "inspector";
    const copilotActive = activePanel === "copilot";
    const tabs = [
      ["inspector", $("tab-inspector")],
      ["copilot", $("tab-copilot")],
    ];
    tabs.forEach(([id, tab]) => {
      const selected = id === activePanel;
      tab.setAttribute("aria-selected", String(selected));
      tab.tabIndex = selected ? 0 : -1;
    });
    $("inspector-panel").hidden = copilotActive;
    $("copilot-panel").hidden = !copilotActive;
    if (focus) {
      $(copilotActive ? "tab-copilot" : "tab-inspector").focus();
    }
  }

  $("tab-inspector").addEventListener("click", () => setPanel("inspector"));
  $("tab-copilot").addEventListener("click", () => setPanel("copilot"));
  $("right-panel").querySelector('[role="tablist"]').addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const next =
      event.key === "Home"
        ? "inspector"
        : event.key === "End"
          ? "copilot"
          : activePanel === "inspector"
            ? "copilot"
            : "inspector";
    setPanel(next, { focus: true });
  });

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

  function markSaving() {
    if (els.saveStatus) {
      els.saveStatus.dataset.state = "saving";
      if (els.saveLabel) els.saveLabel.textContent = t("deck.saving");
    }
  }

  function markSaved() {
    if (saveTimer) window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      if (els.saveStatus) {
        els.saveStatus.dataset.state = "saved";
        if (els.saveLabel) els.saveLabel.textContent = t("deck.saved");
      }
    }, 350);
  }

  async function run(action, extra, { silent = false } = {}) {
    markSaving();
    try {
      const payload = await client.command(action, extra);
      applyView(payload.view);
      markSaved();
      return true;
    } catch (error) {
      markSaved();
      if (!silent) toast(`${t("toast.error")} — ${error.message}`);
      return false;
    }
  }

  let panOffset = { x: 0, y: 0 };
  let spacePressed = false;
  let isPanning = false;
  let panStart = { x: 0, y: 0 };

  function setZoom(delta) {
    if (delta === 0) {
      currentZoomIdx = 0;
      panOffset = { x: 0, y: 0 };
    } else {
      currentZoomIdx = Math.max(0, Math.min(ZOOM_LEVELS.length - 1, currentZoomIdx + delta));
    }
    const val = ZOOM_LEVELS[currentZoomIdx];
    if (els.zoomValue) {
      els.zoomValue.textContent = val === "fit" ? t("zoom.fit") : `${Math.round(val * 100)}%`;
    }
    const d = deck();
    if (d) fitStage(d);
  }

  if (els.zoomOut) els.zoomOut.addEventListener("click", () => setZoom(-1));
  if (els.zoomIn) els.zoomIn.addEventListener("click", () => setZoom(1));
  if (els.zoomValue) els.zoomValue.addEventListener("click", () => setZoom(0));

  // Canvas pan with Space + Drag, and continuous zoom with Ctrl + Wheel.
  const stageEl = $("stage");
  if (stageEl) {
    document.addEventListener("keydown", (e) => {
      const tag = (e.target.tagName || "").toLowerCase();
      if (tag === "input" || tag === "textarea" || tag === "select" || e.target.isContentEditable) return;
      if (e.code === "Space" && !spacePressed) {
        spacePressed = true;
        stageEl.classList.add("is-space-pan");
      }
    });
    document.addEventListener("keyup", (e) => {
      if (e.code === "Space") {
        spacePressed = false;
        isPanning = false;
        stageEl.classList.remove("is-space-pan", "is-panning");
      }
    });
    stageEl.addEventListener("pointerdown", (e) => {
      if (!spacePressed || e.button !== 0) return;
      isPanning = true;
      panStart = { x: e.clientX - panOffset.x, y: e.clientY - panOffset.y };
      stageEl.classList.add("is-panning");
      stageEl.setPointerCapture?.(e.pointerId);
      e.preventDefault();
    });
    window.addEventListener("pointermove", (e) => {
      if (!isPanning) return;
      panOffset = { x: e.clientX - panStart.x, y: e.clientY - panStart.y };
      const d = deck();
      if (d) fitStage(d);
    });
    const endPan = () => {
      if (!isPanning) return;
      isPanning = false;
      stageEl.classList.remove("is-panning");
    };
    window.addEventListener("pointerup", endPan);
    window.addEventListener("pointercancel", endPan);

    stageEl.addEventListener("wheel", (e) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      setZoom(e.deltaY < 0 ? 1 : -1);
    }, { passive: false });
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
    updateCopilotContext(d);
    maybeSweep();
  }

  function updateCopilotContext(d) {
    if (!els.copilotContextPill || !els.copilotPromptPills) return;
    const current = slide();
    const idx = current ? d.slides.findIndex((s) => s.id === current.id) : -1;
    if (canvas.selectedElement) {
      els.copilotContextPill.textContent = t("copilot.contextElement", {
        id: canvas.selectedElement.id,
      });
      els.copilotContextPill.dataset.target = "element";
    } else if (current && idx >= 0) {
      els.copilotContextPill.textContent = t("copilot.contextSlide", {
        n: idx + 1,
        template: t(`template.${current.template}`),
      });
      delete els.copilotContextPill.dataset.target;
    } else {
      els.copilotContextPill.textContent = t("copilot.contextDeck", {
        n: d.slides.length,
      });
      delete els.copilotContextPill.dataset.target;
    }

    const suggestions = [
      t("copilot.prompt.summarize"),
      t("copilot.prompt.action"),
      t("copilot.prompt.animate"),
      t("copilot.prompt.punchline"),
    ];

    els.copilotPromptPills.innerHTML = "";
    suggestions.forEach((text) => {
      const pill = document.createElement("button");
      pill.type = "button";
      pill.className = "prompt-pill";
      pill.textContent = text;
      pill.addEventListener("click", () => {
        const input = $("copilot-input");
        if (input) {
          input.value = text;
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.focus();
        }
      });
      els.copilotPromptPills.append(pill);
    });
  }

  /// The storyboard is its own module; the shell only hands it the slides.
  const storyboard = createStoryboard({
    root: els.storyboard,
    t,
    escapeHtml,
    hash,
    actions: {
      command: run,
      applyI18n,
      selectSlide: (slideId) => {
        selectedId = slideId;
        stageKey = "";
        applyView(view);
      },
    },
  });

  function renderStoryboard(d) {
    storyboard.render({
      slides: d.slides,
      aspectRatio: d.aspect_ratio,
      selectedId,
      sandboxOrigin,
      sourceQuery,
    });
  }

  function focusSelectedThumb() {
    storyboard.focusSelected();
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
      canvas.resetStructure();
      els.frame.src = `${sandboxOrigin}/s/${encodeURIComponent(current.id)}?static=1&edit=1${sourceQuery}`;
    }
    fitStage(d);
  }

  function fitStage(d) {
    if (!d || els.frameWrap.hidden) return;
    const [w, h] = d.aspect_ratio.split(":").map(Number);
    const zoom = ZOOM_LEVELS[currentZoomIdx];
    const pad = 64;
    const availableW = Math.max(160, $("stage").clientWidth - pad);
    const availableH = Math.max(90, $("stage").clientHeight - pad);
    let baseW = availableW;
    let baseH = (baseW * h) / w;
    if (baseH > availableH) {
      baseH = availableH;
      baseW = (baseH * w) / h;
    }
    const scale = zoom === "fit" ? 1 : zoom;
    const width = baseW * scale;
    const height = baseH * scale;
    els.frameWrap.style.width = `${Math.round(width)}px`;
    els.frameWrap.style.height = `${Math.round(height)}px`;
    els.frameWrap.style.transform = `translate(${Math.round(panOffset.x)}px, ${Math.round(panOffset.y)}px)`;
  }

  // ---------- Bimodal canvas ----------
  // Selection, HUD and image editing live in `canvas.js`; the shell supplies the
  // commands and reacts to selection/structure changes.

  const canvas = createCanvas({
    frame: els.frame,
    t,
    actions: {
      command: run,
      uploadImage: (dataUrl) => client.uploadAsset(dataUrl),
      notify: toast,
      currentSlideId: () => selectedId,
      aspectRatio: () => (deck() ? deck().aspect_ratio : "16:9"),
      openPaletteForElement: (element) => openPalette({ element }),
      onSelectedElement: () => {
        if (view) updateCopilotContext(view.presentation);
      },
      onStructure: () => {
        if (view && inspector.subtab === "slide") renderInspector(view.presentation);
      },
      applyI18n,
    },
  });
  canvas.attach();

  function selectElementById(id) {
    canvas.selectById(id);
  }

  function deselectElement() {
    canvas.clearSelection();
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

  // ---------- Inspector ----------
  // The panel lives in `inspector.js`; the studio supplies the commands and the
  // state it renders from. Keeping the ids means the export progress handler and
  // the E2E suite keep working unchanged.

  const inspector = createInspector({
    root: els.inspector,
    t,
    escapeHtml,
    actions: {
      command: run,
      selectElement: selectElementById,
      redesignSlide: () => {
        askCopilotForElement(t("copilot.prompt.redesign") || "Redesign this slide with an editorial, highly balanced layout while preserving the content.");
      },
      pickImage: (file) => canvas.insertFile(file),
      saveDeck: () => {
        const link = document.createElement("a");
        link.href = `/api/deck?t=${encodeURIComponent(sessionToken)}`;
        link.download = "";
        document.body.append(link);
        link.click();
        link.remove();
        toast(t("toast.downloaded"));
      },
      openDeckFile: async (text) => {
        try {
          const payload = await client.openDeck(text);
          selectedId = null;
          stageKey = "";
          storyboard.invalidate();
          applyView(payload.view);
          toast(t("toast.opened"));
        } catch (error) {
          toast(`${t("toast.error")} — ${error.message}`);
        }
      },
      exportDeck: (format) => startExport(format),
      removeSlide: async (slideId) => {
        const ok = await run("remove_slide", { slide_id: slideId }, { silent: true });
        if (ok) {
          selectedId = null;
          stageKey = "";
          applyView(view);
        }
      },
      aspectRatioChanged: (ratio) => {
        stageKey = "";
        storyboard.invalidate();
        run("set_aspect_ratio", { ratio });
      },
      applyI18n,
      onEmptyDeck: () => {
        if (els.subtabSlide) els.subtabSlide.disabled = true;
        if (els.subtabDeck) {
          els.subtabDeck.classList.add("active");
          els.subtabDeck.setAttribute("aria-selected", "true");
        }
      },
      onSubtabs: (showSlide) => {
        if (els.subtabSlide) {
          els.subtabSlide.disabled = false;
          els.subtabSlide.classList.toggle("active", showSlide);
          els.subtabSlide.setAttribute("aria-selected", String(showSlide));
        }
        if (els.subtabDeck) {
          els.subtabDeck.classList.toggle("active", !showSlide);
          els.subtabDeck.setAttribute("aria-selected", String(!showSlide));
        }
      },
    },
  });

  /// Renders the inspector from the current read model.
  function renderInspector(d) {
    inspector.render({
      deck: d,
      slide: slide(),
      selectedElement: canvas.selectedElement,
      structureNodes: canvas.structureNodes,
    });
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
      const fallbackName =
        format === "pdf" ? "deck.pdf" : format === "pptx" ? "deck.pptx" : "deck.zip";
      saveBlob(blob, filename || fallbackName);
      const readyKey =
        format === "pdf"
          ? "toast.exportedPdf"
          : format === "pptx"
            ? "toast.exportedPptx"
            : "toast.exportedPng";
      toast(t(readyKey));
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
    ["export-pdf", "export-png", "export-pptx"].forEach((id) => {
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


  $("add-slide").addEventListener("click", () => run("add_slide", { template: "hero" }));
  $("empty-cta").addEventListener("click", () => run("add_slide", { template: "hero" }));
  $("empty-copilot").addEventListener("click", () => {
    setPanel("copilot");
    if (copilot) copilot.focus();
  });

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
    if (canvas.selectedElement) canvas.refreshHud();
    if (view) {
      storyboard.invalidate();
      applyView(view);
    }
    renderAgentActivity({ announce: false });
    if (view) setStatus(els.statusDot.dataset.state || "connecting");
    if (library) library.setI18n(applyI18n);
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

  let slideClipboard = null;

  document.addEventListener("keydown", (event) => {
    const tag = (event.target.tagName || "").toLowerCase();
    const typing =
      tag === "input" ||
      tag === "textarea" ||
      tag === "select" ||
      (event.target && event.target.isContentEditable);
    const mod = event.metaKey || event.ctrlKey;
    const key = event.key.toLowerCase();

    // 1. Undo / Redo (never trapped by typing inside regular buttons/storyboard).
    if (mod && key === "z" && !typing) {
      event.preventDefault();
      run(event.shiftKey ? "redo" : "undo");
      return;
    }

    // 2. Presenter launches (F5 / Shift+F5 or Mod+Enter / Mod+Shift+Enter).
    if (!typing && (event.key === "F5" || (mod && (event.key === "Enter" || key === "p")))) {
      event.preventDefault();
      const speaker = event.shiftKey || event.altKey;
      openPresentationInStudio(speaker);
      return;
    }

    if (typing) return;

    // 3. Slide navigation.
    if (event.key === "ArrowDown" || event.key === "ArrowRight") {
      event.preventDefault();
      moveSelection(1);
      return;
    }
    if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
      event.preventDefault();
      moveSelection(-1);
      return;
    }

    // 4. Duplicate (Cmd+D / Ctrl+D).
    if (mod && key === "d" && selectedId) {
      event.preventDefault();
      run("duplicate_slide", { slide_id: selectedId });
      toast(t("toast.duplicated") || t("storyboard.duplicate"));
      return;
    }

    // 5. Delete (Delete or Backspace when not typing). A selected canvas block
    // owns the key: removing the whole slide from under it would be a surprise.
    if ((event.key === "Delete" || event.key === "Backspace") && selectedId && !mod) {
      if (canvas.selectedElement) return;
      const d = deck();
      if (d && d.slides.length > 1) {
        event.preventDefault();
        const idToDelete = selectedId;
        run("remove_slide", { slide_id: idToDelete }, { silent: true }).then((ok) => {
          if (ok) {
            selectedId = null;
            stageKey = "";
            applyView(view);
            toast(t("toast.deleted") || t("action.delete"));
          }
        });
      }
      return;
    }

    // 6. Copy slide (Cmd+C / Ctrl+C without an active text selection).
    if (mod && key === "c" && selectedId) {
      const activeSelection = window.getSelection();
      if (!activeSelection || activeSelection.isCollapsed || !activeSelection.toString().trim()) {
        const current = slide();
        if (current) {
          slideClipboard = JSON.parse(JSON.stringify(current));
          toast(t("toast.slideCopied") || "Slide copied");
        }
      }
      return;
    }

    // 7. Paste slide (Cmd+V / Ctrl+V).
    if (mod && key === "v" && slideClipboard) {
      const d = deck();
      if (d) {
        event.preventDefault();
        const currentIdx = d.slides.findIndex((s) => s.id === selectedId);
        const insertAt = currentIdx >= 0 ? currentIdx + 1 : d.slides.length;
        run("paste_slide", {
          index: insertAt,
          code: slideClipboard.code,
          template: slideClipboard.template,
          animation: slideClipboard.animation || [],
        }).then((ok) => {
          if (ok) {
            toast(t("toast.slidePasted") || "Slide pasted");
          }
        });
      }
      return;
    }
  });

  // Also support paste event (e.g. context menu or paste command).
  document.addEventListener("paste", (event) => {
    if (event.defaultPrevented || !slideClipboard) return;
    const clip = event.clipboardData;
    const hasImage =
      clip && [...(clip.items || [])].some((item) => item.type && item.type.startsWith("image/"));
    if (hasImage) return;
    const target = event.target;
    const tag = ((target && target.tagName) || "").toLowerCase();
    if (tag === "input" || tag === "textarea" || tag === "select" || (target && target.isContentEditable)) return;
    const d = deck();
    if (!d) return;
    event.preventDefault();
    const currentIdx = d.slides.findIndex((s) => s.id === selectedId);
    const insertAt = currentIdx >= 0 ? currentIdx + 1 : d.slides.length;
    run("paste_slide", {
      index: insertAt,
      code: slideClipboard.code,
      template: slideClipboard.template,
      animation: slideClipboard.animation || [],
    }).then((ok) => {
      if (ok) {
        toast(t("toast.slidePasted") || "Slide pasted");
      }
    });
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
      storyboard.invalidate();
      if (view) applyView(view);
    }, 120);
  });

  const TEMPLATES = [
    { id: "hero", icon: "✨" },
    { id: "bullet", icon: "📋" },
    { id: "diagram", icon: "📊" },
    { id: "chart", icon: "📈" },
    { id: "image_layout", icon: "🖼️" },
    { id: "code", icon: "💻" },
  ];

  function setupTemplatePicker() {
    const popover = els.templatePopover;
    const grid = els.templateGrid;
    const pickerBtn = $("add-slide-picker");
    const closeBtn = els.closeTemplatePopover;
    if (!popover || !grid) return;

    grid.innerHTML = "";
    TEMPLATES.forEach((tpl) => {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "template-card";
      card.innerHTML = `<span class="template-card-icon">${tpl.icon}</span><span>${escapeHtml(t("template." + tpl.id))}</span>`;
      card.addEventListener("click", () => {
        popover.hidden = true;
        run("add_slide", { template: tpl.id });
      });
      grid.append(card);
    });

    if (pickerBtn) {
      pickerBtn.addEventListener("click", (e) => {
        e.stopPropagation();
        popover.hidden = !popover.hidden;
      });
    }
    if (closeBtn) {
      closeBtn.addEventListener("click", () => {
        popover.hidden = true;
      });
    }
    document.addEventListener("click", (e) => {
      if (!popover.contains(e.target) && e.target !== pickerBtn && !pickerBtn?.contains(e.target)) {
        popover.hidden = true;
      }
    });
  }

  function setupExportDropdown() {
    if (!els.exportMenuTrigger || !els.exportMenu) return;
    els.exportMenuTrigger.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = !els.exportMenu.hidden;
      els.exportMenu.hidden = open;
      els.exportMenuTrigger.setAttribute("aria-expanded", String(!open));
    });
    document.addEventListener("click", (e) => {
      if (!els.exportMenu.contains(e.target) && e.target !== els.exportMenuTrigger) {
        els.exportMenu.hidden = true;
        els.exportMenuTrigger.setAttribute("aria-expanded", "false");
      }
    });
    if (els.topbarExportPdf) {
      els.topbarExportPdf.addEventListener("click", () => {
        els.exportMenu.hidden = true;
        startExport("pdf");
      });
    }
    if (els.topbarExportPng) {
      els.topbarExportPng.addEventListener("click", () => {
        els.exportMenu.hidden = true;
        startExport("png");
      });
    }
    if (els.topbarExportPptx) {
      els.topbarExportPptx.addEventListener("click", () => {
        els.exportMenu.hidden = true;
        startExport("pptx");
      });
    }
    if (els.topbarSaveDeck) {
      els.topbarSaveDeck.addEventListener("click", () => {
        els.exportMenu.hidden = true;
        const link = document.createElement("a");
        link.href = `/api/deck?t=${encodeURIComponent(sessionToken)}`;
        link.download = "";
        document.body.append(link);
        link.click();
        link.remove();
        toast(t("toast.downloaded"));
      });
    }
  }

  // ---------- In-Studio Presentation Mode (No popup / in-place) ----------
  let isPresenting = false;
  let presentIndex = 0;
  let presentDraw = null;
  let presentSpeakerMode = false;
  let presentBlackout = false;
  let presentSlideKey = "";

  // Steps can only be delivered once the sandboxed document has registered
  // its message listener. Messages posted before that are silently dropped,
  // which left every animated element frozen at `opacity: 0` (a black slide).
  // Reveal state (slide + queued steps + frame readiness) lives in
  // `presenter-core.js`, shared with the standalone presenter so the two can
  // never disagree about when a step plays.
  const presentReveal = createReveal([]);
  // Set right before a deliberate exit from fullscreen (F), so leaving
  // fullscreen does not end the presentation.
  let presentKeepWindowed = false;
  let presentTimerSeconds = 0;
  let presentTimerRunning = false;
  let presentSlideStartSeconds = 0;
  let presentTimerInterval = null;
  let presentHudTimer = null;
  let presentClockInterval = null;
  let presentNotesFontSize = 18;

  const presentEls = {
    overlay: $("present-overlay"),
    screen: $("present-screen"),
    frameWrap: $("present-frame"),
    frame: $("present-slide"),
    empty: $("present-empty"),
    hud: $("hud"),
    current: $("hud-current"),
    total: $("hud-total"),
    progress: $("progress"),
    progressFill: $("progress-fill"),
    hudSpeaker: $("hud-speaker"),
    hudFullscreen: $("hud-fullscreen"),
    hudExitBtn: $("hud-exit-btn"),
    speakerConsole: $("speaker-console"),
    speakerTitle: $("speaker-deck-title"),
    wallClock: $("wall-clock"),
    timerDisplay: $("timer-display"),
    timerToggle: $("timer-toggle"),
    timerReset: $("timer-reset"),
    exitSpeaker: $("exit-speaker"),
    speakerCloseBtn: $("speaker-close-btn"),
    scCurrent: $("sc-current"),
    scTotal: $("sc-total"),
    scCurrentFrame: $("sc-current-frame"),
    scNextFrame: $("sc-next-frame"),
    noNextHint: $("no-next-hint"),
    notesBody: $("speaker-notes-body"),
    notesDown: $("notes-font-down"),
    notesUp: $("notes-font-up"),
    notesPace: $("notes-pace"),
  };

  function formatPresentTime(sec) {
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }

  function startPresentTimer() {
    if (presentTimerRunning) return;
    presentTimerRunning = true;
    if (presentEls.timerToggle) presentEls.timerToggle.textContent = "⏸";
    presentTimerInterval = setInterval(() => {
      presentTimerSeconds += 1;
      if (presentEls.timerDisplay) presentEls.timerDisplay.textContent = formatPresentTime(presentTimerSeconds);
      updatePresentPace();
    }, 1000);
  }

  function pausePresentTimer() {
    presentTimerRunning = false;
    if (presentEls.timerToggle) presentEls.timerToggle.textContent = "▶";
    clearInterval(presentTimerInterval);
  }

  function resetPresentTimer() {
    pausePresentTimer();
    presentTimerSeconds = 0;
    if (presentEls.timerDisplay) presentEls.timerDisplay.textContent = "00:00";
  }

  function updatePresentWallClock() {
    if (presentEls.wallClock) {
      const now = new Date();
      presentEls.wallClock.textContent = now.toTimeString().slice(0, 8);
    }
  }

  function fitPresentFrame() {
    if (!view || !presentEls.frameWrap) return;
    const ratio = view.presentation.aspect_ratio.split(":").map(Number);
    const [w, h] = ratio;
    const availableW = window.innerWidth;
    const availableH = window.innerHeight;
    let width = availableW;
    let height = (width * h) / w;
    if (height > availableH) {
      height = availableH;
      width = (height * w) / h;
    }
    presentEls.frameWrap.style.width = `${Math.round(width)}px`;
    presentEls.frameWrap.style.height = `${Math.round(height)}px`;
  }

  function showPresentHud() {
    if (!presentEls.hud) return;
    presentEls.hud.dataset.visible = "true";
    if (presentHudTimer) clearTimeout(presentHudTimer);
    presentHudTimer = setTimeout(() => {
      if (presentEls.hud) presentEls.hud.dataset.visible = "false";
    }, 2800);
  }

  function renderPresentSpeakerConsole() {
    if (!presentEls.speakerConsole || presentEls.speakerConsole.hidden || !view) return;
    const slides = view.presentation.slides;
    if (!slides.length) return;
    const s = slides[presentIndex];
    if (presentEls.speakerTitle) presentEls.speakerTitle.textContent = view.presentation.title || "SlydeCode";
    if (presentEls.scCurrent) presentEls.scCurrent.textContent = String(presentIndex + 1);
    if (presentEls.scTotal) presentEls.scTotal.textContent = String(slides.length);
    if (presentEls.scCurrentFrame) {
      presentEls.scCurrentFrame.src = `${sandboxOrigin}/s/${encodeURIComponent(s.id)}?static=1${sourceQuery}`;
    }
    if (presentIndex + 1 < slides.length) {
      const nextSlide = slides[presentIndex + 1];
      if (presentEls.scNextFrame) {
        presentEls.scNextFrame.hidden = false;
        presentEls.scNextFrame.src = `${sandboxOrigin}/s/${encodeURIComponent(nextSlide.id)}?static=1${sourceQuery}`;
      }
      if (presentEls.noNextHint) presentEls.noNextHint.hidden = true;
    } else {
      if (presentEls.scNextFrame) presentEls.scNextFrame.hidden = true;
      if (presentEls.noNextHint) presentEls.noNextHint.hidden = false;
    }
    if (presentEls.notesBody) {
      if (s.notes && s.notes.trim()) {
        presentEls.notesBody.textContent = s.notes;
        presentEls.notesBody.classList.remove("speaker-notes-empty");
      } else {
        presentEls.notesBody.innerHTML = `<span class="speaker-notes-empty">(${t("present.noNotes")})</span>`;
      }
    }
    updatePresentPace();
  }

  /** Reading-time estimate and pace for the current slide. */
  function updatePresentPace() {
    if (!presentEls.notesPace) return;
    const current = view && view.presentation.slides[presentIndex];
    const expected = readingSeconds(current && current.notes);
    if (!expected) {
      presentEls.notesPace.dataset.state = "none";
      presentEls.notesPace.textContent = "";
      return;
    }
    const elapsed = Math.max(0, presentTimerSeconds - presentSlideStartSeconds);
    const state = paceState(elapsed, expected);
    presentEls.notesPace.dataset.state = state;
    presentEls.notesPace.textContent = `${t("present.pace.estimate", {
      time: formatClock(expected),
    })} · ${t(`present.pace.${state}`)}`;
  }

  function renderPresentSlide() {
    if (!view || !isPresenting) return;
    const slides = view.presentation.slides;
    if (!slides.length) {
      if (presentEls.frameWrap) presentEls.frameWrap.hidden = true;
      if (presentEls.empty) presentEls.empty.hidden = false;
      return;
    }
    if (presentEls.empty) presentEls.empty.hidden = true;
    if (presentEls.frameWrap) presentEls.frameWrap.hidden = false;

    presentIndex = Math.min(slides.length - 1, Math.max(0, presentIndex));
    const s = slides[presentIndex];

    if (presentEls.current) presentEls.current.textContent = String(presentIndex + 1);
    if (presentEls.total) presentEls.total.textContent = String(slides.length);

    const percent = progressPercent(presentIndex, slides.length);
    if (presentEls.progressFill) presentEls.progressFill.style.width = `${percent}%`;
    if (presentEls.progress) presentEls.progress.setAttribute("aria-valuenow", String(percent));

    const key = `${s.id}:${s.code.length}:${s.animation.length}`;
    if (key !== presentSlideKey) {
      presentSlideKey = key;
      presentReveal.reload();
      presentSlideStartSeconds = presentTimerSeconds;
      if (presentDraw) presentDraw.clear();
      if (presentEls.frame) {
        presentEls.frame.src = `${sandboxOrigin}/s/${encodeURIComponent(s.id)}?p=${presentIndex}${sourceQuery}`;
      }
    }

    fitPresentFrame();
    renderPresentSpeakerConsole();
    showPresentHud();
  }

  function postPresentStep(step) {
    const win = presentEls.frame && presentEls.frame.contentWindow;
    if (!win) return;
    win.postMessage({ __slyde: true, type: "animate", step }, "*");
  }

  /** Plays the slide's first step once the document is listening. */
  function flushPresentSteps() {
    const step = presentReveal.takeIntro();
    if (step) postPresentStep(step);
  }

  function advancePresentSlide() {
    const action = presentReveal.advance();
    if (action.type === "step") {
      postPresentStep(action.step);
      return;
    }
    if (action.type === "slide") showPresentSlide(action);
  }

  function goPresentDelta(delta) {
    if (!view) return;
    const action = presentReveal.delta(delta);
    if (action) showPresentSlide(action);
  }

  function jumpPresentIndex(target) {
    if (!view) return;
    const action = presentReveal.goTo(target);
    if (action) showPresentSlide(action);
  }

  /// Renders the slide the reveal controller just loaded.
  function showPresentSlide(action) {
    presentIndex = action.index;
    presentSlideKey = "";
    renderPresentSlide();
  }

  function togglePresentBlackout() {
    presentBlackout = !presentBlackout;
    if (presentEls.frameWrap) presentEls.frameWrap.style.opacity = presentBlackout ? "0" : "1";
  }

  function togglePresentSpeakerMode(force) {
    presentSpeakerMode = typeof force === "boolean" ? force : !presentSpeakerMode;
    if (presentEls.speakerConsole) presentEls.speakerConsole.hidden = !presentSpeakerMode;
    if (presentSpeakerMode && !presentTimerRunning && presentTimerSeconds === 0) {
      startPresentTimer();
    }
    if (view) renderPresentSpeakerConsole();
  }

  function handlePresentKeydown(event) {
    if (!isPresenting) return;
    const action = keyToAction(event);
    if (!action) return;
    event.preventDefault();
    switch (action) {
      case "advance":
        advancePresentSlide();
        break;
      case "back":
        goPresentDelta(-1);
        break;
      case "first":
        jumpPresentIndex(0);
        break;
      case "last":
        if (view) jumpPresentIndex(view.presentation.slides.length - 1);
        break;
      case "blackout":
        togglePresentBlackout();
        break;
      case "speaker":
        togglePresentSpeakerMode();
        break;
      case "fullscreen":
        togglePresentFullscreen();
        break;
      case "laser":
        if (presentDraw) presentDraw.toggle("laser");
        break;
      case "pen":
        if (presentDraw) presentDraw.toggle("pen");
        break;
      case "clearDraw":
        if (presentDraw) presentDraw.clear();
        break;
      case "escape":
        if (presentBlackout) {
          togglePresentBlackout();
        } else if (presentSpeakerMode) {
          togglePresentSpeakerMode(false);
        } else {
          closePresentation();
        }
        break;
      default:
        break;
    }
  }

  function handlePresentClick(event) {
    if (!isPresenting) return;
    // The click that opens the presenter is registered on `document` while it
    // is still bubbling, so it would otherwise advance the deck and consume the
    // first animation step. Only clicks landing inside the overlay count.
    if (!presentEls.overlay || !presentEls.overlay.contains(event.target)) return;
    if (event.target.closest("#hud") || event.target.closest("#speaker-console")) return;
    if (event.clientX < window.innerWidth / 2) goPresentDelta(-1);
    else advancePresentSlide();
  }

  function togglePresentFullscreen() {
    if (document.fullscreenElement) {
      presentKeepWindowed = true;
      document.exitFullscreen().catch(() => {
        presentKeepWindowed = false;
      });
    } else {
      document.documentElement.requestFullscreen().catch(() => {});
    }
  }

  // Chrome consumes Escape to leave fullscreen instead of dispatching it to the
  // page, so an unexpected exit from fullscreen is the presenter's exit gesture.
  function handlePresentFullscreenChange() {
    if (!isPresenting || document.fullscreenElement) return;
    if (presentKeepWindowed) {
      presentKeepWindowed = false;
      return;
    }
    closePresentation();
  }

  function openPresentationInStudio(speaker = false) {
    const d = deck();
    if (!d || !d.slides.length) {
      toast(t("present.empty"));
      return;
    }
    isPresenting = true;
    presentBlackout = false;
    presentReveal.setSlides(d.slides);
    presentIndex = Math.max(0, d.slides.findIndex((s) => s.id === selectedId));
    presentReveal.goTo(presentIndex, { force: true });
    presentSlideKey = "";

    if (presentEls.overlay) presentEls.overlay.hidden = false;
    document.documentElement.requestFullscreen().catch(() => {});

    if (speaker) {
      togglePresentSpeakerMode(true);
    } else {
      togglePresentSpeakerMode(false);
    }

    if (!presentClockInterval) {
      updatePresentWallClock();
      presentClockInterval = setInterval(updatePresentWallClock, 1000);
    }

    if (!presentDraw && presentEls.frameWrap) {
      presentDraw = mountPresenterDraw(presentEls.frameWrap);
    }
    renderPresentSlide();

    document.addEventListener("keydown", handlePresentKeydown);
    document.addEventListener("click", handlePresentClick);
    document.addEventListener("fullscreenchange", handlePresentFullscreenChange);
    window.addEventListener("mousemove", showPresentHud);
    window.addEventListener("resize", fitPresentFrame);
  }

  function closePresentation() {
    if (!isPresenting) return;
    isPresenting = false;
    if (presentDraw) {
      presentDraw.toggle("none");
      presentDraw.clear();
    }
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    }
    if (presentEls.overlay) presentEls.overlay.hidden = true;

    document.removeEventListener("keydown", handlePresentKeydown);
    document.removeEventListener("click", handlePresentClick);
    document.removeEventListener("fullscreenchange", handlePresentFullscreenChange);
    window.removeEventListener("mousemove", showPresentHud);
    window.removeEventListener("resize", fitPresentFrame);

    if (view && view.presentation.slides[presentIndex]) {
      selectedId = view.presentation.slides[presentIndex].id;
    }
    stageKey = "";
    applyView(view);
  }

  function initPresenterListeners() {
    if (presentEls.hudExitBtn) {
      presentEls.hudExitBtn.addEventListener("click", (e) => {
        e.stopPropagation();
        closePresentation();
      });
    }
    if (presentEls.speakerCloseBtn) {
      presentEls.speakerCloseBtn.addEventListener("click", (e) => {
        e.stopPropagation();
        closePresentation();
      });
    }
    if (presentEls.exitSpeaker) {
      presentEls.exitSpeaker.addEventListener("click", (e) => {
        e.stopPropagation();
        togglePresentSpeakerMode(false);
      });
    }
    if (presentEls.hudSpeaker) {
      presentEls.hudSpeaker.addEventListener("click", (e) => {
        e.stopPropagation();
        togglePresentSpeakerMode();
      });
    }
    if (presentEls.hudFullscreen) {
      presentEls.hudFullscreen.addEventListener("click", (e) => {
        e.stopPropagation();
        togglePresentFullscreen();
      });
    }
    if (presentEls.timerToggle) {
      presentEls.timerToggle.addEventListener("click", (e) => {
        e.stopPropagation();
        if (presentTimerRunning) pausePresentTimer();
        else startPresentTimer();
      });
    }
    if (presentEls.timerReset) {
      presentEls.timerReset.addEventListener("click", (e) => {
        e.stopPropagation();
        resetPresentTimer();
      });
    }
    if (presentEls.notesDown) {
      presentEls.notesDown.addEventListener("click", (e) => {
        e.stopPropagation();
        presentNotesFontSize = Math.max(14, presentNotesFontSize - 2);
        if (presentEls.notesBody) presentEls.notesBody.style.fontSize = `${presentNotesFontSize}px`;
      });
    }
    if (presentEls.notesUp) {
      presentEls.notesUp.addEventListener("click", (e) => {
        e.stopPropagation();
        presentNotesFontSize = Math.min(32, presentNotesFontSize + 2);
        if (presentEls.notesBody) presentEls.notesBody.style.fontSize = `${presentNotesFontSize}px`;
      });
    }
    if (presentEls.frame) {
      presentEls.frame.addEventListener("load", () => {
        // `load` fires after the slide runtime installed its listener, and it is
        // the only signal tied to *this* document. A stale `ready` from the
        // document being replaced must never consume the intro step.
        presentReveal.markReady();
        flushPresentSteps();
      });
    }
    window.addEventListener("message", (event) => {
      const data = event.data;
      if (!data || data.__slyde !== true) return;
      const frameWindow = presentEls.frame && presentEls.frame.contentWindow;
      if (data.type === "ready") {
        presentReveal.markReady();
        return;
      }
      if (data.type === "key") {
        // A focused slide document owns the keyboard; the sandbox runtime
        // forwards each key so navigation never dies after touching the slide.
        if (!isPresenting || !frameWindow || event.source !== frameWindow) return;
        handlePresentKeydown(
          new KeyboardEvent("keydown", {
            key: data.key,
            code: data.code,
            altKey: !!data.altKey,
            ctrlKey: !!data.ctrlKey,
            metaKey: !!data.metaKey,
            shiftKey: !!data.shiftKey,
            bubbles: false,
            cancelable: true,
          })
        );
      }
    });
  }

  function setupPresentGroup() {
    initPresenterListeners();

    if (els.present) {
      els.present.addEventListener("click", () => openPresentationInStudio(false));
    }
    if (els.presentMenuTrigger && els.presentMenu) {
      els.presentMenuTrigger.addEventListener("click", (e) => {
        e.stopPropagation();
        const open = !els.presentMenu.hidden;
        els.presentMenu.hidden = open;
        els.presentMenuTrigger.setAttribute("aria-expanded", String(!open));
      });
      document.addEventListener("click", (e) => {
        if (!els.presentMenu.contains(e.target) && e.target !== els.presentMenuTrigger) {
          els.presentMenu.hidden = true;
          els.presentMenuTrigger.setAttribute("aria-expanded", "false");
        }
      });
    }
    if (els.presentFullscreen) {
      els.presentFullscreen.addEventListener("click", () => {
        if (els.presentMenu) els.presentMenu.hidden = true;
        openPresentationInStudio(false);
      });
    }
    if (els.presentSpeaker) {
      els.presentSpeaker.addEventListener("click", () => {
        if (els.presentMenu) els.presentMenu.hidden = true;
        openPresentationInStudio(true);
      });
    }
    const presentWindow = $("present-window");
    if (presentWindow) {
      presentWindow.addEventListener("click", () => {
        if (els.presentMenu) els.presentMenu.hidden = true;
        const d = deck();
        const slideIndex = d ? Math.max(0, d.slides.findIndex((s) => s.id === selectedId)) : 0;
        const hash = slideIndex > 0 ? `#${slideIndex + 1}` : "";
        window.open(`/present?t=${encodeURIComponent(sessionToken)}${hash}`, "_blank");
      });
    }
  }

  function openLibrary() {
    if (!library) {
      toast(t("library.unavailable"));
      return;
    }
    library.open();
  }

  /// Applies the read model a library operation returns.
  ///
  /// Opening, creating and duplicating a deck all end with a different deck in
  /// the studio, so the stage, the storyboard selection and the canvas selection
  /// have to be rebuilt from scratch rather than patched.
  function applyLibraryPayload(payload) {
    if (!payload) return;
    // A different deck means a different conversation.
    if (typeof payload.active === "string" && payload.active !== activeDeckId) {
      activeDeckId = payload.active;
      if (copilot && typeof copilot.refreshThread === "function") copilot.refreshThread();
    }
    if (!payload.view) return;
    stageKey = "";
    storyboard.invalidate();
    canvas.clearSelection({ notifySandbox: false });
    selectedId = payload.view.presentation.slides[0]?.id || "";
    applyView(payload.view);
  }

  function setupDeckQuickTools() {
    if (els.topbarLibrary) {
      els.topbarLibrary.addEventListener("click", () => {
        openLibrary();
      });
    }

    if (els.topbarNewDeck) {
      els.topbarNewDeck.addEventListener("click", async () => {
        // With a deck library, "new" creates another deck file instead of
        // replacing the one the user has open. Without one (memory-only server)
        // it falls back to the in-place command.
        if (library) {
          try {
            // Name it in the user's language: the file is theirs, so the title
            // should not be an English default.
            const payload = await client.deckNew(t("deck.untitled"));
            applyLibraryPayload(payload);
            toast(t("library.created"));
            els.title?.focus();
            els.title?.select?.();
          } catch (error) {
            toast(`${t("toast.error")} — ${error.message}`);
          }
          return;
        }
        if (!confirm(t("action.confirmNewDeck"))) return;
        // `new_deck` is a UI command: `create_presentation` belongs to the agent
        // surface and `/api/command` rejects it, so the button used to do
        // nothing at all.
        const created = await run("new_deck", {
          title: t("deck.untitled"),
          template: "hero",
        });
        if (created) toast(t("toast.newDeck"));
        els.title?.focus();
        els.title?.select?.();
      });
    }

    if (els.topbarSaveDeckBtn) {
      els.topbarSaveDeckBtn.addEventListener("click", () => {
        const link = document.createElement("a");
        link.href = `/api/deck?t=${encodeURIComponent(sessionToken)}`;
        link.download = "";
        document.body.append(link);
        link.click();
        link.remove();
        toast(t("toast.downloaded"));
      });
    }

    if (els.topbarOpenDeck && els.topbarFilePicker) {
      els.topbarOpenDeck.addEventListener("click", () => {
        els.topbarFilePicker.click();
      });
      els.topbarFilePicker.addEventListener("change", async (e) => {
        const file = e.target.files && e.target.files[0];
        if (!file) return;
        try {
          const text = await file.text();
          const res = await client.openDeck(text);
          if (res && res.view) {
            // `applyView` already re-renders the stage; the bare `renderStage()`
            // that used to live here crashed on the missing deck argument and
            // swallowed the whole open flow. Blur first: `applyView` skips the
            // title field while it has focus, to avoid fighting the typist.
            stageKey = "";
            selectedId = res.view.presentation.slides[0]?.id || "";
            els.title.blur();
            applyView(res.view);
            toast(t("toast.deckOpened"));
          }
        } catch (err) {
          console.error("Failed to open deck", err);
          toast(err.message || "Failed to open deck");
        } finally {
          els.topbarFilePicker.value = "";
        }
      });
    }
  }

  if (els.subtabSlide) {
    els.subtabSlide.addEventListener("click", () => {
      inspector.showSubtabs(true);
    });
  }
  if (els.subtabDeck) {
    els.subtabDeck.addEventListener("click", () => {
      inspector.showSubtabs(false);
    });
  }

  // ---------- start ----------

  (async () => {
    try {
      const payload = await client.getState();
      sandboxOrigin = payload.sandbox_origin;
      sandboxToken = payload.sandbox_token || "";
      activeDeckId = payload.active_deck || null;
      sourceQuery = sandboxToken ? `&t=${encodeURIComponent(sandboxToken)}` : "";
      setStatus("connecting");
      setupTemplatePicker();
      setupExportDropdown();
      setupPresentGroup();
      setupDeckQuickTools();
      applyView(payload.view);
      syncLangButtons();
      syncThemeToggle();
      applyI18n();
      palette = mountCommandPalette({
        root: $("command-palette"),
        t,
        buildActions: buildPaletteActions,
      });
      library = mountDeckLibrary({
        root: $("deck-library"),
        client,
        t,
        notify: toast,
        onApplied: applyLibraryPayload,
      });
      library.setI18n(applyI18n);
      copilot = mountCopilot({
        client,
        getSelectedSlideId: () => selectedId,
        getSelectedElementId: () => (canvas.selectedElement ? canvas.selectedElement.id : null),
        notify: toast,
        getDeckId: () => activeDeckId,
      });
      client.connect({ onEvent: handleEvent, onStatus: setStatus });
    } catch (error) {
      showFatal(error.message);
    }
  })();
}
