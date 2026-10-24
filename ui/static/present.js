import { createClient, readToken } from "./api.js";
import { apply as applyI18n, t } from "./i18n.js";

const token = readToken();
const frame = document.getElementById("present-slide");
const frameWrap = document.getElementById("present-frame");
const empty = document.getElementById("present-empty");
const hud = document.getElementById("hud");
const currentEl = document.getElementById("current");
const totalEl = document.getElementById("total");
const progress = document.getElementById("progress-fill");
const progressBar = document.getElementById("progress");

const speakerConsole = document.getElementById("speaker-console");
const speakerTitle = document.getElementById("speaker-deck-title");
const scCurrent = document.getElementById("sc-current");
const scTotal = document.getElementById("sc-total");
const scCurrentFrame = document.getElementById("sc-current-frame");
const scNextFrame = document.getElementById("sc-next-frame");
const noNextHint = document.getElementById("no-next-hint");
const speakerNotesBody = document.getElementById("speaker-notes-body");
const wallClock = document.getElementById("wall-clock");
const timerDisplay = document.getElementById("timer-display");
const timerToggle = document.getElementById("timer-toggle");
const timerReset = document.getElementById("timer-reset");
const exitSpeaker = document.getElementById("exit-speaker");
const hudSpeaker = document.getElementById("hud-speaker");
const notesFontDown = document.getElementById("notes-font-down");
const notesFontUp = document.getElementById("notes-font-up");

let view = null;
let index = 0;
let sandboxOrigin = "";
let slideKey = "";
let hudTimer = null;
let pendingSteps = [];
let timerSeconds = 0;
let timerRunning = false;
let timerInterval = null;
let notesFontSize = 18;
let isSpeakerMode = new URLSearchParams(window.location.search).get("speaker") === "1";

applyI18n();

function updateClock() {
  if (wallClock) {
    const now = new Date();
    wallClock.textContent = now.toTimeString().slice(0, 8);
  }
}
setInterval(updateClock, 1000);
updateClock();

function formatTime(sec) {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function startTimer() {
  if (timerRunning) return;
  timerRunning = true;
  if (timerToggle) timerToggle.textContent = "⏸";
  timerInterval = setInterval(() => {
    timerSeconds += 1;
    if (timerDisplay) timerDisplay.textContent = formatTime(timerSeconds);
  }, 1000);
}

function pauseTimer() {
  timerRunning = false;
  if (timerToggle) timerToggle.textContent = "▶";
  clearInterval(timerInterval);
}

if (timerToggle) {
  timerToggle.addEventListener("click", () => {
    if (timerRunning) pauseTimer();
    else startTimer();
  });
}

if (timerReset) {
  timerReset.addEventListener("click", () => {
    pauseTimer();
    timerSeconds = 0;
    if (timerDisplay) timerDisplay.textContent = "00:00";
  });
}

function toggleSpeakerMode(force) {
  isSpeakerMode = typeof force === "boolean" ? force : !isSpeakerMode;
  if (speakerConsole) speakerConsole.hidden = !isSpeakerMode;
  if (isSpeakerMode && !timerRunning && timerSeconds === 0) {
    startTimer();
  }
  if (view) renderSpeakerConsole();
}

if (exitSpeaker) exitSpeaker.addEventListener("click", () => toggleSpeakerMode(false));
if (hudSpeaker) hudSpeaker.addEventListener("click", () => toggleSpeakerMode(true));

if (notesFontDown) {
  notesFontDown.addEventListener("click", () => {
    notesFontSize = Math.max(14, notesFontSize - 2);
    if (speakerNotesBody) speakerNotesBody.style.fontSize = `${notesFontSize}px`;
  });
}
if (notesFontUp) {
  notesFontUp.addEventListener("click", () => {
    notesFontSize = Math.min(32, notesFontSize + 2);
    if (speakerNotesBody) speakerNotesBody.style.fontSize = `${notesFontSize}px`;
  });
}

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
    if (isSpeakerMode) toggleSpeakerMode(true);
    client.connect({ onEvent: onEvent, onStatus: () => {} });
  } catch {
    empty.hidden = false;
  }

  function onEvent(event) {
    if (event.type === "state_changed") applyView(event.view);
  }

  function renderSpeakerConsole() {
    if (!speakerConsole || speakerConsole.hidden || !view) return;
    const slides = view.presentation.slides;
    if (!slides.length) return;
    const slide = slides[index];
    if (speakerTitle) speakerTitle.textContent = view.presentation.title || "SlydeCode";
    if (scCurrent) scCurrent.textContent = String(index + 1);
    if (scTotal) scTotal.textContent = String(slides.length);
    if (scCurrentFrame) {
      scCurrentFrame.src = `${sandboxOrigin}/s/${encodeURIComponent(slide.id)}?static=1`;
    }
    if (index + 1 < slides.length) {
      const nextSlide = slides[index + 1];
      if (scNextFrame) {
        scNextFrame.hidden = false;
        scNextFrame.src = `${sandboxOrigin}/s/${encodeURIComponent(nextSlide.id)}?static=1`;
      }
      if (noNextHint) noNextHint.hidden = true;
    } else {
      if (scNextFrame) scNextFrame.hidden = true;
      if (noNextHint) noNextHint.hidden = false;
    }
    if (speakerNotesBody) {
      if (slide.notes && slide.notes.trim()) {
        speakerNotesBody.textContent = slide.notes;
        speakerNotesBody.classList.remove("speaker-notes-empty");
      } else {
        speakerNotesBody.innerHTML = `<span class="speaker-notes-empty">(${t("present.noNotes")})</span>`;
      }
    }
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
      progressBar.setAttribute("aria-valuenow", "0");
      return;
    }

    empty.hidden = true;
    frameWrap.hidden = false;
    index = Math.min(index, slides.length - 1);

    const slide = slides[index];
    totalEl.textContent = String(slides.length);
    currentEl.textContent = String(index + 1);
    const percent = Math.round(((index + 1) / slides.length) * 100);
    progress.style.width = `${percent}%`;
    progressBar.setAttribute("aria-valuenow", String(percent));

    const key = `${slide.id}:${slide.code.length}:${slide.animation.length}`;
    if (key !== slideKey) {
      slideKey = key;
      pendingSteps = slide.animation.slice();
      frame.src = `${sandboxOrigin}/s/${encodeURIComponent(slide.id)}?p=${index}`;
    }

    fit();
    renderSpeakerConsole();
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
    } else if (event.key.toLowerCase() === "s") {
      event.preventDefault();
      toggleSpeakerMode();
    } else if (event.key.toLowerCase() === "f") {
      toggleFullscreen();
    } else if (event.key === "Escape") {
      if (isSpeakerMode) {
        toggleSpeakerMode(false);
      } else if (document.fullscreenElement) {
        document.exitFullscreen();
      } else {
        window.close();
      }
    }
  });

  document.addEventListener("click", (event) => {
    if (event.target.closest("#hud") || event.target.closest("#speaker-console")) return;
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
