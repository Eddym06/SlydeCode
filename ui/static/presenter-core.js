// Reveal state machine shared by the studio overlay and the standalone presenter.
//
// Both surfaces used to implement "which slide, which animation step, what does a
// key do" on their own, and they had already drifted once (the presenter flushed
// every step on load while the studio played one per advance). The DOM is
// different on each surface, so this module owns only the state and returns
// actions: the caller decides how to play a step or swap a frame.
//
// It is pure, which is what makes it unit-testable in `tests/ui/components.test.mjs`.

/**
 * @param {Array<{id: string, code?: string, animation?: Array<unknown>}>} slides
 */
export function createReveal(slides = []) {
  let deck = Array.isArray(slides) ? slides : [];
  let index = 0;
  let steps = [];
  let ready = false;
  let intro = false;

  const clamp = (value) =>
    Math.min(Math.max(0, Number.isFinite(value) ? Math.trunc(value) : 0), Math.max(0, deck.length - 1));

  const keyOf = (slide) =>
    slide ? `${slide.id}:${(slide.code || "").length}:${(slide.animation || []).length}` : "";

  function load(target) {
    index = clamp(target);
    const slide = deck[index] || null;
    steps = slide && Array.isArray(slide.animation) ? slide.animation.slice() : [];
    ready = false;
    intro = false;
    return { index, slide, key: keyOf(slide) };
  }

  const api = {
    get slides() {
      return deck;
    },
    get index() {
      return index;
    },
    get slide() {
      return deck[index] || null;
    },
    get key() {
      return keyOf(deck[index]);
    },
    /** Steps still queued for the current slide. */
    get pending() {
      return steps.length;
    },
    get ready() {
      return ready;
    },

    /**
     * Replaces the deck without moving when the current slide still exists.
     *
     * A deck that shrank below the cursor reloads the clamped slide, and an empty
     * deck clears the queue: keeping steps from a slide that is gone would make
     * the next advance "wait" forever.
     */
    setSlides(next) {
      deck = Array.isArray(next) ? next : [];
      if (!deck.length) {
        index = 0;
        steps = [];
        ready = false;
        intro = false;
        return;
      }
      if (index > deck.length - 1) load(deck.length - 1);
    },

    /** Loads a slide. `force` reloads even when the index did not change. */
    goTo(target, { force = false } = {}) {
      if (clamp(target) === index && !force) return null;
      return load(target);
    },

    delta(amount) {
      return api.goTo(index + amount);
    },

    /** Reloads the current index (the slide content changed under us). */
    reload() {
      return load(index);
    },

    /**
     * The frame holding the slide is listening. Until this is called, an advance
     * waits instead of spending a step nobody would see.
     */
    markReady() {
      ready = true;
    },

    /**
     * Plays the slide's first step once per load. Returns the step, or null when
     * there is nothing to play yet (or nothing to play at all).
     */
    takeIntro() {
      if (!ready || intro) return null;
      intro = true;
      return steps.length ? steps.shift() : null;
    },

    /**
     * One user advance: a queued step, a wait for the frame, the next slide, or
     * the end of the deck.
     */
    advance() {
      if (steps.length) {
        if (!ready) return { type: "wait" };
        return { type: "step", step: steps.shift() };
      }
      if (index + 1 < deck.length) {
        return { type: "slide", ...load(index + 1) };
      }
      return { type: "end" };
    },

    /** One user step back, clamped to the first slide. */
    back() {
      const target = api.goTo(index - 1);
      return target ? { type: "slide", ...target } : null;
    },

    /** First/last slide, used by Home and End. */
    first() {
      const target = api.goTo(0);
      return target ? { type: "slide", ...target } : null;
    },
    last() {
      const target = api.goTo(deck.length - 1);
      return target ? { type: "slide", ...target } : null;
    },
  };

  // A new controller starts on the first slide with its steps queued, so a
  // caller that forgets to call `goTo` still advances correctly.
  load(0);
  return api;
}

/**
 * Maps a keydown to a presenter action, or null when the key is not ours.
 *
 * Both surfaces share this so `B`, `S`, `F`, `Escape`, arrows and Space cannot
 * mean different things depending on where the presentation is running.
 */
export function keyToAction(event) {
  if (!event || typeof event.key !== "string") return null;
  const key = event.key;
  const lower = key.toLowerCase();
  if (key === "ArrowRight" || key === " " || key === "Spacebar" || key === "PageDown" || key === "Enter") {
    return "advance";
  }
  if (key === "ArrowLeft" || key === "PageUp" || key === "Backspace") return "back";
  if (key === "Home") return "first";
  if (key === "End") return "last";
  // A browser shortcut (Ctrl+P, Ctrl+C, Cmd+F…) is never a presenter command.
  if (event.ctrlKey || event.metaKey || event.altKey) return null;
  if (lower === "b" || key === ".") return "blackout";
  if (lower === "s") return "speaker";
  if (lower === "f") return "fullscreen";
  if (lower === "l") return "laser";
  if (lower === "p") return "pen";
  if (lower === "c") return "clearDraw";
  if (key === "Escape") return "escape";
  return null;
}

/** Progress of the current slide, as the ARIA value and the bar width. */
export function progressPercent(index, total) {
  if (!total || total < 1) return 0;
  return Math.round(((Math.min(index, total - 1) + 1) / total) * 100);
}
