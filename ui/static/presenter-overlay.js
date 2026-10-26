// In-studio presentation overlay.
//
// The overlay owns everything about presenting from inside the studio: which
// slide is showing, the reveal controller, the speaker console and its timer,
// the HUD, blackout, fullscreen handling and the laser/pen tools. It used to be a
// block of fifteen `let`s inside the shell's boot closure, which meant the
// shell's own state and the presenter's were one tangle. Now the presenter keeps
// its state to itself and reaches the shell only through the getters and the
// single callback below, so neither side can reassign the other's variables.
//
// The reveal state machine itself lives in `presenter-core.js` and is shared with
// the standalone `/present` page, so the two surfaces cannot disagree about when
// a step plays.

import { createReveal, keyToAction, progressPercent } from "./presenter-core.js";
import { mountPresenterDraw } from "./presenter-draw.js";
import { formatClock, paceState, readingSeconds } from "./teleprompter.js";

/**
 * @param {object} options
 * @param {(id: string) => HTMLElement | null} options.$  DOM lookup by id
 * @param {(key: string, vars?: object) => string} options.t
 * @param {(message: string) => void} options.toast
 * @param {() => object | null} options.getView  the studio's current read model
 * @param {() => object | null} options.getDeck  its `presentation`
 * @param {() => string} options.getSandboxOrigin
 * @param {() => string} options.getSourceQuery  `&t=…` suffix for slide URLs
 * @param {() => string | null} options.getSelectedId  slide selected in the storyboard
 * @param {(info: { slideId: string | null }) => void} options.onClosed
 *   called after the overlay closes, with the slide that was showing
 */
export function createPresenterOverlay({
  $,
  t,
  toast,
  getView,
  getDeck,
  getSandboxOrigin,
  getSourceQuery,
  getSelectedId,
  onClosed,
}) {
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
      if (!getView() || !presentEls.frameWrap) return;
      const ratio = getView().presentation.aspect_ratio.split(":").map(Number);
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
      if (!presentEls.speakerConsole || presentEls.speakerConsole.hidden || !getView()) return;
      const slides = getView().presentation.slides;
      if (!slides.length) return;
      const s = slides[presentIndex];
      if (presentEls.speakerTitle) presentEls.speakerTitle.textContent = getView().presentation.title || "SlydeCode";
      if (presentEls.scCurrent) presentEls.scCurrent.textContent = String(presentIndex + 1);
      if (presentEls.scTotal) presentEls.scTotal.textContent = String(slides.length);
      if (presentEls.scCurrentFrame) {
        presentEls.scCurrentFrame.src = `${getSandboxOrigin()}/s/${encodeURIComponent(s.id)}?static=1${getSourceQuery()}`;
      }
      if (presentIndex + 1 < slides.length) {
        const nextSlide = slides[presentIndex + 1];
        if (presentEls.scNextFrame) {
          presentEls.scNextFrame.hidden = false;
          presentEls.scNextFrame.src = `${getSandboxOrigin()}/s/${encodeURIComponent(nextSlide.id)}?static=1${getSourceQuery()}`;
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
      const current = getView() && getView().presentation.slides[presentIndex];
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
      if (!getView() || !isPresenting) return;
      const slides = getView().presentation.slides;
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
          presentEls.frame.src = `${getSandboxOrigin()}/s/${encodeURIComponent(s.id)}?p=${presentIndex}${getSourceQuery()}`;
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
      if (!getView()) return;
      const action = presentReveal.delta(delta);
      if (action) showPresentSlide(action);
    }

    function jumpPresentIndex(target) {
      if (!getView()) return;
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
      if (getView()) renderPresentSpeakerConsole();
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
          if (getView()) jumpPresentIndex(getView().presentation.slides.length - 1);
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
      const d = getDeck();
      if (!d || !d.slides.length) {
        toast(t("present.empty"));
        return;
      }
      isPresenting = true;
      presentBlackout = false;
      presentReveal.setSlides(d.slides);
      presentIndex = Math.max(0, d.slides.findIndex((s) => s.id === getSelectedId()));
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

      const current = getView();
      const slide = current && current.presentation.slides[presentIndex];
      onClosed({ slideId: slide ? slide.id : null });
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

  return {
    /** Opens the overlay on the selected slide; `speaker` starts the console too. */
    open: openPresentationInStudio,
    close: closePresentation,
    isPresenting: () => isPresenting,
    /** Wires the HUD, console and slide-frame events. Call once at boot. */
    bindControls: initPresenterListeners,
  };
}
