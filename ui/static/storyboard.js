// Studio storyboard: the list of slides, its thumbnails and the drag-and-drop.
//
// Extracted from the studio shell for the same reason as the inspector: it is a
// panel with its own markup and its own interaction (drag to reorder, hover
// actions, roving tabindex), and the shell should only own the deck state.
//
// Every thumbnail is a live slide document rendered from the sandbox origin, so
// the list shows exactly what the presenter will show.

/**
 * @param {object} options
 * @param {HTMLElement} options.root
 * @param {(key: string, vars?: object) => string} options.t
 * @param {(value: unknown) => string} options.escapeHtml
 * @param {(text: string) => string} options.hash
 * @param {object} options.actions
 *   command(action, payload), selectSlide(id), applyI18n(root)
 */
export function createStoryboard({ root, t, escapeHtml, hash, actions }) {
  if (!root) throw new Error("the storyboard needs its container");

  let key = "";
  let slides = [];
  let selectedId = null;
  let sandboxOrigin = "";
  let sourceQuery = "";

  function invalidate() {
    key = "";
  }

  /// Roving tabindex: only the selected thumb is in the tab order, so Tab jumps
  /// straight into the deck and the arrow keys walk it.
  function syncSelection() {
    root.querySelectorAll(".thumb").forEach((element) => {
      const current = element.dataset.id === selectedId;
      element.setAttribute("aria-current", String(current));
      element.tabIndex = current ? 0 : -1;
    });
  }

  function focusSelected() {
    const target = [...root.querySelectorAll(".thumb")].find(
      (element) => element.dataset.id === selectedId
    );
    if (target) target.focus();
  }

  function thumbMarkup(slide, index, total, ratio) {
    const [w, h] = ratio;
    const label = t("storyboard.slide", {
      n: index + 1,
      template: t(`template.${slide.template}`),
    });
    return {
      label,
      html: `
        <span class="thumb-media" style="aspect-ratio:${w}/${h}">
          <iframe sandbox="allow-scripts" loading="lazy" tabindex="-1" aria-hidden="true" title=""
            src="${sandboxOrigin}/s/${encodeURIComponent(slide.id)}?static=1${sourceQuery}"></iframe>
        </span>
        <span class="thumb-foot">
          <span class="thumb-index">${String(index + 1).padStart(2, "0")}</span>
          <span class="thumb-template">${escapeHtml(t(`template.${slide.template}`))}</span>
          <span class="thumb-actions">
            ${
              index > 0
                ? `<button type="button" class="thumb-btn" data-action="up" data-id="${slide.id}" data-idx="${index}" title="${escapeHtml(t("storyboard.moveUp"))}" aria-label="${escapeHtml(t("storyboard.moveUp"))}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 19V5M5 12l7-7 7 7"/></svg></button>`
                : ""
            }
            ${
              index < total - 1
                ? `<button type="button" class="thumb-btn" data-action="down" data-id="${slide.id}" data-idx="${index}" title="${escapeHtml(t("storyboard.moveDown"))}" aria-label="${escapeHtml(t("storyboard.moveDown"))}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 5v14M5 12l7 7 7-7"/></svg></button>`
                : ""
            }
            <button type="button" class="thumb-btn" data-action="dup" data-id="${slide.id}" title="${escapeHtml(t("storyboard.duplicate"))}" aria-label="${escapeHtml(t("storyboard.duplicate"))}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button>
            <button type="button" class="thumb-btn thumb-btn-danger" data-action="del" data-id="${slide.id}" title="${escapeHtml(t("storyboard.delete"))}" aria-label="${escapeHtml(t("storyboard.delete"))}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg></button>
          </span>
        </span>`,
    };
  }

  function render({ slides: next, aspectRatio, selectedId: id, sandboxOrigin: origin, sourceQuery: query }) {
    slides = next || [];
    selectedId = id || null;
    sandboxOrigin = origin || "";
    sourceQuery = query || "";

    const nextKey =
      slides.map((slide) => `${slide.id}:${hash(slide.code)}:${slide.template}`).join("|") +
      `:${aspectRatio}`;
    if (nextKey === key) {
      syncSelection();
      return;
    }
    key = nextKey;

    const hadFocus = root.contains(document.activeElement);
    const ratio = String(aspectRatio || "16:9")
      .split(":")
      .map(Number);

    root.innerHTML = "";
    if (!slides.length) {
      const hint = document.createElement("p");
      hint.className = "hint";
      hint.style.padding = "8px 4px";
      hint.textContent = t("empty.body");
      root.append(hint);
      actions.applyI18n(root);
      return;
    }

    slides.forEach((slide, index) => {
      const { label, html } = thumbMarkup(slide, index, slides.length, ratio);
      const button = document.createElement("button");
      button.type = "button";
      button.className = "thumb";
      button.dataset.id = slide.id;
      button.draggable = true;
      button.tabIndex = slide.id === selectedId ? 0 : -1;
      button.setAttribute("aria-current", String(slide.id === selectedId));
      button.setAttribute("aria-label", label);

      button.addEventListener("dragstart", (event) => {
        event.dataTransfer.setData("text/plain", String(index));
        event.dataTransfer.effectAllowed = "move";
        button.classList.add("dragging");
      });
      button.addEventListener("dragend", () => {
        button.classList.remove("dragging");
        root.querySelectorAll(".thumb").forEach((thumb) => {
          thumb.classList.remove("drag-over-top", "drag-over-bottom");
        });
      });
      button.addEventListener("dragover", (event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        const rect = button.getBoundingClientRect();
        const mid = rect.top + rect.height / 2;
        if (event.clientY < mid) {
          button.classList.add("drag-over-top");
          button.classList.remove("drag-over-bottom");
        } else {
          button.classList.add("drag-over-bottom");
          button.classList.remove("drag-over-top");
        }
      });
      button.addEventListener("dragleave", () => {
        button.classList.remove("drag-over-top", "drag-over-bottom");
      });
      button.addEventListener("drop", (event) => {
        event.preventDefault();
        const from = Number(event.dataTransfer.getData("text/plain"));
        if (Number.isNaN(from) || from === index) return;
        const rect = button.getBoundingClientRect();
        const mid = rect.top + rect.height / 2;
        let to = index;
        if (event.clientY >= mid && from > index) to += 1;
        actions.command("move_slide", { from, to });
      });

      button.innerHTML = html;

      button.querySelectorAll(".thumb-btn").forEach((actionButton) => {
        actionButton.addEventListener("click", (event) => {
          event.stopPropagation();
          const action = actionButton.dataset.action;
          const slideId = actionButton.dataset.id;
          const slideIndex = Number(actionButton.dataset.idx);
          if (action === "dup") actions.command("duplicate_slide", { slide_id: slideId });
          else if (action === "del") actions.command("remove_slide", { slide_id: slideId });
          else if (action === "up") actions.command("move_slide", { from: slideIndex, to: slideIndex - 1 });
          else if (action === "down") actions.command("move_slide", { from: slideIndex, to: slideIndex + 1 });
        });
      });

      button.addEventListener("click", () => actions.selectSlide(slide.id));
      root.append(button);
    });

    actions.applyI18n(root);
    if (hadFocus) focusSelected();
  }

  return {
    render,
    invalidate,
    focusSelected,
    syncSelection,
    get key() {
      return key;
    },
  };
}
