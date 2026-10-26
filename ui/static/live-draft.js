// Live preview of the slide the copilot is writing right now.
//
// Slow models take tens of seconds to finish a slide; the canvas would sit
// empty until the closing brace arrives. The server forwards the markup written
// so far and this module paints it, scaled to the stage, in a locked-down frame:
// no scripts, no network, no storage. It is a sketch of the work in progress;
// the real slide replaces it the moment it is applied.

import { THEME_COLORS } from "./layout-previews.js";

/** How long a quiet stream keeps its last sketch before the overlay fades. */
export const IDLE_MS = 4500;

const escapeAttr = (value) => String(value).replace(/[&"<>]/g, (c) => ({ "&": "&amp;", '"': "&quot;", "<": "&lt;", ">": "&gt;" })[c]);

/**
 * The self-contained document shown in the preview frame.
 * @param {{html: string, themeId?: string, size: {w: number, h: number}}} input
 */
export function buildDraftDocument({ html, themeId, size }) {
  const colors = THEME_COLORS[themeId] || THEME_COLORS.studio;
  return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:">
<style>
:root{--slyde-bg:${colors.bg};--slyde-fg:${colors.fg};--slyde-accent:${colors.accent};--slyde-muted:color-mix(in srgb,${colors.fg} 62%,transparent);--slyde-font:system-ui,-apple-system,"Segoe UI",sans-serif}
html,body{margin:0;width:${size.w}px;height:${size.h}px;overflow:hidden;background:var(--slyde-bg);color:var(--slyde-fg);font-family:var(--slyde-font)}
*{box-sizing:border-box}
</style></head><body>${html}</body></html>`;
}

/** Scale that fits a canvas of `size` inside `box` without cropping. */
export function fitScale(size, box) {
  if (!size.w || !size.h || !box.w || !box.h) return 1;
  return Math.min(box.w / size.w, box.h / size.h);
}

/**
 * @param {{host: HTMLElement, getTheme: () => string, getSize: () => {w: number, h: number}, t: (key: string) => string}} options
 */
export function mountLiveDraft({ host, getTheme, getSize, t }) {
  const layer = document.createElement("div");
  layer.className = "live-draft";
  layer.hidden = true;
  layer.setAttribute("aria-hidden", "true");
  layer.innerHTML = `<iframe class="live-draft-frame" sandbox="" tabindex="-1" aria-hidden="true"></iframe>
    <span class="live-draft-badge"></span>`;
  host.append(layer);
  const frame = layer.querySelector("iframe");
  const badge = layer.querySelector(".live-draft-badge");
  frame.title = t("copilot.draftBadge");
  let last = "";
  let idleTimer = 0;
  let fadeTimer = 0;
  let pending = null;
  let raf = 0;

  function layout() {
    const size = getSize();
    const scale = fitScale(size, { w: layer.clientWidth, h: layer.clientHeight });
    frame.style.width = `${size.w}px`;
    frame.style.height = `${size.h}px`;
    frame.style.transform = `scale(${scale})`;
  }

  function paint() {
    raf = 0;
    if (pending === null) return;
    const html = pending;
    pending = null;
    if (html === last) return;
    last = html;
    badge.textContent = t("copilot.draftBadge");
    layout();
    frame.srcdoc = buildDraftDocument({ html, themeId: getTheme(), size: getSize() });
  }

  function hide() {
    window.clearTimeout(idleTimer);
    if (layer.hidden) return;
    layer.classList.remove("is-on");
    window.clearTimeout(fadeTimer);
    fadeTimer = window.setTimeout(() => {
      layer.hidden = true;
      frame.removeAttribute("srcdoc");
      last = "";
    }, 220);
  }

  function update(html) {
    if (!html) return;
    window.clearTimeout(fadeTimer);
    if (layer.hidden) {
      layer.hidden = false;
      // Next frame so the fade-in transition runs from the hidden state.
      window.requestAnimationFrame(() => layer.classList.add("is-on"));
    }
    pending = html;
    if (!raf) raf = window.requestAnimationFrame(paint);
    window.clearTimeout(idleTimer);
    idleTimer = window.setTimeout(hide, IDLE_MS);
  }

  if (typeof ResizeObserver === "function") new ResizeObserver(layout).observe(layer);
  document.addEventListener("slyde:draft", (event) => update(event.detail && event.detail.html));
  document.addEventListener("slyde:draft-end", hide);

  return { update, hide, isVisible: () => !layer.hidden };
}
