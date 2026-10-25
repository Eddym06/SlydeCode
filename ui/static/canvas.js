// Bimodal canvas: element selection, the floating HUD and image editing.
//
// The sandbox owns hit-testing (it sees the real DOM), this module owns the
// selection, the chrome around it and the messages, and the shell owns the deck.
// Nothing here rewrites a slide: every change goes through `command`
// (`mutate_element`, `set_geometry`, `insert_image`), so undo/redo, scoping and
// autosave keep working.
//
// Extracted from the studio shell, where it was ~300 lines of state, HUD markup
// and message plumbing interleaved with the rest of the boot closure.

/** Canvas size in CSS pixels for a deck aspect ratio. */
export function canvasSize(aspectRatio) {
  if (aspectRatio === "4:3") return { w: 1024, h: 768 };
  if (aspectRatio === "9:16") return { w: 720, h: 1280 };
  return { w: 1280, h: 720 };
}

/** Centres a new picture at 60% of the canvas width, keeping its aspect ratio. */
export function placementFor(natural, aspectRatio) {
  const { w: W, h: H } = canvasSize(aspectRatio);
  const ratio = natural && natural.w > 0 && natural.h > 0 ? natural.h / natural.w : 9 / 16;
  let w = Math.round(W * 0.6);
  let h = Math.round(w * ratio);
  if (h > H * 0.9) {
    h = Math.round(H * 0.9);
    w = Math.round(h / ratio);
  }
  return {
    x: Math.round((W - w) / 2),
    y: Math.round((H - h) / 2),
    w: Math.max(16, w),
    h: Math.max(16, h),
  };
}

export function fileToDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error || new Error("could not read the file"));
    reader.readAsDataURL(file);
  });
}

export function imageSize(dataUrl) {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve({ w: image.naturalWidth, h: image.naturalHeight });
    image.onerror = () => resolve({ w: 1024, h: 576 });
    image.src = dataUrl;
  });
}

/**
 * @param {object} options
 * @param {HTMLIFrameElement} options.frame the stage iframe
 * @param {(key: string, vars?: object) => string} options.t
 * @param {object} options.actions
 *   command, uploadImage, notify, currentSlideId, aspectRatio,
 *   openPaletteForElement, onSelectedElement, onStructure, applyI18n
 */
export function createCanvas({ frame, t, actions }) {
  if (!frame) throw new Error("the canvas needs its frame");

  let selected = null; // { id, tag, text, rect, viewport, slideId }
  let hud = null;
  let paddingStep = 0;
  let editing = false;
  let structure = [];

  // ---------- HUD ----------

  function ensureHud() {
    if (hud) return hud;
    const wrap = document.querySelector(".stage-wrap");
    if (!wrap) return null;
    hud = document.createElement("div");
    hud.className = "element-hud";
    hud.id = "element-hud";
    hud.setAttribute("role", "toolbar");
    hud.setAttribute("data-i18n-aria", "elementHud.label");
    hud.hidden = true;
    hud.innerHTML = `
      <span class="element-hud-tag" id="element-hud-tag"></span>
      <span class="element-hud-sep" aria-hidden="true"></span>
      <button type="button" class="element-hud-btn" data-element-action="align-left"
              data-i18n-title="elementHud.alignLeft" data-i18n-aria="elementHud.alignLeft">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M4 6h16M4 12h10M4 18h13"/>
        </svg>
      </button>
      <button type="button" class="element-hud-btn" data-element-action="align-center"
              data-i18n-title="elementHud.alignCenter" data-i18n-aria="elementHud.alignCenter">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M4 6h16M7 12h10M6 18h12"/>
        </svg>
      </button>
      <button type="button" class="element-hud-btn" data-element-action="align-right"
              data-i18n-title="elementHud.alignRight" data-i18n-aria="elementHud.alignRight">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="M4 6h16M10 12h10M7 18h13"/>
        </svg>
      </button>
      <button type="button" class="element-hud-btn" data-element-action="padding"
              data-i18n-title="elementHud.padding" data-i18n-aria="elementHud.padding">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
          <rect x="4" y="4" width="16" height="16" rx="2"/><rect x="8" y="8" width="8" height="8" rx="1"/>
        </svg>
      </button>
      <button type="button" class="element-hud-btn element-hud-accent" data-element-action="ai"
              data-i18n-title="elementHud.ai" data-i18n-aria="elementHud.ai">
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <path d="M12 2.6c.7 5.3 1.9 6.5 7.2 7.2-5.3.7-6.5 1.9-7.2 7.2-.7-5.3-1.9-6.5-7.2-7.2 5.3-.7 6.5-1.9 7.2-7.2Zm6.7 11.1c.3 2.4.9 3 3.3 3.3-2.4.3-3 .9-3.3 3.3-.3-2.4-.9-3-3.3-3.3 2.4-.3 3-.9 3.3-3.3Z"/>
        </svg>
      </button>
      <button type="button" class="element-hud-btn" data-element-action="close"
              data-i18n-title="elementHud.deselect" data-i18n-aria="elementHud.deselect">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <path d="m6 6 12 12M18 6 6 18"/>
        </svg>
      </button>`;
    wrap.append(hud);
    actions.applyI18n(hud);
    hud.addEventListener("click", onHudClick);
    return hud;
  }

  function renderHud() {
    const layer = ensureHud();
    if (!layer) return;
    if (!selected) {
      layer.hidden = true;
      return;
    }
    const frameRect = frame.getBoundingClientRect();
    const viewport = selected.viewport || { w: frameRect.width, h: frameRect.height };
    const scale = viewport.w ? frameRect.width / viewport.w : 1;
    const rect = selected.rect;
    const left = frameRect.left + rect.x * scale;
    const top = frameRect.top + rect.y * scale;
    const width = rect.w * scale;

    layer.hidden = false;
    const tag = layer.querySelector("#element-hud-tag");
    if (tag) tag.textContent = `#${selected.id}`;
    const hudWidth = layer.offsetWidth;
    const x = Math.min(
      Math.max(8, left + width / 2 - hudWidth / 2),
      Math.max(8, window.innerWidth - hudWidth - 8)
    );
    const y = Math.max(8, top - layer.offsetHeight - 10);
    layer.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }

  function applyStyle(style) {
    if (!selected) return;
    // Generated handles repeat in every slide, so the edit is always scoped.
    actions.command("mutate_element", {
      slide_id: selected.slideId || actions.currentSlideId(),
      element_id: selected.id,
      style,
    });
  }

  function onHudClick(event) {
    const button = event.target.closest("[data-element-action]");
    if (!button || !selected) return;
    const action = button.dataset.elementAction;
    if (action === "close") {
      clearSelection();
    } else if (action === "ai") {
      actions.openPaletteForElement(selected);
    } else if (action === "padding") {
      const steps = ["padding: 0.35em 0.5em", "padding: 1.1em 1.4em", "padding: 2em 2.4em"];
      paddingStep = (paddingStep + 1) % steps.length;
      applyStyle(steps[paddingStep]);
    } else if (action.startsWith("align-")) {
      applyStyle(`text-align: ${action.slice(6)}`);
    }
  }

  // ---------- selection ----------

  function clearSelection({ notifySandbox = true } = {}) {
    const id = selected && selected.id;
    selected = null;
    renderHud();
    actions.onSelectedElement(null);
    if (notifySandbox && frame.contentWindow) {
      frame.contentWindow.postMessage({ __slyde: true, type: "clear-selection" }, "*");
    }
  }

  function resetStructure() {
    structure = [];
  }

  function selectById(id) {
    if (!id || !frame.contentWindow) return;
    frame.contentWindow.postMessage({ __slyde: true, type: "select-by-id", id }, "*");
  }

  // ---------- images ----------

  async function insertFile(file) {
    const dataUrl = await fileToDataUrl(file);
    await insertDataUrl(dataUrl, await imageSize(dataUrl));
  }

  async function insertDataUrl(dataUrl, natural) {
    const slideId = actions.currentSlideId();
    if (!dataUrl || !slideId) return;
    try {
      const src = await actions.uploadImage(dataUrl);
      const rect = placementFor(natural, actions.aspectRatio());
      const inserted = await actions.command("insert_image", { slide_id: slideId, src, ...rect });
      if (inserted) actions.notify(t("toast.imageAdded"));
    } catch (error) {
      actions.notify(`${t("toast.error")} — ${error.message}`);
    }
  }

  function fileFromClipboard(event) {
    const items = (event.clipboardData && event.clipboardData.items) || [];
    for (const item of items) {
      if (item.type && item.type.startsWith("image/")) {
        const file = item.getAsFile();
        if (file) return file;
      }
    }
    const files = (event.dataTransfer && event.dataTransfer.files) || [];
    for (const file of files) {
      if (file.type && file.type.startsWith("image/")) return file;
    }
    return null;
  }

  /// Paste or drop an image anywhere in the studio shell. The slide document has
  /// its own listeners inside the iframe; this covers the panels around it.
  async function insertFromEvent(event) {
    const target = event.target;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
    const file = fileFromClipboard(event);
    if (!file) return;
    event.preventDefault();
    await insertFile(file);
  }

  function commitGeometry(elementId, rect) {
    const slideId = actions.currentSlideId();
    if (!elementId || !rect || !slideId) return;
    actions.command("set_geometry", {
      slide_id: slideId,
      element_id: elementId,
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      w: Math.round(rect.w),
      h: Math.round(rect.h),
    });
  }

  // ---------- sandbox messages ----------

  function handleMessage(event) {
    if (event.source !== frame.contentWindow) return;
    const data = event.data;
    if (!data || data.__slyde !== true) return;

    if (data.type === "select") {
      const changed = !selected || selected.id !== data.id;
      selected = { ...data, slideId: actions.currentSlideId() };
      paddingStep = 0;
      renderHud();
      actions.onSelectedElement(selected);
      // Pull focus back to the studio so its shortcuts keep working. Skip it on
      // the second click of a double-click and while editing, otherwise the
      // sandbox loses focus mid-edit and commits immediately.
      if (changed && !editing) {
        document.body.tabIndex = -1;
        document.body.focus({ preventScroll: true });
      }
    } else if (data.type === "editing") {
      editing = true;
    } else if (data.type === "deselect") {
      editing = false;
      selected = null;
      renderHud();
      actions.onSelectedElement(null);
    } else if (data.type === "text") {
      editing = false;
      const id = data.id;
      const value = typeof data.value === "string" ? data.value : "";
      if (id) {
        actions.command("mutate_element", {
          slide_id: actions.currentSlideId(),
          element_id: id,
          text: value,
        });
      }
    } else if (data.type === "image-paste") {
      insertDataUrl(data.dataUrl, data.natural);
    } else if (data.type === "geometry" || data.type === "image-geometry") {
      // `image-geometry` is the pre-gizmo name; both carry stage pixels.
      commitGeometry(data.id, data.rect);
    } else if (data.type === "dragging") {
      // The sandbox repaints its own outline while dragging; the HUD would trail
      // behind it, so it steps aside until the new selection rect arrives.
      if (hud) hud.hidden = true;
    } else if (data.type === "tree") {
      structure = Array.isArray(data.nodes) ? data.nodes : [];
      actions.onStructure(structure);
    }
  }

  /// Wires the listeners once, at boot.
  function attach() {
    window.addEventListener("message", handleMessage);
    document.addEventListener("paste", insertFromEvent);
    document.addEventListener("drop", insertFromEvent);
    frame.addEventListener("load", () => {
      if (selected && frame.contentWindow) {
        frame.contentWindow.postMessage(
          { __slyde: true, type: "select-by-id", id: selected.id },
          "*"
        );
      }
    });
  }

  /// Re-applies translations to the HUD and repositions it (language switch).
  function refreshHud() {
    if (hud) actions.applyI18n(hud);
    renderHud();
  }

  return {
    attach,
    refreshHud,
    renderHud,
    resetStructure,
    selectById,
    clearSelection,
    insertFile,
    get selectedElement() {
      return selected;
    },
    get structureNodes() {
      return structure;
    },
  };
}
