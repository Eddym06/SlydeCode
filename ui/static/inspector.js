// Studio design panel (the "inspector"): slide tab, deck tab and the animation timeline.
//
// It used to live inside the studio's boot closure, which made the shell 2,400
// lines of intertwined state and markup. The inspector is a self-contained panel:
// it renders from the read model it is given and sends commands through the
// callbacks, so it never touches the deck itself.
//
// The DOM ids are deliberately unchanged (`#theme-select`, `#notes`, `#export-pdf`,
// …): they are the contract with the export progress handler and with the E2E
// suite. What changed is who owns the markup and the event wiring.

import { createCodeEditor } from "./code-editor.js";
import { describeStructure, labelFor } from "./element-labels.js";
import { TRANSITION_KINDS, DEFAULT_TRANSITION_MS } from "./transitions.js";
import { LAYOUTS } from "./layouts.js";
import { layoutPreview } from "./layout-previews.js";
import { THEME_COLORS } from "./layout-previews.js";

/** Every theme the backend accepts, with the dots the swatches show. */
export const THEME_CHOICES = [
  { id: "studio", dots: ["#7C3AED", "#FF6B4A"] },
  { id: "paper", dots: ["#2C2B2A", "#D97706"] },
  { id: "night", dots: ["#9B6BFF", "#FF8265"] },
  { id: "original", dots: ["#7C3AED", "#1B1530"] },
  { id: "academic", dots: ["#111111", "#0055D4"] },
  { id: "pitch", dots: ["#0F172A", "#38BDF8"] },
];

/** Animation kinds the sandbox knows how to play, in the order the editor lists them. */
export const ANIMATION_KINDS = [
  "fade_up",
  "fade",
  "scale_in",
  "slide_in_left",
  "slide_in_right",
  "typewriter",
  "spin",
  "rotate_in",
  "pulse",
  "flip_up",
  "blur_in",
  "bounce_in",
];

export const DEFAULT_EASING = "cubic-bezier(.16,1,.3,1)";
const NOTES_DEBOUNCE_MS = 450;
const PNG_SCALE_KEY = "slyde.export.pngScale";
const PNG_SCALES = [1, 2];
const DEFAULT_PNG_SCALE = 2;

/** The PNG quality the user last picked; 2x (sharp on a retina projector) by default. */
export function readPngScale() {
  try {
    const stored = Number(localStorage.getItem(PNG_SCALE_KEY));
    return PNG_SCALES.includes(stored) ? stored : DEFAULT_PNG_SCALE;
  } catch {
    return DEFAULT_PNG_SCALE;
  }
}

function writePngScale(scale) {
  if (!PNG_SCALES.includes(scale)) return;
  try {
    localStorage.setItem(PNG_SCALE_KEY, String(scale));
  } catch {
    // Private windows can refuse storage; the choice then lasts until reload.
  }
}

/**
 * @param {object} options
 * @param {HTMLElement} options.root  the inspector container
 * @param {(key: string, vars?: object) => string} options.t
 * @param {(value: unknown) => string} options.escapeHtml
 * @param {object} options.actions
 *   command, selectElement, pickImage, saveDeck, openDeckFile, exportDeck,
 *   slideRemoved, aspectRatioChanged, applyI18n
 */
export function createInspector({ root, t, escapeHtml, actions }) {
  if (!root) throw new Error("the inspector needs its container");

  let subtab = "slide";
  let notesTimer = null;
  let lastEnterKey = "";
  let codeEditor = null; // survives re-renders so typing is never overwritten
  let devOpen = false;
  let current = { deck: null, slide: null, selectedElement: null, structureNodes: [] };

  // ---------- building blocks ----------

  const ICONS = {
    layout:
      '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 10h18M9 10v10"/>',
    notes: '<path d="M5 4h14v12l-4 4H5z"/><path d="M15 20v-4h4M8 9h8M8 13h4"/>',
    motion: '<path d="M4 12c3-7 5-7 8 0s5 7 8 0"/>',
    elements:
      '<path d="M12 3 3 8l9 5 9-5z"/><path d="m3 13 9 5 9-5"/>',
    palette:
      '<path d="M12 3a9 9 0 1 0 0 18c1.1 0 1.8-.8 1.8-1.7 0-.5-.2-.9-.5-1.2-.3-.3-.5-.7-.5-1.2 0-.9.8-1.7 1.7-1.7H17a4 4 0 0 0 4-4C21 6.6 17 3 12 3Z"/>',
    ratio: '<rect x="3" y="6" width="18" height="12" rx="2.5"/>',
    file: '<path d="M7 3h7l4 4v13a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/><path d="M14 3v5h5"/>',
    export: '<path d="M12 15V4M8 8l4-4 4 4"/><path d="M5 14v4a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-4"/>',
    code: '<path d="m8 8-4 4 4 4M16 8l4 4-4 4"/>',
    image: '<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="10" r="1.6"/><path d="m21 16-5-5-8 8"/>',
    copy: '<rect x="9" y="9" width="12" height="12" rx="2.5"/><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"/>',
    trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12M9 7V4h6v3"/>',
    play: '<path d="M8 5.5v13l10-6.5z" fill="currentColor"/>',
    up: '<path d="M12 19V5M6 11l6-6 6 6"/>',
    down: '<path d="M12 5v14M6 13l6 6 6-6"/>',
    close: '<path d="M6 6l12 12M18 6 6 18"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    layout: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 9h18M10 9v11"/>',
    transition: '<rect x="3" y="5" width="10" height="14" rx="2"/><rect x="11" y="5" width="10" height="14" rx="2" opacity=".55"/>',
    history: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/>',
    sparkle: '<path d="M12 3c.7 5 1.9 6.3 7 7-5.1.7-6.3 2-7 7-.7-5-1.9-6.3-7-7 5.100-.7 6.300-2 7-7Z"/>',
  };

  function icon(name, size = 16) {
    return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] || ""}</svg>`;
  }

  /** One titled card. `--i` staggers the entrance when the view changes. */
  function card(index, iconName, title, body, { id = "", extra = "", attrs = "" } = {}) {
    return `
      <section class="card"${id ? ` id="${id}"` : ""}${attrs ? ` ${attrs}` : ""} style="--i:${index}">
        <header class="card-head">
          <span class="card-icon">${icon(iconName, 15)}</span>
          <h3 class="card-title">${escapeHtml(title)}</h3>
          ${extra}
        </header>
        <div class="card-body">${body}</div>
      </section>`;
  }

  // ---------- animation editor ----------

  function renderAnimationEditor(slide) {
    const steps = slide.animation || [];
    const rows = steps
      .map(
        (step, index) => `
        <div class="anim-step" data-index="${index}" data-easing="${escapeHtml(step.easing || DEFAULT_EASING)}">
          <span class="step-index" aria-hidden="true">${index + 1}</span>
          <div class="anim-step-main">
            <div class="anim-step-head">
              <select class="anim-kind" data-field="kind" aria-label="${escapeHtml(t("inspector.stepKind"))}">
                ${ANIMATION_KINDS.map(
                  (kind) =>
                    `<option value="${kind}"${kind === step.kind ? " selected" : ""}>${kind}</option>`
                ).join("")}
              </select>
              <span class="step-tools">
                <button type="button" class="step-play" data-play="${index}"
                        title="${escapeHtml(t("action.play"))}" aria-label="${escapeHtml(t("action.play"))}">${icon("play", 12)}</button>
                <button type="button" class="anim-btn" data-move="-1"
                        title="${escapeHtml(t("animation.moveUp"))}" aria-label="${escapeHtml(t("animation.moveUp"))}"
                        ${index === 0 ? "disabled" : ""}>${icon("up", 13)}</button>
                <button type="button" class="anim-btn" data-move="1"
                        title="${escapeHtml(t("animation.moveDown"))}" aria-label="${escapeHtml(t("animation.moveDown"))}"
                        ${index === steps.length - 1 ? "disabled" : ""}>${icon("down", 13)}</button>
                <button type="button" class="anim-btn anim-btn-danger" data-remove="${index}"
                        title="${escapeHtml(t("animation.remove"))}" aria-label="${escapeHtml(t("animation.remove"))}">${icon("close", 13)}</button>
              </span>
            </div>
            <div class="anim-step-fields">
              <label class="anim-field anim-field-target">
                <span>${escapeHtml(t("inspector.stepTarget"))}</span>
                <input class="anim-input" data-field="target" value="${escapeHtml(step.target)}"
                       spellcheck="false" autocomplete="off">
              </label>
              <label class="anim-field">
                <span>${escapeHtml(t("inspector.stepDelay"))}</span>
                <input class="anim-input" type="number" data-field="delay" value="${step.delay_ms}"
                       min="0" max="10000" step="10">
              </label>
              <label class="anim-field">
                <span>${escapeHtml(t("inspector.stepDuration"))}</span>
                <input class="anim-input" type="number" data-field="duration" value="${step.duration_ms}"
                       min="1" max="5000" step="10">
              </label>
            </div>
          </div>
        </div>`
      )
      .join("");

    return `
      <div class="anim-editor" id="anim-editor">
        <div class="anim-timeline">
          ${rows || `<p class="hint empty-hint">${escapeHtml(t("inspector.noAnimation"))}</p>`}
        </div>
        <button type="button" class="anim-add" id="anim-add">${icon("plus", 14)}<span>${escapeHtml(t("inspector.addStep"))}</span></button>
      </div>`;
  }

  function bindAnimationEditor(slide) {
    const editor = root.querySelector("#anim-editor");
    if (!editor) return;

    const collect = () =>
      [...editor.querySelectorAll(".anim-step")].map((row) => ({
        kind: row.querySelector('[data-field="kind"]').value,
        target: row.querySelector('[data-field="target"]').value.trim(),
        delay_ms: Math.max(0, Number(row.querySelector('[data-field="delay"]').value) || 0),
        duration_ms: Math.max(1, Number(row.querySelector('[data-field="duration"]').value) || 520),
        easing: row.dataset.easing || DEFAULT_EASING,
      }));

    const save = (animation) =>
      actions.command("set_animation", { slide_id: slide.id, animation });

    editor.addEventListener("change", (event) => {
      if (!event.target.matches("[data-field]")) return;
      save(collect());
    });

    editor.addEventListener("click", (event) => {
      const play = event.target.closest("[data-play]");
      if (play) {
        actions.command("trigger_animation", {
          slide_id: slide.id,
          step_index: Number(play.dataset.play),
        });
        return;
      }

      const remove = event.target.closest("[data-remove]");
      if (remove) {
        const steps = collect();
        steps.splice(Number(remove.dataset.remove), 1);
        save(steps);
        return;
      }

      const move = event.target.closest("[data-move]");
      if (move) {
        const steps = collect();
        const from = Number(move.closest(".anim-step").dataset.index);
        const to = from + Number(move.dataset.move);
        if (to < 0 || to >= steps.length) return;
        const [moved] = steps.splice(from, 1);
        steps.splice(to, 0, moved);
        save(steps);
        return;
      }

      if (event.target.closest("#anim-add")) {
        const steps = collect();
        steps.push({
          kind: "fade_up",
          target: ".title",
          delay_ms: 90,
          duration_ms: 560,
          easing: DEFAULT_EASING,
        });
        save(steps);
      }
    });
  }

  // ---------- sections ----------

  function deckMarkup(deck) {
    // A palette the copilot designed is not one of the presets: it gets its own
    // swatch, named by the model, and stays selected until a preset replaces it.
    const custom = deck.theme.id === "custom" ? deck.theme : null;
    const choices = custom
      ? [{ id: "custom", dots: [custom.accent, custom.foreground] }, ...THEME_CHOICES]
      : THEME_CHOICES;
    const label = (choice) => (choice.id === "custom" ? custom.name : t(`theme.${choice.id}`));
    const colorsOf = (choice) =>
      choice.id === "custom"
        ? { bg: custom.background, fg: custom.foreground, accent: custom.accent }
        : THEME_COLORS[choice.id] || THEME_COLORS.studio;
    const themeOptions = choices
      .map(
        (choice) =>
          `<option value="${choice.id}" ${deck.theme.id === choice.id ? "selected" : ""}${
            choice.id === "custom" ? " disabled" : ""
          }>${escapeHtml(label(choice))}</option>`
      )
      .join("");
    const themeSwatches = choices.map(
      (choice) => `
          <button type="button" class="theme-swatch" ${
            choice.id === "custom" ? "disabled" : `data-theme="${choice.id}"`
          } role="radio" aria-checked="${deck.theme.id === choice.id}" aria-label="${escapeHtml(label(choice))}">
            <span class="swatch-preview" style="--sw-bg:${colorsOf(choice).bg};--sw-fg:${colorsOf(choice).fg};--sw-a:${colorsOf(choice).accent}">
              <span class="swatch-line"></span><span class="swatch-line short"></span>
              <span class="swatch-dots">
                ${choice.dots
                  .map((color) => `<span class="swatch-dot" style="background:${color}"></span>`)
                  .join("")}
              </span>
            </span>
            <span class="swatch-name">${escapeHtml(label(choice))}</span>
          </button>`
    ).join("");
    const ratios = ["16:9", "4:3", "9:16"];
    const ratioOptions = ratios
      .map(
        (ratio) =>
          `<option value="${ratio}" ${deck.aspect_ratio === ratio ? "selected" : ""}>${ratio}</option>`
      )
      .join("");
    const ratioButtons = ratios
      .map((ratio) => {
        const [w, h] = ratio.split(":").map(Number);
        const scale = 18 / Math.max(w, h);
        return `<button type="button" class="ratio-btn" data-ratio="${ratio}" role="radio" aria-checked="${
          deck.aspect_ratio === ratio
        }"><span class="ratio-shape" style="width:${Math.round(w * scale)}px;height:${Math.round(
          h * scale
        )}px"></span><span>${ratio}</span></button>`;
      })
      .join("");

    return [
      card(
        0,
        "palette",
        t("inspector.themeSwatches"),
        `<div class="theme-swatches" role="radiogroup" aria-label="${escapeHtml(t("inspector.themeSwatches"))}">${themeSwatches}</div>
         <label class="field sr-field">
           <span class="hint">${escapeHtml(t("inspector.theme"))}</span>
           <select class="select" id="theme-select">${themeOptions}</select>
         </label>`
      ),
      card(
        1,
        "ratio",
        t("inspector.aspect"),
        `<div class="ratio-group" role="radiogroup" aria-label="${escapeHtml(t("inspector.aspect"))}">${ratioButtons}</div>
         <select class="select sr-field" id="ratio-select" aria-label="${escapeHtml(t("inspector.aspect"))}">${ratioOptions}</select>`
      ),
      card(
        2,
        "file",
        t("inspector.file"),
        `<div class="tile-row">
           <button type="button" class="tile-btn" id="save-deck">${icon("file", 16)}<span>${escapeHtml(t("inspector.saveCopy"))}</span></button>
           <button type="button" class="tile-btn" id="open-deck">${icon("export", 16)}<span>${escapeHtml(t("inspector.open"))}</span></button>
         </div>
         <p class="hint">${escapeHtml(t("inspector.autosave"))}</p>
         <input type="file" id="deck-file" accept=".slyde,application/json" hidden>`
      ),
      card(
        3,
        "history",
        t("versions.title"),
        `<div id="versions-host" class="versions-host"></div>`,
        { id: "versions-card" }
      ),
      card(
        4,
        "export",
        t("export.section"),
        `<div class="tile-row tile-row-3">
           <button type="button" class="tile-btn tile-badge" data-badge=".pdf" id="export-pdf">${escapeHtml(t("export.pdf"))}</button>
           <button type="button" class="tile-btn tile-badge" data-badge=".zip" id="export-png">${escapeHtml(t("export.png"))}</button>
           <button type="button" class="tile-btn tile-badge" data-badge=".pptx" id="export-pptx">${escapeHtml(t("export.pptx"))}</button>
           <button type="button" class="tile-btn tile-badge" data-badge=".html" id="export-html" style="grid-column: 1 / -1">${escapeHtml(t("export.html"))}</button>
         </div>
         <label class="export-quality">
           <span class="hint">${escapeHtml(t("export.quality"))}</span>
           <select class="select" id="export-scale">
             <option value="1"${readPngScale() === 1 ? " selected" : ""}>${escapeHtml(t("export.quality.standard"))}</option>
             <option value="2"${readPngScale() === 2 ? " selected" : ""}>${escapeHtml(t("export.quality.high"))}</option>
           </select>
         </label>
         <p class="hint export-status" id="export-status" role="status" aria-live="polite">${escapeHtml(t("export.hint"))}</p>`,
        { id: "export-section", attrs: 'aria-busy="false"' }
      ),
    ].join("");
  }

  /** Which layout tile to mark: the one that wrote this slide, else its template. */
  function activeLayoutOf(slide) {
    const byClass = {
      "layout-hero": "hero",
      "layout-bullet": "bullet",
      "layout-image": "image_layout",
      "layout-columns": "two_columns",
      "layout-quote": "quote",
      "layout-number": "big_number",
      "layout-section": "section",
    };
    const match = /class="slide (layout-[a-z]+)/.exec(slide.code || "");
    if (match && byClass[match[1]]) return byClass[match[1]];
    return ["hero", "bullet", "image_layout"].includes(slide.template) ? slide.template : null;
  }

  function layoutMarkup(slide) {
    const themeId = (current.deck && current.deck.theme && current.deck.theme.id) || "studio";
    const active = activeLayoutOf(slide);
    const tiles = LAYOUTS.map(
      (id) => `<button type="button" class="layout-tile" data-layout="${id}" role="radio" aria-checked="${id === active}">
        <span class="layout-tile-art">${layoutPreview(id, themeId)}</span>
        <span>${escapeHtml(t(`layout.${id}`))}</span></button>`
    ).join("");
    return `
      <div class="layout-grid" role="radiogroup" aria-label="${escapeHtml(t("layout.title"))}">${tiles}</div>
      <button type="button" class="tile-btn tile-wide" id="layout-all"${active ? "" : " disabled"}>${escapeHtml(t("layout.applyAll"))}</button>
      <p class="hint">${escapeHtml(t("layout.hint"))}</p>`;
  }

  function transitionMarkup(slide) {
    const current = (slide.transition && slide.transition.kind) || "none";
    const duration = (slide.transition && slide.transition.duration_ms) || DEFAULT_TRANSITION_MS;
    const chips = TRANSITION_KINDS.map(
      (kind) => `<button type="button" class="transition-chip" data-transition="${kind}" role="radio"
        aria-checked="${kind === current}"><span class="transition-demo" data-kind="${kind}" aria-hidden="true"><i></i><i></i></span>
        <span>${escapeHtml(t(`transition.${kind}`))}</span></button>`
    ).join("");
    return `
      <div class="transition-grid" role="radiogroup" aria-label="${escapeHtml(t("transition.title"))}">${chips}</div>
      <label class="transition-duration${current === "none" ? " is-off" : ""}">
        <span>${escapeHtml(t("transition.duration"))}</span>
        <input type="range" id="transition-duration" min="150" max="1500" step="50" value="${duration}"
               ${current === "none" ? "disabled" : ""} aria-label="${escapeHtml(t("transition.duration"))}">
        <output id="transition-output">${duration} ms</output>
      </label>
      <button type="button" class="tile-btn tile-wide" id="transition-all">${escapeHtml(t("transition.applyAll"))}</button>`;
  }

  function slideMarkup(slide, { selectedElement, structureNodes }) {
    const described = describeStructure(structureNodes);
    const tree = described.length
      ? `<ul class="element-tree">${described
          .map(
            (entry) => `<li>
                    <button type="button" class="element-tree-item${
                      selectedElement && selectedElement.id === entry.id ? " is-selected" : ""
                    }" data-tree-id="${escapeHtml(entry.id)}">
                      <span class="element-tree-tag">${escapeHtml(labelFor(entry, t))}</span>
                      ${entry.text ? `<span class="element-tree-text">${escapeHtml(entry.text)}</span>` : ""}
                    </button>
                  </li>`
          )
          .join("")}</ul>`
      : `<p class="hint">${escapeHtml(t("tree.empty"))}</p>`;
    const steps = (slide.animation || []).length;

    return [
      card(
        0,
        "layout",
        t("inspector.slide"),
        `<div class="slide-identity">
           <span class="layout-badge">${escapeHtml(t(`template.${slide.template}`))}</span>
         </div>
         <div class="slide-quick-actions">
           <button type="button" class="tile-btn" id="inspector-insert-image">${icon("image", 16)}<span>${escapeHtml(t("inspector.insertImage"))}</span></button>
           <input type="file" id="inspector-image-file" accept="image/png,image/jpeg,image/webp,image/gif" hidden>
           <button type="button" class="tile-btn" id="inspector-redesign-slide">${icon("sparkle", 16)}<span>${escapeHtml(t("action.redesign"))}</span></button>
           <button type="button" class="tile-btn" id="inspector-duplicate-slide">${icon("copy", 16)}<span>${escapeHtml(t("inspector.duplicate"))}</span></button>
           <button type="button" class="tile-btn tile-danger" id="delete-slide">${icon("trash", 16)}<span>${escapeHtml(t("action.delete"))}</span></button>
         </div>`
      ),
      card(1, "layout", t("layout.title"), layoutMarkup(slide)),
      card(
        2,
        "notes",
        t("inspector.notes"),
        `<textarea class="notes" id="notes" aria-label="${escapeHtml(t("inspector.notes"))}" placeholder="${escapeHtml(
          t("inspector.notesPlaceholder")
        )}">${escapeHtml(slide.notes || "")}</textarea>`
      ),
      card(
        3,
        "motion",
        t("inspector.animation"),
        renderAnimationEditor(slide),
        { extra: steps ? `<span class="card-count">${steps}</span>` : "" }
      ),
      card(4, "transition", t("transition.title"), transitionMarkup(slide)),
      card(5, "elements", t("tree.title"), tree, {
        extra: described.length ? `<span class="card-count">${described.length}</span>` : "",
      }),
      `<details class="dev card" style="--i:6"${devOpen ? " open" : ""}>
        <summary><span class="card-icon">${icon("code", 15)}</span><span class="card-title">${escapeHtml(t("inspector.developer"))}</span></summary>
        <p class="hint">${escapeHtml(t("inspector.developerHint"))}</p>
        <p class="hint dev-meta"><span class="mono id-chip" title="${escapeHtml(slide.id)}">${escapeHtml(slide.id)}</span></p>
        <div id="code-editor-host"></div>
      </details>`,
    ].join("");
  }

  // ---------- bindings ----------

  function bindDeckControls() {
    const theme = root.querySelector("#theme-select");
    if (theme) {
      theme.addEventListener("change", () =>
        actions.command("set_theme", { theme_id: theme.value })
      );
    }

    root.querySelectorAll(".theme-swatch").forEach((swatch) => {
      swatch.addEventListener("click", () => {
        const themeId = swatch.dataset.theme;
        const select = root.querySelector("#theme-select");
        if (select) select.value = themeId;
        actions.command("set_theme", { theme_id: themeId });
      });
    });

    const ratio = root.querySelector("#ratio-select");
    if (ratio) {
      ratio.addEventListener("change", () => actions.aspectRatioChanged(ratio.value));
    }

    root.querySelectorAll(".ratio-btn").forEach((button) => {
      button.addEventListener("click", () => {
        if (button.getAttribute("aria-checked") === "true") return;
        actions.aspectRatioChanged(button.dataset.ratio);
      });
    });

    const save = root.querySelector("#save-deck");
    if (save) save.addEventListener("click", () => actions.saveDeck());

    const open = root.querySelector("#open-deck");
    const file = root.querySelector("#deck-file");
    if (open && file) {
      open.addEventListener("click", () => file.click());
      file.addEventListener("change", async () => {
        const chosen = file.files && file.files[0];
        file.value = "";
        if (!chosen) return;
        await actions.openDeckFile(await chosen.text());
      });
    }

    for (const [id, format] of [
      ["export-pdf", "pdf"],
      ["export-png", "png"],
      ["export-pptx", "pptx"],
      ["export-html", "html"],
    ]) {
      const button = root.querySelector(`#${id}`);
      if (!button) continue;
      button.addEventListener("click", () =>
        actions.exportDeck(format, format === "png" ? { scale: readPngScale() } : {})
      );
    }

    const scaleSelect = root.querySelector("#export-scale");
    if (scaleSelect) {
      scaleSelect.addEventListener("change", () => writePngScale(Number(scaleSelect.value)));
    }

    actions.applyI18n(root);
  }

  function bindLayouts(slide) {
    const grid = root.querySelector(".layout-grid");
    if (!grid) return;
    const all = root.querySelector("#layout-all");
    grid.addEventListener("click", (event) => {
      const tile = event.target.closest("[data-layout]");
      if (!tile) return;
      grid.querySelectorAll("[data-layout]").forEach((node) => {
        node.setAttribute("aria-checked", String(node === tile));
      });
      all.disabled = false;
      actions.applyLayout(tile.dataset.layout, false, slide.id);
    });
    // The layout in use is read from the markup, so the button survives the
    // re-render that follows every change.
    all.addEventListener("click", () => {
      const tile = grid.querySelector('[aria-checked="true"]');
      if (tile) actions.applyLayout(tile.dataset.layout, true, slide.id);
    });
  }

  function bindTransition(slide) {
    const grid = root.querySelector(".transition-grid");
    if (!grid) return;
    const range = root.querySelector("#transition-duration");
    const output = root.querySelector("#transition-output");
    const kindNow = () =>
      (grid.querySelector('[aria-checked="true"]') || { dataset: { transition: "none" } }).dataset.transition;
    const send = (kind, all) =>
      actions.command("set_transition", {
        slide_ids: all ? null : [slide.id],
        kind,
        duration_ms: Number(range.value),
      });
    grid.addEventListener("click", (event) => {
      const chip = event.target.closest("[data-transition]");
      if (chip) send(chip.dataset.transition, false);
    });
    range.addEventListener("input", () => {
      output.textContent = `${range.value} ms`;
    });
    range.addEventListener("change", () => send(kindNow(), false));
    root.querySelector("#transition-all").addEventListener("click", () => send(kindNow(), true));
  }

  function mountCodeEditor(slide, editorState) {
    const host = root.querySelector("#code-editor-host");
    const details = root.querySelector("details.dev");
    if (details) {
      details.addEventListener("toggle", () => {
        devOpen = details.open;
      });
    }
    if (!host) return;
    if (codeEditor && codeEditor.slideId === slide.id) {
      if (!codeEditor.dirty && !editorState) codeEditor.setValue(slide.code);
    } else {
      codeEditor = createCodeEditor({
        value: slide.code,
        label: t("inspector.developer"),
        onSave: (code) =>
          actions.command("set_slide_code", { slide_id: slide.id, code }, { silent: true }),
        onBlur: () => window.setTimeout(() => render(), 0),
      });
      codeEditor.slideId = slide.id;
    }
    host.append(codeEditor.el);
    if (editorState) codeEditor.restore(editorState);
  }

  // ---------- render ----------

  function render(state = current) {
    current = { ...current, ...state };
    const { deck, slide: currentSlide, selectedElement, structureNodes } = current;
    if (!deck) return;

    const active = document.activeElement;
    const notesFocused = Boolean(
      active && active.classList && active.classList.contains("notes")
    );
    // The textarea is rebuilt on every render, so its caret must be restored or
    // typing in the middle of a note jumps to the end on each autosave.
    const caret =
      notesFocused && typeof active.selectionStart === "number"
        ? { start: active.selectionStart, end: active.selectionEnd, scroll: active.scrollTop }
        : null;

    const deckSection = deckMarkup(deck);
    // Cards animate in only when the user lands on a different slide or tab.
    // A remote edit re-renders the same view and must not replay the entrance.
    const enterKey = `${subtab}|${currentSlide ? currentSlide.id : "-"}`;
    const entering = enterKey !== lastEnterKey;
    lastEnterKey = enterKey;

    if (!currentSlide) {
      root.innerHTML = `<div id="inspector-deck-section">${deckSection}</div>`;
      root.classList.toggle("is-entering", entering);
      actions.onEmptyDeck();
      bindDeckControls();
      mountVersions();
      return;
    }

    const showSlide = subtab === "slide";
    // The editor outlives the markup: detach it so the rebuild does not destroy
    // the text the user is typing (and its caret).
    const editorState = codeEditor && codeEditor.focused ? codeEditor.selection() : null;
    if (codeEditor) codeEditor.el.remove();
    root.innerHTML = `
      <div id="inspector-slide-section" ${showSlide ? "" : 'style="display:none"'}>
        ${slideMarkup(currentSlide, { selectedElement, structureNodes })}
      </div>
      <div id="inspector-deck-section" ${showSlide ? 'style="display:none"' : ""}>
        ${deckSection}
      </div>`;

    root.classList.toggle("is-entering", entering);
    actions.onSubtabs(showSlide);
    bindDeckControls();
    mountVersions();

    root.querySelectorAll("[data-tree-id]").forEach((button) => {
      button.addEventListener("click", () => actions.selectElement(button.dataset.treeId));
    });

    const insertImageButton = root.querySelector("#inspector-insert-image");
    const imageInput = root.querySelector("#inspector-image-file");
    if (insertImageButton && imageInput) {
      insertImageButton.addEventListener("click", () => imageInput.click());
      imageInput.addEventListener("change", async () => {
        const file = imageInput.files && imageInput.files[0];
        imageInput.value = "";
        if (!file) return;
        await actions.pickImage(file);
      });
    }

    const duplicate = root.querySelector("#inspector-duplicate-slide");
    if (duplicate) {
      duplicate.addEventListener("click", () =>
        actions.command("duplicate_slide", { slide_id: currentSlide.id })
      );
    }

    const notes = root.querySelector("#notes");
    if (notes) {
      notes.addEventListener("input", () => {
        if (notesTimer) window.clearTimeout(notesTimer);
        notesTimer = window.setTimeout(() => {
          actions.command(
            "set_notes",
            { slide_id: currentSlide.id, notes: notes.value || null },
            { silent: true }
          );
        }, NOTES_DEBOUNCE_MS);
      });
    }

    bindAnimationEditor(currentSlide);
    bindTransition(currentSlide);
    bindLayouts(currentSlide);
    mountCodeEditor(currentSlide, editorState);

    const redesign = root.querySelector("#inspector-redesign-slide");
    if (redesign) {
      redesign.addEventListener("click", () => {
        if (actions.redesignSlide) actions.redesignSlide(currentSlide.id);
      });
    }

    const remove = root.querySelector("#delete-slide");
    if (remove) {
      remove.addEventListener("click", () => actions.removeSlide(currentSlide.id));
    }

    if (notesFocused && notes) {
      notes.focus();
      if (caret) {
        try {
          notes.setSelectionRange(caret.start, caret.end);
        } catch {
          /* Detached nodes throw; focus alone is still an improvement. */
        }
        notes.scrollTop = caret.scroll;
      }
    }
    actions.applyI18n(root);
  }

  /// The versions list owns its container; the panel only points it at the new one.
  function mountVersions() {
    const host = root.querySelector("#versions-host");
    if (host && actions.mountVersions) actions.mountVersions(host);
  }

  return {
    render,
    get subtab() {
      return subtab;
    },
    showSubtabs(showSlide) {
      subtab = showSlide ? "slide" : "deck";
      render();
    },
  };
}
