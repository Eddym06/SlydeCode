// Pure timing model for the AI activity chip.
//
// The server announces agent calls, but a mutation is nearly instantaneous:
// without a minimum hold the chip would flash for a few milliseconds. This
// module turns structured events into a legible sequence:
//
//   active (min 1s) -> done (2.6s) -> idle
//
// The done state persists long enough to be read as confirmation; there is no
// inferred "still working" tail, because after the command the agent is done.
// `thinking` is reserved for a future explicit generation channel.
//
// It is deliberately DOM-free so the timings can be tested with the Node test
// runner instead of a browser clock.

export const TIMING = Object.freeze({
  minPhase: 1000,
  doneHold: 2600,
  thinkHold: 2400,
  errorHold: 4200,
});

const ACTIVE_PHASES = new Set(["writing", "adjusting", "animating"]);
const KNOWN_PHASES = new Set([
  "thinking",
  "writing",
  "adjusting",
  "animating",
  "done",
  "error",
]);

export function idleActivity() {
  return {
    phase: "idle",
    action: null,
    slideId: null,
    slideIndex: null,
    activeUntil: 0,
    doneUntil: 0,
    hideAt: 0,
    pendingDone: false,
  };
}

function slideIdOf(event) {
  return typeof event.slide_id === "string" ? event.slide_id : null;
}

function slideIndexOf(event) {
  return Number.isInteger(event.slide_index) ? event.slide_index : null;
}

export function receiveActivity(current, event, now = Date.now()) {
  if (!event || !KNOWN_PHASES.has(event.phase)) return current;

  const slideId = slideIdOf(event);
  const slideIndex = slideIndexOf(event);

  if (ACTIVE_PHASES.has(event.phase)) {
    const activeUntil = now + TIMING.minPhase;
    return {
      phase: event.phase,
      action: event.action || null,
      slideId,
      slideIndex,
      activeUntil,
      doneUntil: 0,
      hideAt: activeUntil,
      pendingDone: false,
    };
  }

  if (event.phase === "done") {
    const activeUntil = current.activeUntil || 0;
    const doneUntil = Math.max(now, activeUntil) + TIMING.doneHold;
    const next = {
      ...current,
      action: event.action || current.action,
      slideId: slideId || current.slideId,
      slideIndex: slideIndex ?? current.slideIndex,
      doneUntil,
      hideAt: doneUntil,
    };

    // Keep the active phase visible for its minimum before switching.
    if (current.phase === "idle" || now >= activeUntil) {
      return {
        ...next,
        phase: "done",
        activeUntil: 0,
        pendingDone: false,
      };
    }
    return { ...next, pendingDone: true };
  }

  if (event.phase === "error") {
    return {
      ...current,
      phase: "error",
      action: event.action || current.action,
      slideId: slideId || current.slideId,
      slideIndex: slideIndex ?? current.slideIndex,
      activeUntil: 0,
      doneUntil: 0,
      hideAt: now + TIMING.errorHold,
      pendingDone: false,
    };
  }

  // `thinking` is reserved for a future explicit generation channel.
  const hideAt = Math.max(current.hideAt || 0, now + TIMING.thinkHold);
  return {
    ...current,
    phase: "thinking",
    action: event.action || current.action,
    slideId: slideId || current.slideId,
    slideIndex: slideIndex ?? current.slideIndex,
    activeUntil: 0,
    doneUntil: 0,
    hideAt,
    pendingDone: false,
  };
}

export function tickActivity(current, now = Date.now()) {
  if (!current || current.phase === "idle") return current || idleActivity();

  if (ACTIVE_PHASES.has(current.phase)) {
    if (now < current.activeUntil) return current;
    if (current.pendingDone) {
      return {
        ...current,
        phase: "done",
        activeUntil: 0,
        pendingDone: false,
      };
    }
    return idleActivity();
  }

  if (current.phase === "done") {
    if (now < current.doneUntil) return current;
    return idleActivity();
  }

  if (now < current.hideAt) return current;
  return idleActivity();
}

export function nextTransition(current, now = Date.now()) {
  if (!current || current.phase === "idle") return null;
  let at = current.hideAt;
  if (ACTIVE_PHASES.has(current.phase)) at = current.activeUntil;
  else if (current.phase === "done") at = current.doneUntil;
  return at && at > now ? at : null;
}
