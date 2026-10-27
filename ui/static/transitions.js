// Slide-to-slide transitions for the presenter.
//
// Every slide is its own document inside one iframe, so a transition is two
// short animations of that iframe: the old slide leaves, the new document loads
// while it is hidden, the new slide arrives. Only `opacity` and `transform` are
// animated, so it stays on the compositor and costs no layout.

export const TRANSITION_KINDS = ["none", "fade", "slide", "push", "zoom", "flip"];
export const DEFAULT_TRANSITION_MS = 450;

/**
 * Keyframes for the two halves of a transition.
 * @param {string} kind one of TRANSITION_KINDS
 * @param {1 | -1} direction 1 going forward, -1 going back
 * @returns {{leave: Keyframe[], enter: Keyframe[]} | null} null for a cut
 */
export function keyframesFor(kind, direction = 1) {
  const sign = direction < 0 ? -1 : 1;
  switch (kind) {
    case "fade":
      return {
        leave: [{ opacity: 1 }, { opacity: 0 }],
        enter: [{ opacity: 0 }, { opacity: 1 }],
      };
    case "slide":
      return {
        leave: [
          { opacity: 1, transform: "translateX(0)" },
          { opacity: 0, transform: `translateX(${-6 * sign}%)` },
        ],
        enter: [
          { opacity: 0, transform: `translateX(${6 * sign}%)` },
          { opacity: 1, transform: "translateX(0)" },
        ],
      };
    case "push":
      return {
        leave: [{ transform: "translateX(0)" }, { transform: `translateX(${-100 * sign}%)` }],
        enter: [{ transform: `translateX(${100 * sign}%)` }, { transform: "translateX(0)" }],
      };
    case "zoom":
      return {
        leave: [
          { opacity: 1, transform: "scale(1)" },
          { opacity: 0, transform: "scale(1.08)" },
        ],
        enter: [
          { opacity: 0, transform: "scale(0.92)" },
          { opacity: 1, transform: "scale(1)" },
        ],
      };
    case "flip":
      return {
        leave: [
          { opacity: 1, transform: "perspective(1200px) rotateY(0deg)" },
          { opacity: 0, transform: `perspective(1200px) rotateY(${-70 * sign}deg)` },
        ],
        enter: [
          { opacity: 0, transform: `perspective(1200px) rotateY(${70 * sign}deg)` },
          { opacity: 1, transform: "perspective(1200px) rotateY(0deg)" },
        ],
      };
    default:
      return null;
  }
}

/** The transition declared on a slide, or null for a cut. */
export function transitionOf(slide) {
  const transition = slide && slide.transition;
  if (!transition || !TRANSITION_KINDS.includes(transition.kind) || transition.kind === "none") return null;
  return { kind: transition.kind, duration: Math.min(2000, Math.max(120, transition.duration_ms || DEFAULT_TRANSITION_MS)) };
}

function reducedMotion() {
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

/**
 * @param {HTMLIFrameElement} frame the presenter's iframe
 * @returns {{go: (swap: () => void, transition: object | null, direction: 1 | -1) => Promise<void>}}
 */
export function createTransitioner(frame) {
  let token = 0;

  function waitForLoad() {
    return new Promise((resolve) => {
      const done = () => {
        frame.removeEventListener("load", done);
        window.clearTimeout(timer);
        resolve();
      };
      const timer = window.setTimeout(done, 1500);
      frame.addEventListener("load", done, { once: true });
    });
  }

  return {
    /**
     * Runs `swap` (which points the frame at the next slide) wrapped in the
     * transition. Rapid advances cancel the previous one, so the presenter never
     * waits on an animation the audience no longer needs.
     */
    async go(swap, transition, direction = 1) {
      const mine = ++token;
      frame.getAnimations().forEach((animation) => animation.cancel());
      const frames = transition && !reducedMotion() ? keyframesFor(transition.kind, direction) : null;
      if (!frames || typeof frame.animate !== "function") {
        swap();
        return;
      }
      const half = Math.round(transition.duration * 0.42);
      const options = { easing: "cubic-bezier(0.4, 0, 0.2, 1)", fill: "forwards" };
      try {
        await frame.animate(frames.leave, { ...options, duration: half }).finished;
        if (mine !== token) return;
        // Stay on the last frame of the exit while the next document loads.
        const loaded = waitForLoad();
        swap();
        await loaded;
        if (mine !== token) return;
        frame.getAnimations().forEach((animation) => animation.cancel());
        await frame.animate(frames.enter, {
          ...options,
          duration: transition.duration - half,
        }).finished;
        if (mine === token) frame.getAnimations().forEach((animation) => animation.cancel());
      } catch {
        // A cancelled animation rejects: the newer transition owns the frame now.
        if (mine === token) swap();
      }
    },
  };
}
