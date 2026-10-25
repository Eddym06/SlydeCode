import { createClient, readToken } from "./api.js";
import { apply as applyI18n, t } from "./i18n.js";
import { formatClock, paceState, readingSeconds } from "./teleprompter.js";

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
const speakerCloseBtn = document.getElementById("speaker-close-btn");
const hudSpeaker = document.getElementById("hud-speaker");
const hudExit = document.getElementById("hud-exit");
const notesFontDown = document.getElementById("notes-font-down");
const notesFontUp = document.getElementById("notes-font-up");
const notesPace = document.getElementById("notes-pace");

let view = null;
let index = 0;
const hashVal = parseInt(window.location.hash.replace(/[^0-9]/g, ""), 10);
if (!isNaN(hashVal) && hashVal >= 1) {
  index = hashVal - 1;
}
let sandboxOrigin = "";
// Slide documents require their own token; it is not the session token.
let sourceQuery = "";
let slideKey = "";
let hudTimer = null;
let pendingSteps = [];
let introPlayed = false;
let timerSeconds = 0;
let timerRunning = false;
let timerInterval = null;
let slideStartSeconds = 0;
let notesFontSize = 18;
let isSpeakerMode = new URLSearchParams(window.location.search).get("speaker") === "1";
let isBlackout = false;
let client = null;
let keepWindowedOnExit = false;

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
    updatePace();
  }, 1000);
}

function pauseTimer() {
  timerRunning = false;
  if (timerToggle) timerToggle.textContent = "▶";
  clearInterval(timerInterval);
}

function resetTimer() {
  pauseTimer();
  timerSeconds = 0;
  if (timerDisplay) timerDisplay.textContent = "00:00";
}

if (timerToggle) {
  timerToggle.addEventListener("click", () => {
    if (timerRunning) pauseTimer();
    else startTimer();
  });
}

if (timerReset) {
  timerReset.addEventListener("click", resetTimer);
}

function toggleSpeakerMode(force) {
  isSpeakerMode = typeof force === "boolean" ? force : !isSpeakerMode;
  if (speakerConsole) speakerConsole.hidden = !isSpeakerMode;
  if (isSpeakerMode && !timerRunning && timerSeconds === 0) {
    startTimer();
  }
  if (view) renderSpeakerConsole();
}

function renderSpeakerConsole() {
  if (!speakerConsole || speakerConsole.hidden || !view) return;
  const slides = view.presentation.slides;
  if (!slides.length) return;
  const slide = slides[index];  if (speakerTitle) speakerTitle.textContent = view.presentation.title || "SlydeCode";
  if (scCurrent) scCurrent.textContent = String(index + 1);
  if (scTotal) scTotal.textContent = String(slides.length);
  if (scCurrentFrame) {
    scCurrentFrame.src = `${sandboxOrigin}/s/${encodeURIComponent(slide.id)}?static=1${sourceQuery}`;
  }
  if (index + 1 < slides.length) {
    const nextSlide = slides[index + 1];
    if (scNextFrame) {
      scNextFrame.hidden = false;
      scNextFrame.src = `${sandboxOrigin}/s/${encodeURIComponent(nextSlide.id)}?static=1${sourceQuery}`;
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
  updatePace();
}

/** Reading-time estimate and pace for the current slide. */
function updatePace() {
  if (!notesPace) return;
  const slide = view && view.presentation.slides[index];
  const expected = readingSeconds(slide && slide.notes);
  if (!expected) {
    notesPace.dataset.state = "none";
    notesPace.textContent = "";
    return;
  }
  const elapsed = Math.max(0, timerSeconds - slideStartSeconds);
  const state = paceState(elapsed, expected);
  notesPace.dataset.state = state;
  notesPace.textContent = `${t("present.pace.estimate", {
    time: formatClock(expected),
  })} · ${t(`present.pace.${state}`)}`;
}

function applyView(next) {
  view = next;
  const slides = view.presentation.slides;
  if (!slides.length) {
    if (frameWrap) frameWrap.hidden = true;
    if (empty) empty.hidden = false;
    if (totalEl) totalEl.textContent = "0";
    if (currentEl) currentEl.textContent = "0";
    if (progress) progress.style.width = "0%";
    if (progressBar) progressBar.setAttribute("aria-valuenow", "0");
    return;
  }

  if (empty) empty.hidden = true;
  if (frameWrap) frameWrap.hidden = false;
  index = Math.min(index, slides.length - 1);

  const slide = slides[index];
  if (totalEl) totalEl.textContent = String(slides.length);
  if (currentEl) currentEl.textContent = String(index + 1);
  const percent = Math.round(((index + 1) / slides.length) * 100);
  if (progress) progress.style.width = `${percent}%`;
  if (progressBar) progressBar.setAttribute("aria-valuenow", String(percent));

  const key = `${slide.id}:${slide.code.length}:${slide.animation.length}`;
  if (key !== slideKey) {
    slideKey = key;
    slideStartSeconds = timerSeconds;
    pendingSteps = slide.animation.slice();
    introPlayed = false;
    if (frame) frame.src = `${sandboxOrigin}/s/${encodeURIComponent(slide.id)}?p=${index}${sourceQuery}`;
  }

  try {
    history.replaceState(null, "", `#${index + 1}`);
  } catch {}

  fit();
  renderSpeakerConsole();
}

function toggleBlackout() {
  isBlackout = !isBlackout;
  if (frameWrap) frameWrap.style.opacity = isBlackout ? "0" : "1";
}

function fit() {
  if (!view || !frameWrap) return;
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
  // One step per advance: the first plays as the slide appears, the rest wait
  // for the presenter's next click or key press.
  if (introPlayed) return;
  introPlayed = true;
  const step = pendingSteps.shift();
  if (step) {
    frame?.contentWindow?.postMessage({ __slyde: true, type: "animate", step }, "*");
  }
}

function advance() {
  if (pendingSteps.length > 0) {
    const step = pendingSteps.shift();
    frame?.contentWindow?.postMessage({ __slyde: true, type: "animate", step }, "*");
    return;
  }
  go(1);
}

function go(delta) {
  if (!view) return;
  const slides = view.presentation.slides;
  const next = Math.min(slides.length - 1, Math.max(0, index + delta));
  if (next === index) return;
  index = next;
  slideKey = "";
  applyView(view);
}

function jumpTo(targetIndex) {
  if (!view) return;
  const slides = view.presentation.slides;
  const next = Math.min(slides.length - 1, Math.max(0, targetIndex));
  if (next === index) return;
  index = next;
  slideKey = "";
  applyView(view);
}

function exitToStudio() {
  try {
    window.close();
  } catch {}
  window.location.href = `/?t=${encodeURIComponent(token)}`;
}

async function toggleFullscreen() {
  try {
    if (document.fullscreenElement) {
      keepWindowedOnExit = true;
      await document.exitFullscreen();
    } else {
      await document.documentElement.requestFullscreen();
    }
  } catch {
    keepWindowedOnExit = false;
    /* Fullscreen can be denied; presenting still works. */
  }
}

function showHud() {
  if (!hud) return;
  hud.dataset.visible = "true";
  if (hudTimer) window.clearTimeout(hudTimer);
  hudTimer = window.setTimeout(() => {
    if (hud) hud.dataset.visible = "false";
  }, 2600);
}

// Event Listeners
if (exitSpeaker) exitSpeaker.addEventListener("click", () => toggleSpeakerMode(false));
if (speakerCloseBtn) speakerCloseBtn.addEventListener("click", exitToStudio);
if (hudSpeaker) hudSpeaker.addEventListener("click", () => toggleSpeakerMode(true));
if (hudExit) hudExit.addEventListener("click", exitToStudio);

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

window.addEventListener("message", (event) => {
  const data = event.data;
  if (!data || data.__slyde !== true) return;
  if (data.type === "ready") {
    playPending();
    return;
  }
  if (data.type === "key") {
    // A focused slide document owns the keyboard; the sandbox runtime forwards
    // each key so navigation never dies after touching the slide.
    if (!frame || event.source !== frame.contentWindow) return;
    onKeydown(
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

if (frame) {
  frame.addEventListener("load", () => {
    frame.style.opacity = "1";
    window.setTimeout(playPending, 100);
  });
}

window.addEventListener("resize", () => {
  if (view) fit();
});

function onKeydown(event) {
  if (event.key === "ArrowRight" || event.key === " " || event.key === "PageDown" || event.key === "Enter") {
    event.preventDefault();
    advance();
  } else if (event.key === "ArrowLeft" || event.key === "PageUp" || event.key === "Backspace") {
    event.preventDefault();
    go(-1);
  } else if (event.key === "Home") {
    event.preventDefault();
    jumpTo(0);
  } else if (event.key === "End") {
    event.preventDefault();
    if (view) jumpTo(view.presentation.slides.length - 1);
  } else if (event.key.toLowerCase() === "b" || event.key === ".") {
    event.preventDefault();
    toggleBlackout();
  } else if (event.key.toLowerCase() === "s") {
    event.preventDefault();
    toggleSpeakerMode();
  } else if (event.key.toLowerCase() === "f") {
    event.preventDefault();
    toggleFullscreen();
  } else if (event.key === "Escape") {
    if (isBlackout) {
      toggleBlackout();
    } else if (isSpeakerMode) {
      toggleSpeakerMode(false);
    } else if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      exitToStudio();
    }
  }
}

document.addEventListener("keydown", onKeydown);

// Chrome consumes Escape to leave fullscreen instead of dispatching it, so an
// unexpected exit from fullscreen ends the presentation.
document.addEventListener("fullscreenchange", () => {
  if (document.fullscreenElement || keepWindowedOnExit) {
    keepWindowedOnExit = false;
    return;
  }
  exitToStudio();
});

document.addEventListener("click", (event) => {
  if (event.target.closest("#hud") || event.target.closest("#speaker-console")) return;
  if (event.clientX < window.innerWidth / 2) go(-1);
  else advance();
});

window.addEventListener("mousemove", showHud);
showHud();

async function start(sessionToken) {
  client = createClient(sessionToken);
  try {
    const payload = await client.getState();
    sandboxOrigin = payload.sandbox_origin;
    sourceQuery = payload.sandbox_token
      ? `&t=${encodeURIComponent(payload.sandbox_token)}`
      : "";
    applyView(payload.view);
    if (isSpeakerMode) toggleSpeakerMode(true);
    client.connect({
      onEvent: (e) => {
        if (e.type === "state_changed") applyView(e.view);
      },
      onStatus: () => {},
    });
  } catch (err) {
    console.error("Presentation start failed:", err);
    if (empty) empty.hidden = false;
  }
}

if (token) {
  start(token);
} else {
  if (empty) empty.hidden = false;
}
