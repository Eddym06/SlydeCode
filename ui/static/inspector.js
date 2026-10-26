// Studio inspector: slide tab, deck tab and the animation timeline.
//
// It used to live inside the studio's boot closure, which made the shell 2,400
// lines of intertwined state and markup. The inspector is a self-contained panel:
// it renders from the read model it is given and sends commands through the
// callbacks, so it never touches the deck itself.
//
// The DOM ids are deliberately unchanged (`#theme-select`, `#notes`, `#export-pdf`,
// …): they are the contract with the export progress handler and with the E2E
// suite. What changed is who owns the markup and the event wiring.

/** Every theme the backend accepts, with the dots the swatches show. */
export const THEME_CHOICES = [
  { id: "studio", dots: ["#7C3AED", "#FF6B4A"] },
  { id: "paper", dots: ["#2C2B2A", "#D97706"] },
  { id: "night", dots: ["#9B6BFF", "#FF8265"] },
  { id: "terminal", dots: ["#00FF66", "#00E5FF"] },
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
  let current = { deck: null, slide: null, selectedElement: null, structureNodes: [] };

  // ---------- animation editor ----------

  function renderAnimationEditor(slide) {
    const rows = (slide.animation || [])
      .map(
        (step, index) => `
        <div class="anim-step" data-index="${index}" data-easing="${escapeHtml(step.easing || DEFAULT_EASING)}">
          <div class="anim-step-head">
            <span class="step-index">${index + 1}</span>
            <select class="anim-kind" data-field="kind" aria-label="${escapeHtml(t("inspector.stepKind"))}">
              ${ANIMATION_KINDS.map(
                (kind) =>
                  `<option value="${kind}"${kind === step.kind ? " selected" : ""}>${kind}</option>`
              ).join("")}
            </select>
            <button type="button" class="step-play" data-play="${index}"
                    data-i18n-title="action.play" aria-label="${escapeHtml(t("action.play"))}">▶</button>
            <button type="button" class="anim-btn" data-move="-1"
                    data-i18n-title="animation.moveUp" aria-label="${escapeHtml(t("animation.moveUp"))}"
                    ${index === 0 ? "disabled" : ""}>↑</button>
            <button type="button" class="anim-btn" data-move="1"
                    data-i18n-title="animation.moveDown" aria-label="${escapeHtml(t("animation.moveDown"))}"
                    ${index === slide.animation.length - 1 ? "disabled" : ""}>↓</button>
            <button type="button" class="anim-btn anim-btn-danger" data-remove="${index}"
                    data-i18n-title="animation.remove" aria-label="${escapeHtml(t("animation.remove"))}">✕</button>
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
        </div>`
      )
      .join("");

    return `
      <div class="anim-editor" id="anim-editor">
        ${rows || `<p class="hint">${escapeHtml(t("inspector.noAnimation"))}</p>`}
        <button type="button" class="anim-add" id="anim-add">+ ${escapeHtml(t("inspector.addStep"))}</button>
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
    const themeOptions = THEME_CHOICES.map(
      (choice) =>
        `<option value="${choice.id}" ${deck.theme.id === choice.id ? "selected" : ""}>${escapeHtml(
          t(`theme.${choice.id}`)
        )}</option>`
    ).join("");
    const themeSwatches = THEME_CHOICES.map(
      (choice) => `
          <button type="button" class="theme-swatch" data-theme="${choice.id}" role="radio" aria-checked="${
            deck.theme.id === choice.id
          }" aria-label="${escapeHtml(t(`theme.${choice.id}`))}">
            <span class="swatch-dots">
              ${choice.dots
                .map((color) => `<span class="swatch-dot" style="background:${color}"></span>`)
                .join("")}
            </span>
            <span class="swatch-name">${escapeHtml(t(`theme.${choice.id}`))}</span>
          </button>`
    ).join("");
    const ratioOptions = ["16:9", "4:3", "9:16"]
      .map(
        (ratio) =>
          `<option value="${ratio}" ${deck.aspect_ratio === ratio ? "selected" : ""}>${ratio}</option>`
      )
      .join("");

    return `
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.themeSwatches"))}</span>
        <div class="theme-swatches" role="radiogroup" aria-label="${escapeHtml(t("inspector.themeSwatches"))}">
          ${themeSwatches}
        </div>
      </section>
      <section class="field">
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
          <button type="button" class="file-btn" id="export-pptx">${escapeHtml(t("export.pptx"))}</button>
          <button type="button" class="file-btn" id="export-html">${escapeHtml(t("export.html"))}</button>
        </div>
        <label class="export-quality">
          <span class="hint">${escapeHtml(t("export.quality"))}</span>
          <select class="select" id="export-scale">
            <option value="1"${readPngScale() === 1 ? " selected" : ""}>${escapeHtml(t("export.quality.standard"))}</option>
            <option value="2"${readPngScale() === 2 ? " selected" : ""}>${escapeHtml(t("export.quality.high"))}</option>
          </select>
        </label>
        <p class="hint export-status" id="export-status" role="status" aria-live="polite">${escapeHtml(t("export.hint"))}</p>
      </section>`;
  }

  function slideMarkup(slide, { selectedElement, structureNodes }) {
    const tree = structureNodes.length
      ? `<ul class="element-tree">${structureNodes
          .map(
            (node) => `<li>
                    <button type="button" class="element-tree-item${
                      selectedElement && selectedElement.id === node.id ? " is-selected" : ""
                    }" data-tree-id="${escapeHtml(node.id)}">
                      <span class="element-tree-tag">${escapeHtml(node.tag)}</span>
                      <span class="element-tree-id">#${escapeHtml(node.id)}</span>
                      ${node.text ? `<span class="element-tree-text">${escapeHtml(node.text)}</span>` : ""}
                    </button>
                  </li>`
          )
          .join("")}</ul>`
      : `<p class="hint">${escapeHtml(t("tree.empty"))}</p>`;

    return `
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.slide"))}</span>
        <div class="field-value">${escapeHtml(t(`template.${slide.template}`))}</div>
        <div class="mono" title="${escapeHtml(slide.id)}">${escapeHtml(slide.id)}</div>
        <div class="slide-quick-actions">
          <button type="button" class="btn btn-primary" id="inspector-redesign-slide" style="gap: 5px;">
            <svg viewBox="0 0 24 24" fill="currentColor" width="13" height="13"><path d="M12 2.6c.7 5.3 1.9 6.5 7.2 7.2-5.3.7-6.5 1.9-7.2 7.2-.7-5.3-1.9-6.5-7.2-7.2 5.3-.7 6.5-1.9 7.2-7.2Zm6.7 11.1c.3 2.4.9 3 3.3 3.3-2.4.3-3 .9-3.3 3.3-.3-2.4-.9-3-3.3-3.3 2.4-.3 3-.9 3.3-3.3Z"/></svg>
            ${escapeHtml(t("action.redesign"))}
          </button>
          <button type="button" class="btn" id="inspector-insert-image">${escapeHtml(t("inspector.insertImage"))}</button>
          <input type="file" id="inspector-image-file" accept="image/png,image/jpeg,image/webp,image/gif" hidden>
          <button type="button" class="btn" id="inspector-duplicate-slide">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="14" height="14"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
            ${escapeHtml(t("inspector.duplicate"))}
          </button>
          <button type="button" class="danger-btn" id="delete-slide">${escapeHtml(t("action.delete"))}</button>
        </div>
      </section>
      <section class="field">
        <label class="field-label" for="notes">${escapeHtml(t("inspector.notes"))}</label>
        <textarea class="notes" id="notes" placeholder="${escapeHtml(t("inspector.notesPlaceholder"))}">${escapeHtml(slide.notes || "")}</textarea>
      </section>
      <section class="field">
        <span class="field-label">${escapeHtml(t("inspector.animation"))}</span>
        ${renderAnimationEditor(slide)}
      </section>
      <section class="field">
        <span class="field-label">${escapeHtml(t("tree.title"))}</span>
        ${tree}
      </section>
      <details class="dev">
        <summary>${escapeHtml(t("inspector.developer"))}</summary>
        <p class="hint">${escapeHtml(t("inspector.developerHint"))}</p>
        <pre class="dev-code">${escapeHtml(slide.code)}</pre>
      </details>`;
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

    if (!currentSlide) {
      root.innerHTML = `<div id="inspector-deck-section">${deckSection}</div>`;
      actions.onEmptyDeck();
      bindDeckControls();
      return;
    }

    const showSlide = subtab === "slide";
    root.innerHTML = `
      <div id="inspector-slide-section" ${showSlide ? "" : 'style="display:none"'}>
        ${slideMarkup(currentSlide, { selectedElement, structureNodes })}
      </div>
      <div id="inspector-deck-section" ${showSlide ? 'style="display:none"' : ""}>
        ${deckSection}
      </div>`;

    actions.onSubtabs(showSlide);
    bindDeckControls();

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

    const redesign = root.querySelector("#inspector-redesign-slide");
    if (redesign) {
      redesign.addEventListener("click", () => {
        if (actions.redesignSlide) actions.redesignSlide(currentSlide.id);
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
