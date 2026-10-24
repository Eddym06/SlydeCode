import { createClient, readToken } from "./api.js";
import { apply as applyI18n } from "./i18n.js";

const token = readToken();
const frame = document.getElementById("present-slide");
const frameWrap = document.getElementById("present-frame");
const empty = document.getElementById("present-empty");
const hud = document.getElementById("hud");
const currentEl = document.getElementById("current");
const totalEl = document.getElementById("total");
const progress = document.getElementById("progress-fill");

let view = null;
let index = 0;
let sandboxOrigin = "";
let slideKey = "";
let hudTimer = null;
let pendingSteps = [];

applyI18n();

if (token) {
  start(token);
} else {
  empty.hidden = false;
}

async function start(sessionToken) {
  const client = createClient(sessionToken);
  try {
    const payload = await client.getState();
    sandboxOrigin = payload.sandbox_origin;
    applyView(payload.view);
    client.connect({ onEvent: onEvent, onStatus: () => {} });
  } catch {
    empty.hidden = false;
  }

  function onEvent(event) {
    if (event.type === "state_changed") applyView(event.view);
  }

  function applyView(next) {
    view = next;
    const slides = view.presentation.slides;
    if (!slides.length) {
      frameWrap.hidden = true;
      empty.hidden = false;
      totalEl.textContent = "0";
      currentEl.textContent = "0";
      progress.style.width = "0%";
      return;
    }

    empty.hidden = true;
    frameWrap.hidden = false;
    index = Math.min(index, slides.length - 1);

    const slide = slides[index];
    totalEl.textContent = String(slides.length);
    currentEl.textContent = String(index + 1);
    progress.style.width = `${((index + 1) / slides.length) * 100}%`;

    const key = `${slide.id}:${slide.code.length}:${slide.animation.length}`;
    if (key !== slideKey) {
      slideKey = key;
      pendingSteps = slide.animation.slice();
      frame.src = `${sandboxOrigin}/s/${encodeURIComponent(slide.id)}?p=${index}`;
    }

    fit();
  }

  function fit() {
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
    frameWrap.style.width = `${Math.round(width)}px`;
    frameWrap.style.height = `${Math.round(height)}px`;
  }

  function playPending() {
    const steps = pendingSteps;
    pendingSteps = [];
    steps.forEach((step) => {
      frame.contentWindow?.postMessage({ __slyde: true, type: "animate", step }, "*");
    });
  }

  window.addEventListener("message", (event) => {
    const data = event.data;
    if (!data || data.__slyde !== true) return;
    if (data.type === "ready") playPending();
  });

  frame.addEventListener("load", () => {
    // Fallback for the ready handshake; posting twice is harmless because each
    // animation is a fresh WAAPI call on a visible element.
    window.setTimeout(playPending, 120);
  });

  function go(delta) {
    if (!view) return;
    const slides = view.presentation.slides;
    const next = Math.min(slides.length - 1, Math.max(0, index + delta));
    if (next === index) return;
    index = next;
    slideKey = "";
    applyView(view);
  }

  window.addEventListener("resize", () => {
    if (view) fit();
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "ArrowRight" || event.key === " " || event.key === "PageDown" || event.key === "Enter") {
      event.preventDefault();
      go(1);
    } else if (event.key === "ArrowLeft" || event.key === "PageUp" || event.key === "Backspace") {
      event.preventDefault();
      go(-1);
    } else if (event.key.toLowerCase() === "f") {
      toggleFullscreen();
    } else if (event.key === "Escape") {
      if (document.fullscreenElement) document.exitFullscreen();
      else window.close();
    }
  });

  document.addEventListener("click", (event) => {
    if (event.target.closest("#hud")) return;
    if (event.clientX < window.innerWidth / 2) go(-1);
    else go(1);
  });

  async function toggleFullscreen() {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await document.documentElement.requestFullscreen();
    } catch {
      /* Fullscreen can be denied; presenting still works. */
    }
  }

  function showHud() {
    hud.dataset.visible = "true";
    if (hudTimer) window.clearTimeout(hudTimer);
    hudTimer = window.setTimeout(() => {
      hud.dataset.visible = "false";
    }, 2600);
  }

  window.addEventListener("mousemove", showHud);
  showHud();
}
