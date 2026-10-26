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

import { isTextBlock, kindOfTag } from "./element-labels.js";

/** Canvas size in CSS pixels for a deck aspect ratio. */
export function canvasSize(aspectRatio) {
  if (aspectRatio === "4:3") return { w: 1024, h: 768 };
  if (aspectRatio === "9:16") return { w: 720, h: 1280 };
  return { w: 1280, h: 720 };
}

/**
 * New positions for a multi-selection after an align or distribute action.
 * Pure: boxes are `{id, x, y, w, h}` in canvas pixels. Returns null for an
 * unknown action; positions are rounded and kept inside the canvas.
 */
export function arrangeBoxes(action, boxes, canvas) {
  const next = boxes.map((box) => ({ ...box }));
  const minX = Math.min(...boxes.map((b) => b.x));
  const maxR = Math.max(...boxes.map((b) => b.x + b.w));
  const minY = Math.min(...boxes.map((b) => b.y));
  const maxB = Math.max(...boxes.map((b) => b.y + b.h));
  switch (action) {
    case "arrange-left": next.forEach((b) => (b.x = minX)); break;
    case "arrange-right": next.forEach((b) => (b.x = maxR - b.w)); break;
    case "arrange-center": next.forEach((b) => (b.x = (minX + maxR) / 2 - b.w / 2)); break;
    case "arrange-top": next.forEach((b) => (b.y = minY)); break;
    case "arrange-bottom": next.forEach((b) => (b.y = maxB - b.h)); break;
    case "arrange-middle": next.forEach((b) => (b.y = (minY + maxB) / 2 - b.h / 2)); break;
    case "distribute-h":
    case "distribute-v": {
      const [pos, size] = action === "distribute-h" ? ["x", "w"] : ["y", "h"];
      const ordered = [...next].sort((a, b) => a[pos] - b[pos]);
      if (ordered.length < 3) break;
      const first = ordered[0];
      const last = ordered[ordered.length - 1];
      const inner = ordered.reduce((sum, b) => sum + b[size], 0);
      const gap = (last[pos] + last[size] - first[pos] - inner) / (ordered.length - 1);
      let cursor = first[pos] + first[size] + gap;
      for (let i = 1; i < ordered.length - 1; i += 1) {
        ordered[i][pos] = cursor;
        cursor += ordered[i][size] + gap;
      }
      break;
    }
    default:
      return null;
  }
  const clamp = (value, max, size) => Math.max(0, Math.min(Math.round(value), Math.max(0, max - size)));
  return next.map((b) => ({
    id: b.id,
    x: clamp(b.x, canvas.w, b.w),
    y: clamp(b.y, canvas.h, b.h),
    w: Math.round(b.w),
    h: Math.round(b.h),
  }));
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

  let selected = null; // { id, tag, text, rect, geom, style, viewport, slideId }
  let many = []; // the blocks of a multi-selection (empty for a single block)
  let hud = null;
  let paddingStep = 0;
  let editing = false;
  let structure = [];
  let colorOpen = false;
  let hudKey = "";
  let announcer = null;

  const ICON = {
    left: '<path d="M4 6h16M4 12h10M4 18h13"/>',
    center: '<path d="M4 6h16M7 12h10M6 18h12"/>',
    right: '<path d="M4 6h16M10 12h10M7 18h13"/>',
    list: '<path d="M9 6h11M9 12h11M9 18h11"/><circle cx="4.5" cy="6" r="1" fill="currentColor"/><circle cx="4.5" cy="12" r="1" fill="currentColor"/><circle cx="4.5" cy="18" r="1" fill="currentColor"/>',
    padding: '<rect x="4" y="4" width="16" height="16" rx="2"/><rect x="8" y="8" width="8" height="8" rx="1"/>',
    close: '<path d="m6 6 12 12M18 6 6 18"/>',
    minus: '<path d="M6 12h12"/>',
    plus: '<path d="M12 6v12M6 12h12"/>',
    // alignment of a multi-selection
    aLeft: '<path d="M4 4v16"/><rect x="7" y="6" width="12" height="4" rx="1"/><rect x="7" y="14" width="7" height="4" rx="1"/>',
    aCenter: '<path d="M12 4v16"/><rect x="5" y="6" width="14" height="4" rx="1"/><rect x="8" y="14" width="8" height="4" rx="1"/>',
    aRight: '<path d="M20 4v16"/><rect x="5" y="6" width="12" height="4" rx="1"/><rect x="10" y="14" width="7" height="4" rx="1"/>',
    aTop: '<path d="M4 4h16"/><rect x="6" y="7" width="4" height="12" rx="1"/><rect x="14" y="7" width="4" height="7" rx="1"/>',
    aMiddle: '<path d="M4 12h16"/><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="8" width="4" height="8" rx="1"/>',
    aBottom: '<path d="M4 20h16"/><rect x="6" y="5" width="4" height="12" rx="1"/><rect x="14" y="10" width="4" height="7" rx="1"/>',
    dH: '<path d="M4 4v16M20 4v16"/><rect x="9" y="7" width="6" height="10" rx="1"/>',
    dV: '<path d="M4 4h16M4 20h16"/><rect x="7" y="9" width="10" height="6" rx="1"/>',
  };
  const icon = (name) =>
    `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICON[name]}</svg>`;
  const SPARK =
    '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 2.6c.7 5.3 1.9 6.5 7.2 7.2-5.3.7-6.5 1.9-7.2 7.2-.7-5.3-1.9-6.5-7.2-7.2 5.3-.7 6.5-1.9 7.2-7.2Zm6.7 11.1c.3 2.4.9 3 3.3 3.3-2.4.3-3 .9-3.3 3.3-.3-2.4-.9-3-3.3-3.3 2.4-.3 3-.9 3.3-3.3Z"/></svg>';

  // ---------- HUD ----------

  const btn = (action, label, body, extra = "") =>
    `<button type="button" class="element-hud-btn${extra}" data-element-action="${action}"
        title="${escapeAttr(label)}" aria-label="${escapeAttr(label)}">${body}</button>`;
  const escapeAttr = (value) => String(value).replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");

  /** Colors offered for text: the deck theme's own, then neutrals. */
  function paletteColors() {
    const theme = actions.themeColors ? actions.themeColors() : null;
    const base = theme ? [theme.fg, theme.accent, theme.bg] : [];
    return [...base, "#ffffff", "#17161a", "#7c3aed", "#ff6b4a", "#16a34a", "#2563eb", "#f59e0b", "#dc2626"]
      .filter((color, index, all) => all.indexOf(color) === index)
      .slice(0, 12);
  }

  function singleMarkup() {
    const style = selected.style || {};
    const kind = kindOfTag(selected.tag, selected.text);
    const label = t(`element.${kind}`);
    const text = isTextBlock(selected.tag, selected.text);
    const textTools = text
      ? `${btn("bold", t("elementHud.bold"), "<b>B</b>", style.bold ? " is-on" : "")}
         ${btn("italic", t("elementHud.italic"), "<i>I</i>", style.italic ? " is-on" : "")}
         <span class="element-hud-sep" aria-hidden="true"></span>
         ${btn("size-down", t("elementHud.sizeDown"), icon("minus"))}
         <output class="element-hud-size" aria-label="${escapeAttr(t("elementHud.size"))}">${Math.round(style.size || 0)}</output>
         ${btn("size-up", t("elementHud.sizeUp"), icon("plus"))}
         ${btn("color", t("elementHud.color"), `<span class="hud-swatch" style="background:${escapeAttr(style.color || "#000")}"></span>`, colorOpen ? " is-on" : "")}
         <span class="element-hud-sep" aria-hidden="true"></span>
         ${btn("align-left", t("elementHud.alignLeft"), icon("left"), style.align === "left" ? " is-on" : "")}
         ${btn("align-center", t("elementHud.alignCenter"), icon("center"), style.align === "center" ? " is-on" : "")}
         ${btn("align-right", t("elementHud.alignRight"), icon("right"), style.align === "right" ? " is-on" : "")}
         ${btn("list", t("elementHud.list"), icon("list"), style.list ? " is-on" : "")}`
      : "";
    const swatches = colorOpen
      ? `<div class="element-hud-colors" role="group" aria-label="${escapeAttr(t("elementHud.color"))}">
          ${paletteColors()
            .map(
              (color) =>
                `<button type="button" class="hud-color" data-color="${escapeAttr(color)}" style="background:${escapeAttr(color)}" aria-label="${escapeAttr(color)}"></button>`
            )
            .join("")}
          <label class="hud-color hud-color-custom" title="${escapeAttr(t("elementHud.customColor"))}">
            <input type="color" data-color-input value="${escapeAttr(style.color || "#000000")}" aria-label="${escapeAttr(t("elementHud.customColor"))}">
          </label>
        </div>`
      : "";
    return `
      <span class="element-hud-tag">${escapeAttr(label)}</span>
      <span class="element-hud-sep" aria-hidden="true"></span>
      ${textTools}
      ${text ? '<span class="element-hud-sep" aria-hidden="true"></span>' : ""}
      ${btn("padding", t("elementHud.padding"), icon("padding"))}
      ${btn("ai", t("elementHud.ai"), SPARK, " element-hud-accent")}
      ${btn("close", t("elementHud.deselect"), icon("close"))}
      ${swatches}`;
  }

  function multiMarkup() {
    return `
      <span class="element-hud-tag">${escapeAttr(t("elementHud.selected", { n: many.length }))}</span>
      <span class="element-hud-sep" aria-hidden="true"></span>
      ${btn("arrange-left", t("elementHud.arrangeLeft"), icon("aLeft"))}
      ${btn("arrange-center", t("elementHud.arrangeCenter"), icon("aCenter"))}
      ${btn("arrange-right", t("elementHud.arrangeRight"), icon("aRight"))}
      <span class="element-hud-sep" aria-hidden="true"></span>
      ${btn("arrange-top", t("elementHud.arrangeTop"), icon("aTop"))}
      ${btn("arrange-middle", t("elementHud.arrangeMiddle"), icon("aMiddle"))}
      ${btn("arrange-bottom", t("elementHud.arrangeBottom"), icon("aBottom"))}
      <span class="element-hud-sep" aria-hidden="true"></span>
      ${btn("distribute-h", t("elementHud.distributeH"), icon("dH"))}
      ${btn("distribute-v", t("elementHud.distributeV"), icon("dV"))}
      ${btn("close", t("elementHud.deselect"), icon("close"))}`;
  }

  function ensureHud() {
    if (hud) return hud;
    const wrap = document.querySelector(".stage-wrap");
    if (!wrap) return null;
    hud = document.createElement("div");
    hud.className = "element-hud";
    hud.id = "element-hud";
    hud.setAttribute("role", "toolbar");
    hud.setAttribute("aria-label", t("elementHud.label"));
    hud.hidden = true;
    wrap.append(hud);
    hud.addEventListener("click", onHudClick);
    hud.addEventListener("input", (event) => {
      if (event.target.matches("[data-color-input]")) applyStyle(`color: ${event.target.value}`);
    });
    // The HUD must never steal the selection from the canvas.
    hud.addEventListener("pointerdown", (event) => {
      if (!event.target.closest("input")) event.preventDefault();
    });
    return hud;
  }

  function renderHud() {
    const layer = ensureHud();
    if (!layer) return;
    if (!selected && !many.length) {
      layer.hidden = true;
      hudKey = "";
      return;
    }
    const anchor = many.length > 1 ? unionRect(many) : selected.rect;
    const viewport = (many.length > 1 ? many[0].viewport : selected.viewport) || {};
    const frameRect = frame.getBoundingClientRect();
    const scale = viewport.w ? frameRect.width / viewport.w : 1;
    const left = frameRect.left + anchor.x * scale;
    const top = frameRect.top + anchor.y * scale;
    const width = anchor.w * scale;

    // Rebuild the markup only when what it shows changed: a stable DOM keeps
    // hover and focus states while the user keeps editing.
    const key = many.length > 1
      ? `m:${many.map((item) => item.id).join(",")}`
      : `s:${selected.id}:${JSON.stringify(selected.style || {})}:${colorOpen}:${t("elementHud.label")}`;
    layer.hidden = false;
    if (key !== hudKey) {
      hudKey = key;
      layer.innerHTML = many.length > 1 ? multiMarkup() : singleMarkup();
      layer.setAttribute("aria-label", t("elementHud.label"));
    }
    const hudWidth = layer.offsetWidth;
    const x = Math.min(
      Math.max(8, left + width / 2 - hudWidth / 2),
      Math.max(8, window.innerWidth - hudWidth - 8)
    );
    const above = top - layer.offsetHeight - 10;
    // Not enough room above the block: tuck the toolbar below it instead.
    const y = above >= 8 ? above : Math.min(frameRect.top + (anchor.y + anchor.h) * scale + 10, window.innerHeight - layer.offsetHeight - 8);
    layer.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }

  function unionRect(items) {
    const xs = items.map((item) => item.rect.x);
    const ys = items.map((item) => item.rect.y);
    const rs = items.map((item) => item.rect.x + item.rect.w);
    const bs = items.map((item) => item.rect.y + item.rect.h);
    const x = Math.min(...xs);
    const y = Math.min(...ys);
    return { x, y, w: Math.max(...rs) - x, h: Math.max(...bs) - y };
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

  /** Aligns or distributes a multi-selection; one undo step. */
  function arrange(action) {
    if (many.length < 2) return;
    const next = arrangeBoxes(
      action,
      many.map((item) => ({ id: item.id, ...item.geom })),
      canvasSize(actions.aspectRatio())
    );
    if (!next) return;
    actions.command("set_geometries", {
      slide_id: actions.currentSlideId(),
      items: next.map((b) => ({ element_id: b.id, x: b.x, y: b.y, w: b.w, h: b.h })),
    });
  }

  function onHudClick(event) {
    const swatch = event.target.closest("[data-color]");
    if (swatch && selected) {
      applyStyle(`color: ${swatch.dataset.color}`);
      colorOpen = false;
      return;
    }
    const button = event.target.closest("[data-element-action]");
    if (!button) return;
    const action = button.dataset.elementAction;
    if (action === "close") {
      clearSelection();
      return;
    }
    if (action.startsWith("arrange-") || action.startsWith("distribute-")) {
      arrange(action);
      return;
    }
    if (!selected) return;
    const style = selected.style || {};
    if (action === "ai") {
      actions.openPaletteForElement(selected);
    } else if (action === "padding") {
      const steps = ["padding: 0.35em 0.5em", "padding: 1.1em 1.4em", "padding: 2em 2.4em"];
      paddingStep = (paddingStep + 1) % steps.length;
      applyStyle(steps[paddingStep]);
    } else if (action.startsWith("align-")) {
      applyStyle(`text-align: ${action.slice(6)}`);
    } else if (action === "bold") {
      applyStyle(`font-weight: ${style.bold ? 400 : 700}`);
    } else if (action === "italic") {
      applyStyle(`font-style: ${style.italic ? "normal" : "italic"}`);
    } else if (action === "size-down" || action === "size-up") {
      const size = Math.max(8, Math.round((style.size || 24) * (action === "size-up" ? 1.1 : 1 / 1.1)));
      applyStyle(`font-size: ${size}px`);
    } else if (action === "color") {
      colorOpen = !colorOpen;
      renderHud();
    } else if (action === "list") {
      post("toggle-list");
    }
  }

  function post(type, extra = {}) {
    if (frame.contentWindow) frame.contentWindow.postMessage({ __slyde: true, type, ...extra }, "*");
  }

  /** Reads a short sentence to screen readers; the canvas itself is a picture. */
  function announce(message) {
    if (!announcer) {
      announcer = document.createElement("p");
      announcer.className = "sr-only";
      announcer.setAttribute("role", "status");
      announcer.setAttribute("aria-live", "polite");
      announcer.id = "canvas-announcer";
      document.body.append(announcer);
    }
    announcer.textContent = "";
    window.setTimeout(() => {
      announcer.textContent = message;
    }, 30);
  }

  // ---------- selection ----------

  function clearSelection({ notifySandbox = true } = {}) {
    selected = null;
    many = [];
    colorOpen = false;
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
      many = [];
      selected = { ...data, slideId: actions.currentSlideId() };
      paddingStep = 0;
      if (changed) colorOpen = false;
      renderHud();
      actions.onSelectedElement(selected);
      if (changed) announce(t("canvas.selected", { kind: t(`element.${kindOfTag(data.tag, data.text)}`), text: data.text || "" }));
      // Pull focus back to the studio so its shortcuts keep working. Skip it on
      // the second click of a double-click and while editing, otherwise the
      // sandbox loses focus mid-edit and commits immediately.
      if (changed && !editing) focusCanvas();
    } else if (data.type === "multi") {
      selected = null;
      many = Array.isArray(data.items) ? data.items : [];
      renderHud();
      actions.onSelectedElement(null);
      announce(t("canvas.multi", { n: many.length }));
      if (!editing) focusCanvas();
    } else if (data.type === "html") {
      if (data.id && typeof data.value === "string") {
        actions.command("mutate_element", {
          slide_id: actions.currentSlideId(),
          element_id: data.id,
          html: data.value,
        });
      }
    } else if (data.type === "geometry-many") {
      const items = (Array.isArray(data.items) ? data.items : [])
        .filter((item) => item && item.id && item.rect)
        .map((item) => ({
          element_id: item.id,
          x: Math.round(item.rect.x),
          y: Math.round(item.rect.y),
          w: Math.round(item.rect.w),
          h: Math.round(item.rect.h),
        }));
      if (items.length) {
        actions.command("set_geometries", { slide_id: actions.currentSlideId(), items });
      }
    } else if (data.type === "editing") {
      editing = true;
    } else if (data.type === "deselect") {
      editing = false;
      selected = null;
      many = [];
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
    } else if (data.type === "image-geometry") {
      commitGeometry(data.id, data.rect);
    } else if (data.type === "tree") {
      structure = Array.isArray(data.nodes) ? data.nodes : [];
      actions.onStructure(structure);
    }
  }

  /** Keyboard focus goes to the canvas region, never into the slide document. */
  function focusCanvas() {
    const region = document.getElementById("stage");
    if (region) region.focus({ preventScroll: true });
    else {
      document.body.tabIndex = -1;
      document.body.focus({ preventScroll: true });
    }
  }

  /// Keyboard model of the canvas region (`#stage`): Enter picks the first block
  /// (or edits the picked one), Tab and Shift+Tab walk the blocks in reading
  /// order, arrows nudge (Shift = 10px), Escape lets go. All without a mouse.
  function onCanvasKeydown(event) {
    if (event.target !== event.currentTarget) return;
    const hasSelection = Boolean(selected) || many.length > 1;
    const step = event.shiftKey ? 10 : 1;
    const arrows = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
    if (event.key === "Tab" && hasSelection) {
      event.preventDefault();
      post("select-step", { delta: event.shiftKey ? -1 : 1 });
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (selected) post("edit-selected");
      else post("select-step", { delta: 1 });
    } else if (event.key === "Escape" && hasSelection) {
      event.preventDefault();
      event.stopPropagation();
      clearSelection();
      announce(t("canvas.cleared"));
    } else if (arrows[event.key] && hasSelection) {
      event.preventDefault();
      event.stopPropagation();
      post("nudge", { dx: arrows[event.key][0], dy: arrows[event.key][1] });
    }
  }

  /// Wires the listeners once, at boot.
  function attach() {
    const region = document.getElementById("stage");
    if (region) {
      region.tabIndex = 0;
      region.setAttribute("role", "group");
      region.setAttribute("aria-label", t("canvas.label"));
      region.addEventListener("keydown", onCanvasKeydown);
    }
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
    hudKey = "";
    const region = document.getElementById("stage");
    if (region) region.setAttribute("aria-label", t("canvas.label"));
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
